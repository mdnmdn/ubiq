//! Graph checks (P1.4): relationship ids, id uniqueness, dangling references, grid placement,
//! ranges, the workflow semantic contract, `mainPath`, phases and groups, the lifecycle `main`
//! lane. No geometry: those rules live with each type's layout (P2-P6).
//!
//! # Stage order (00 section 3.1, `shared/cli.mjs:loadDiagram`, `render-*.mjs`)
//!
//! A failing stage stops the pipeline, so `compile.rs` (P1.5) calls these in this order, after the
//! schema stage has passed and `Doc::from_value` has succeeded:
//!
//! 1. [`relationship_ids`] (V5): `relationship/duplicate-id`.
//! 2. [`crate::engineering::check`] (V6): `engineering/*`, architecture with a profile.
//! 3. [`crate::evidence::check_shape`] (V7), then [`crate::evidence::root_required`] while
//!    verification is deferred (D19): `repository-evidence/*`.
//! 4. [`layout_rules`] (V9, the graph part): everything else here.
//!
//! [`check`] is V5 then V9 with the stop between them, for callers that have no engineering or
//! evidence stage to interleave (tests, tools).
//!
//! # Wire codes (D7)
//!
//! Most rules are `layout/constraint` with Archify's sentence as the message and no `subject.path`:
//! that is what Archify says. `subject.rule` carries the 00 rule id so a caller can tell them apart.
//! Workflow v2 has specific codes, and v2 pre-routing failures are thrown one at a time by Archify
//! (the first wins), which is mirrored. Where Archify throws a fix list that it verifies by
//! recompiling (`acceptsFix`), the fixes here are the unverified candidates.
//!
//! # Archify quirks kept
//!
//! - A `Map` keyed by id collapses duplicates (last value wins, first position), so every "unknown
//!   id" check runs against that, not against the raw array ([`Index`]).
//! - An architecture component with no placement that a connection touches crashes Archify
//!   (`internal/unclassified`) before any layout rule runs; that is the whole V9 answer then.
//! - The workflow semantic contract short-circuits: one `semantic-node-reference` hides the rest,
//!   and any semantic diagnostic hides every layout-time rule.

use std::collections::{HashMap, HashSet};

use serde_json::{Map, Value, json};

use crate::diag::{Diagnostic, Subject};
use crate::model::Doc;
use crate::model::architecture::{Architecture, Layout};
use crate::model::common::SchemaVersion;
use crate::model::dataflow::Dataflow;
use crate::model::lifecycle::Lifecycle;
use crate::model::sequence::Sequence;
use crate::model::workflow::{Node as WfNode, Workflow};

/// Workflow columns are `0..=5` in both versions (`layout.colXs.length`).
const WORKFLOW_COLS: f64 = 6.0;
/// Dataflow rows are `0..=4` (`layout.rowYs.length`).
const DATAFLOW_ROWS: f64 = 5.0;
/// `DEFAULT_GRID.cols` (`architecture/grid.mjs`).
const DEFAULT_GRID_COLS: f64 = 4.0;

/// V5 then V9, stopping after V5 when it fails. See the module doc for the full pipeline order.
pub fn check(doc: &Doc, raw: &Value) -> Vec<Diagnostic> {
    let ids = relationship_ids(doc);
    if !ids.is_empty() {
        return ids;
    }
    layout_rules(doc, raw)
}

/// V5: optional relationship ids are unique per collection (`shared/cli.mjs:validateRelationshipIds`).
/// Reports every duplicate; an absent or empty id is ignored.
pub fn relationship_ids(doc: &Doc) -> Vec<Diagnostic> {
    let (collection, ids): (&str, Vec<Option<&str>>) = match doc {
        Doc::Architecture(d) => (
            "connections",
            opt_ids(d.connections.iter().flatten(), |c| c.id.as_deref()),
        ),
        Doc::Workflow(d) => ("edges", opt_ids(d.edges.iter(), |e| e.id.as_deref())),
        Doc::Sequence(d) => ("messages", opt_ids(d.messages.iter(), |m| m.id.as_deref())),
        Doc::Dataflow(d) => ("flows", opt_ids(d.flows.iter(), |f| f.id.as_deref())),
        Doc::Lifecycle(d) => (
            "transitions",
            opt_ids(d.transitions.iter(), |t| t.id.as_deref()),
        ),
    };
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for (index, id) in ids.into_iter().enumerate() {
        let Some(id) = id.filter(|id| !id.is_empty()) else {
            continue;
        };
        if !seen.insert(id) {
            let mut subject =
                Subject::of(doc.diagram_type()).with_rule("relationship/duplicate-id");
            subject.collection = Some(collection.to_owned());
            out.push(
                Diagnostic::error(
                    "relationship/duplicate-id",
                    &format!(
                        "/{collection}/{index}/id duplicates relationship id {}",
                        json_str(id)
                    ),
                )
                .with_subject(subject),
            );
        }
    }
    out
}

fn opt_ids<'a, T: 'a>(
    items: impl Iterator<Item = &'a T>,
    id: impl Fn(&'a T) -> Option<&'a str>,
) -> Vec<Option<&'a str>> {
    items.map(id).collect()
}

/// V9, graph part: the rules of 00 section 3.4b that need no geometry, for the document's type.
/// `raw` is the parsed JSON the `Doc` came from; no rule needs it yet (every number is already an
/// `f64` in the model), it is kept so the signature does not change when one does.
pub fn layout_rules(doc: &Doc, _raw: &Value) -> Vec<Diagnostic> {
    match doc {
        Doc::Architecture(d) => architecture(d),
        Doc::Workflow(d) => workflow(d),
        Doc::Sequence(d) => sequence(d),
        Doc::Dataflow(d) => dataflow(d),
        Doc::Lifecycle(d) => lifecycle(d),
    }
}

