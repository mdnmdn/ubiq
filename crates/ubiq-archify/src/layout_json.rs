//! `--layout-json` receipts (P3.6, `00` section 5.10, `01` section 5): the architecture layout as
//! Archify's `validate architecture --layout-json` prints it, and nothing else.
//!
//! ```text
//! { ok, diagram_type, layout: {mode: "free"} | {mode: "grid", origin, cols, gapX, gapY, cellW, cellH},
//!   viewBox, components[{id, type, label, x, y, width, height, row?, col?, pos?}],
//!   boundaries[{kind, label, x, y, width, height, wraps}],
//!   connections[{from, to, label, variant, route, points, labelAt?}],
//!   labels[{text, x, y, width, height, labelAt}] }
//! ```
//!
//! Coordinates of components, boundaries and labels are `Math.round`ed; route `points` are not
//! (they are repair inputs). A layout the gates reject is still measurable: the same report with
//! `ok: false`, `schemaVersion`, `source: "renderer"`, `error`, `diagnostics` and
//! `contract: "archify-architecture-layout-v1"` (exit 1). Any failure before the layout (parse,
//! schema, relationship ids, engineering profile) is the ordinary `validate` failure receipt, and
//! an authored legend that does not fit never appears here: Archify checks it when it draws.

use serde_json::{Map, Value, json};

use crate::compile::{self, Layout, Opts, WorkflowGeometry};
use crate::diag::{Diagnostic, Receipt, Stage, js_round, json_num};
use crate::geom::{Pt, Rect, Side};
use crate::layout::architecture::{PlacedComponent, grid_of};
use crate::layout::architecture_build::{ArchitectureGeometry, variant_of};
use crate::model::Doc;
use crate::model::architecture::{Architecture, BoundaryKind, ConnectionRoute};

/// `contract` of a rejected layout.
pub const ARCHITECTURE_CONTRACT: &str = "archify-architecture-layout-v1";

/// What a layout request answers.
#[derive(Debug)]
pub enum LayoutReply {
    /// The layout report (also a rejected one, `ok: false`) and the exit code (0, or 1 when rejected).
    Layout { json: Value, exit_code: i32 },
    /// The document never reached the layout: Archify's ordinary failure receipt.
    Failed(Box<Receipt>),
}

impl LayoutReply {
    /// The JSON to print: the report, or the failure receipt.
    pub fn json(&self) -> Value {
        match self {
            LayoutReply::Layout { json, .. } => json.clone(),
            LayoutReply::Failed(receipt) => serde_json::to_value(receipt.as_ref()).unwrap_or(Value::Null),
        }
    }

    pub fn exit_code(&self) -> i32 {
        match self {
            LayoutReply::Layout { exit_code, .. } => *exit_code,
            LayoutReply::Failed(receipt) => receipt.exit_code(),
        }
    }
}

/// A JS `Math.round` result as a JSON number (`NaN` is `null`, as `JSON.stringify` prints it).
fn rounded(x: f64) -> Value {
    json_num(js_round(x))
}

fn rounded_point(p: Pt) -> Value {
    Value::Array(vec![rounded(p[0]), rounded(p[1])])
}

fn kind_name(kind: BoundaryKind) -> &'static str {
    match kind {
        BoundaryKind::Region => "region",
        BoundaryKind::SecurityGroup => "security-group",
    }
}

fn name_of<T: serde::Serialize>(value: &T) -> Value {
    serde_json::to_value(value).unwrap_or(Value::Null)
}

fn is_integer(v: Option<f64>) -> Option<f64> {
    v.filter(|n| n.is_finite() && n.fract() == 0.0)
}

/// `componentBox` (`shared/layout-report.mjs`).
pub fn component_box(doc: &Architecture, pc: &PlacedComponent) -> Value {
    let c = &doc.components[pc.index];
    let mut m = Map::new();
    m.insert("id".into(), pc.id.clone().into());
    m.insert("type".into(), name_of(&c.kind));
    m.insert("label".into(), c.label.clone().into());
    m.insert("x".into(), rounded(pc.rect.x));
    m.insert("y".into(), rounded(pc.rect.y));
    m.insert("width".into(), json_num(pc.rect.width));
    m.insert("height".into(), json_num(pc.rect.height));
    if let Some(row) = is_integer(c.row) {
        m.insert("row".into(), json_num(row));
    }
    if let Some(col) = is_integer(c.col) {
        m.insert("col".into(), json_num(col));
    }
    if let Some(pos) = c.pos {
        m.insert("pos".into(), rounded_point(pos));
    }
    Value::Object(m)
}

