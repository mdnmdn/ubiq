//! `validateWorkflow` for `schema_version` 1 (`workflow-compiler.mjs:2502-2836`), measured on the
//! [`LegacyPlan`] the layout draws: `workflow/column-capacity` first (it throws alone), then every
//! problem of one workflow document in Archify's order, graph rules and geometry rules together.
//!
//! Rules: unique lane, node, phase and group ids; per node the lane, the column (integer `0..5`),
//! finite geometry, the label width (6.8 per unit + 6), the brand rail, sublabel/tag at their 6 px
//! minimum, the lane bounds (`x` inside the 640 px lane, `y` under the title strip and above the lane
//! bottom); phases (integer, ordered, in range, label width, no overlap); groups (lane, integer,
//! ordered, in range, at least one node); nodes 8 px apart within a lane; per edge the endpoints and
//! the 28 px length; then the shared gates (`clean-flow/*`, `composition/*`), `mainPath`, label/node
//! and label/label collisions, label/route clearance, and the v1 canvas: the lane width (viewBox width
//! at least 696), the legend inside the height, and the label canvas containment.
//!
//! # Where this differs from the lifecycle gate
//!
//! - `cleanAmbiguousCorridorProblems` hands its rich diagnostic to `workflowDiagnostics`, not to
//!   `recordDiagnostic`: it takes the place of its problem (`Sink::detailed`) instead of coming first.
//! - [`Review::compiler`] is the compiler's own view of the same problems (`throwDiagnosticProblems`:
//!   the layout-json receipt's `diagnostics`): one per problem in problem order, a plain
//!   `layout/constraint` unless the corridor gate attached details. [`Outcome::diagnostics`] is the
//!   `validate --json` view (the recorded rich diagnostics first).
//! - `column-capacity` throws one diagnostic with its own message as the error text
//!   ([`Review::error`]). Its `supportedFixes` are the verified ones Archify finds by recompiling a
//!   mutated copy: "migrate to schema_version 2" (the v2 compile of the migrated document,
//!   [`migration_provides_capacity`]), the free column and the reduced widths.

use serde_json::Value;

use super::clean_flow::{edge_through_node, endpoint_side};
use super::composition::{
    FrameInfo, ambiguous_corridor, container_border_run, label_canvas_containment, label_route_clearance, proper_crossing,
    route_rhythm,
};
use super::suggest::{label_obstacle_fix, label_pair_fix};
use super::{CheckedSide, Ctx, Gate, LabelRect, Obstacle, Outcome, PLAN_KEYS, Rel, SideOrigin, Sink, evidence, num};
use crate::diag::{Diagnostic, Subject, js_round};
use crate::geom::{Frame, Rect, is_finite_point, rects_overlap};
use super::workflow_v2;
use crate::layout::workflow::legacy::{
    COLUMNS, LANE_TITLE_H, LANE_W, LANE_X, LegacyPlan, NodeBox, build_plan, edge_width,
};
use crate::layout::workflow::migrate::{LEGACY_COLUMN_CENTERS, intrinsic_workflow, mapped_candidate, planning_workflow};
use crate::layout::workflow::readable::{LayoutFeedback, place};
use crate::model::common::SchemaVersion;
use crate::model::workflow::{Edge, EdgeRoute, Workflow};
use crate::route::dataflow::side as model_side;
use crate::text::{self, DiagramType};

pub(super) const CTX: Ctx<'static> = Ctx { diagram: "workflow", collection: "edges" };

/// `nodeTextFit.labelMinimum` (the brand rail check), `sublabelMinimum` and `tagMinimum`.
pub(super) const LABEL_MINIMUM: f64 = 9.0;
pub(super) const TEXT_MINIMUM: f64 = 6.0;
/// Nodes of one lane closer than this overlap; the shortest edge.
pub(super) const NODE_GAP: f64 = 8.0;
pub(super) const MIN_EDGE: f64 = 28.0;
/// `rectsOverlap(label, obstacle, -2)`: labels may touch by 2 px.
pub(super) const LABEL_GAP: f64 = -2.0;
/// The lane stays `LANE_W` wide, so the canvas needs `laneX + laneW + 16`.
const CANVAS_MARGIN: f64 = 16.0;
/// `legendY() + 18` must fit the canvas height.
const LEGEND_BOTTOM_PAD: f64 = 18.0;