// ---- helpers -----------------------------------------------------------------------------------

/// A JS `Map` built from `items.map(i => [i.id, i])`: unique ids in first-seen order, each holding
/// the last item with that id.
struct Index<'a, T> {
    order: Vec<&'a str>,
    map: HashMap<&'a str, &'a T>,
}

impl<'a, T> Index<'a, T> {
    fn new(items: &'a [T], id: impl Fn(&'a T) -> &'a str) -> Self {
        let mut order = Vec::new();
        let mut map = HashMap::new();
        for item in items {
            let key = id(item);
            if map.insert(key, item).is_none() {
                order.push(key);
            }
        }
        Index { order, map }
    }
    fn has(&self, id: &str) -> bool {
        self.map.contains_key(id)
    }
    fn get(&self, id: &str) -> Option<&'a T> {
        self.map.get(id).copied()
    }
    fn len(&self) -> usize {
        self.order.len()
    }
    fn values(&self) -> impl Iterator<Item = &'a T> + '_ {
        self.order.iter().map(|id| self.map[id])
    }
}

/// `layout/constraint`, the code Archify gives every graph rule that has no code of its own.
fn constraint(diagram_type: &str, rule: &str, message: String) -> Diagnostic {
    Diagnostic::error("layout/constraint", &message)
        .with_subject(Subject::of(diagram_type).with_rule(rule))
}

fn json_str(s: &str) -> String {
    Value::String(s.to_owned()).to_string()
}

/// A number the way JS prints it in a template string (`-0` is `0`).
fn num(x: f64) -> String {
    if x == 0.0 {
        "0".to_owned()
    } else {
        format!("{x}")
    }
}

fn is_int(x: f64) -> bool {
    x.is_finite() && x.fract() == 0.0
}

/// `a || b` over an optional label.
fn or_else<'a>(label: Option<&'a str>, fallback: &'a str) -> &'a str {
    label.filter(|l| !l.is_empty()).unwrap_or(fallback)
}

fn evidence(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(map) => map,
        _ => Map::new(),
    }
}

// ---- architecture ------------------------------------------------------------------------------

fn architecture(a: &Architecture) -> Vec<Diagnostic> {
    const DT: &str = "architecture";
    let comps = Index::new(&a.components, |c| c.id.as_str());
    let grid = a.layout.as_ref();
    let placed = |c: &crate::model::architecture::Component| match grid {
        Some(_) => {
            c.pos.is_some()
                || (is_int(c.row.unwrap_or(f64::NAN)) && is_int(c.col.unwrap_or(f64::NAN)))
        }
        None => c.pos.is_some(),
    };

    // Archify measures every component before it validates anything. A connection that touches an
    // unplaced one (NaN position) crashes the router: `internal/unclassified`, nothing else.
    let unplaced: HashSet<&str> = comps
        .values()
        .filter(|c| !placed(c))
        .map(|c| c.id.as_str())
        .collect();
    let touching = a
        .connections
        .iter()
        .flatten()
        .find(|c| unplaced.contains(c.from.as_str()) || unplaced.contains(c.to.as_str()));
    if let Some(conn) = touching {
        let id = if unplaced.contains(conn.from.as_str()) {
            &conn.from
        } else {
            &conn.to
        };
        let how = if grid.is_some() {
            "pos [x, y] or integer row and col under layout.mode \"grid\""
        } else {
            "pos [x, y] (free placement)"
        };
        // Archify's own text is V8's "Cannot read properties of undefined (reading '0')"; the code
        // is kept (D7), the sentence is the cause.
        return vec![
            Diagnostic::error(
                "internal/unclassified",
                &format!(
                    "Component \"{id}\" has no placement and connection \"{}\" -> \"{}\" uses it; Archify crashes on this. Give it {how}.",
                    conn.from, conn.to
                ),
            )
            .with_subject(Subject::of(DT).with_rule("architecture/placement-missing"))
            .with_fixes([format!("add {how} to component \"{id}\"")]),
        ];
    }

    let mut out = Vec::new();
    if comps.len() != a.components.len() {
        out.push(constraint(
            DT,
            "graph/duplicate-node-id",
            "Component ids must be unique.".to_owned(),
        ));
    }
    grid_placement(a, grid, &mut out);

    for boundary in a.boundaries.iter().flatten() {
        for id in &boundary.wraps {
            if !comps.has(id) {
                out.push(constraint(
                    DT,
                    "graph/unknown-container",
                    format!(
                        "Boundary \"{}\" wraps unknown component \"{id}\".",
                        boundary.label
                    ),
                ));
            }
        }
    }
    for conn in a.connections.iter().flatten() {
        if !comps.has(&conn.from) {
            out.push(constraint(
                DT,
                "graph/unknown-endpoint",
                format!(
                    "Connection \"{}\" references unknown source \"{}\".",
                    or_else(conn.label.as_deref(), &conn.from),
                    conn.from
                ),
            ));
        }
        if !comps.has(&conn.to) {
            out.push(constraint(
                DT,
                "graph/unknown-endpoint",
                format!(
                    "Connection \"{}\" references unknown target \"{}\".",
                    or_else(conn.label.as_deref(), &conn.to),
                    conn.to
                ),
            ));
        }
    }
    out
}