/// `buildLayoutReport` of `render-architecture.mjs`: the `ok: true` report of `g`.
pub fn architecture_report(doc: &Architecture, g: &ArchitectureGeometry) -> Map<String, Value> {
    let p = &g.placement;
    let connections = doc.connections.as_deref().unwrap_or(&[]);
    let layout = match grid_of(doc) {
        Some(grid) => json!({
            "mode": "grid",
            "origin": [json_num(grid.origin[0]), json_num(grid.origin[1])],
            "cols": json_num(grid.cols),
            "gapX": json_num(grid.gap_x),
            "gapY": json_num(grid.gap_y),
            "cellW": json_num(grid.cell_w),
            "cellH": json_num(grid.cell_h),
        }),
        None => json!({"mode": "free"}),
    };
    let boundaries: Vec<Value> = p
        .boundaries
        .iter()
        .map(|b| {
            json!({
                "kind": kind_name(b.raw.kind),
                "label": b.raw.label,
                "x": rounded(b.rect.x),
                "y": rounded(b.rect.y),
                "width": rounded(b.rect.width),
                "height": rounded(b.rect.height),
                "wraps": b.raw.wraps,
            })
        })
        .collect();
    let routed: Vec<Value> = g
        .connections
        .iter()
        .map(|geom| {
            let c = &connections[geom.index];
            let mut m = Map::new();
            m.insert("from".into(), c.from.clone().into());
            m.insert("to".into(), c.to.clone().into());
            m.insert("label".into(), c.label.clone().map_or(Value::Null, Value::from));
            m.insert("variant".into(), variant_of(c.variant).into());
            m.insert("route".into(), name_of(&c.route.unwrap_or(ConnectionRoute::Auto)));
            m.insert("points".into(), Value::Array(geom.points.iter().map(|q| Value::Array(vec![json_num(q[0]), json_num(q[1])])).collect()));
            if let Some(l) = geom.label.as_ref().filter(|_| c.label.as_deref().is_some_and(|t| !t.is_empty())) {
                m.insert("labelAt".into(), rounded_point(l.at));
            }
            Value::Object(m)
        })
        .collect();
    let labels: Vec<Value> = g
        .connections
        .iter()
        .filter_map(|geom| geom.label.as_ref())
        .map(|l| {
            json!({
                "text": l.text,
                "x": rounded(l.rect.x),
                "y": rounded(l.rect.y),
                "width": rounded(l.rect.width),
                "height": 14,
                "labelAt": rounded_point(l.at),
            })
        })
        .collect();
    let mut m = Map::new();
    m.insert("ok".into(), true.into());
    m.insert("diagram_type".into(), "architecture".into());
    m.insert("layout".into(), layout);
    m.insert("viewBox".into(), json!([json_num(p.view_box[0]), json_num(p.view_box[1])]));
    m.insert("components".into(), Value::Array(p.components.iter().map(|pc| component_box(doc, pc)).collect()));
    m.insert("boundaries".into(), Value::Array(boundaries));
    m.insert("connections".into(), Value::Array(routed));
    m.insert("labels".into(), Value::Array(labels));
    m
}

/// The layout receipt of an architecture document (`validate architecture <file> --layout-json`).
///
/// Runs the same stages as [`compile::compile`], so the `quality` of `opts` reaches the gates the
/// way `--quality` does; the layout itself never depends on it.
pub fn architecture(text: &str, opts: &Opts<'_>) -> LayoutReply {
    architecture_with(text, opts, None)
}

/// [`architecture`] with the host's repository-evidence verifier (`--repo-root`).
pub fn architecture_with(text: &str, opts: &Opts<'_>, verify: Option<compile::EvidenceCheck<'_>>) -> LayoutReply {
    let compiled = compile::compile_with(text, &Opts { doc_type: Some("architecture"), ..opts.clone() }, verify);
    let Some(Doc::Architecture(doc)) = &compiled.doc else {
        return LayoutReply::Failed(Box::new(compiled.receipt));
    };
    let rejected = |compiled: &compile::Compiled| {
        (compiled.receipt.error.clone().unwrap_or_default(), serde_json::to_value(&compiled.receipt.diagnostics).unwrap_or(Value::Null))
    };
    match (&compiled.layout, &compiled.architecture) {
        (Some(Layout::Architecture { geometry, .. }), _) => {
            LayoutReply::Layout { json: Value::Object(architecture_report(doc, geometry)), exit_code: 0 }
        }
        (_, Some(run)) if run.valid => {
            LayoutReply::Layout { json: Value::Object(architecture_report(doc, &run.geometry)), exit_code: 0 }
        }
        (_, Some(run)) => {
            let mut m = architecture_report(doc, &run.geometry);
            let (error, diagnostics) = rejected(&compiled);
            m.insert("ok".into(), false.into());
            m.insert("schemaVersion".into(), 1.into());
            m.insert("source".into(), "renderer".into());
            m.insert("error".into(), error.into());
            m.insert("diagnostics".into(), diagnostics);
            m.insert("contract".into(), ARCHITECTURE_CONTRACT.into());
            LayoutReply::Layout { json: Value::Object(m), exit_code: 1 }
        }
        _ => LayoutReply::Failed(Box::new(compiled.receipt)),
    }
}