/// What the gates measured, for the success receipt and for the compiler's failure receipt.
#[derive(Debug, Default, Clone)]
pub struct Review {
    pub rels: Vec<Rel>,
    pub labels: Vec<LabelRect>,
    /// The lanes, exception frames and groups the border-run gate checks.
    pub frames: Vec<FrameInfo>,
    pub view_box: [f64; 2],
    /// The thrown error text when it is not the usual `Workflow layout validation failed:` list
    /// (`column-capacity` throws its own message).
    pub error: Option<String>,
    /// `throwDiagnosticProblems`' diagnostics, in problem order (the layout-json receipt's).
    pub compiler: Vec<Diagnostic>,
    /// `workflowDiagnostics` that are not problems: v2's warnings below showcase (always empty in v1).
    pub warnings: Vec<Diagnostic>,
}

/// The rule id (D7) a plain sentence of `validateWorkflow` carries in `subject.rule`.
pub(super) fn rule_of(message: &str) -> &'static str {
    let has = |s: &str| message.contains(s);
    if message == "Node ids must be unique." {
        "graph/duplicate-node-id"
    } else if message.ends_with("ids must be unique.") {
        "graph/duplicate-container-id"
    } else if has("uses unknown lane") {
        "graph/unknown-container"
    } else if has("valid columns are integers") {
        "workflow/invalid-node-column"
    } else if message.starts_with("Phase ") && (has("integer fromCol") || has("uses invalid columns")) {
        "workflow/phase-range"
    } else if message.starts_with("Group ") && (has("integer fromCol") || has("uses invalid columns")) {
        "workflow/group-range"
    } else if has("overlaps phase") {
        "workflow/phase-overlap"
    } else if has("does not contain any nodes") {
        "workflow/group-empty"
    } else if has("non-finite coordinates") {
        "geom/non-finite"
    } else if has("brand top rail") {
        "geom/brand-rail"
    } else if has("legible minimum") {
        "geom/text-min-fit"
    } else if has("exceeds the horizontal bounds") || has("collides with the title or boundary") {
        "geom/node-out-of-bounds"
    } else if has("apart in lane") {
        "geom/node-overlap"
    } else if has("references unknown") && message.starts_with("Edge ") {
        "graph/unknown-endpoint"
    } else if has("is too short") {
        "geom/edge-too-short"
    } else if message.starts_with("mainPath") {
        "workflow/main-path"
    } else if has("overlaps node") {
        "geom/label-node-overlap"
    } else if message.starts_with("Labels ") {
        "geom/label-label-overlap"
    } else if message.starts_with("viewBox width") || message.starts_with("Legend exceeds") {
        "workflow/v1-canvas"
    } else {
        "geom/label-too-wide"
    }
}

/// The authored routing controls of an edge, in [`PLAN_KEYS`] order.
pub(super) fn controls(e: &Edge) -> Vec<&'static str> {
    let present = |key: &str| match key {
        "route" => e.route.is_some(),
        "via" => e.via.is_some(),
        "channelX" => e.channel_x.is_some(),
        "channelY" => e.channel_y.is_some(),
        "fromSide" => e.from_side.is_some(),
        "toSide" => e.to_side.is_some(),
        "bias" => e.bias.is_some(),
        "labelAt" => e.label_at.is_some(),
        "labelDx" => e.label_dx.is_some(),
        "labelDy" => e.label_dy.is_some(),
        "labelSegment" => e.label_segment.is_some(),
        _ => false,
    };
    PLAN_KEYS.into_iter().filter(|k| present(k)).collect()
}

/// `verticalIntervalsOverlap(a, b, clearance)`: the two boxes' vertical extents, about their lane
/// centre, come within `clearance`.
fn vertical_intervals_overlap(plan: &LegacyPlan<'_>, a: &NodeBox, b: &NodeBox, clearance: f64) -> bool {
    let offset = |n: &NodeBox| plan.doc.nodes[n.index].y_offset.filter(|o| o.is_finite()).unwrap_or(0.0);
    (offset(a) - offset(b)).abs() < a.rect.height / 2.0 + b.rect.height / 2.0 + clearance
}

/// `acceptsFix`: the document with `mutate` applied lays out and validates clean (with no fix search of
/// its own).
fn accepts_fix(plan: &LegacyPlan<'_>, gate: Gate, mutate: impl FnOnce(&mut Workflow)) -> bool {
    let mut candidate = plan.doc.clone();
    mutate(&mut candidate);
    let planned = LegacyPlan::new(&candidate);
    let (outcome, _) = validate(&planned, gate, false);
    outcome.ok() && build_plan(&planned).is_ok()
}