/// `architecture/grid.mjs:validateGridPlacement` and the free-placement `pos` check.
fn grid_placement(a: &Architecture, grid: Option<&Layout>, out: &mut Vec<Diagnostic>) {
    const DT: &str = "architecture";
    let Some(layout) = grid else {
        for c in &a.components {
            if c.pos.is_none() {
                out.push(constraint(
                    DT,
                    "architecture/placement-missing",
                    format!(
                        "Component \"{}\" must include pos [x, y] when layout.mode is omitted (free placement).",
                        c.id
                    ),
                ));
            }
        }
        return;
    };
    let cols = layout.cols.unwrap_or(DEFAULT_GRID_COLS);
    let mut seen: HashMap<(i64, i64), &str> = HashMap::new();
    for c in &a.components {
        if c.pos.is_some() {
            continue; // pos wins; row/col are optional hints
        }
        let (Some(row), Some(col)) = (c.row.filter(|r| is_int(*r)), c.col.filter(|v| is_int(*v)))
        else {
            out.push(constraint(
                DT,
                "architecture/placement-missing",
                format!(
                    "Component \"{}\" needs pos [x,y] or grid row/col when layout.mode is \"grid\".",
                    c.id
                ),
            ));
            continue;
        };
        if row < 0.0 || col < 0.0 {
            out.push(constraint(
                DT,
                "architecture/grid-cell",
                format!(
                    "Component \"{}\" row/col must be non-negative integers.",
                    c.id
                ),
            ));
            continue;
        }
        if col >= cols {
            out.push(constraint(
                DT,
                "architecture/grid-cell",
                format!(
                    "Component \"{}\" col {} exceeds layout.cols {} (valid: 0..{}).",
                    c.id,
                    num(col),
                    num(cols),
                    num(cols - 1.0)
                ),
            ));
        }
        match seen.get(&(row as i64, col as i64)) {
            Some(first) => out.push(constraint(
                DT,
                "architecture/grid-cell",
                format!(
                    "Components \"{first}\" and \"{}\" share grid cell row {} col {}.",
                    c.id,
                    num(row),
                    num(col)
                ),
            )),
            None => {
                seen.insert((row as i64, col as i64), c.id.as_str());
            }
        }
    }
}

// ---- sequence ----------------------------------------------------------------------------------

fn sequence(s: &Sequence) -> Vec<Diagnostic> {
    const DT: &str = "sequence";
    let parts = Index::new(&s.participants, |p| p.id.as_str());
    let mut out = Vec::new();
    if parts.len() != s.participants.len() {
        out.push(constraint(
            DT,
            "graph/duplicate-node-id",
            "Participant ids must be unique.".to_owned(),
        ));
    }
    for m in &s.messages {
        if !parts.has(&m.from) {
            out.push(constraint(
                DT,
                "graph/unknown-endpoint",
                format!(
                    "Message \"{}\" references unknown source \"{}\".",
                    m.label, m.from
                ),
            ));
        }
        if !parts.has(&m.to) {
            out.push(constraint(
                DT,
                "graph/unknown-endpoint",
                format!(
                    "Message \"{}\" references unknown target \"{}\".",
                    m.label, m.to
                ),
            ));
        }
    }
    for seg in s.segments.iter().flatten() {
        if seg.to <= seg.from {
            out.push(constraint(
                DT,
                "sequence/inverted-range",
                format!(
                    "Segment \"{}\" has invalid y range (from {} to {}) — \"to\" must be greater than \"from\".",
                    seg.label,
                    num(seg.from),
                    num(seg.to)
                ),
            ));
        }
    }
    for act in s.activations.iter().flatten() {
        if !parts.has(&act.participant) {
            out.push(constraint(
                DT,
                "graph/unknown-container",
                format!(
                    "Activation references unknown participant \"{}\".",
                    act.participant
                ),
            ));
        }
        if act.to <= act.from {
            out.push(constraint(
                DT,
                "sequence/inverted-range",
                format!(
                    "Activation for \"{}\" has invalid time range — \"to\" must be greater than \"from\".",
                    act.participant
                ),
            ));
        }
    }
    out
}

// ---- dataflow ----------------------------------------------------------------------------------

fn dataflow(d: &Dataflow) -> Vec<Diagnostic> {
    const DT: &str = "dataflow";
    let nodes = Index::new(&d.nodes, |n| n.id.as_str());
    let stage_count = d.stages.len() as f64;
    let mut out = Vec::new();
    if nodes.len() != d.nodes.len() {
        out.push(constraint(
            DT,
            "graph/duplicate-node-id",
            "Node ids must be unique.".to_owned(),
        ));
    }
    for n in nodes.values() {
        if n.stage.is_nan() || n.stage < 0.0 || n.stage >= stage_count {
            out.push(constraint(
                DT,
                "dataflow/stage-range",
                format!(
                    "Node \"{}\" uses invalid stage {} — valid stages are 0..{}.",
                    n.id,
                    num(n.stage),
                    num(stage_count - 1.0)
                ),
            ));
        }
        if n.row.is_nan() || n.row < 0.0 || n.row >= DATAFLOW_ROWS {
            out.push(constraint(
                DT,
                "dataflow/row-range",
                format!(
                    "Node \"{}\" uses invalid row {} — valid rows are 0..{}.",
                    n.id,
                    num(n.row),
                    num(DATAFLOW_ROWS - 1.0)
                ),
            ));
        }
    }
    for f in &d.flows {
        if !nodes.has(&f.from) {
            out.push(constraint(
                DT,
                "graph/unknown-endpoint",
                format!(
                    "Flow \"{}\" references unknown source \"{}\".",
                    or_else(Some(&f.label), &f.from),
                    f.from
                ),
            ));
        }
        if !nodes.has(&f.to) {
            out.push(constraint(
                DT,
                "graph/unknown-endpoint",
                format!(
                    "Flow \"{}\" references unknown target \"{}\".",
                    or_else(Some(&f.label), &f.to),
                    f.to
                ),
            ));
        }
        // "must include a short data label" is the schema's `minLength: 1`, never reached here.
    }
    out
}

// ---- lifecycle ---------------------------------------------------------------------------------

