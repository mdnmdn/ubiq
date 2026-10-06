//! `validateDataflow` (`render-dataflow.mjs:120-300`): every problem of one dataflow document, in
//! Archify's order, graph rules and geometry rules together. Archify collects them in one pass over
//! *measured* nodes (stage and row are used as authored, even out of range), so this does too:
//! [`validate`] does not need a successful [`crate::layout::dataflow::build`].
//!
//! The graph sentences (`Node ids must be unique.`, the stage/row ranges, unknown endpoints) are the
//! ones of `graph.rs`; here they sit where `validateDataflow` emits them, which only matters for the
//! order of the receipt's `error` text. Geometry rules: bounds, label width, text minimum, brand
//! rail, node overlap 10, flow length 34, diagonal `via`, label/node and label/label collisions,
//! stage width, then the shared gates (`clean-flow/*`, `composition/*`).

use std::collections::HashMap;

use super::clean_flow::{edge_through_node, endpoint_side};
use super::composition::{
    FrameInfo, ambiguous_corridor, container_border_run, label_canvas_containment, label_route_clearance, proper_crossing,
    route_rhythm,
};
use super::suggest::{label_obstacle_fix, label_pair_fix};
use super::{CheckedSide, Ctx, Gate, LabelRect, Obstacle, Outcome, Rel, SideOrigin, Sink, num};
use crate::diag::{Subject, js_round};
use crate::geom::{Frame, Rect, Side, is_finite_point, rects_overlap};
use crate::labels::{self, Hints};
use crate::layout::dataflow::{COL_GAP, LEFT_X, NODE_H, NODE_W, ROW_YS, STAGE_BOTTOM_PAD, STAGE_H, STAGE_RADIUS, STAGE_W, STAGE_Y, DEFAULT_VIEW_BOX, stage_frame, stage_x};
use crate::model::dataflow::{Dataflow, Flow, FlowRoute};
use crate::route::dataflow::{flow_sides, route_all, side};
use crate::text::{self, DiagramType};

const CTX: Ctx<'static> = Ctx { diagram: "dataflow", collection: "flows" };

/// `nodeTextFit.sublabelMinimum` / `tagMinimum`.
const TEXT_MINIMUM: f64 = 6.0;
/// The shortest flow, `34` px between its end points.
const MIN_FLOW: f64 = 34.0;
/// Nodes closer than this overlap.
const NODE_GAP: f64 = 10.0;
/// The brand rail check's minimum font.
const BRAND_MINIMUM: f64 = 8.0;
/// `rectsOverlap(label, obstacle, -2)`: labels may touch by 2 px.
const LABEL_GAP: f64 = -2.0;

/// What the gates measured, for the success receipt and for callers that want the numbers.
#[derive(Debug, Default, Clone)]
pub struct Review {
    pub rels: Vec<Rel>,
    pub labels: Vec<LabelRect>,
    pub frames: Vec<FrameInfo>,
    pub view_box: [f64; 2],
}

/// A node as `measureNode` sees it: stage and row exactly as authored.
struct Measured<'a> {
    node: &'a crate::model::dataflow::Node,
    rect: Rect,
    cx: f64,
    cy: f64,
}

fn row_y(row: f64) -> f64 {
    if row.fract() == 0.0 && row >= 0.0 && (row as usize) < ROW_YS.len() { ROW_YS[row as usize] } else { f64::NAN }
}

fn measure(node: &crate::model::dataflow::Node) -> Measured<'_> {
    let width = node.width.filter(|w| *w != 0.0).unwrap_or(NODE_W);
    let height = node.height.filter(|h| *h != 0.0).unwrap_or(NODE_H);
    let cx = LEFT_X + node.stage * COL_GAP;
    let y = row_y(node.row) + node.y_offset.unwrap_or(0.0);
    Measured { node, rect: Rect::new(cx - width / 2.0, y, width, height), cx, cy: y + height / 2.0 }
}

/// `nodes`: a `Map` by id, the last duplicate's value at the first one's position.
fn node_map(nodes: &[crate::model::dataflow::Node]) -> (Vec<Measured<'_>>, HashMap<&str, usize>) {
    let mut out: Vec<Measured<'_>> = Vec::new();
    let mut at: HashMap<&str, usize> = HashMap::new();
    for n in nodes {
        let m = measure(n);
        match at.get(n.id.as_str()) {
            Some(&i) => out[i] = m,
            None => {
                at.insert(n.id.as_str(), out.len());
                out.push(m);
            }
        }
    }
    (out, at)
}

