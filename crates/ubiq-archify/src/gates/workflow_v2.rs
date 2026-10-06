//! `validateWorkflow` for `schema_version` 2 (`workflow-compiler.mjs:2502-2836`), measured on the
//! readable layout and the routes the router settled on ([`GateInput`]); it runs between routing and
//! the viewBox, as Archify does, and is the `gates` of [`readable_build::Options`].
//!
//! What cannot reach it in v2 is thrown earlier: duplicate lane and node ids, unknown endpoints and
//! lanes, and invalid node columns are `validateReadableInputsBeforeRouting` (`graph.rs`), and the
//! legacy `column-capacity` rule is v1 only. What is left, in Archify's order: per node the label
//! width (6.8 per unit + 6), the brand rail, sublabel and tag at their 6 px minimum and the lane
//! bounds; phases (integer, ordered, in range, label width, no overlap); groups (lane, integer,
//! ordered, in range, at least one node); nodes 8 px apart within a lane; the 28 px shortest edge;
//! then the shared gates (`clean-flow/*`, `composition/*`), `mainPath`, label/node and label/label
//! collisions and label/route clearance.
//!
//! # Severity and where diagnostics go
//!
//! Crossings, ambiguous corridors and arrowhead collisions are judged on merged waypoints, include
//! pairs that share an endpoint, and exempt a bounded terminal trunk (`shortWorkflowTrunk`). Below
//! showcase they are **warnings**: they land in `workflowDiagnostics` ([`Review::warnings`], the
//! `--layout-json` receipt of a clean layout) and never in a problem. In showcase they are errors
//! that replace their problem's plain `layout/constraint` ([`Sink::detailed`]), like v1's corridor.
//! The border-run, rhythm and label clearance rules read the profile as the other types do.

use serde_json::Value;

use super::clean_flow::{edge_through_node, endpoint_side};
use super::composition::{
    FrameInfo, collect_arrowhead_collisions_with, container_border_run, label_route_clearance, proper_crossing_workflow_v2,
    route_rhythm, ambiguous_corridor_with,
};
use super::suggest::{label_obstacle_fix, label_pair_fix};
use super::workflow_v1::{CTX, LABEL_GAP, LABEL_MINIMUM, MIN_EDGE, NODE_GAP, Review, TEXT_MINIMUM, controls, rule_of};
use super::{CheckedSide, Gate, LabelRect, Obstacle, Outcome, Rel, SideOrigin, Sink, num};
use crate::diag::{Diagnostic, Severity, Subject, js_round, json_num};
use crate::geom::{Frame, Pt, is_finite_point, rects_overlap};
use crate::layout::workflow::readable::{COLUMN_COUNT, Failure, LANE_TITLE_H, LANE_X, ReadablePlacement};
use crate::layout::workflow::readable_build::{self, GateInput, Options, ReadableGeometry};
use crate::model::common::{QualityProfile, Variant};
use crate::model::workflow::{Edge, EdgeRoute, Workflow};
use crate::route::dataflow::side as model_side;
use crate::route::workflow::{Acceptor, EdgeLabel, RoutedEdge, Sides, stroke_width};
use crate::scene::Scene;
use crate::text::{self, DiagramType};

/// The profile the gates see for a document nobody overrides: its authored `meta.quality_profile`.
pub fn gate_of(w: &Workflow) -> Gate {
    Gate(w.meta.quality_profile.map(|q| match q {
        QualityProfile::Standard => "standard",
        QualityProfile::Showcase => "showcase",
    }))
}

/// `compileWorkflowWithFeedback` for a v2 document under `gate`: the router, the gates, the canvas.
/// With `discover`, a fix is only offered when a recompile of the mutated document (with
/// `discover` off, as `acceptsFix` does) succeeds.
pub fn compile(w: &Workflow, gate: Gate, discover: bool) -> Result<ReadableGeometry, Failure> {
    // `qualityResolvedWorkflow`: the router reads the profile from the document, so an override
    // (`--quality`) is written into a copy first.
    let resolved = gate.0.map(|q| if q == "showcase" { QualityProfile::Showcase } else { QualityProfile::Standard });
    if resolved.is_some() && resolved != w.meta.quality_profile {
        let mut copy = w.clone();
        copy.meta.quality_profile = resolved;
        return compile(&copy, gate, discover);
    }
    let nested = |doc: &Workflow| compile(doc, gate, false).is_ok();
    let accepts: &Acceptor<'_> = &nested;
    let early = crate::graph::workflow_prologue(w, discover.then_some(accepts));
    if let Some(first) = early.first() {
        return Err(Failure::new(first.message.clone(), early));
    }
    let gates = |input: &GateInput<'_>| gates(input, gate);
    readable_build::compile_geometry(w, &Options { accepts: discover.then_some(accepts), gates: Some(&gates) })
}