fn lifecycle(l: &Lifecycle) -> Vec<Diagnostic> {
    const DT: &str = "lifecycle";
    let v2 = l.schema_version == SchemaVersion::V2;
    let states = Index::new(&l.states, |s| s.id.as_str());
    let lane_ids: HashSet<&str> = l.lanes.iter().map(|lane| lane.id.as_str()).collect();
    let mut out = Vec::new();
    if states.len() != l.states.len() {
        out.push(constraint(
            DT,
            "graph/duplicate-node-id",
            "State ids must be unique.".to_owned(),
        ));
    }
    if lane_ids.len() != l.lanes.len() {
        out.push(constraint(
            DT,
            "graph/duplicate-container-id",
            "Lane ids must be unique.".to_owned(),
        ));
    }
    if !lane_ids.contains("main") {
        let message = if v2 {
            "Lifecycle diagrams need a lane with id \"main\" (the first row). Lane ids \"main\" and \"terminal\" are reserved: \"main\" renders as the first row, \"terminal\" as the last, and every other lane in authored order."
        } else {
            "Lifecycle diagrams need a lane with id \"main\" (the phase rail). Lane ids \"main\" and \"terminal\" are reserved: \"main\" maps to the top phase band, \"terminal\" to the bottom outcome band, and all other lanes share the middle event band."
        };
        out.push(constraint(
            DT,
            "lifecycle/main-lane-required",
            message.to_owned(),
        ));
    }
    for s in states.values() {
        if !lane_ids.contains(s.lane.as_str()) {
            out.push(constraint(
                DT,
                "graph/unknown-container",
                format!("State \"{}\" uses unknown lane \"{}\".", s.id, s.lane),
            ));
            continue;
        }
        let (max_col, message) = if v2 {
            (
                5.0,
                format!(
                    "State \"{}\" uses invalid column {} — every lifecycle row has integer columns 0..4.",
                    s.id,
                    num(s.col)
                ),
            )
        } else {
            // v1 bands: `main` is the 5-column phase rail, every other lane has 3 columns.
            let (band, max) = if s.lane == "main" {
                ("phase", 5.0)
            } else if s.lane == "terminal" {
                ("outcome", 3.0)
            } else {
                ("event", 3.0)
            };
            (
                max,
                format!(
                    "State \"{}\" uses invalid column {} — the {band} band has integer columns 0..{}.",
                    s.id,
                    num(s.col),
                    num(max - 1.0)
                ),
            )
        };
        if !is_int(s.col) || s.col < 0.0 || s.col >= max_col {
            out.push(constraint(DT, "lifecycle/col-range", message));
        }
    }
    for t in &l.transitions {
        if !states.has(&t.from) {
            out.push(constraint(
                DT,
                "graph/unknown-endpoint",
                format!(
                    "Transition \"{}\" references unknown source \"{}\".",
                    or_else(t.label.as_deref(), &t.from),
                    t.from
                ),
            ));
        }
        if !states.has(&t.to) {
            out.push(constraint(
                DT,
                "graph/unknown-endpoint",
                format!(
                    "Transition \"{}\" references unknown target \"{}\".",
                    or_else(t.label.as_deref(), &t.to),
                    t.to
                ),
            ));
        }
    }
    out
}

// ---- workflow ----------------------------------------------------------------------------------

/// The workflow order of work (00 section 3.1): semantic contract (short-circuits), v2 pre-routing
/// (the first failure only), then `validateWorkflow`.
fn workflow(w: &Workflow) -> Vec<Diagnostic> {
    let v2 = w.schema_version == SchemaVersion::V2;
    let semantic = semantic_contract(w);
    if !semantic.is_empty() {
        return semantic;
    }
    if v2 && let Some(first) = pre_routing(w, None) {
        return vec![first];
    }
    validate_workflow(w, v2)
}

/// What Archify throws before it lays a workflow out: the semantic contract (all of it), else for
/// v2 the first failure of `validateReadableInputsBeforeRouting`'s graph half. The rest of the graph
/// rules wait for the layout (`gates::workflow_v1`, `gates::workflow_v2`). With `accepts`
/// (`acceptsFix`: the full compile of a mutated copy), the pre-routing fixes are the verified ones.
pub fn workflow_prologue(w: &Workflow, accepts: Option<&dyn Fn(&Workflow) -> bool>) -> Vec<Diagnostic> {
    let semantic = semantic_contract(w);
    if !semantic.is_empty() {
        return semantic;
    }
    if w.schema_version == SchemaVersion::V2 && let Some(first) = pre_routing(w, accepts) {
        return vec![first];
    }
    Vec::new()
}

/// Lane order for `canonicalReadableWorkflow` (v2 sorts nodes, phases and groups before use).
fn lane_order(w: &Workflow) -> HashMap<&str, usize> {
    let mut order = HashMap::new();
    for (i, lane) in w.lanes.iter().enumerate() {
        order.entry(lane.id.as_str()).or_insert(i);
    }
    order
}

