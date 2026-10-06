//! The graph stages (P1.4) against Archify's own goldens.
//!
//! Every `tests/fixtures/*.golden.json` whose document passes the schema is run through the
//! pipeline order of `graph.rs` (relationship ids, engineering, evidence, graph rules), stopping at
//! the first failing stage like Archify. The diagnostics Archify gave that belong to these rules are
//! picked out of its receipt (`is_graph_rule`: their codes, and the `layout/constraint` sentences
//! of the graph rules, not the geometry ones) and must equal ours as `(code, subject.path,
//! severity)` and by message. Where a golden also holds geometry or `clean-flow` errors, those are
//! the layout engine's (P2-P6) and are ignored; our output must not contain anything Archify did
//! not say.
//!
//! The one message not compared is `internal/unclassified`: Archify's is a V8 stack message.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use ubiq_archify::diag::{Diagnostic, Severity, dedup_by_message};
use ubiq_archify::model::Doc;
use ubiq_archify::{engineering, evidence, graph, schema};
use serde_json::{Value, json};

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// The V5 -> V6 -> V7 -> V9 order a `validate` without `--repo-root` runs.
fn staged(doc: &Doc, raw: &Value) -> Vec<Diagnostic> {
    for stage in [
        graph::relationship_ids(doc),
        engineering::check(doc, raw),
        evidence::check_shape(doc, raw),
        evidence::root_required(doc).into_iter().collect(),
    ] {
        if !stage.is_empty() {
            return stage;
        }
    }
    graph::layout_rules(doc, raw)
}

/// Whether a diagnostic of Archify's receipt is one of the rules of this task.
fn is_graph_rule(code: &str, message: &str) -> bool {
    const CODES: &[&str] = &[
        "relationship/duplicate-id",
        "internal/unclassified",
        "workflow/duplicate-lane-id",
        "workflow/duplicate-node-id",
        "workflow/unknown-edge-endpoint",
        "workflow/unknown-node-lane",
        "workflow/invalid-node-column",
        "workflow/semantic-node-reference",
        "workflow/unexpected-root",
        "workflow/unexpected-terminal",
        "workflow/required-edge",
        "workflow/required-path",
    ];
    if CODES.contains(&code)
        || code.starts_with("engineering/")
        || code.starts_with("repository-evidence/")
    {
        return true;
    }
    if code != "layout/constraint" {
        return false;
    }
    let first = message.lines().next().unwrap_or_default();
    const EXACT: &[&str] = &[
        "Component ids must be unique.",
        "Participant ids must be unique.",
        "Node ids must be unique.",
        "State ids must be unique.",
        "Lane ids must be unique.",
        "Phase ids must be unique.",
        "Group ids must be unique.",
    ];
    const CONTAINS: &[&str] = &[
        " share grid cell ",
        " exceeds layout.cols ",
        "needs pos [x,y] or grid",
        "must include pos [x, y] when layout.mode",
        "row/col must be non-negative",
        " wraps unknown component ",
        " references unknown source ",
        " references unknown target ",
        "Activation references unknown participant",
        " uses unknown lane ",
        " uses invalid row ",
        " uses invalid stage ",
        " has invalid time range",
        " has invalid y range",
        " uses invalid column",
        " must use integer fromCol/toCol",
        " overlaps phase ",
        " does not contain any nodes",
    ];
    EXACT.contains(&first)
        || first.starts_with("Lifecycle diagrams need a lane")
        || first.starts_with("mainPath ")
        || (first.starts_with("Phase \"") && !first.starts_with("Phase label"))
        || (first.starts_with("Group \"") && first.contains(" uses invalid columns "))
        || CONTAINS.iter().any(|needle| first.contains(needle))
}

