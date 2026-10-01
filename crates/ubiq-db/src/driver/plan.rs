//! The portable query plan [`Connection::explain`](super::Connection::explain) returns, and the
//! parsers that turn each engine's output into it: Postgres `EXPLAIN (FORMAT JSON)`, MySQL
//! `EXPLAIN FORMAT=TREE` / `EXPLAIN ANALYZE` text and the tabular `EXPLAIN`, SQLite
//! `EXPLAIN QUERY PLAN` rows, SQL Server showplan XML (a hand scan of `<RelOp>` elements — no XML
//! crate).
//!
//! Units: costs are the engine's own (not comparable across engines), times are milliseconds,
//! actual rows and times are totals over every loop of the node.

use crate::plan::PlanNode;
use serde_json::Value as Json;

/// One top node as the root; several under a synthetic `label` node.
fn root_of(mut roots: Vec<PlanNode>, label: &str) -> PlanNode {
    if roots.len() == 1 {
        return roots.remove(0);
    }
    PlanNode {
        children: roots,
        ..PlanNode::new(label)
    }
}

fn join_detail(parts: Vec<String>) -> Option<String> {
    (!parts.is_empty()).then(|| parts.join("; "))
}

// ---------------------------------------------------------------- Postgres

/// `EXPLAIN (FORMAT JSON[, ANALYZE])` output: `[{"Plan": {…}, …}]`.
pub(super) fn from_pg_json(raw: &str) -> Result<PlanNode, String> {
    let v: Json = serde_json::from_str(raw).map_err(|e| format!("plan is not JSON: {e}"))?;
    let roots: Vec<PlanNode> = v
        .as_array()
        .ok_or("plan is not a JSON array")?
        .iter()
        .filter_map(|q| q.get("Plan"))
        .map(pg_node)
        .collect();
    if roots.is_empty() {
        return Err("no plan in the EXPLAIN output".into());
    }
    Ok(root_of(roots, "Query"))
}

fn pg_node(n: &Json) -> PlanNode {
    let s = |k: &str| n.get(k).and_then(Json::as_str);
    let f = |k: &str| n.get(k).and_then(Json::as_f64);
    let mut parts = Vec::new();
    if let Some(rel) = s("Relation Name") {
        let mut on = match s("Schema") {
            Some(sc) => format!("on {sc}.{rel}"),
            None => format!("on {rel}"),
        };
        if let Some(a) = s("Alias").filter(|a| *a != rel) {
            on.push(' ');
            on.push_str(a);
        }
        parts.push(on);
    } else if let Some(x) = s("CTE Name").or_else(|| s("Function Name")) {
        parts.push(format!("on {x}"));
    }
    if let Some(i) = s("Index Name") {
        parts.push(format!("using {i}"));
    }
    if let Some(j) = s("Join Type") {
        parts.push(format!("{j} join"));
    }
    if let Some(st) = s("Strategy") {
        parts.push(st.to_string());
    }
    for k in [
        "Hash Cond",
        "Merge Cond",
        "Index Cond",
        "Recheck Cond",
        "Join Filter",
        "Filter",
    ] {
        if let Some(c) = s(k) {
            parts.push(format!("{k}: {c}"));
        }
    }
    if let Some(keys) = n.get("Sort Key").and_then(Json::as_array) {
        let keys: Vec<&str> = keys.iter().filter_map(Json::as_str).collect();
        parts.push(format!("Sort Key: {}", keys.join(", ")));
    }
    let loops = f("Actual Loops").unwrap_or(1.0);
    PlanNode {
        label: s("Node Type").unwrap_or("?").to_string(),
        detail: join_detail(parts),
        est_rows: f("Plan Rows"),
        est_cost: f("Total Cost"),
        actual_rows: f("Actual Rows").map(|r| r * loops),
        actual_ms: f("Actual Total Time").map(|t| t * loops),
        children: n
            .get("Plans")
            .and_then(Json::as_array)
            .map(|a| a.iter().map(pg_node).collect())
            .unwrap_or_default(),
    }
}

// ---------------------------------------------------------------- MySQL