/// `semanticContractDiagnostics` (`workflow-compiler.mjs`).
fn semantic_contract(w: &Workflow) -> Vec<Diagnostic> {
    const DT: &str = "workflow";
    let Some(checks) = &w.semantic_checks else {
        return Vec::new();
    };
    // v2 iterates the canonical (lane, col, id) node order; it shows in the diagnostic order only.
    let mut nodes: Vec<&WfNode> = w.nodes.iter().collect();
    if w.schema_version == SchemaVersion::V2 {
        let lanes = lane_order(w);
        nodes.sort_by(|a, b| {
            let rank = |n: &WfNode| lanes.get(n.lane.as_str()).copied().unwrap_or(usize::MAX);
            rank(a)
                .cmp(&rank(b))
                .then(a.col.total_cmp(&b.col))
                .then(a.id.cmp(&b.id))
        });
    }
    let mut node_ids: Vec<&str> = Vec::new();
    let mut known: HashSet<&str> = HashSet::new();
    for n in &nodes {
        if known.insert(n.id.as_str()) {
            node_ids.push(n.id.as_str());
        }
    }
    let mut incoming: HashMap<&str, usize> = node_ids.iter().map(|id| (*id, 0)).collect();
    let mut outgoing = incoming.clone();
    let mut adjacency: HashMap<&str, Vec<&str>> =
        node_ids.iter().map(|id| (*id, Vec::new())).collect();
    for e in &w.edges {
        if !known.contains(e.from.as_str()) || !known.contains(e.to.as_str()) {
            continue;
        }
        *outgoing.entry(e.from.as_str()).or_default() += 1;
        *incoming.entry(e.to.as_str()).or_default() += 1;
        let next = adjacency.entry(e.from.as_str()).or_default();
        if !next.contains(&e.to.as_str()) {
            next.push(e.to.as_str());
        }
    }

    let diag = |code: &str, message: String, subject: Subject, ev: Value, fixes: Vec<String>| {
        Diagnostic::error(code, &message)
            .with_subject(subject.with_rule(code))
            .with_evidence(evidence(ev))
            .with_fixes(fixes)
    };

    let mut referenced: Vec<(&str, String)> = Vec::new();
    for (i, id) in checks.allowed_roots.iter().flatten().enumerate() {
        referenced.push((id, format!("/semanticChecks/allowedRoots/{i}")));
    }
    for (i, id) in checks.allowed_terminals.iter().flatten().enumerate() {
        referenced.push((id, format!("/semanticChecks/allowedTerminals/{i}")));
    }
    for (i, r) in checks.required_edges.iter().flatten().enumerate() {
        referenced.push((&r.from, format!("/semanticChecks/requiredEdges/{i}/from")));
        referenced.push((&r.to, format!("/semanticChecks/requiredEdges/{i}/to")));
    }
    for (i, r) in checks.required_paths.iter().flatten().enumerate() {
        referenced.push((&r.from, format!("/semanticChecks/requiredPaths/{i}/from")));
        referenced.push((&r.to, format!("/semanticChecks/requiredPaths/{i}/to")));
    }
    let mut out = Vec::new();
    for (id, path) in &referenced {
        if known.contains(id) {
            continue;
        }
        out.push(diag(
            "workflow/semantic-node-reference",
            format!("Workflow semantic contract references unknown node \"{id}\" at {path}."),
            Subject::of(DT)
                .with_extra("node", json!(id))
                .with_path(path.clone()),
            json!({ "knownNodes": node_ids }),
            vec![
                format!("replace \"{id}\" with an existing node id"),
                "add the missing node before compiling".to_owned(),
            ],
        ));
    }
    if !out.is_empty() {
        return out;
    }

    if let Some(roots) = &checks.allowed_roots {
        let allowed: HashSet<&str> = roots.iter().map(String::as_str).collect();
        let listed = dedup(roots);
        for id in &node_ids {
            if incoming[id] > 0 || allowed.contains(id) {
                continue;
            }
            out.push(diag(
                "workflow/unexpected-root",
                format!("Workflow node \"{id}\" has no incoming edge and is not declared in semanticChecks.allowedRoots."),
                Subject::of(DT)
                    .with_extra("node", json!(id))
                    .with_path("/semanticChecks/allowedRoots"),
                json!({ "incomingEdges": 0, "allowedRoots": listed }),
                vec![
                    format!("add the missing incoming edge to \"{id}\""),
                    format!("declare \"{id}\" in semanticChecks.allowedRoots if it is an intentional source"),
                ],
            ));
        }
    }
    if let Some(terminals) = &checks.allowed_terminals {
        let allowed: HashSet<&str> = terminals.iter().map(String::as_str).collect();
        let listed = dedup(terminals);
        for id in &node_ids {
            if outgoing[id] > 0 || allowed.contains(id) {
                continue;
            }
            out.push(diag(
                "workflow/unexpected-terminal",
                format!("Workflow node \"{id}\" has no outgoing edge and is not declared in semanticChecks.allowedTerminals."),
                Subject::of(DT)
                    .with_extra("node", json!(id))
                    .with_path("/semanticChecks/allowedTerminals"),
                json!({ "outgoingEdges": 0, "allowedTerminals": listed }),
                vec![
                    format!("add the missing outgoing edge from \"{id}\""),
                    format!("declare \"{id}\" in semanticChecks.allowedTerminals if it is an intentional sink"),
                ],
            ));
        }
    }
    let authored: HashSet<(&str, &str)> = w
        .edges
        .iter()
        .map(|e| (e.from.as_str(), e.to.as_str()))
        .collect();
    for (i, r) in checks.required_edges.iter().flatten().enumerate() {
        if authored.contains(&(r.from.as_str(), r.to.as_str())) {
            continue;
        }
        out.push(diag(
            "workflow/required-edge",
            format!(
                "Workflow semantic contract requires edge \"{}\" -> \"{}\", but no authored edge matches it.",
                r.from, r.to
            ),
            Subject::of(DT)
                .with_extra("from", json!(r.from))
                .with_extra("to", json!(r.to))
                .with_path(format!("/semanticChecks/requiredEdges/{i}")),
            json!({ "authoredEdgeCount": w.edges.len() }),
            vec![format!(
                "add an edge from \"{}\" to \"{}\" without deleting the semantic requirement",
                r.from, r.to
            )],
        ));
    }
    for (i, r) in checks.required_paths.iter().flatten().enumerate() {
        if reachable(&adjacency, &r.from, &r.to) {
            continue;
        }
        let mut reach: Vec<&str> = vec![r.from.as_str()];
        for next in adjacency.get(r.from.as_str()).into_iter().flatten() {
            if !reach.contains(next) {
                reach.push(next);
            }
        }
        out.push(diag(
            "workflow/required-path",
            format!(
                "Workflow semantic contract requires a directed path from \"{}\" to \"{}\", but none exists.",
                r.from, r.to
            ),
            Subject::of(DT)
                .with_extra("from", json!(r.from))
                .with_extra("to", json!(r.to))
                .with_path(format!("/semanticChecks/requiredPaths/{i}")),
            json!({ "reachableNodes": reach }),
            vec![format!(
                "restore a directed path from \"{}\" to \"{}\" without weakening the semantic requirement",
                r.from, r.to
            )],
        ));
    }
    out
}