/// [`compile`] plus the scene.
pub fn build(w: &Workflow, gate: Gate) -> Result<(Scene, ReadableGeometry), Failure> {
    let g = compile(w, gate, true)?;
    Ok((readable_build::scene(&g), g))
}

/// The hook [`compile`] hands the router: [`validate`] as a thrown failure, or its warnings.
fn gates(input: &GateInput<'_>, gate: Gate) -> Result<Vec<Diagnostic>, Failure> {
    let (outcome, review) = validate(input, gate);
    if outcome.ok() {
        return Ok(review.warnings);
    }
    let lines: Vec<String> = outcome.problems.iter().map(|p| format!("- {p}")).collect();
    Err(Failure {
        error: format!("Workflow layout validation failed:\n{}", lines.join("\n")),
        diagnostics: review.compiler,
        reported: Some(outcome.diagnostics),
    })
}

/// Rels, label rects and frames of a routed layout. `sides` (one per route) are only needed by the
/// endpoint-side gate, not by the composition receipt.
fn measure(p: &ReadablePlacement, edges: &[RoutedEdge], sides: &[Sides], labels: &[EdgeLabel], frames: &[crate::route::workflow::CompositionFrame]) -> (Vec<Rel>, Vec<LabelRect>, Vec<FrameInfo>) {
    let w = p.workflow();
    let rels = edges
        .iter()
        .enumerate()
        .map(|(k, re)| {
            let e = &w.edges[re.index];
            let has_geometry = e.route.is_some_and(|r| r != EdgeRoute::Auto) || e.via.is_some();
            // An authored side wins; else the inferred one when the route is not authored geometry.
            let pick = |authored: Option<crate::model::common::Side>, inferred: Option<crate::geom::Side>| -> CheckedSide {
                authored
                    .map(|s| (model_side(s), SideOrigin::Authored))
                    .or_else(|| (!has_geometry).then_some(inferred).flatten().map(|s| (s, SideOrigin::Inferred)))
            };
            let inferred = sides.get(k);
            Rel {
                index: re.index,
                id: e.id.clone(),
                from: e.from.clone(),
                to: e.to.clone(),
                label: e.label.clone().unwrap_or_default(),
                points: re.points.clone(),
                controls: controls(e),
                arrow_width: stroke_width(e),
                from_side: pick(e.from_side, inferred.map(|s| s.from)),
                to_side: pick(e.to_side, inferred.map(|s| s.to)),
            }
        })
        .collect();
    let label_rects = labels
        .iter()
        .map(|l| {
            let e = &w.edges[l.index];
            LabelRect {
                rel: l.index,
                text: l.text.clone(),
                rect: l.rect,
                anchor: l.at,
                authored_at: e.label_at.is_some(),
                authored_dx: e.label_dx.filter(|v| v.is_finite()).unwrap_or(0.0),
                authored_dy: e.label_dy.filter(|v| v.is_finite()).unwrap_or(0.0),
            }
        })
        .collect();
    let frame_infos = frames
        .iter()
        .map(|f| FrameInfo {
            frame: Frame::Rect { rect: f.rect, radius: f.radius },
            kind: f.kind.to_owned(),
            id: Value::String(f.id.clone()),
            label: Some(f.label.clone()),
        })
        .collect();
    (rels, label_rects, frame_infos)
}

/// What the composition receipt reads from a laid-out v2 document, at its final canvas.
pub fn review_of(g: &ReadableGeometry) -> Review {
    let frames = frames_of(&g.placement);
    let (rels, labels, frames) = measure(&g.placement, &g.edges, &[], &g.labels, &frames);
    Review { rels, labels, frames, view_box: g.view_box, warnings: g.diagnostics.clone(), ..Review::default() }
}

/// `workflowCompositionFrames` without the router that owns them (lanes, exception insets, groups).
fn frames_of(p: &ReadablePlacement) -> Vec<crate::route::workflow::CompositionFrame> {
    use crate::route::workflow::CompositionFrame;
    let mut out = Vec::new();
    for lane in &p.lanes {
        out.push(CompositionFrame { id: format!("lane-{}", lane.index), kind: "lane", label: lane.label.clone(), rect: lane.rect, radius: 10.0 });
        if let Some(ex) = lane.exception {
            out.push(CompositionFrame {
                id: format!("lane-{}-exception", lane.index),
                kind: "exception-lane",
                label: format!("{} exception", lane.label),
                rect: ex,
                radius: 8.0,
            });
        }
    }
    for g in &p.groups {
        out.push(CompositionFrame { id: format!("group-{}", g.index), kind: "group", label: g.label.clone(), rect: g.rect, radius: 9.0 });
    }
    out
}