/// `verifiedLegacyAlternative`: the nearest free column of the target's lane that clears `required`
/// from the source and that Archify accepts, if any.
fn verified_alternative(plan: &LegacyPlan<'_>, gate: Gate, from: &NodeBox, to: &NodeBox, required: f64) -> Option<usize> {
    let to_node = &plan.doc.nodes[to.index];
    let to_width = to.rect.width;
    let mut candidates: Vec<usize> = (0..COLUMNS.len()).filter(|&c| c as f64 != to.col).collect();
    candidates.sort_by(|&a, &b| {
        let (da, db) = ((a as f64 - to.col).abs(), (b as f64 - to.col).abs());
        da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal).then(a.cmp(&b))
    });
    for col in candidates {
        let center = COLUMNS[col];
        let candidate = Rect::new(center - to_width / 2.0, to.rect.y, to_width, to.rect.height);
        let occupied = plan.nodes.iter().filter(|n| n.lane == to.lane && n.id != to.id);
        if occupied.into_iter().any(|n| rects_overlap(&candidate, &n.rect, NODE_GAP)) {
            continue;
        }
        let clearance = (center - from.cx).abs() - from.rect.width / 2.0 - to_width / 2.0;
        if clearance < required {
            continue;
        }
        let id = to_node.id.clone();
        if accepts_fix(plan, gate, |doc| {
            if let Some(n) = doc.nodes.iter_mut().find(|n| n.id == id) {
                n.col = col as f64;
            }
        }) {
            return Some(col);
        }
    }
    None
}

/// `verifiedReducedWidths`: shave the wider node first (never below 32 px) until both fit `required`
/// clearance, if the labels still fit and Archify accepts the result.
fn verified_widths(plan: &LegacyPlan<'_>, gate: Gate, from: &NodeBox, to: &NodeBox, required: f64) -> Option<[f64; 2]> {
    let budget = 2.0 * ((to.cx - from.cx).abs() - required);
    if budget < 64.0 {
        return None;
    }
    let mut widths = [from.rect.width, to.rect.width];
    let mut excess = widths[0] + widths[1] - budget;
    let order = if widths[0] >= widths[1] { [0, 1] } else { [1, 0] };
    for index in order {
        let reduction = excess.min(widths[index] - 32.0);
        widths[index] -= reduction;
        excess -= reduction;
    }
    if excess > 0.0001 {
        return None;
    }
    let fits = [from, to].into_iter().zip(widths).all(|(n, width)| {
        let node = &plan.doc.nodes[n.index];
        let available = text::available_width(width);
        text::units(&node.label) as f64 * 6.8 <= width + 6.0
            && node.sublabel.as_deref().filter(|s| !s.is_empty()).is_none_or(|s| text::min_text_width(s, TEXT_MINIMUM) <= available)
            && node.tag.as_deref().filter(|s| !s.is_empty()).is_none_or(|t| text::min_text_width(t, TEXT_MINIMUM) <= available)
    });
    if !fits {
        return None;
    }
    let serialized = widths.map(|w| ((w + 1e-9) * 100.0).floor() / 100.0);
    let clearance = (to.cx - from.cx).abs() - serialized[0] / 2.0 - serialized[1] / 2.0;
    if clearance + 0.0001 < required {
        return None;
    }
    let (from_id, to_id) = (from.id.clone(), to.id.clone());
    accepts_fix(plan, gate, |doc| {
        for (id, width) in [(from_id, serialized[0]), (to_id, serialized[1])] {
            if let Some(n) = doc.nodes.iter_mut().find(|n| n.id == id) {
                n.width = Some(width);
            }
        }
    })
    .then_some(serialized)
}