/// A `Set` spread: unique, first-seen order.
fn dedup(ids: &[String]) -> Vec<&str> {
    let mut seen = HashSet::new();
    ids.iter()
        .map(String::as_str)
        .filter(|id| seen.insert(*id))
        .collect()
}

/// BFS over the authored edges (`reachable`).
fn reachable(adjacency: &HashMap<&str, Vec<&str>>, from: &str, to: &str) -> bool {
    let mut visited: HashSet<&str> = HashSet::from([from]);
    let mut pending = std::collections::VecDeque::from([from]);
    while let Some(current) = pending.pop_front() {
        if current == to {
            return true;
        }
        for next in adjacency.get(current).into_iter().flatten() {
            if visited.insert(next) {
                pending.push_back(next);
            }
        }
    }
    false
}

/// `-2`, `-3`, ... until the id is free (`unusedId`).
fn unused_id(base: &str, used: &HashSet<&str>) -> String {
    (2..)
        .map(|suffix| format!("{base}-{suffix}"))
        .find(|candidate| !used.contains(candidate.as_str()))
        .unwrap_or_default()
}

/// `validateReadableInputsBeforeRouting`, the graph part (v2). Archify throws the first failure:
/// duplicate lane id, duplicate node id, unknown edge endpoint, then per node (in authored order)
/// unknown lane and invalid column. Geometry (non-finite, node overlap) is not here.
fn pre_routing(w: &Workflow, accepts: Option<&dyn Fn(&Workflow) -> bool>) -> Option<Diagnostic> {
    const DT: &str = "workflow";
    let lane_ids: HashSet<&str> = w.lanes.iter().map(|l| l.id.as_str()).collect();
    let node_ids: HashSet<&str> = w.nodes.iter().map(|n| n.id.as_str()).collect();
    // A fix line: the candidate text, kept as is without a verifier; with one, `verified` wording and
    // only when the mutated copy compiles.
    let fix = |text: String, verified: String, mutate: &dyn Fn(&mut Workflow)| -> Option<String> {
        match accepts {
            None => Some(text),
            Some(accepts) => {
                let mut copy = w.clone();
                mutate(&mut copy);
                accepts(&copy).then_some(verified)
            }
        }
    };

    let mut first: HashMap<&str, usize> = HashMap::new();
    for (i, lane) in w.lanes.iter().enumerate() {
        if let Some(&prev) = first.get(lane.id.as_str()) {
            let replacement = unused_id(&lane.id, &lane_ids);
            return Some(
                Diagnostic::error(
                    "workflow/duplicate-lane-id",
                    &format!("Workflow lane id \"{}\" is duplicated.", lane.id),
                )
                .with_subject(
                    Subject::of(DT)
                        .with_rule("workflow/duplicate-lane-id")
                        .with_extra("lane", json!(lane.id))
                        .with_path(format!("/lanes/{i}/id")),
                )
                .with_evidence(evidence(json!({
                    "duplicateLaneId": lane.id,
                    "firstPath": format!("/lanes/{prev}/id"),
                    "duplicatePath": format!("/lanes/{i}/id"),
                })))
                .with_fixes(fix(
                    format!("rename /lanes/{i}/id to unique id \"{replacement}\""),
                    format!("rename /lanes/{i}/id to verified unique id \"{replacement}\""),
                    &|d| d.lanes[i].id = replacement.clone(),
                )),
            );
        }
        first.insert(lane.id.as_str(), i);
    }
    first.clear();
    for (i, node) in w.nodes.iter().enumerate() {
        if let Some(&prev) = first.get(node.id.as_str()) {
            let replacement = unused_id(&node.id, &node_ids);
            return Some(
                Diagnostic::error(
                    "workflow/duplicate-node-id",
                    &format!("Workflow node id \"{}\" is duplicated.", node.id),
                )
                .with_subject(
                    Subject::of(DT)
                        .with_rule("workflow/duplicate-node-id")
                        .with_extra("node", json!(node.id))
                        .with_path(format!("/nodes/{i}/id")),
                )
                .with_evidence(evidence(json!({
                    "duplicateNodeId": node.id,
                    "firstPath": format!("/nodes/{prev}/id"),
                    "duplicatePath": format!("/nodes/{i}/id"),
                })))
                .with_fixes(fix(
                    format!("rename /nodes/{i}/id to unique id \"{replacement}\""),
                    format!("rename /nodes/{i}/id to verified unique id \"{replacement}\""),
                    &|d| d.nodes[i].id = replacement.clone(),
                )),
            );
        }
        first.insert(node.id.as_str(), i);
    }

    let mut available: Vec<&str> = node_ids.iter().copied().collect();
    available.sort_unstable();
    for (i, e) in w.edges.iter().enumerate() {
        for (field, endpoint, id) in [("from", "source", &e.from), ("to", "target", &e.to)] {
            if node_ids.contains(id.as_str()) {
                continue;
            }
            let name =
                e.id.clone()
                    .unwrap_or_else(|| format!("{}->{}", e.from, e.to));
            return Some(
                Diagnostic::error(
                    "workflow/unknown-edge-endpoint",
                    &format!("Workflow edge \"{name}\" references unknown {endpoint} \"{id}\"."),
                )
                .with_subject(
                    Subject::of(DT)
                        .with_rule("workflow/unknown-edge-endpoint")
                        .with_extra("edge", json!(e.id))
                        .with_path(format!("/edges/{i}/{field}"))
                        .with_extra("from", json!(e.from))
                        .with_extra("to", json!(e.to)),
                )
                .with_evidence(evidence(json!({
                    "endpoint": endpoint,
                    "unknownNodeId": id,
                    "availableNodeIds": available,
                })))
                .with_fixes(available.iter().filter_map(|n| {
                    fix(
                        format!("set /edges/{i}/{field} to node id \"{n}\""),
                        format!("set /edges/{i}/{field} to verified node id \"{n}\""),
                        &|d| {
                            if field == "from" {
                                d.edges[i].from = (*n).to_owned();
                            } else {
                                d.edges[i].to = (*n).to_owned();
                            }
                        },
                    )
                })),
            );
        }
    }

    let mut lanes: Vec<&str> = lane_ids.iter().copied().collect();
    lanes.sort_unstable();
    for (i, n) in w.nodes.iter().enumerate() {
        if !lane_ids.contains(n.lane.as_str()) {
            return Some(
                Diagnostic::error(
                    "workflow/unknown-node-lane",
                    &format!(
                        "Workflow node \"{}\" uses unknown lane \"{}\".",
                        n.id, n.lane
                    ),
                )
                .with_subject(
                    Subject::of(DT)
                        .with_rule("workflow/unknown-node-lane")
                        .with_extra("node", json!(n.id))
                        .with_path(format!("/nodes/{i}/lane")),
                )
                .with_evidence(evidence(json!({
                    "unknownLaneId": n.lane,
                    "availableLaneIds": lanes,
                })))
                .with_fixes(lanes.iter().filter_map(|l| {
                    fix(
                        format!("set /nodes/{i}/lane to lane id \"{l}\""),
                        format!("set /nodes/{i}/lane to verified lane id \"{l}\""),
                        &|d| d.nodes[i].lane = (*l).to_owned(),
                    )
                })),
            );
        }
        if !is_int(n.col) || n.col < 0.0 || n.col >= WORKFLOW_COLS {
            return Some(
                Diagnostic::error(
                    "workflow/invalid-node-column",
                    &format!(
                        "Workflow node \"{}\" uses column {}, but valid columns are integers 0..{}.",
                        n.id,
                        num(n.col),
                        num(WORKFLOW_COLS - 1.0)
                    ),
                )
                .with_subject(
                    Subject::of(DT)
                        .with_rule("workflow/invalid-node-column")
                        .with_extra("node", json!(n.id))
                        .with_path(format!("/nodes/{i}/col")),
                )
                .with_evidence(evidence(json!({
                    "actualColumn": n.col,
                    "minimumColumn": 0,
                    "maximumColumn": WORKFLOW_COLS - 1.0,
                })))
                .with_fixes((0..WORKFLOW_COLS as usize).filter_map(|c| {
                    fix(
                        format!("set /nodes/{i}/col to column {c}"),
                        format!("set /nodes/{i}/col to verified column {c}"),
                        &|d| d.nodes[i].col = c as f64,
                    )
                })),
            );
        }
    }
    None
}