fn read(path: &Path) -> Value {
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

type Key = (String, Option<String>, Severity, String);

fn key(code: &str, path: Option<&str>, severity: Severity, message: &str) -> Key {
    // Archify's text for the crash is a V8 message; compare the code and path only.
    let message = if code == "internal/unclassified" {
        ""
    } else {
        message
    };
    (
        code.to_owned(),
        path.map(str::to_owned),
        severity,
        message.to_owned(),
    )
}

fn ours(diagnostics: Vec<Diagnostic>) -> BTreeSet<Key> {
    dedup_by_message(diagnostics)
        .iter()
        .map(|d| {
            key(
                &d.code,
                d.subject.path.as_deref(),
                d.severity,
                d.message.lines().next().unwrap_or_default(),
            )
        })
        .collect()
}

#[test]
fn graph_rules_match_the_goldens() {
    let dir = fixtures_dir();
    let mut names: Vec<_> = fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.to_string_lossy().ends_with(".golden.json"))
        .collect();
    names.sort();

    let (mut compared, mut with_rules, mut skipped, mut failures) = (0, 0, Vec::new(), Vec::new());
    for path in names {
        let case = read(&path);
        let name = case["name"].as_str().unwrap().to_owned();
        let (Some(doc_type), Some(source)) = (case["type"].as_str(), case.get("source_doc")) else {
            skipped.push(format!("{name}: no parsed document"));
            continue;
        };
        let authored = &case["validate_receipt"]["authored"];
        let receipt = &authored["receipt"];
        let golden: Vec<&Value> = receipt["diagnostics"]
            .as_array()
            .map(|a| a.iter().collect())
            .unwrap_or_default();
        // The stages before ours stopped Archify, or the document cannot be a `Doc`.
        let before = golden.iter().any(|d| {
            let code = d["code"].as_str().unwrap_or_default();
            ["schema/", "input/", "output/", "portable-path/"]
                .iter()
                .any(|f| code.starts_with(f))
        });
        if before || !schema::validate(doc_type, source).is_empty() {
            skipped.push(format!("{name}: stops before the graph stages"));
            continue;
        }
        let doc = match Doc::from_value(source) {
            Ok(doc) => doc,
            Err(e) => {
                failures.push(format!("{name}: Doc::from_value: {e}"));
                continue;
            }
        };
        let expected: BTreeSet<Key> = golden
            .iter()
            .filter(|d| {
                is_graph_rule(
                    d["code"].as_str().unwrap_or_default(),
                    d["message"].as_str().unwrap_or_default(),
                )
            })
            .map(|d| {
                let severity = if d["severity"] == "warning" {
                    Severity::Warning
                } else {
                    Severity::Error
                };
                key(
                    d["code"].as_str().unwrap(),
                    d["subject"]["path"].as_str(),
                    severity,
                    d["message"]
                        .as_str()
                        .unwrap()
                        .lines()
                        .next()
                        .unwrap_or_default(),
                )
            })
            .collect();
        let actual = ours(staged(&doc, source));
        compared += 1;
        if !expected.is_empty() {
            with_rules += 1;
        }
        if actual != expected {
            failures.push(format!(
                "{name}\n  only in Archify: {:?}\n  only in ours:    {:?}",
                expected.difference(&actual).collect::<Vec<_>>(),
                actual.difference(&expected).collect::<Vec<_>>()
            ));
        }
    }
    eprintln!(
        "graph goldens: {compared} compared, {with_rules} with graph-rule diagnostics, {} skipped",
        skipped.len()
    );
    assert!(
        failures.is_empty(),
        "{} mismatches:\n{}",
        failures.len(),
        failures.join("\n")
    );
    // Guard against the harness quietly comparing nothing.
    assert!(compared >= 90, "only {compared} goldens compared");
    assert!(
        with_rules >= 60,
        "only {with_rules} goldens exercise a graph rule"
    );
}

// ---- rules the goldens do not reach ------------------------------------------------------------

fn arch(extra: Value) -> Value {
    let mut doc = json!({
        "schema_version": 1, "diagram_type": "architecture",
        "meta": {"title": "t", "output": "a.html"},
        "components": [
            {"id": "a", "type": "backend", "label": "A", "pos": [40, 80]},
            {"id": "b", "type": "backend", "label": "B", "pos": [240, 80]},
        ],
    });
    for (k, v) in extra.as_object().unwrap() {
        doc[k] = v.clone();
    }
    doc
}

fn run(raw: &Value) -> Vec<Diagnostic> {
    assert!(schema::validate(raw["diagram_type"].as_str().unwrap(), raw).is_empty());
    staged(&Doc::from_value(raw).unwrap(), raw)
}

fn messages(diagnostics: &[Diagnostic]) -> Vec<&str> {
    diagnostics.iter().map(|d| d.message.as_str()).collect()
}

#[test]
fn free_placement_without_a_connection_is_a_constraint_not_a_crash() {
    // Measured against Archify: with no connection touching it the layout survives and
    // `validateArchitecture` says this.
    let mut doc = arch(json!({}));
    doc["components"][0].as_object_mut().unwrap().remove("pos");
    let diagnostics = run(&doc);
    assert_eq!(
        messages(&diagnostics),
        ["Component \"a\" must include pos [x, y] when layout.mode is omitted (free placement)."]
    );
    assert_eq!(diagnostics[0].code, "layout/constraint");
    assert_eq!(
        diagnostics[0].subject.rule.as_deref(),
        Some("architecture/placement-missing")
    );
}

#[test]
fn an_unplaced_component_on_a_connection_is_the_archify_crash() {
    let mut doc = arch(json!({"connections": [{"from": "a", "to": "b", "label": "x"}]}));
    doc["components"][1].as_object_mut().unwrap().remove("pos");
    let diagnostics = run(&doc);
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].code, "internal/unclassified");
    assert_eq!(diagnostics[0].subject.path, None);
}