/// `EXPLAIN FORMAT=TREE` / `EXPLAIN ANALYZE` text: one `-> operator  (cost=… rows=…)
/// (actual time=a..b rows=r loops=l)` line per node, nesting by indentation.
pub(super) fn from_mysql_tree(raw: &str) -> PlanNode {
    fn attach(stack: &mut [(usize, PlanNode)], roots: &mut Vec<PlanNode>, n: PlanNode) {
        match stack.last_mut() {
            Some((_, parent)) => parent.children.push(n),
            None => roots.push(n),
        }
    }
    let mut roots = Vec::new();
    let mut stack: Vec<(usize, PlanNode)> = Vec::new();
    for line in raw.lines() {
        let trimmed = line.trim_start();
        let Some(text) = trimmed.strip_prefix("->") else {
            // a label that wrapped onto the next line
            if let Some((_, n)) = stack.last_mut().filter(|_| !trimmed.is_empty()) {
                n.label.push(' ');
                n.label.push_str(trimmed.trim_end());
            }
            continue;
        };
        let indent = line.len() - trimmed.len();
        while stack.last().is_some_and(|(d, _)| *d >= indent) {
            let (_, n) = stack.pop().expect("checked");
            attach(&mut stack, &mut roots, n);
        }
        stack.push((indent, mysql_line(text.trim())));
    }
    while let Some((_, n)) = stack.pop() {
        attach(&mut stack, &mut roots, n);
    }
    root_of(roots, "Query")
}

fn mysql_line(text: &str) -> PlanNode {
    let cut = ["(cost=", "(actual time=", "(never executed)", "(rows="]
        .iter()
        .filter_map(|m| text.find(m))
        .min()
        .unwrap_or(text.len());
    let (head, figures) = text.split_at(cut);
    let head = head.trim_end();
    let (label, detail) = match head.split_once(": ") {
        Some((l, d)) => (l.to_string(), Some(d.to_string())),
        None => (head.to_string(), None),
    };
    let group = |open: &str| {
        figures
            .find(open)
            .map(|i| &figures[i + open.len()..])
            .map(|g| g.split(')').next().unwrap_or(g))
    };
    let est = group("(cost=")
        .map(|g| format!("cost={g}"))
        .or_else(|| group("(rows=").map(|g| format!("rows={g}")));
    let actual = group("(actual time=").map(|g| format!("time={g}"));
    let loops = actual.as_deref().and_then(|a| num_after(a, "loops="));
    let loops = loops.unwrap_or(1.0);
    PlanNode {
        label,
        detail,
        est_rows: est.as_deref().and_then(|e| num_after(e, "rows=")),
        est_cost: est.as_deref().and_then(|e| num_after(e, "cost=")),
        actual_rows: actual
            .as_deref()
            .and_then(|a| num_after(a, "rows="))
            .map(|r| r * loops),
        actual_ms: actual
            .as_deref()
            .and_then(|a| a.strip_prefix("time="))
            .and_then(|t| t.split_whitespace().next())
            .and_then(|t| t.rsplit("..").next())
            .and_then(|t| t.parse::<f64>().ok())
            .map(|t| t * loops),
        children: Vec::new(),
    }
}

/// The number right after `key` (`rows=1.5e3` → 1500).
fn num_after(s: &str, key: &str) -> Option<f64> {
    let at = s
        .match_indices(key)
        .find(|(i, _)| *i == 0 || !s.as_bytes()[i - 1].is_ascii_alphanumeric())?
        .0;
    let rest = &s[at + key.len()..];
    let end = rest
        .find(|c: char| !(c.is_ascii_digit() || matches!(c, '.' | 'e' | 'E' | '+' | '-')))
        .unwrap_or(rest.len());
    rest[..end].parse().ok()
}

/// Tabular `EXPLAIN` (MariaDB, MySQL before 8.0.16; MariaDB's `ANALYZE` adds `r_rows`): one
/// child per row under a `Query` root.
/// `columns` are the result's column names; cells are text or NULL.
pub(super) fn from_mysql_table(columns: &[String], rows: &[Vec<Option<String>>]) -> PlanNode {
    let idx = |n: &str| columns.iter().position(|c| c.eq_ignore_ascii_case(n));
    let get = |row: &[Option<String>], n: &str| {
        idx(n)
            .and_then(|i| row.get(i).cloned().flatten())
            .filter(|v| !v.is_empty())
    };
    let children = rows
        .iter()
        .map(|r| {
            let label = [get(r, "select_type"), get(r, "table")]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(" ");
            let mut parts = Vec::new();
            for k in ["type", "key", "ref"] {
                if let Some(v) = get(r, k) {
                    parts.push(format!("{k}={v}"));
                }
            }
            if let Some(x) = get(r, "Extra") {
                parts.push(x);
            }
            PlanNode {
                label: if label.is_empty() { "?".into() } else { label },
                detail: join_detail(parts),
                est_rows: get(r, "rows").and_then(|v| v.parse().ok()),
                // MariaDB `ANALYZE` / `EXPLAIN ANALYZE`
                actual_rows: get(r, "r_rows").and_then(|v| v.parse().ok()),
                ..PlanNode::default()
            }
        })
        .collect();
    PlanNode {
        children,
        ..PlanNode::new("Query")
    }
}