fn controls(flow: &Flow) -> Vec<&'static str> {
    [
        ("route", flow.route.is_some()),
        ("via", flow.via.is_some()),
        ("channelX", flow.channel_x.is_some()),
        ("channelY", flow.channel_y.is_some()),
        ("fromSide", flow.from_side.is_some()),
        ("toSide", flow.to_side.is_some()),
        ("labelAt", flow.label_at.is_some()),
        ("labelDx", flow.label_dx.is_some()),
        ("labelDy", flow.label_dy.is_some()),
        ("labelSegment", flow.label_segment.is_some()),
    ]
    .into_iter()
    .filter_map(|(k, present)| present.then_some(k))
    .collect()
}

fn sides_to_check(flow: &Flow, inferred: (Side, Side)) -> (CheckedSide, CheckedSide) {
    let has_route_geometry = flow.route.is_some_and(|r| r != FlowRoute::Auto) || flow.via.is_some();
    let pick = |authored: Option<crate::model::common::Side>, inferred: Side| {
        authored
            .map(|s| (side(s), SideOrigin::Authored))
            .or_else(|| (!has_route_geometry).then_some((inferred, SideOrigin::Inferred)))
    };
    (pick(flow.from_side, inferred.0), pick(flow.to_side, inferred.1))
}

/// The rule id (D7) a plain sentence of `validateDataflow` carries in `subject.rule`.
fn rule_of(message: &str) -> &'static str {
    let has = |s: &str| message.contains(s);
    if message == "Node ids must be unique." {
        "graph/duplicate-node-id"
    } else if has("uses invalid stage") {
        "dataflow/stage-range"
    } else if has("uses invalid row") {
        "dataflow/row-range"
    } else if has("non-finite coordinates") {
        "geom/non-finite"
    } else if has("exceeds the horizontal bounds") || has("exceeds the readable diagram area") {
        "geom/node-out-of-bounds"
    } else if has("is wider than node") {
        "geom/label-too-wide"
    } else if has("brand top rail") {
        "geom/brand-rail"
    } else if has("legible minimum") {
        "geom/text-min-fit"
    } else if has("less than 10px apart") {
        "geom/node-overlap"
    } else if has("references unknown") {
        "graph/unknown-endpoint"
    } else if has("must include a short data label") {
        "graph/missing-label"
    } else if has("is too short") {
        "geom/edge-too-short"
    } else if has("overlaps node") {
        "geom/label-node-overlap"
    } else if message.starts_with("Labels ") {
        "geom/label-label-overlap"
    } else {
        "dataflow/diagonal-via"
    }
}