#[test]
fn grid_default_cols_and_shared_cells() {
    // `layout.cols` defaults to 4; negative row/col is the schema's `minimum`.
    let mut doc = arch(json!({"layout": {"mode": "grid"}}));
    let comps = doc["components"].as_array_mut().unwrap();
    comps[0] = json!({"id": "a", "type": "backend", "label": "A", "row": 0, "col": 4});
    comps[1] = json!({"id": "b", "type": "backend", "label": "B", "row": 0, "col": 4});
    assert_eq!(
        messages(&run(&doc)),
        [
            "Component \"a\" col 4 exceeds layout.cols 4 (valid: 0..3).",
            "Component \"b\" col 4 exceeds layout.cols 4 (valid: 0..3).",
            "Components \"a\" and \"b\" share grid cell row 0 col 4."
        ]
    );
}

#[test]
fn lifecycle_v1_columns_follow_the_band() {
    let raw = json!({
        "schema_version": 1, "diagram_type": "lifecycle",
        "meta": {"title": "t", "output": "a.html"},
        "lanes": [{"id": "main", "label": "Main"}, {"id": "other", "label": "Other"}],
        "states": [
            {"id": "s1", "type": "start", "label": "S1", "lane": "main", "col": 4},
            {"id": "s2", "type": "active", "label": "S2", "lane": "other", "col": 3},
        ],
        "transitions": [],
    });
    assert_eq!(
        messages(&run(&raw)),
        ["State \"s2\" uses invalid column 3 — the event band has integer columns 0..2."]
    );
}

#[test]
fn evidence_source_shape_runs_in_document_order() {
    let repo = json!({"url": "https://github.com/o/r", "revision": "0123456789abcdef0123456789abcdef01234567"});
    let with = |source: Value| {
        let mut doc = arch(json!({}));
        doc["meta"]["repository"] = repo.clone();
        doc["components"][0]["sources"] = json!([source]);
        // Archify stops at `root-required` when no repository root is given; the shape comes first.
        let doc = Doc::from_value(&doc).unwrap();
        evidence::check_shape(&doc, &Value::Null)
    };
    assert!(with(json!({"path": "src/a.rs", "line": 3})).is_empty());
    assert_eq!(
        with(json!({"path": "/src/a.rs"}))[0].code,
        "repository-evidence/path-invalid"
    );
    assert_eq!(
        with(json!({"path": "src/../a.rs"}))[0].code,
        "repository-evidence/path-escape"
    );
    assert_eq!(
        with(json!({"path": ".git/config"}))[0].code,
        "repository-evidence/path-escape"
    );
    assert_eq!(
        with(json!({"path": "a.rs", "end_line": 4}))[0].code,
        "repository-evidence/line-required"
    );
    let bad = with(json!({"path": "a.rs", "line": 5, "end_line": 4}));
    assert_eq!(bad[0].code, "repository-evidence/line-range-invalid");
    assert_eq!(
        bad[0].subject.path.as_deref(),
        Some("/components/0/sources/0")
    );
    // A repository with no source anywhere.
    let mut doc = arch(json!({}));
    doc["meta"]["repository"] = repo;
    let doc = Doc::from_value(&doc).unwrap();
    assert_eq!(
        evidence::check_shape(&doc, &Value::Null)[0].code,
        "repository-evidence/source-required"
    );
}

#[test]
fn repository_urls() {
    let check = |url: &str, extra: Value| {
        let mut doc = arch(json!({}));
        doc["meta"]["repository"] =
            json!({"url": url, "revision": "0123456789abcdef0123456789abcdef01234567"});
        for (k, v) in extra.as_object().unwrap() {
            doc["meta"]["repository"][k] = v.clone();
        }
        doc["components"][0]["sources"] = json!([{"path": "a.rs"}]);
        let doc = Doc::from_value(&doc).unwrap();
        evidence::check_shape(&doc, &Value::Null)
            .first()
            .map(|d| d.code.clone())
    };
    let none = json!({});
    assert_eq!(check("https://github.com/o/r", none.clone()), None);
    assert_eq!(check("https://github.com/o/r.git", none.clone()), None);
    assert_eq!(
        check("git@github.com:o/r.git", none.clone()).as_deref(),
        Some("repository-evidence/links-unsupported")
    );
    assert_eq!(
        check("git@github.com:o/r.git", json!({"link_mode": "local-only"})),
        None
    );
    assert_eq!(
        check(
            "https://gitlab.example.com/o/r",
            json!({"link_mode": "local-only"})
        ),
        None
    );
    assert_eq!(
        check("https://user:pw@github.com/o/r", none.clone()).as_deref(),
        Some("repository-evidence/url-invalid")
    );
    assert_eq!(
        check("https://github.com/o/r?x=1", none.clone()).as_deref(),
        Some("repository-evidence/url-invalid")
    );
    assert_eq!(
        check("https://github.com/o/../r", none.clone()).as_deref(),
        Some("repository-evidence/url-invalid")
    );
    assert_eq!(
        check("/home/me/repo", none.clone()).as_deref(),
        Some("repository-evidence/url-invalid")
    );
    assert_eq!(
        check("https://github.com/o/r", json!({"provider": "gitee"})).as_deref(),
        Some("repository-evidence/provider-invalid")
    );
    assert_eq!(
        check("https://github.com/o/r/extra", none).as_deref(),
        Some("repository-evidence/links-unsupported")
    );
}