/// `readableMigrationProvidesCapacity` (with `discoverFixes` on): under the readable columns the two
/// nodes clear `required`, and the document migrated to schema 2 (its X pins carried over by
/// [`mapped_candidate`]) compiles under `gate` (the resolved profile: the migrated copy is cloned
/// from the quality-resolved document), growing an authored viewBox once if that is all it lacks.
fn migration_provides_capacity(plan: &LegacyPlan<'_>, gate: Gate, from: &NodeBox, to: &NodeBox, required: f64) -> bool {
    let col_x = |xs: &[f64; 6], col: f64| if col.fract() == 0.0 && col >= 0.0 && (col as usize) < xs.len() { xs[col as usize] } else { f64::NAN };
    let mut as_v2 = plan.doc.clone();
    as_v2.schema_version = SchemaVersion::V2;
    let xs = place(&as_v2, &LayoutFeedback::default()).layout.col_xs;
    let distance = (col_x(&xs, to.col) - col_x(&xs, from.col)).abs();
    if distance - from.rect.width / 2.0 - to.rect.width / 2.0 < required {
        return false;
    }
    let compile_v2 = |doc: &Workflow| workflow_v2::compile(doc, gate, false);
    let Ok(planned) = compile_v2(&intrinsic_workflow(plan.doc)).or_else(|_| compile_v2(&planning_workflow(plan.doc))) else {
        return false;
    };
    let Some(mut candidate) = mapped_candidate(plan.doc, &LEGACY_COLUMN_CENTERS, &planned.placement.layout.col_xs) else {
        return false;
    };
    let compiled = compile_v2(&candidate);
    let Err(failure) = &compiled else { return true };
    let only_capacity = !failure.diagnostics.is_empty() && failure.diagnostics.iter().all(|d| d.code == "workflow/viewbox-capacity");
    let needed = failure
        .diagnostics
        .iter()
        .filter(|_| only_capacity)
        .find_map(|d| d.evidence.get("requiredViewBox").and_then(Value::as_array).filter(|a| a.len() == 2))
        .map(|a| [a[0].as_f64().unwrap_or(f64::NAN), a[1].as_f64().unwrap_or(f64::NAN)]);
    match (candidate.meta.view_box, needed) {
        (Some(vb), Some(required)) => {
            candidate.meta.view_box = Some([vb[0].max(required[0]), vb[1].max(required[1])]);
            compile_v2(&candidate).is_ok()
        }
        _ => false,
    }
}

/// `enforceLegacyColumnCapacity`: the first edge between two columns of one lane whose nodes overlap,
/// or leave less than the direct clearance (28, else 8), under the fixed centres.
fn column_capacity(plan: &LegacyPlan<'_>, gate: Gate, discover: bool) -> Option<Diagnostic> {
    for edge in &plan.doc.edges {
        let (Some(from), Some(to)) = (plan.node(&edge.from), plan.node(&edge.to)) else { continue };
        if from.lane != to.lane || from.col == to.col || !vertical_intervals_overlap(plan, from, to, NODE_GAP) {
            continue;
        }
        let distance = (to.cx - from.cx).abs();
        let actual = distance - from.rect.width / 2.0 - to.rect.width / 2.0;
        let direct = edge.via.is_none()
            && matches!(edge.route, None | Some(EdgeRoute::Auto) | Some(EdgeRoute::Straight))
            && (from.cy - to.cy).abs() < 0.0001;
        let required = if direct { 28.0 } else { 8.0 };
        if actual >= required {
            continue;
        }
        let capacity = if actual < 0.0 {
            format!("overlap by {}px", num(js_round(actual).abs()))
        } else {
            format!("leave only {}px of direct clearance", num(js_round(actual)))
        };
        let message = format!(
            "Workflow columns {}\u{2192}{} place nodes \"{}\" and \"{}\" so they {capacity} under the fixed-v1 layout.",
            num(from.col),
            num(to.col),
            from.id,
            to.id
        );
        let mut fixes = Vec::new();
        if discover {
            if migration_provides_capacity(plan, gate, from, to, required) {
                fixes.push("migrate this workflow to schema_version 2".to_owned());
            }
            if let Some(col) = verified_alternative(plan, gate, from, to, required) {
                fixes.push(format!("move node \"{}\" to verified free column {col}", to.id));
            }
            if let Some(w) = verified_widths(plan, gate, from, to, required) {
                let r = |x: f64| num(js_round(x * 100.0) / 100.0);
                fixes.push(format!("set node widths \"{}\"={}px and \"{}\"={}px", from.id, r(w[0]), to.id, r(w[1])));
            }
        }
        let subject = Subject::of("workflow")
            .with_extra("edge", edge.id.clone().map_or(Value::Null, Value::from))
            .with_extra("from", edge.from.clone().into())
            .with_extra("to", edge.to.clone().into())
            .with_extra("fromCol", crate::diag::json_num(from.col))
            .with_extra("toCol", crate::diag::json_num(to.col));
        let ev = evidence([
            ("centerDistancePx", crate::diag::json_num(distance)),
            ("nodeWidthsPx", Value::Array(vec![crate::diag::json_num(from.rect.width), crate::diag::json_num(to.rect.width)])),
            ("actualSignedClearancePx", crate::diag::json_num(actual)),
            ("requiredDirectClearancePx", crate::diag::json_num(required)),
        ]);
        return Some(
            Diagnostic::error("workflow/column-capacity", &message)
                .with_subject(subject)
                .with_evidence(ev)
                .with_fixes(fixes)
                .with_suppresses(["workflow/short-edge", "clean-flow/endpoint-side-direction", "workflow/label-node-overlap"]),
        );
    }
    None
}