fn variant_of(e: &Edge) -> Variant {
    e.variant.unwrap_or(Variant::Default)
}

/// `shortWorkflowTrunk`: two same-looking edges that share a source or target and run the same
/// way for at most 24 px at that end are one trunk, not an ambiguous corridor.
fn short_trunk(a: &Edge, b: &Edge, lp: &[Pt], rp: &[Pt], ls: usize, rs: usize, length: f64) -> bool {
    if length > 24.0 + 0.0001 {
        return false;
    }
    if variant_of(a) != variant_of(b) || stroke_width(a) != stroke_width(b) || a.role != b.role {
        return false;
    }
    let same = |p: Pt, q: Pt| (p[0] - q[0]).abs() < 0.0001 && (p[1] - q[1]).abs() < 0.0001;
    let source = a.from == b.from && ls == 0 && rs == 0 && same(lp[0], rp[0]);
    let target = a.to == b.to && ls == lp.len() - 2 && rs == rp.len() - 2 && same(lp[lp.len() - 1], rp[rp.len() - 1]);
    if !source && !target {
        return false;
    }
    let (p, q) = (lp[ls], lp[ls + 1]);
    let (r, s) = (rp[rs], rp[rs + 1]);
    (q[0] - p[0]) * (s[0] - r[0]) + (q[1] - p[1]) * (s[1] - r[1]) > 0.0
}

/// The v2 arrowhead check: `composition/arrowhead-collision`, an error in showcase, else a warning.
fn arrowhead_collisions(rels: &[Rel], edges: &[Edge], gate: Gate) -> Vec<Diagnostic> {
    let severity = if gate.showcase() { Severity::Error } else { Severity::Warning };
    let edge_of = |pos: usize| &edges[rels[pos].index];
    let exempt = |l: usize, r: usize, lp: &[Pt], rp: &[Pt], length: f64| {
        short_trunk(edge_of(l), edge_of(r), lp, rp, lp.len() - 2, rp.len() - 2, length)
    };
    let name = |r: &Rel| r.id.clone().filter(|id| !id.is_empty()).unwrap_or_else(|| format!("{}->{}", r.from, r.to));
    collect_arrowhead_collisions_with(rels, &|_| true, &exempt)
        .into_iter()
        .map(|hit| {
            let (left, right) = (&rels[hit.left], &rels[hit.right]);
            let message = format!(
                "[composition/arrowhead-collision] workflow arrows \"{}\" and \"{}\" overlap at their destination (clearance {}px; minimum {}px).",
                name(left),
                name(right),
                num(hit.distance),
                num(hit.minimum)
            );
            let subject = Subject::of("workflow")
                .with_extra("edge", name(left).into())
                .with_extra("from", left.from.clone().into())
                .with_extra("to", left.to.clone().into());
            let ev = super::evidence([
                ("otherEdge", name(right).into()),
                ("distancePx", json_num(hit.distance)),
                ("minimumPx", json_num(hit.minimum)),
            ]);
            Diagnostic::new("composition/arrowhead-collision", severity, &message)
                .with_subject(subject)
                .with_evidence(ev)
                .with_fixes(["use separate destination sides or ports with clearance for both arrow markers"])
        })
        .collect()
}