/// `contract` of the dataflow, sequence and lifecycle receipts. Archify prints none for them
/// (`--layout-json` is architecture only, `00` section 5.10), so these are this crate's own: the
/// resolved geometry, in the shape of the architecture report.
pub const CONTRACT: &str = "ubiq-archify-layout-v1";

fn rect_json(r: &Rect) -> Map<String, Value> {
    let mut m = Map::new();
    m.insert("x".into(), rounded(r.x));
    m.insert("y".into(), rounded(r.y));
    m.insert("width".into(), rounded(r.width));
    m.insert("height".into(), rounded(r.height));
    m
}

fn rect_with(id: &str, r: &Rect) -> Map<String, Value> {
    let mut m = Map::new();
    m.insert("id".into(), id.into());
    m.extend(rect_json(r));
    m
}

fn label_json(text: Option<&str>, rect: &Rect, at: Pt) -> Value {
    let mut m = Map::new();
    if let Some(text) = text {
        m.insert("text".into(), text.into());
    }
    m.extend(rect_json(rect));
    m.insert("labelAt".into(), rounded_point(at));
    Value::Object(m)
}

fn points_json(points: &[Pt]) -> Value {
    Value::Array(points.iter().map(|q| Value::Array(vec![json_num(q[0]), json_num(q[1])])).collect())
}

fn side_name(side: Side) -> &'static str {
    match side {
        Side::Left => "left",
        Side::Right => "right",
        Side::Top => "top",
        Side::Bottom => "bottom",
    }
}

fn view_box_json(vb: [f64; 2]) -> Value {
    json!([json_num(vb[0]), json_num(vb[1])])
}

/// The receipt of a laid-out dataflow, sequence or lifecycle document: node rects, route
/// points, label rects and the viewBox, from the same geometry the gates and the painter read.
/// `None` for the other types.
pub fn report(compiled: &compile::Compiled) -> Option<Value> {
    let mut m = Map::new();
    m.insert("ok".into(), true.into());
    m.insert("contract".into(), CONTRACT.into());
    match (&compiled.layout, &compiled.doc) {
        (Some(Layout::Dataflow { geometry: g, .. }), Some(Doc::Dataflow(doc))) => {
            m.insert("diagram_type".into(), "dataflow".into());
            m.insert("viewBox".into(), view_box_json(g.view_box));
            m.insert("frames".into(), Value::Array(g.frames.iter().map(|r| Value::Object(rect_json(r))).collect()));
            let nodes = g.nodes.iter().map(|n| {
                let mut o = rect_with(&n.id, &n.rect);
                o.insert("stage".into(), n.stage.into());
                o.insert("row".into(), n.row.into());
                Value::Object(o)
            });
            m.insert("nodes".into(), nodes.collect());
            let flows = g.flows.iter().map(|f| {
                let mut o = Map::new();
                o.insert("from".into(), f.from.clone().into());
                o.insert("to".into(), f.to.clone().into());
                o.insert("fromSide".into(), side_name(f.from_side).into());
                o.insert("toSide".into(), side_name(f.to_side).into());
                o.insert("points".into(), points_json(&f.points));
                let text = doc.flows.get(f.key).map(|flow| flow.label.as_str());
                o.insert("label".into(), label_json(text, &f.label.rect, f.label.at));
                Value::Object(o)
            });
            m.insert("flows".into(), flows.collect());
        }
        (Some(Layout::Sequence { geometry: g, .. }), Some(Doc::Sequence(doc))) => {
            m.insert("diagram_type".into(), "sequence".into());
            m.insert("viewBox".into(), view_box_json(g.view_box));
            m.insert("lifelineEnd".into(), rounded(g.lifeline_end));
            let participants = g.participants.iter().map(|p| {
                let mut o = rect_with(&p.id, &p.rect);
                o.insert("cx".into(), rounded(p.cx));
                Value::Object(o)
            });
            m.insert("participants".into(), participants.collect());
            let messages = g.messages.iter().map(|msg| {
                let mut o = Map::new();
                o.insert("from".into(), msg.from.clone().into());
                o.insert("to".into(), msg.to.clone().into());
                o.insert("points".into(), points_json(&msg.points));
                let text = doc.messages.get(msg.key).map(|x| x.label.as_str());
                o.insert("label".into(), label_json(text, &msg.label, msg.label_at));
                Value::Object(o)
            });
            m.insert("messages".into(), messages.collect());
            let segments = g.segments.iter().map(|s| {
                json!({"frame": Value::Object(rect_json(&s.frame)), "label": Value::Object(rect_json(&s.label))})
            });
            m.insert("segments".into(), segments.collect());
            m.insert("activations".into(), Value::Array(g.activations.iter().map(|r| Value::Object(rect_json(r))).collect()));
        }
        (Some(Layout::Lifecycle { geometry: g, .. }), Some(Doc::Lifecycle(doc))) => {
            m.insert("diagram_type".into(), "lifecycle".into());
            m.insert("viewBox".into(), view_box_json(g.view_box));
            let states = g.states.iter().map(|s| {
                let mut o = rect_with(&s.id, &s.rect);
                o.insert("final".into(), s.final_border.into());
                o.insert("initial".into(), s.initial_marker.into());
                Value::Object(o)
            });
            m.insert("states".into(), states.collect());
            let transitions = g.transitions.iter().map(|t| {
                let mut o = Map::new();
                o.insert("from".into(), t.from.clone().into());
                o.insert("to".into(), t.to.clone().into());
                o.insert("fromSide".into(), side_name(t.from_side).into());
                o.insert("toSide".into(), side_name(t.to_side).into());
                o.insert("points".into(), points_json(&t.points));
                if let Some(l) = &t.label {
                    let text = doc.transitions.get(t.key).and_then(|x| x.label.as_deref());
                    o.insert("label".into(), label_json(text, &l.rect, l.at));
                }
                Value::Object(o)
            });
            m.insert("transitions".into(), transitions.collect());
            let bands = g.bands.iter().map(|b| {
                let mut o = rect_json(&b.rect);
                o.insert("label".into(), b.label.clone().into());
                Value::Object(o)
            });
            m.insert("bands".into(), bands.collect());
        }
        _ => return None,
    }
    Some(Value::Object(m))
}