/// Every problem of the planned v1 document under `gate`, plus what was measured.
pub fn validate_v1(plan: &LegacyPlan<'_>, gate: Gate) -> (Outcome, Review) {
    validate(plan, gate, true)
}

/// [`validate_v1`] with the fix search switched on or off (`discoverFixes`).
fn validate(plan: &LegacyPlan<'_>, gate: Gate, discover: bool) -> (Outcome, Review) {
    let doc = plan.doc;
    let view_box = plan.view_box;

    if let Some(thrown) = column_capacity(plan, gate, discover) {
        let message = thrown.message.clone();
        let review = Review { view_box, error: Some(message.clone()), compiler: vec![thrown.clone()], ..Review::default() };
        return (Outcome { diagnostics: vec![thrown], problems: vec![message] }, review);
    }

    let mut sink = Sink::default();
    let lane_ids: std::collections::HashSet<&str> = doc.lanes.iter().map(|l| l.id.as_str()).collect();
    if lane_ids.len() != doc.lanes.len() {
        sink.problem("Lane ids must be unique.".to_owned());
    }
    if plan.nodes.len() != doc.nodes.len() {
        sink.problem("Node ids must be unique.".to_owned());
    }
    let phases = doc.phases.as_deref().unwrap_or_default();
    let groups = doc.groups.as_deref().unwrap_or_default();
    if phases.iter().map(|p| p.id.as_str()).collect::<std::collections::HashSet<_>>().len() != phases.len() {
        sink.problem("Phase ids must be unique.".to_owned());
    }
    if groups.iter().map(|g| g.id.as_str()).collect::<std::collections::HashSet<_>>().len() != groups.len() {
        sink.problem("Group ids must be unique.".to_owned());
    }

    let cols = COLUMNS.len() as f64;
    for n in &plan.nodes {
        let node = &doc.nodes[n.index];
        if !lane_ids.contains(n.lane.as_str()) {
            sink.problem(format!("Node \"{}\" uses unknown lane \"{}\".", n.id, n.lane));
            continue;
        }
        if n.col.fract() != 0.0 || n.col < 0.0 || n.col >= cols || !n.col.is_finite() {
            sink.problem(format!(
                "Node \"{}\" uses column {}, but valid columns are integers 0..{}.",
                n.id,
                num(n.col),
                num(cols - 1.0)
            ));
            continue;
        }
        let r = n.rect;
        if !is_finite_point(&[r.x, r.y, n.cx, n.cy]) {
            sink.problem(format!(
                "Node \"{}\" produced non-finite coordinates — check col, width, height, and yOffset are numbers.",
                n.id
            ));
            continue;
        }
        let (k, p) = text::validation_label(DiagramType::Workflow);
        let estimate = text::units(&node.label) as f64 * k;
        if estimate > r.width + p {
            sink.problem(format!(
                "Label \"{}\" (~{}px) is wider than node \"{}\" ({}px) — shorten the label or increase node.width.",
                node.label,
                num(js_round(estimate)),
                n.id,
                num(r.width)
            ));
        }
        if node.brand.is_some()
            && let Some((available, required)) = text::brand_rail_problem(&node.label, r.width, LABEL_MINIMUM)
        {
            sink.problem(format!(
                "Node \"{}\" brand top rail leaves {}px for its label, but \"{}\" needs ~{}px at the {}px legible minimum — widen the node or shorten the label.",
                n.id,
                num(available.max(0.0)),
                node.label,
                num(required.ceil()),
                num(LABEL_MINIMUM)
            ));
        }
        let available = text::available_width(r.width);
        for (field, value) in [("Sublabel", &node.sublabel), ("Tag", &node.tag)] {
            let Some(value) = value.as_deref().filter(|v| !v.is_empty()) else { continue };
            let needed = text::min_text_width(value, TEXT_MINIMUM);
            if needed > available {
                sink.problem(format!(
                    "{field} \"{value}\" needs ~{}px at the {}px legible minimum, but node \"{}\" provides {}px — shorten the {} or increase node.width.",
                    num(needed.ceil()),
                    num(TEXT_MINIMUM),
                    n.id,
                    num(available),
                    field.to_lowercase()
                ));
            }
        }
        let top = plan.lane_top(&n.lane);
        let content_top = top + LANE_TITLE_H + 0.0;
        if r.x < LANE_X || r.x + r.width > LANE_X + LANE_W {
            sink.problem(format!("Node \"{}\" exceeds the horizontal bounds of lane \"{}\".", n.id, n.lane));
        }
        if r.y < content_top || r.y + r.height > top + plan.lane_height(&n.lane) {
            sink.problem(format!("Node \"{}\" collides with the title or boundary of lane \"{}\".", n.id, n.lane));
        }
    }

    let mut ranges: Vec<&crate::model::workflow::Phase> = Vec::new();
    for phase in phases {
        if phase.from_col.fract() != 0.0 || phase.to_col.fract() != 0.0 {
            sink.problem(format!("Phase \"{}\" must use integer fromCol/toCol values.", phase.id));
            continue;
        }
        if phase.from_col < 0.0 || phase.to_col >= cols || phase.from_col > phase.to_col {
            sink.problem(format!(
                "Phase \"{}\" uses invalid columns {}..{}; use an ordered range within 0..{}.",
                phase.id,
                num(phase.from_col),
                num(phase.to_col),
                num(cols - 1.0)
            ));
        } else {
            ranges.push(phase);
        }
        let estimate = text::units(&phase.label) as f64 * 5.6;
        let width = plan.phase_span(phase).width;
        if estimate > width + 8.0 {
            sink.problem(format!(
                "Phase label \"{}\" (~{}px) is wider than its {}px span — shorten the label or widen the phase range.",
                phase.label,
                num(js_round(estimate)),
                num(js_round(width))
            ));
        }
    }
    ranges.sort_by(|a, b| a.from_col.total_cmp(&b.from_col).then(a.to_col.total_cmp(&b.to_col)));
    for (i, earlier) in ranges.iter().enumerate() {
        for later in &ranges[i + 1..] {
            if later.from_col > earlier.to_col {
                break;
            }
            sink.problem(format!(
                "Phase \"{}\" ({}..{}) overlaps phase \"{}\" ({}..{}) — start at col {} or later, or end the earlier phase at col {}.",
                later.id,
                num(later.from_col),
                num(later.to_col),
                earlier.id,
                num(earlier.from_col),
                num(earlier.to_col),
                num(earlier.to_col + 1.0),
                num(later.from_col - 1.0)
            ));
        }
    }

    for group in groups {
        if !lane_ids.contains(group.lane.as_str()) {
            sink.problem(format!("Group \"{}\" uses unknown lane \"{}\".", group.id, group.lane));
            continue;
        }
        if group.from_col.fract() != 0.0 || group.to_col.fract() != 0.0 {
            sink.problem(format!("Group \"{}\" must use integer fromCol/toCol values.", group.id));
            continue;
        }
        if group.from_col < 0.0 || group.to_col >= cols || group.from_col > group.to_col {
            sink.problem(format!(
                "Group \"{}\" uses invalid columns {}..{}; use an ordered range within 0..{}.",
                group.id,
                num(group.from_col),
                num(group.to_col),
                num(cols - 1.0)
            ));
        }
        if !plan.nodes.iter().any(|n| n.lane == group.lane && n.col >= group.from_col && n.col <= group.to_col) {
            sink.problem(format!(
                "Group \"{}\" does not contain any nodes — align its lane/columns with the parallel or branch work it frames.",
                group.id
            ));
        }
    }

    let mut lanes: Vec<(&str, Vec<&NodeBox>)> = Vec::new();
    for n in &plan.nodes {
        match lanes.iter_mut().find(|(lane, _)| *lane == n.lane) {
            Some((_, members)) => members.push(n),
            None => lanes.push((n.lane.as_str(), vec![n])),
        }
    }
    for (lane, members) in &lanes {
        for (i, a) in members.iter().enumerate() {
            for b in &members[i + 1..] {
                if rects_overlap(&a.rect, &b.rect, NODE_GAP) {
                    sink.problem(format!(
                        "Nodes \"{}\" and \"{}\" are less than 8px apart in lane \"{lane}\" — move one to another col, adjust yOffset, or reduce width/height.",
                        a.id, b.id
                    ));
                }
            }
        }
    }

    for (edge, route) in doc.edges.iter().zip(&plan.routes) {
        if plan.node(&edge.from).is_none() {
            sink.problem(format!("Edge \"{}\" references unknown source \"{}\".", edge.label.as_deref().filter(|l| !l.is_empty()).unwrap_or(&edge.from), edge.from));
        }
        if plan.node(&edge.to).is_none() {
            sink.problem(format!("Edge \"{}\" references unknown target \"{}\".", edge.label.as_deref().filter(|l| !l.is_empty()).unwrap_or(&edge.to), edge.to));
        }
        if let Some(route) = route
            && route.points.len() == 2
        {
            let (start, end) = (route.points[0], route.points[1]);
            let length = (end[0] - start[0]).hypot(end[1] - start[1]);
            if length < MIN_EDGE {
                sink.problem(format!(
                    "Edge \"{}\" -> \"{}\" is too short ({}px; minimum 28px) — move the nodes farther apart or use a verified orthogonal route with readable clearance.",
                    edge.from,
                    edge.to,
                    num(js_round(length))
                ));
            }
        }
    }

    // The relations the shared gates see: edges whose nodes both exist.
    let rels: Vec<Rel> = doc
        .edges
        .iter()
        .zip(&plan.routes)
        .enumerate()
        .filter_map(|(index, (e, route))| {
            let route = route.as_ref()?;
            let has_geometry = e.route.is_some_and(|r| r != EdgeRoute::Auto) || e.via.is_some();
            // An authored side wins; else the inferred one when the route is not authored geometry.
            let pick = |authored: Option<crate::model::common::Side>, inferred| -> CheckedSide {
                authored.map(|s| (model_side(s), SideOrigin::Authored)).or_else(|| (!has_geometry).then_some((inferred, SideOrigin::Inferred)))
            };
            Some(Rel {
                index,
                id: e.id.clone(),
                from: e.from.clone(),
                to: e.to.clone(),
                label: e.label.clone().unwrap_or_default(),
                points: route.points.clone(),
                controls: controls(e),
                arrow_width: edge_width(e),
                from_side: pick(e.from_side, route.from_side),
                to_side: pick(e.to_side, route.to_side),
            })
        })
        .collect();
    let obstacles: Vec<Obstacle> = plan.nodes.iter().map(|n| Obstacle { id: n.id.clone(), rect: n.rect }).collect();

    endpoint_side(
        &CTX,
        &rels,
        "keep automatic routing, or choose fromSide/toSide and via points whose first and final segments cross node borders perpendicularly",
        &mut sink,
    );
    edge_through_node(
        &CTX,
        &rels,
        &obstacles,
        "node",
        2.0,
        "adjust fromSide/toSide, set route/via or channel coordinates, or move the node to a clearer lane/column",
        &mut sink,
    );
    proper_crossing(
        &CTX,
        &rels,
        gate,
        false,
        "adjust route/via, bias, or channel coordinates so the edges use separate lane corridors",
        &mut sink,
    );
    // The corridor diagnostic goes to `workflowDiagnostics`: it sits at its problem's place.
    let mut corridors = Sink::default();
    ambiguous_corridor(
        &CTX,
        &rels,
        gate,
        "adjust route/via, bias, or channel coordinates so unrelated edges do not visually merge",
        &mut corridors,
    );
    for d in corridors.recorded {
        sink.detailed(d);
    }
    let frames: Vec<FrameInfo> = plan
        .frames()
        .into_iter()
        .map(|f| FrameInfo {
            frame: Frame::Rect { rect: f.rect, radius: f.radius },
            kind: f.kind.to_owned(),
            id: Value::String(f.id),
            label: Some(f.label),
        })
        .collect();
    container_border_run(
        &CTX,
        &rels,
        &frames,
        gate,
        "adjust route/via, bias, or channel coordinates so the edge crosses the lane or group perpendicularly instead of following its border",
        &mut sink,
    );
    route_rhythm(
        &CTX,
        &rels,
        gate,
        "adjust route/via, bias, or channel coordinates so each turn has a readable run-up",
        &mut sink,
    );

    if let Some(main_path) = &doc.main_path {
        for id in main_path {
            if plan.node(id).is_none() {
                sink.problem(format!("mainPath references unknown node \"{id}\"."));
            }
        }
        for pair in main_path.windows(2) {
            let (Some(from), Some(to)) = (plan.node(&pair[0]), plan.node(&pair[1])) else { continue };
            if !doc.edges.iter().any(|e| e.from == pair[0] && e.to == pair[1]) {
                sink.problem(format!(
                    "mainPath step \"{}\" -> \"{}\" has no matching edge — add the edge or remove the pair from mainPath.",
                    pair[0], pair[1]
                ));
            }
            if to.col < from.col {
                sink.problem(format!(
                    "mainPath step \"{}\" -> \"{}\" moves backward from col {} to {} — use a return edge outside mainPath for loops.",
                    pair[0],
                    pair[1],
                    num(from.col),
                    num(to.col)
                ));
            }
        }
    }

    let labels: Vec<LabelRect> = doc
        .edges
        .iter()
        .zip(&plan.labels)
        .enumerate()
        .filter_map(|(index, (e, label))| {
            let label = label.as_ref()?;
            Some(LabelRect {
                rel: index,
                text: label.text.clone(),
                rect: label.rect,
                anchor: label.at,
                authored_at: e.label_at.is_some(),
                authored_dx: e.label_dx.filter(|v| v.is_finite()).unwrap_or(0.0),
                authored_dy: e.label_dy.filter(|v| v.is_finite()).unwrap_or(0.0),
            })
        })
        .collect();
    for label in &labels {
        for o in &obstacles {
            if rects_overlap(&label.rect, &o.rect, LABEL_GAP) {
                sink.problem(format!(
                    "Label \"{}\" overlaps node \"{}\" — adjust labelDx/labelDy/labelSegment or set labelAt.\n{}",
                    label.text,
                    o.id,
                    label_obstacle_fix(label, o, "node", view_box, &obstacles)
                ));
            }
        }
    }
    for (i, a) in labels.iter().enumerate() {
        for b in &labels[i + 1..] {
            if rects_overlap(&a.rect, &b.rect, LABEL_GAP) {
                sink.problem(format!(
                    "Labels \"{}\" and \"{}\" overlap — adjust labelDx/labelDy/labelSegment or route one relationship through a separate corridor.\n{}",
                    a.text,
                    b.text,
                    label_pair_fix(a, b)
                ));
            }
        }
    }
    label_route_clearance(
        &CTX,
        &rels,
        &labels,
        gate,
        "adjust labelAt, labelDx, labelDy, or labelSegment; otherwise adjust the other relationship route/via/channel",
        &mut sink,
    );

    // v1 only: the canvas around the fixed lanes.
    if view_box[0] < LANE_X + LANE_W + CANVAS_MARGIN {
        sink.problem(format!(
            "viewBox width {} clips the {}px lanes — set meta.viewBox[0] to at least {}.",
            num(view_box[0]),
            num(LANE_W),
            num(LANE_X + LANE_W + CANVAS_MARGIN)
        ));
    }
    if plan.legend_y() + LEGEND_BOTTOM_PAD > view_box[1] {
        sink.problem(format!(
            "Legend exceeds viewBox height {} — set meta.viewBox[1] to at least {}.",
            num(view_box[1]),
            num(plan.legend_y() + LEGEND_BOTTOM_PAD)
        ));
    }
    label_canvas_containment(&CTX, &rels, &labels, view_box, gate, &mut sink);

    let compiler: Vec<Diagnostic> = sink
        .problems
        .iter()
        .map(|m| {
            sink.details
                .iter()
                .find(|d| &d.message == m)
                .cloned()
                .unwrap_or_else(|| Diagnostic::error("layout/constraint", m).with_subject(Subject::of("workflow")))
        })
        .collect();
    let outcome = sink.finish(|message| Subject::of("workflow").with_rule(rule_of(message)));
    (outcome, Review { rels, labels, frames, view_box, error: None, compiler, warnings: Vec::new() })
}

/// The success receipt's `composition` block.
pub fn receipt(review: &Review, reported: &str, gate: Gate) -> Value {
    super::composition::receipt(&review.rels, &review.labels, &review.frames, review.view_box, reported, gate)
}