/// Every problem of the routed v2 document under `gate`, plus what was measured.
pub fn validate(input: &GateInput<'_>, gate: Gate) -> (Outcome, Review) {
    let p = input.placement;
    let doc = p.workflow();
    let view_box = input.view_box;
    let mut sink = Sink::default();
    let mut workflow_diagnostics: Vec<Diagnostic> = Vec::new();

    let phases = doc.phases.as_deref().unwrap_or_default();
    let groups = doc.groups.as_deref().unwrap_or_default();
    let cols = COLUMN_COUNT as f64;
    let lane_right = LANE_X + p.layout.lane_w;

    for n in &p.nodes {
        let node = &doc.nodes[n.index];
        if n.col.fract() != 0.0 || n.col < 0.0 || n.col >= cols || !n.col.is_finite() {
            sink.problem(format!("Node \"{}\" uses column {}, but valid columns are integers 0..{}.", n.id, num(n.col), num(cols - 1.0)));
            continue;
        }
        let r = n.rect;
        if !is_finite_point(&[r.x, r.y, n.cx, n.cy]) {
            sink.problem(format!("Node \"{}\" produced non-finite coordinates — check col, width, height, and yOffset are numbers.", n.id));
            continue;
        }
        let (k, pad) = text::validation_label(DiagramType::Workflow);
        let estimate = text::units(&node.label) as f64 * k;
        if estimate > r.width + pad {
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
        let top = p.lane_top(&n.lane);
        let content_top = top + LANE_TITLE_H + p.group_header_h(&n.lane);
        if r.x < LANE_X || r.x + r.width > lane_right {
            sink.problem(format!("Node \"{}\" exceeds the horizontal bounds of lane \"{}\".", n.id, n.lane));
        }
        if r.y < content_top || r.y + r.height > top + p.lane_height(&n.lane) {
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
        let width = p.phase_span(phase).width;
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
        if p.lane_index(&group.lane).is_none() {
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
        if !p.nodes.iter().any(|n| n.lane == group.lane && n.col >= group.from_col && n.col <= group.to_col) {
            sink.problem(format!(
                "Group \"{}\" does not contain any nodes — align its lane/columns with the parallel or branch work it frames.",
                group.id
            ));
        }
    }

    let mut lanes: Vec<(&str, Vec<&crate::layout::workflow::readable::PlacedNode>)> = Vec::new();
    for n in &p.nodes {
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

    for route in input.edges {
        if route.points.len() == 2 {
            let e = &doc.edges[route.index];
            let (start, end) = (route.points[0], route.points[1]);
            let length = (end[0] - start[0]).hypot(end[1] - start[1]);
            if length < MIN_EDGE {
                sink.problem(format!(
                    "Edge \"{}\" -> \"{}\" is too short ({}px; minimum 28px) — move the nodes farther apart or use a verified orthogonal route with readable clearance.",
                    e.from,
                    e.to,
                    num(js_round(length))
                ));
            }
        }
    }

    let (rels, labels, frames) = measure(p, input.edges, input.sides, input.labels, input.frames);
    let obstacles: Vec<Obstacle> = p.nodes.iter().map(|n| Obstacle { id: n.id.clone(), rect: n.rect }).collect();
    // The rich diagnostics of these three go to `workflowDiagnostics`: errors replace their problem,
    // warnings are only reported.
    let mut take = |scratch: Sink, sink: &mut Sink| {
        for d in scratch.recorded {
            if d.severity == Severity::Error {
                sink.detailed(d.clone());
            }
            workflow_diagnostics.push(d);
        }
    };

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
    let mut scratch = Sink::default();
    proper_crossing_workflow_v2(&CTX, &rels, gate, "adjust route/via, bias, or channel coordinates so the edges use separate lane corridors", &mut scratch);
    take(scratch, &mut sink);
    let mut scratch = Sink::default();
    let exempt = |c: &crate::gates::composition::Corridor, lp: &[Pt], rp: &[Pt]| {
        short_trunk(&doc.edges[rels[c.left].index], &doc.edges[rels[c.right].index], lp, rp, c.left_segment, c.right_segment, c.length)
    };
    ambiguous_corridor_with(
        &CTX,
        &rels,
        gate,
        &|_, _| true,
        &exempt,
        true,
        "adjust route/via, bias, or channel coordinates so unrelated edges do not visually merge",
        &mut scratch,
    );
    take(scratch, &mut sink);
    let mut scratch = Sink::default();
    for d in arrowhead_collisions(&rels, &doc.edges, gate) {
        if d.severity == Severity::Error {
            scratch.problems.push(d.message.clone());
        }
        scratch.recorded.push(d);
    }
    take(scratch, &mut sink);
    container_border_run(
        &CTX,
        &rels,
        &frames,
        gate,
        "adjust route/via, bias, or channel coordinates so the edge crosses the lane or group perpendicularly instead of following its border",
        &mut sink,
    );
    route_rhythm(&CTX, &rels, gate, "adjust route/via, bias, or channel coordinates so each turn has a readable run-up", &mut sink);

    if let Some(main_path) = &doc.main_path {
        for id in main_path {
            if p.node(id).is_none() {
                sink.problem(format!("mainPath references unknown node \"{id}\"."));
            }
        }
        for pair in main_path.windows(2) {
            let (Some(from), Some(to)) = (p.node(&pair[0]), p.node(&pair[1])) else { continue };
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
    (outcome, Review { rels, labels, frames, view_box, error: None, compiler, warnings: workflow_diagnostics })
}