/// `validateWorkflow`, the graph part: ids, lanes, columns, phases, groups, endpoints, `mainPath`.
fn validate_workflow(w: &Workflow, v2: bool) -> Vec<Diagnostic> {
    const DT: &str = "workflow";
    let nodes = Index::new(&w.nodes, |n| n.id.as_str());
    let lane_ids: HashSet<&str> = w.lanes.iter().map(|l| l.id.as_str()).collect();
    let mut out = Vec::new();

    if lane_ids.len() != w.lanes.len() {
        out.push(constraint(
            DT,
            "graph/duplicate-container-id",
            "Lane ids must be unique.".to_owned(),
        ));
    }
    if nodes.len() != w.nodes.len() {
        out.push(constraint(
            DT,
            "graph/duplicate-node-id",
            "Node ids must be unique.".to_owned(),
        ));
    }
    let phases_all = w.phases.as_deref().unwrap_or_default();
    let groups_all = w.groups.as_deref().unwrap_or_default();
    if phases_all
        .iter()
        .map(|p| p.id.as_str())
        .collect::<HashSet<_>>()
        .len()
        != phases_all.len()
    {
        out.push(constraint(
            DT,
            "graph/duplicate-container-id",
            "Phase ids must be unique.".to_owned(),
        ));
    }
    if groups_all
        .iter()
        .map(|g| g.id.as_str())
        .collect::<HashSet<_>>()
        .len()
        != groups_all.len()
    {
        out.push(constraint(
            DT,
            "graph/duplicate-container-id",
            "Group ids must be unique.".to_owned(),
        ));
    }

    for n in nodes.values() {
        if !lane_ids.contains(n.lane.as_str()) {
            out.push(constraint(
                DT,
                "graph/unknown-container",
                format!("Node \"{}\" uses unknown lane \"{}\".", n.id, n.lane),
            ));
            continue;
        }
        if !is_int(n.col) || n.col < 0.0 || n.col >= WORKFLOW_COLS {
            out.push(constraint(
                DT,
                "workflow/invalid-node-column",
                format!(
                    "Node \"{}\" uses column {}, but valid columns are integers 0..{}.",
                    n.id,
                    num(n.col),
                    num(WORKFLOW_COLS - 1.0)
                ),
            ));
        }
    }

    // v2 sorts phases and groups before validating (`canonicalReadableWorkflow`).
    let mut phases: Vec<&crate::model::workflow::Phase> = phases_all.iter().collect();
    let mut groups: Vec<&crate::model::workflow::Group> = groups_all.iter().collect();
    if v2 {
        let lanes = lane_order(w);
        phases.sort_by(|a, b| {
            a.from_col
                .total_cmp(&b.from_col)
                .then(a.to_col.total_cmp(&b.to_col))
                .then(a.id.cmp(&b.id))
        });
        groups.sort_by(|a, b| {
            let rank = |g: &crate::model::workflow::Group| {
                lanes.get(g.lane.as_str()).copied().unwrap_or(usize::MAX)
            };
            rank(a)
                .cmp(&rank(b))
                .then(a.from_col.total_cmp(&b.from_col))
                .then(a.to_col.total_cmp(&b.to_col))
                .then(a.id.cmp(&b.id))
        });
    }

    let range_ok = |from: f64, to: f64| from >= 0.0 && to < WORKFLOW_COLS && from <= to;
    let mut ranges: Vec<&crate::model::workflow::Phase> = Vec::new();
    for p in &phases {
        if !is_int(p.from_col) || !is_int(p.to_col) {
            out.push(constraint(
                DT,
                "workflow/phase-range",
                format!("Phase \"{}\" must use integer fromCol/toCol values.", p.id),
            ));
            continue;
        }
        if range_ok(p.from_col, p.to_col) {
            ranges.push(p);
        } else {
            out.push(constraint(
                DT,
                "workflow/phase-range",
                format!(
                    "Phase \"{}\" uses invalid columns {}..{}; use an ordered range within 0..{}.",
                    p.id,
                    num(p.from_col),
                    num(p.to_col),
                    num(WORKFLOW_COLS - 1.0)
                ),
            ));
        }
    }
    ranges.sort_by(|a, b| {
        a.from_col
            .total_cmp(&b.from_col)
            .then(a.to_col.total_cmp(&b.to_col))
    });
    for i in 0..ranges.len() {
        for j in i + 1..ranges.len() {
            let (earlier, later) = (ranges[i], ranges[j]);
            if later.from_col > earlier.to_col {
                break;
            }
            out.push(constraint(
                DT,
                "workflow/phase-overlap",
                format!(
                    "Phase \"{}\" ({}..{}) overlaps phase \"{}\" ({}..{}) — start at col {} or later, or end the earlier phase at col {}.",
                    later.id,
                    num(later.from_col),
                    num(later.to_col),
                    earlier.id,
                    num(earlier.from_col),
                    num(earlier.to_col),
                    num(earlier.to_col + 1.0),
                    num(later.from_col - 1.0)
                ),
            ));
        }
    }

    for g in &groups {
        if !lane_ids.contains(g.lane.as_str()) {
            out.push(constraint(
                DT,
                "graph/unknown-container",
                format!("Group \"{}\" uses unknown lane \"{}\".", g.id, g.lane),
            ));
            continue;
        }
        if !is_int(g.from_col) || !is_int(g.to_col) {
            out.push(constraint(
                DT,
                "workflow/group-range",
                format!("Group \"{}\" must use integer fromCol/toCol values.", g.id),
            ));
            continue;
        }
        if !range_ok(g.from_col, g.to_col) {
            out.push(constraint(
                DT,
                "workflow/group-range",
                format!(
                    "Group \"{}\" uses invalid columns {}..{}; use an ordered range within 0..{}.",
                    g.id,
                    num(g.from_col),
                    num(g.to_col),
                    num(WORKFLOW_COLS - 1.0)
                ),
            ));
        }
        let contained = nodes
            .values()
            .any(|n| n.lane == g.lane && n.col >= g.from_col && n.col <= g.to_col);
        if !contained {
            out.push(constraint(
                DT,
                "workflow/group-empty",
                format!(
                    "Group \"{}\" does not contain any nodes — align its lane/columns with the parallel or branch work it frames.",
                    g.id
                ),
            ));
        }
    }

    for e in &w.edges {
        if !nodes.has(&e.from) {
            out.push(constraint(
                DT,
                "graph/unknown-endpoint",
                format!(
                    "Edge \"{}\" references unknown source \"{}\".",
                    or_else(e.label.as_deref(), &e.from),
                    e.from
                ),
            ));
        }
        if !nodes.has(&e.to) {
            out.push(constraint(
                DT,
                "graph/unknown-endpoint",
                format!(
                    "Edge \"{}\" references unknown target \"{}\".",
                    or_else(e.label.as_deref(), &e.to),
                    e.to
                ),
            ));
        }
    }

    if let Some(path) = &w.main_path {
        for id in path {
            if !nodes.has(id) {
                out.push(constraint(
                    DT,
                    "workflow/main-path",
                    format!("mainPath references unknown node \"{id}\"."),
                ));
            }
        }
        for pair in path.windows(2) {
            let (from_id, to_id) = (&pair[0], &pair[1]);
            let (Some(from), Some(to)) = (nodes.get(from_id), nodes.get(to_id)) else {
                continue;
            };
            if !w.edges.iter().any(|e| &e.from == from_id && &e.to == to_id) {
                out.push(constraint(
                    DT,
                    "workflow/main-path",
                    format!("mainPath step \"{from_id}\" -> \"{to_id}\" has no matching edge — add the edge or remove the pair from mainPath."),
                ));
            }
            if to.col < from.col {
                out.push(constraint(
                    DT,
                    "workflow/main-path",
                    format!(
                        "mainPath step \"{from_id}\" -> \"{to_id}\" moves backward from col {} to {} — use a return edge outside mainPath for loops.",
                        num(from.col),
                        num(to.col)
                    ),
                ));
            }
        }
    }
    out
}