/// Every problem of `doc` under `gate`, plus what was measured.
pub fn validate(doc: &Dataflow, gate: Gate) -> (Outcome, Review) {
    let view_box = doc.meta.view_box.unwrap_or(DEFAULT_VIEW_BOX);
    let mut sink = Sink::default();
    let (nodes, at) = node_map(&doc.nodes);
    let stage_count = doc.stages.len() as f64;

    if nodes.len() != doc.nodes.len() {
        sink.problem("Node ids must be unique.".to_owned());
    }
    let area_top = STAGE_Y + STAGE_H + 22.0;
    for m in &nodes {
        let n = m.node;
        if n.stage.is_nan() || n.stage < 0.0 || n.stage >= stage_count {
            sink.problem(format!(
                "Node \"{}\" uses invalid stage {} — valid stages are 0..{}.",
                n.id,
                num(n.stage),
                num(stage_count - 1.0)
            ));
        }
        if n.row.is_nan() || n.row < 0.0 || n.row >= ROW_YS.len() as f64 {
            sink.problem(format!(
                "Node \"{}\" uses invalid row {} — valid rows are 0..{}.",
                n.id,
                num(n.row),
                ROW_YS.len() - 1
            ));
        }
        let r = m.rect;
        if !is_finite_point(&[r.x, r.y, m.cx, m.cy]) {
            sink.problem(format!(
                "Node \"{}\" produced non-finite coordinates — check stage, row, width, height, and yOffset are numbers.",
                n.id
            ));
            continue;
        }
        if r.x < 24.0 || r.x + r.width > view_box[0] - 24.0 {
            sink.problem(format!(
                "Node \"{}\" exceeds the horizontal bounds of the viewBox — reduce node.width or increase meta.viewBox[0].",
                n.id
            ));
        }
        let bottom = view_box[1] - STAGE_BOTTOM_PAD;
        if r.y < area_top || r.y + r.height > bottom {
            sink.problem(format!(
                "Node \"{}\" exceeds the readable diagram area — keep y between {} and {} (adjust row/yOffset or increase meta.viewBox[1]).",
                n.id,
                num(area_top),
                num(bottom)
            ));
        }
        if text::label_too_wide(DiagramType::Dataflow, &n.label, r.width) {
            let estimate = text::units(&n.label) as f64 * 6.2;
            sink.problem(format!(
                "Label \"{}\" (~{}px) is wider than node \"{}\" ({}px) — shorten the label or increase node.width.",
                n.label,
                num(js_round(estimate)),
                n.id,
                num(r.width)
            ));
        }
        if n.brand.is_some()
            && let Some((available, required)) = text::brand_rail_problem(&n.label, r.width, BRAND_MINIMUM)
        {
            sink.problem(format!(
                "Node \"{}\" brand top rail leaves {}px for its label, but \"{}\" needs ~{}px at the {}px legible minimum — widen the node or shorten the label.",
                n.id,
                num(available.max(0.0)),
                n.label,
                num(required.ceil()),
                num(BRAND_MINIMUM)
            ));
        }
        let available = text::available_width(r.width);
        for (field, value) in [("Sublabel", &n.sublabel), ("Tag", &n.tag)] {
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
    }

    for i in 0..doc.nodes.len() {
        for j in i + 1..doc.nodes.len() {
            let (a, b) = (&nodes[at[doc.nodes[i].id.as_str()]], &nodes[at[doc.nodes[j].id.as_str()]]);
            if rects_overlap(&a.rect, &b.rect, NODE_GAP) {
                sink.problem(format!(
                    "Nodes \"{}\" and \"{}\" are less than 10px apart — move one to another stage/row or adjust yOffset.",
                    a.node.id, b.node.id
                ));
            }
        }
    }

    // Routes for every flow whose ends exist (`pathFor`).
    let rects: HashMap<String, Rect> = nodes.iter().map(|m| (m.node.id.clone(), m.rect)).collect();
    let routed = route_all(&doc.flows, &rects);

    for (flow, routed) in doc.flows.iter().zip(&routed) {
        let from = rects.get(&flow.from);
        let to = rects.get(&flow.to);
        let name = |end: &str| if flow.label.is_empty() { end.to_owned() } else { flow.label.clone() };
        if from.is_none() {
            sink.problem(format!("Flow \"{}\" references unknown source \"{}\".", name(&flow.from), flow.from));
        }
        if to.is_none() {
            sink.problem(format!("Flow \"{}\" references unknown target \"{}\".", name(&flow.to), flow.to));
        }
        if flow.label.is_empty() {
            sink.problem(format!("Flow \"{}\" -> \"{}\" must include a short data label.", flow.from, flow.to));
        }
        let Some(routed) = routed else { continue };
        let (start, end) = (routed.points[0], routed.points[routed.points.len() - 1]);
        let distance = (end[0] - start[0]).hypot(end[1] - start[1]);
        if distance < MIN_FLOW {
            sink.problem(format!(
                "Flow \"{}\" is too short ({}px; minimum 34px) — route it through a channel or spread its nodes.",
                flow.label,
                num(js_round(distance))
            ));
        }
        if let Some(via) = &flow.via {
            for (i, w) in routed.points.windows(2).enumerate() {
                let diagonal = (w[0][0] - w[1][0]).abs() > 0.01 && (w[0][1] - w[1][1]).abs() > 0.01;
                if !diagonal {
                    continue;
                }
                let via_index = (i as i64).min(via.len() as i64 - 1);
                sink.problem(format!(
                    "Flow \"{}\" has a diagonal segment from ({}, {}) to ({}, {}) — align via[{via_index}] with its adjacent point by sharing the same x or y coordinate.",
                    flow.label,
                    num(w[0][0]),
                    num(w[0][1]),
                    num(w[1][0]),
                    num(w[1][1])
                ));
            }
        }
    }

    // The relations the shared gates see.
    let rels: Vec<Rel> = doc
        .flows
        .iter()
        .zip(&routed)
        .enumerate()
        .filter_map(|(index, (flow, routed))| {
            let routed = routed.as_ref()?;
            let (from, to) = (rects.get(&flow.from)?, rects.get(&flow.to)?);
            let (from_side, to_side) = sides_to_check(flow, flow_sides(flow, from, to));
            Some(Rel {
                index,
                id: flow.id.clone(),
                from: flow.from.clone(),
                to: flow.to.clone(),
                label: flow.label.clone(),
                points: routed.points.clone(),
                controls: controls(flow),
                arrow_width: flow
                    .width
                    .filter(|w| *w != 0.0)
                    .unwrap_or(if flow.variant == Some(crate::model::common::Variant::Emphasis) { 1.8 } else { 1.5 }),
                from_side,
                to_side,
            })
        })
        .collect();
    let obstacles: Vec<Obstacle> = nodes.iter().map(|m| Obstacle { id: m.node.id.clone(), rect: m.rect }).collect();
    let frames: Vec<FrameInfo> = doc
        .stages
        .iter()
        .enumerate()
        .map(|(i, stage)| FrameInfo {
            frame: Frame::Rect { rect: stage_frame(i, view_box), radius: STAGE_RADIUS },
            kind: "stage".to_owned(),
            id: i.into(),
            label: Some(stage.label.clone()),
        })
        .collect();

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
        "adjust fromSide/toSide, set route/via or channelX/channelY, or move the node to another stage/row",
        &mut sink,
    );
    proper_crossing(
        &CTX,
        &rels,
        gate,
        false,
        "adjust route/via or channelX/channelY so the flows use separate stage corridors",
        &mut sink,
    );
    ambiguous_corridor(
        &CTX,
        &rels,
        gate,
        "adjust route/via or channelX/channelY so unrelated flows do not visually merge",
        &mut sink,
    );
    container_border_run(
        &CTX,
        &rels,
        &frames,
        gate,
        "adjust route/via or channelX/channelY so the flow crosses the stage perpendicularly instead of following its border",
        &mut sink,
    );
    route_rhythm(
        &CTX,
        &rels,
        gate,
        "adjust route/via or channelX/channelY so each turn uses a clear inter-stage corridor",
        &mut sink,
    );

    // Label rects: only flows with a label and both ends.
    let labels: Vec<LabelRect> = doc
        .flows
        .iter()
        .zip(&routed)
        .enumerate()
        .filter_map(|(index, (flow, routed))| {
            let routed = routed.as_ref()?;
            if flow.label.is_empty() {
                return None;
            }
            let hints = Hints { at: flow.label_at, dx: flow.label_dx, dy: flow.label_dy, segment: flow.label_segment };
            let l = labels::dataflow_label(&hints, &routed.points, &flow.label, flow.classification.as_deref());
            Some(LabelRect {
                rel: index,
                text: flow.label.clone(),
                rect: l.rect,
                anchor: l.at,
                authored_at: flow.label_at.is_some(),
                authored_dx: flow.label_dx.filter(|v| v.is_finite()).unwrap_or(0.0),
                authored_dy: flow.label_dy.filter(|v| v.is_finite()).unwrap_or(0.0),
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
                    "Labels \"{}\" and \"{}\" overlap — adjust labelDx/labelDy.\n{}",
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
        "adjust labelAt, labelDx, labelDy, or labelSegment; otherwise adjust the other flow route/via/channelX/channelY",
        &mut sink,
    );
    label_canvas_containment(&CTX, &rels, &labels, view_box, gate, &mut sink);

    let last = stage_x(doc.stages.len().saturating_sub(1));
    if last + STAGE_W / 2.0 > view_box[0] - 24.0 {
        sink.problem(format!(
            "Stages exceed viewBox width — set meta.viewBox[0] to at least {}.",
            num((last + STAGE_W / 2.0 + 24.0).ceil())
        ));
    }

    let outcome = sink.finish(|message| Subject::of("dataflow").with_rule(rule_of(message)));
    (outcome, Review { rels, labels, frames, view_box })
}