/// The layout receipt of a dataflow, sequence or lifecycle document: [`report`], or the ordinary
/// failure receipt when the document did not reach its layout (the gates reject it: there is no
/// rejected-layout report for these types).
pub fn other(text: &str, opts: &Opts<'_>) -> LayoutReply {
    other_with(text, opts, None)
}

/// [`other`] with the host's repository-evidence verifier (`--repo-root`).
pub fn other_with(text: &str, opts: &Opts<'_>, verify: Option<compile::EvidenceCheck<'_>>) -> LayoutReply {
    let compiled = compile::compile_with(text, opts, verify);
    match report(&compiled) {
        Some(json) => LayoutReply::Layout { json, exit_code: 0 },
        None => LayoutReply::Failed(Box::new(compiled.receipt)),
    }
}

/// The workflow compiler's receipt (`compileWorkflow().receipt`, what `render-workflow.mjs
/// --layout-json` prints): `contract` is `fixed-v1` or `readable-v2`.
///
/// ```text
/// { contract, viewBox, requiredViewBox, columns[6],
///   nodes[{id, lane, col, x, y, width, height}], edges[{id, from, to, points}],
///   labels[{edge, label, x, y, width, height}], diagnostics }
/// ```
///
/// Nothing is rounded. A rejected layout is `{contract, diagnostics}` (exit 1): the compiler's
/// diagnostics, one per problem in problem order. `diagnostics` of a clean v2 layout are its warnings.
pub fn workflow_receipt(contract: &str, geometry: &WorkflowGeometry, diagnostics: &[Diagnostic]) -> Value {
    let point = |p: &Pt| json!([json_num(p[0]), json_num(p[1])]);
    let edge_id = |id: &Option<String>| id.clone().map_or(Value::Null, Value::from);
    let node = |id: &str, lane: &str, col: f64, r: &Rect| {
        json!({"id": id, "lane": lane, "col": json_num(col), "x": json_num(r.x), "y": json_num(r.y), "width": json_num(r.width), "height": json_num(r.height)})
    };
    let label = |id: &Option<String>, text: &str, at: Pt, width: f64, height: f64| {
        json!({"edge": edge_id(id), "label": text, "x": json_num(at[0]), "y": json_num(at[1]), "width": json_num(width), "height": json_num(height)})
    };
    let (view_box, required, columns, nodes, edges, labels): (_, _, Vec<f64>, Vec<Value>, Vec<Value>, Vec<Value>) = match geometry {
        WorkflowGeometry::V1(g) => (
            g.view_box,
            g.required_view_box,
            g.columns.to_vec(),
            g.nodes.iter().map(|n| node(&n.id, &n.lane, n.col, &n.rect)).collect(),
            g.edges
                .iter()
                .map(|e| json!({"id": edge_id(&e.id), "from": e.from, "to": e.to, "points": e.points.iter().map(point).collect::<Vec<_>>()}))
                .collect(),
            g.edges.iter().filter_map(|e| e.label.as_ref().map(|l| label(&e.id, &l.text, l.at, l.rect.width, l.rect.height))).collect(),
        ),
        WorkflowGeometry::V2(g) => {
            let w = g.placement.workflow();
            (
                g.view_box,
                g.required_view_box,
                g.placement.layout.col_xs.to_vec(),
                g.placement.nodes.iter().map(|n| node(&n.id, &n.lane, n.col, &n.rect)).collect(),
                g.edges
                    .iter()
                    .map(|r| {
                        let e = &w.edges[r.index];
                        json!({"id": edge_id(&e.id), "from": e.from, "to": e.to, "points": r.points.iter().map(point).collect::<Vec<_>>()})
                    })
                    .collect(),
                g.labels.iter().map(|l| label(&w.edges[l.index].id, &l.text, l.at, l.rect.width, l.rect.height)).collect(),
            )
        }
    };
    json!({
        "contract": contract,
        "viewBox": view_box_json(view_box),
        "requiredViewBox": view_box_json(required),
        "columns": columns.iter().map(|c| json_num(*c)).collect::<Vec<_>>(),
        "nodes": nodes,
        "edges": edges,
        "labels": labels,
        "diagnostics": serde_json::to_value(diagnostics).unwrap_or(Value::Null),
    })
}