// ---------------------------------------------------------------- SQLite

/// `EXPLAIN QUERY PLAN` rows as `(id, parent, detail)`: a tree under a `QUERY PLAN` root.
pub(super) fn from_sqlite_eqp(rows: &[(i64, i64, String)]) -> PlanNode {
    fn under(parent: i64, rows: &[(i64, i64, String)], depth: usize) -> Vec<PlanNode> {
        if depth > 64 {
            return Vec::new();
        }
        rows.iter()
            .filter(|r| r.1 == parent && r.0 != parent)
            .map(|(id, _, d)| {
                let (label, detail) = match d.find(" USING ") {
                    Some(i) => (d[..i].to_string(), Some(d[i + 1..].to_string())),
                    None => (d.clone(), None),
                };
                PlanNode {
                    label,
                    detail,
                    children: under(*id, rows, depth + 1),
                    ..PlanNode::default()
                }
            })
            .collect()
    }
    PlanNode {
        children: under(0, rows, 0),
        ..PlanNode::new("QUERY PLAN")
    }
}

// ---------------------------------------------------------------- SQL Server

/// Showplan XML (`SET SHOWPLAN_XML` / `SET STATISTICS XML`), one or more documents: every
/// `<RelOp>` is a node, nested as in the XML; the first `<Object>` inside it names what it reads;
/// `<RunTimeCountersPerThread>` gives the actual figures (rows summed, time the slowest thread).
pub(super) fn from_mssql_xml(raw: &str) -> Result<PlanNode, String> {
    let mut roots = Vec::new();
    // (node, it already has its object)
    let mut stack: Vec<(PlanNode, bool)> = Vec::new();
    let mut statements = Vec::new();
    let mut any = false;
    for t in xml_tags(raw) {
        any = true;
        match t.name {
            "RelOp" if t.start => {
                let physical = attr(t.attrs, "PhysicalOp").unwrap_or_else(|| "?".into());
                let logical = attr(t.attrs, "LogicalOp").filter(|l| *l != physical);
                let node = PlanNode {
                    label: physical,
                    detail: logical,
                    est_rows: attr(t.attrs, "EstimateRows").and_then(|v| v.parse().ok()),
                    est_cost: attr(t.attrs, "EstimatedTotalSubtreeCost")
                        .and_then(|v| v.parse().ok()),
                    ..PlanNode::default()
                };
                if t.end {
                    attach_xml(&mut stack, &mut roots, node);
                } else {
                    stack.push((node, false));
                }
            }
            "RelOp" if t.end => {
                if let Some((n, _)) = stack.pop() {
                    attach_xml(&mut stack, &mut roots, n);
                }
            }
            "Object" if t.start => {
                if let Some((n, seen @ false)) = stack.last_mut() {
                    *seen = true;
                    let part = |k: &str| {
                        attr(t.attrs, k)
                            .map(|v| v.trim_matches(|c| c == '[' || c == ']').to_string())
                    };
                    let table = [part("Schema"), part("Table")]
                        .into_iter()
                        .flatten()
                        .collect::<Vec<_>>()
                        .join(".");
                    let mut on = Vec::new();
                    if !table.is_empty() {
                        on.push(format!("on {table}"));
                    }
                    if let Some(i) = part("Index") {
                        on.push(format!("index {i}"));
                    }
                    let mut parts: Vec<String> = n.detail.take().into_iter().collect();
                    if !on.is_empty() {
                        parts.push(on.join(" "));
                    }
                    n.detail = join_detail(parts);
                }
            }
            "RunTimeCountersPerThread" if t.start => {
                if let Some((n, _)) = stack.last_mut() {
                    if let Some(r) = attr(t.attrs, "ActualRows").and_then(|v| v.parse::<f64>().ok())
                    {
                        n.actual_rows = Some(n.actual_rows.unwrap_or(0.0) + r);
                    }
                    if let Some(ms) =
                        attr(t.attrs, "ActualElapsedms").and_then(|v| v.parse::<f64>().ok())
                    {
                        n.actual_ms = Some(n.actual_ms.map_or(ms, |m: f64| m.max(ms)));
                    }
                }
            }
            "StmtSimple" if t.start => {
                statements
                    .push(attr(t.attrs, "StatementType").unwrap_or_else(|| "Statement".into()));
            }
            _ => {}
        }
    }
    if !any {
        return Err("the showplan is not XML".into());
    }
    while let Some((n, _)) = stack.pop() {
        attach_xml(&mut stack, &mut roots, n);
    }
    if roots.is_empty() {
        // statements without a plan of their own (SET, DECLARE, …)
        return Ok(root_of(
            statements.into_iter().map(PlanNode::new).collect(),
            "Batch",
        ));
    }
    Ok(root_of(roots, "Batch"))
}