/// `validate workflow <file> --layout-json` (and `archify_layout`): [`workflow_receipt`] for a
/// document that reached the workflow compiler, laid out or rejected; anything that failed before
/// it (parse, schema, relationship ids) is the ordinary failure receipt.
pub fn workflow(text: &str, opts: &Opts<'_>) -> LayoutReply {
    workflow_with(text, opts, None)
}

/// [`workflow`] with the host's repository-evidence verifier (`--repo-root`).
pub fn workflow_with(text: &str, opts: &Opts<'_>, verify: Option<compile::EvidenceCheck<'_>>) -> LayoutReply {
    let compiled = compile::compile_with(text, &Opts { doc_type: Some("workflow"), ..opts.clone() }, verify);
    let Some(run) = &compiled.workflow else {
        return LayoutReply::Failed(Box::new(compiled.receipt));
    };
    match &compiled.layout {
        Some(Layout::Workflow { geometry, .. }) => {
            LayoutReply::Layout { json: workflow_receipt(run.contract, geometry, &run.diagnostics), exit_code: 0 }
        }
        _ => LayoutReply::Layout {
            json: json!({"contract": run.contract, "diagnostics": serde_json::to_value(&run.diagnostics).unwrap_or(Value::Null)}),
            exit_code: 1,
        },
    }
}

/// The layout report of the type `opts.doc_type` names, or `None` when it names none of the five
/// (the caller then has the validate failure to say why).
pub fn by_type(text: &str, opts: &Opts<'_>) -> Option<LayoutReply> {
    by_type_with(text, opts, None)
}

/// [`by_type`] with the host's repository-evidence verifier (`--repo-root`).
pub fn by_type_with(text: &str, opts: &Opts<'_>, verify: Option<compile::EvidenceCheck<'_>>) -> Option<LayoutReply> {
    match opts.doc_type {
        Some("architecture") => Some(architecture_with(text, opts, verify)),
        Some("dataflow" | "sequence" | "lifecycle") => Some(other_with(text, opts, verify)),
        Some("workflow") => Some(workflow_with(text, opts, verify)),
        _ => None,
    }
}

/// `--layout-json` on a type with no layout receipt: `cli/unsupported-option`, exit 2. Every type
/// has one now, so only an unknown one reaches it.
pub fn unsupported(doc_type: &str, input: &str) -> Receipt {
    let message = "--layout-json is currently supported for architecture and workflow diagrams only.";
    let diagnostic = Diagnostic::error("cli/unsupported-option", message)
        .with_subject(crate::diag::Subject::default().with_extra("option", "--layout-json".into()).with_extra("type", doc_type.into()))
        .with_fixes(["remove --layout-json or use an architecture or workflow diagram"]);
    Receipt::failure("validate", Stage::Arguments, doc_type, input, message, vec![diagnostic])
}