fn attach_xml(stack: &mut [(PlanNode, bool)], roots: &mut Vec<PlanNode>, n: PlanNode) {
    match stack.last_mut() {
        Some((parent, _)) => parent.children.push(n),
        None => roots.push(n),
    }
}

/// One XML tag: `start` for `<a …>` and `<a …/>`, `end` for `</a>` and `<a …/>`.
struct Tag<'a> {
    /// Local name, namespace prefix dropped.
    name: &'a str,
    attrs: &'a str,
    start: bool,
    end: bool,
}

/// Tags of `s` in order; text, comments, CDATA, declarations and processing instructions are
/// skipped. Quoted attribute values may hold `>`.
fn xml_tags(s: &str) -> impl Iterator<Item = Tag<'_>> {
    let mut i = 0;
    std::iter::from_fn(move || {
        loop {
            let lt = i + s[i..].find('<')?;
            let rest = &s[lt..];
            let skip_to = |close: &str| rest.find(close).map(|e| lt + e + close.len());
            if rest.starts_with("<?") {
                i = skip_to("?>")?;
                continue;
            }
            if rest.starts_with("<!--") {
                i = skip_to("-->")?;
                continue;
            }
            if rest.starts_with("<![CDATA[") {
                i = skip_to("]]>")?;
                continue;
            }
            if rest.starts_with("<!") {
                i = skip_to(">")?;
                continue;
            }
            // find the closing `>` outside quotes
            let mut quote = None;
            let mut gt = None;
            for (j, c) in rest.char_indices().skip(1) {
                match (quote, c) {
                    (None, '"' | '\'') => quote = Some(c),
                    (Some(q), c) if c == q => quote = None,
                    (None, '>') => {
                        gt = Some(j);
                        break;
                    }
                    _ => {}
                }
            }
            let gt = gt?;
            i = lt + gt + 1;
            let inner = &rest[1..gt];
            let (closing, inner) = match inner.strip_prefix('/') {
                Some(x) => (true, x),
                None => (false, inner),
            };
            let (selfclosing, inner) = match inner.strip_suffix('/') {
                Some(x) => (true, x),
                None => (false, inner),
            };
            let name_end = inner
                .find(|c: char| c.is_whitespace())
                .unwrap_or(inner.len());
            let qname = &inner[..name_end];
            let name = qname.rsplit(':').next().unwrap_or(qname);
            return Some(Tag {
                name,
                attrs: &inner[name_end..],
                start: !closing,
                end: closing || selfclosing,
            });
        }
    })
}

/// The value of attribute `name` (exact, case-sensitive), entities decoded.
fn attr(attrs: &str, name: &str) -> Option<String> {
    let mut rest = attrs;
    loop {
        rest = rest.trim_start();
        let eq = rest.find('=')?;
        let key = rest[..eq].trim();
        let after = rest[eq + 1..].trim_start();
        let q = after.chars().next().filter(|c| *c == '"' || *c == '\'')?;
        let body = &after[1..];
        let close = body.find(q)?;
        if key == name {
            return Some(decode_entities(&body[..close]));
        }
        rest = &body[close + 1..];
    }
}

fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let tail = &rest[amp..];
        let Some(semi) = tail.find(';').filter(|i| *i <= 10) else {
            out.push('&');
            rest = &tail[1..];
            continue;
        };
        let ent = &tail[1..semi];
        let ch = match ent {
            "lt" => Some('<'),
            "gt" => Some('>'),
            "amp" => Some('&'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            e if e.starts_with("#x") || e.starts_with("#X") => u32::from_str_radix(&e[2..], 16)
                .ok()
                .and_then(char::from_u32),
            e if e.starts_with('#') => e[1..].parse().ok().and_then(char::from_u32),
            _ => None,
        };
        match ch {
            Some(c) => {
                out.push(c);
                rest = &tail[semi + 1..];
            }
            None => {
                out.push('&');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const PG: &str = r#"[
      {
        "Plan": {
          "Node Type": "Hash Join", "Parallel Aware": false, "Join Type": "Inner",
          "Startup Cost": 1.07, "Total Cost": 2.21, "Plan Rows": 3, "Plan Width": 72,
          "Actual Startup Time": 0.031, "Actual Total Time": 0.036, "Actual Rows": 3,
          "Actual Loops": 1, "Hash Cond": "(o.person_id = p.id)",
          "Plans": [
            {
              "Node Type": "Seq Scan", "Parent Relationship": "Outer",
              "Relation Name": "orders", "Schema": "public", "Alias": "o",
              "Startup Cost": 0.00, "Total Cost": 1.03, "Plan Rows": 3,
              "Actual Total Time": 0.005, "Actual Rows": 3, "Actual Loops": 1
            },
            {
              "Node Type": "Hash", "Parent Relationship": "Inner", "Total Cost": 1.03,
              "Plan Rows": 3, "Actual Total Time": 0.010, "Actual Rows": 3, "Actual Loops": 1,
              "Plans": [
                {
                  "Node Type": "Index Scan", "Relation Name": "person", "Schema": "public",
                  "Alias": "person", "Index Name": "person_pkey", "Total Cost": 1.03,
                  "Plan Rows": 3, "Actual Total Time": 0.002, "Actual Rows": 1,
                  "Actual Loops": 3, "Index Cond": "(id > 0)"
                }
              ]
            }
          ]
        },
        "Planning Time": 0.2,
        "Execution Time": 0.07
      }
    ]"#;

    #[test]
    fn postgres_json() {
        let root = from_pg_json(PG).unwrap();
        assert_eq!(root.label, "Hash Join");
        assert_eq!(
            root.detail.as_deref(),
            Some("Inner join; Hash Cond: (o.person_id = p.id)")
        );
        assert_eq!((root.est_rows, root.est_cost), (Some(3.0), Some(2.21)));
        assert_eq!((root.actual_rows, root.actual_ms), (Some(3.0), Some(0.036)));
        assert_eq!(root.count(), 4);
        let scan = &root.children[0];
        assert_eq!(scan.label, "Seq Scan");
        assert_eq!(scan.detail.as_deref(), Some("on public.orders o"));
        let idx = &root.children[1].children[0];
        assert_eq!(
            idx.detail.as_deref(),
            Some("on public.person; using person_pkey; Index Cond: (id > 0)")
        );
        // per-loop figures are multiplied out
        assert_eq!(idx.actual_rows, Some(3.0));
        assert!((idx.actual_ms.unwrap() - 0.006).abs() < 1e-9);

        // no ANALYZE: no actual figures
        let est = from_pg_json(
            r#"[{"Plan": {"Node Type": "Result", "Startup Cost": 0.0, "Total Cost": 0.01, "Plan Rows": 1, "Plan Width": 4}}]"#,
        )
        .unwrap();
        assert_eq!(est.label, "Result");
        assert_eq!(est.actual_rows, None);
        assert!(est.children.is_empty() && est.detail.is_none());
        assert!(from_pg_json("not json").is_err());
        assert!(from_pg_json("[]").is_err());
    }

    const MYSQL_ANALYZE: &str = "\
-> Nested loop inner join  (cost=0.70 rows=1) (actual time=0.040..0.046 rows=2 loops=1)
    -> Filter: (t1.a is not null)  (cost=0.35 rows=1) (actual time=0.020..0.023 rows=2 loops=1)
        -> Table scan on t1  (cost=0.35 rows=1) (actual time=0.019..0.021 rows=2 loops=1)
    -> Single-row index lookup on t2 using PRIMARY (id=t1.a)  (cost=0.35 rows=1) (actual time=0.009..0.010 rows=1 loops=2)
";

    #[test]
    fn mysql_tree() {
        let root = from_mysql_tree(MYSQL_ANALYZE);
        assert_eq!(root.label, "Nested loop inner join");
        assert_eq!((root.est_cost, root.est_rows), (Some(0.70), Some(1.0)));
        assert_eq!((root.actual_rows, root.actual_ms), (Some(2.0), Some(0.046)));
        assert_eq!(root.children.len(), 2);
        let filter = &root.children[0];
        assert_eq!(filter.label, "Filter");
        assert_eq!(filter.detail.as_deref(), Some("(t1.a is not null)"));
        assert_eq!(filter.children[0].label, "Table scan on t1");
        let lookup = &root.children[1];
        assert_eq!(
            lookup.label,
            "Single-row index lookup on t2 using PRIMARY (id=t1.a)"
        );
        assert_eq!(lookup.actual_rows, Some(2.0));
        assert!((lookup.actual_ms.unwrap() - 0.020).abs() < 1e-9);

        // FORMAT=TREE without ANALYZE, a never-executed node, two top-level lines
        let t = from_mysql_tree(
            "-> Table scan on t  (cost=1.2e3 rows=10000)\n-> Rows fetched before execution  (never executed)\n",
        );
        assert_eq!(t.label, "Query");
        assert_eq!(t.children[0].est_rows, Some(10000.0));
        assert_eq!(t.children[0].est_cost, Some(1200.0));
        assert_eq!(t.children[0].actual_rows, None);
        assert_eq!(t.children[1].label, "Rows fetched before execution");
    }

    #[test]
    fn mysql_table() {
        let cols: Vec<String> = [
            "id",
            "select_type",
            "table",
            "type",
            "possible_keys",
            "key",
            "key_len",
            "ref",
            "rows",
            "Extra",
        ]
        .map(String::from)
        .to_vec();
        let s = |v: &str| Some(v.to_string());
        let rows = vec![
            vec![
                s("1"),
                s("SIMPLE"),
                s("t1"),
                s("ALL"),
                None,
                None,
                None,
                None,
                s("100"),
                s("Using where"),
            ],
            vec![
                s("1"),
                s("SIMPLE"),
                s("t2"),
                s("eq_ref"),
                s("PRIMARY"),
                s("PRIMARY"),
                s("4"),
                s("db.t1.a"),
                s("1"),
                None,
            ],
        ];
        let root = from_mysql_table(&cols, &rows);
        assert_eq!(root.label, "Query");
        assert_eq!(root.children[0].label, "SIMPLE t1");
        assert_eq!(
            root.children[0].detail.as_deref(),
            Some("type=ALL; Using where")
        );
        assert_eq!(root.children[0].est_rows, Some(100.0));
        assert_eq!(
            root.children[1].detail.as_deref(),
            Some("type=eq_ref; key=PRIMARY; ref=db.t1.a")
        );
    }

    #[test]
    fn sqlite_eqp() {
        let rows = vec![
            (2, 0, "SCAN o".to_string()),
            (
                5,
                0,
                "SEARCH p USING INTEGER PRIMARY KEY (rowid=?)".to_string(),
            ),
            (9, 0, "USE TEMP B-TREE FOR ORDER BY".to_string()),
            (11, 9, "child".to_string()),
        ];
        let root = from_sqlite_eqp(&rows);
        assert_eq!(root.label, "QUERY PLAN");
        assert_eq!(root.children.len(), 3);
        assert_eq!(root.children[1].label, "SEARCH p");
        assert_eq!(
            root.children[1].detail.as_deref(),
            Some("USING INTEGER PRIMARY KEY (rowid=?)")
        );
        assert_eq!(root.children[2].children[0].label, "child");
    }

    const MSSQL: &str = r#"<?xml version="1.0" encoding="utf-16"?>
<ShowPlanXML xmlns="http://schemas.microsoft.com/sqlserver/2004/07/showplan" Version="1.564" Build="16.0.1000.6">
  <BatchSequence><Batch><Statements>
    <StmtSimple StatementText="SELECT p.name FROM dbo.person p JOIN dbo.orders o ON o.pid = p.id WHERE p.id &gt; 0" StatementType="SELECT">
      <QueryPlan>
        <!-- a comment with <RelOp> inside -->
        <RelOp NodeId="0" PhysicalOp="Nested Loops" LogicalOp="Inner Join" EstimateRows="3" EstimatedTotalSubtreeCost="0.0065704">
          <OutputList><ColumnReference Database="[shop]" Schema="[dbo]" Table="[person]" Alias="[p]" Column="name" /></OutputList>
          <RunTimeInformation>
            <RunTimeCountersPerThread Thread="1" ActualRows="2" ActualElapsedms="3" />
            <RunTimeCountersPerThread Thread="2" ActualRows="1" ActualElapsedms="5" />
          </RunTimeInformation>
          <NestedLoops Optimized="false">
            <RelOp NodeId="1" PhysicalOp="Clustered Index Scan" LogicalOp="Clustered Index Scan" EstimateRows="3" EstimatedTotalSubtreeCost="0.0032853">
              <IndexScan Ordered="false"><Object Database="[shop]" Schema="[dbo]" Table="[orders]" Index="[PK_orders]" Alias="[o]" /></IndexScan>
            </RelOp>
            <RelOp NodeId="2" PhysicalOp="Clustered Index Seek" LogicalOp="Clustered Index Seek" EstimateRows="1" EstimatedTotalSubtreeCost="0.0032831">
              <IndexScan Ordered="true"><Object Database="[shop]" Schema="[dbo]" Table="[person]" Index="[PK_person]" Alias="[p]" />
                <SeekPredicates><SeekPredicateNew><SeekKeys><Prefix ScanType="EQ"><RangeColumns><ColumnReference Column="id" /></RangeColumns></Prefix></SeekKeys></SeekPredicateNew></SeekPredicates>
              </IndexScan>
            </RelOp>
          </NestedLoops>
        </RelOp>
      </QueryPlan>
    </StmtSimple>
  </Statements></Batch></BatchSequence>
</ShowPlanXML>"#;

    #[test]
    fn mssql_showplan() {
        let root = from_mssql_xml(MSSQL).unwrap();
        assert_eq!(root.label, "Nested Loops");
        assert_eq!(root.detail.as_deref(), Some("Inner Join"));
        assert_eq!(root.est_rows, Some(3.0));
        assert_eq!(root.est_cost, Some(0.0065704));
        assert_eq!((root.actual_rows, root.actual_ms), (Some(3.0), Some(5.0)));
        assert_eq!(root.count(), 3);
        let scan = &root.children[0];
        assert_eq!(scan.label, "Clustered Index Scan");
        assert_eq!(
            scan.detail.as_deref(),
            Some("on dbo.orders index PK_orders")
        );
        assert_eq!(scan.actual_rows, None);
        assert_eq!(
            root.children[1].detail.as_deref(),
            Some("on dbo.person index PK_person")
        );

        // two documents (STATISTICS XML gives one per statement) → a Batch root
        let two = from_mssql_xml(&format!("{MSSQL}\n{MSSQL}")).unwrap();
        assert_eq!(two.label, "Batch");
        assert_eq!(two.children.len(), 2);
        // a statement with no RelOp
        let set = from_mssql_xml(
            r#"<ShowPlanXML><BatchSequence><Batch><Statements><StmtSimple StatementType="SET ON/OFF"/></Statements></Batch></BatchSequence></ShowPlanXML>"#,
        )
        .unwrap();
        assert_eq!(set.label, "SET ON/OFF");
        assert!(from_mssql_xml("no xml here").is_err());
    }

    #[test]
    fn xml_bits() {
        assert_eq!(
            decode_entities("a &lt;b&gt; &amp; &#65;&#x42; &bogus; &"),
            "a <b> & AB &bogus; &"
        );
        assert_eq!(
            attr(r#" A="1" B='x>y' C="&quot;q&quot;""#, "B").as_deref(),
            Some("x>y")
        );
        assert_eq!(
            attr(r#" A="1" C="&quot;q&quot;""#, "C").as_deref(),
            Some("\"q\"")
        );
        assert_eq!(attr(r#" AB="1""#, "A"), None);
        let names: Vec<_> = xml_tags(r#"<a x="1>2"><p:b/></a>"#)
            .map(|t| (t.name, t.start, t.end))
            .collect();
        assert_eq!(
            names,
            [("a", true, false), ("b", true, true), ("a", false, true)]
        );
    }

    #[test]
    fn numbers() {
        assert_eq!(num_after("cost=0.35 rows=1", "rows="), Some(1.0));
        assert_eq!(num_after("cost=1e3 rows=2", "cost="), Some(1000.0));
        // `rows=` must not match inside another key
        assert_eq!(num_after("xrows=5 rows=7", "rows="), Some(7.0));
        assert_eq!(num_after("nothing", "rows="), None);
    }
}
