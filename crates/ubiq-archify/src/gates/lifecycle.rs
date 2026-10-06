//! `validateLifecycle` (`render-lifecycle.mjs:349-563`), schema v1 and v2: every problem of one
//! lifecycle document, in Archify's order, graph rules and geometry rules together, measured on
//! the same [`Plan`] the layout draws (Archify validates after its module scope has routed and
//! placed every label).
//!
//! Rules: (v1) the canvas floor (`viewBox[1] - 122 + 4 >= 448`), unique state and lane ids, the
//! `main` lane, per state the lane, the column (v2: 0..4; v1: the band's), finite geometry, the
//! horizontal (v2 28, v1 32) and vertical (v2 44, v1 64, to the area bottom) bounds, the label
//! width (6.2 per unit + 6), the brand rail, sublabel/tag at their minimum (v2 7 px, v1 6 px);
//! states 10 px apart; per transition the endpoints and the 32 px length; then the shared gates
//! (`clean-flow/*`, `composition/*`; a crossing of two planner-routed transitions is resolved by
//! their halos), the showcase label / band-title overlap (authored profile only), label/state and
//! label/label collisions, label/route clearance and canvas containment.

use serde_json::{Value, json};

use super::clean_flow::{edge_through_node, endpoint_side};
use super::composition::{
    FrameInfo, ambiguous_corridor, collect_crossings, label_canvas_containment, label_route_clearance, route_rhythm,
};
use super::suggest::{label_obstacle_fix, label_pair_fix};
use super::{
    CheckedSide, Ctx, Gate, LabelRect, Obstacle, Outcome, Rel, SideOrigin, Sink, describe, evidence, num, point1, pt_json,
    re_plan_hint, rect_json, rel_subject, rel_subject_json,
};
use crate::diag::{Diagnostic, Subject, js_round};
use crate::geom::{is_finite_point, rects_overlap};
use crate::layout::lifecycle::{BandKind, Plan, label_or_note};
use crate::model::common::Variant;
use crate::model::lifecycle::{Transition, TransitionRoute};
use crate::route::dataflow::side as model_side;
use crate::text;

const CTX: Ctx<'static> = Ctx { diagram: "lifecycle", collection: "transitions" };

/// Bounds: `x >= margin`, `x + w <= W - margin`, `y >= top` (v2 28 and 44, v1 32 and 64).
const MARGIN_X: f64 = 28.0;
const TOP_Y: f64 = 44.0;
const V1_MARGIN_X: f64 = 32.0;
const V1_TOP_Y: f64 = 64.0;
/// `stateTextFit.sublabelMinimum` / `tagMinimum` (v2 7, v1 6).
const TEXT_MINIMUM: f64 = 7.0;
const V1_TEXT_MINIMUM: f64 = 6.0;
/// v1: the fixed bands need `areaBottom + 4 >= 448`, i.e. a canvas at least 566 high.
const V1_AREA_FLOOR: f64 = 448.0;
const V1_MIN_HEIGHT: f64 = 566.0;
/// The brand rail check's minimum font.
const BRAND_MINIMUM: f64 = 8.0;
/// States closer than this overlap; the shortest transition.
const STATE_GAP: f64 = 10.0;
const MIN_TRANSITION: f64 = 32.0;
/// `rectsOverlap(label, obstacle, -2)`: labels may touch by 2 px.
const LABEL_GAP: f64 = -2.0;

/// What the gates measured, for the success receipt.
#[derive(Debug, Default, Clone)]
pub struct Review {
    pub rels: Vec<Rel>,
    pub labels: Vec<LabelRect>,
    /// Lifecycle rows are reading guides, not containers: always empty.
    pub frames: Vec<FrameInfo>,
    pub view_box: [f64; 2],
    /// Per entry of `rels`: grid-routed (its crossings with another grid route are resolved).
    pub planned: Vec<bool>,
}

/// The rule id (D7) a plain sentence of `validateLifecycle` carries in `subject.rule`.
fn rule_of(message: &str) -> &'static str {
    let has = |s: &str| message.contains(s);
    if message == "State ids must be unique." {
        "graph/duplicate-node-id"
    } else if message.starts_with("viewBox height ") {
        "lifecycle/v1-canvas"
    } else if message == "Lane ids must be unique." {
        "graph/duplicate-container-id"
    } else if message.starts_with("Lifecycle diagrams need a lane") {
        "lifecycle/main-lane-required"
    } else if has("uses unknown lane") {
        "graph/unknown-container"
    } else if has("uses invalid column") {
        "lifecycle/col-range"
    } else if has("non-finite coordinates") {
        "geom/non-finite"
    } else if has("exceeds the horizontal bounds") || has("exceeds the vertical lifecycle area") {
        "geom/node-out-of-bounds"
    } else if has("brand top rail") {
        "geom/brand-rail"
    } else if has("legible minimum") {
        "geom/text-min-fit"
    } else if has("less than 10px apart") {
        "geom/node-overlap"
    } else if has("references unknown") {
        "graph/unknown-endpoint"
    } else if has("is too short") {
        "geom/edge-too-short"
    } else if has("overlaps lifecycle band title") {
        "composition/label-band-title-overlap"
    } else if has("overlaps state") {
        "geom/label-node-overlap"
    } else if message.starts_with("Labels ") {
        "geom/label-label-overlap"
    } else {
        "geom/label-too-wide"
    }
}

/// The authored routing controls of a transition, in [`super::PLAN_KEYS`] order.
fn controls(t: &Transition) -> Vec<&'static str> {
    [
        ("route", t.route.is_some()),
        ("via", t.via.is_some()),
        ("channelX", t.channel_x.is_some()),
        ("channelY", t.channel_y.is_some()),
        ("fromSide", t.from_side.is_some()),
        ("toSide", t.to_side.is_some()),
        ("labelAt", t.label_at.is_some()),
        ("labelDx", t.label_dx.is_some()),
        ("labelDy", t.label_dy.is_some()),
        ("labelSegment", t.label_segment.is_some()),
    ]
    .into_iter()
    .filter_map(|(k, present)| present.then_some(k))
    .collect()
}

/// `cleanCrossingProblems` with `crossingResolved: plannerRouted(left) && plannerRouted(right)`:
/// the first crossing of a pair is skipped when both are grid-routed (their halos resolve it).
fn crossing(rels: &[Rel], planned: &[bool], gate: Gate, sink: &mut Sink) {
    if !gate.showcase() {
        return;
    }
    const HINT: &str = "adjust route/via or channelX/channelY so the transitions use separate lifecycle corridors";
    for hit in collect_crossings(rels) {
        if planned[hit.left] && planned[hit.right] {
            continue;
        }
        let (left, right) = (&rels[hit.left], &rels[hit.right]);
        let hint = re_plan_hint(&[left, right], HINT);
        let message = format!(
            "[composition/proper-crossing] showcase {} {} crosses {} at [{}] (segments {} and {}) — {hint}.",
            CTX.diagram,
            describe(&CTX, left),
            describe(&CTX, right),
            point1(hit.point),
            hit.left_segment,
            hit.right_segment,
        );
        let ev = evidence([
            ("otherRelationship", rel_subject_json(&CTX, right)),
            ("point", pt_json(hit.point)),
            ("segmentIndex", hit.left_segment.into()),
            ("otherSegmentIndex", hit.right_segment.into()),
        ]);
        sink.error(
            Diagnostic::error("composition/proper-crossing", &message)
                .with_subject(rel_subject(&CTX, left))
                .with_evidence(ev)
                .with_fixes([hint]),
        );
    }
}

/// Every problem of the planned document under `gate`, plus what was measured.
pub fn validate(plan: &Plan<'_>, gate: Gate) -> (Outcome, Review) {
    let doc = plan.doc;
    let view_box = plan.view_box;
    let area_bottom = plan.area_bottom;
    let mut sink = Sink::default();

    let v2 = plan.v2;
    let (margin_x, top_y, text_minimum) = if v2 { (MARGIN_X, TOP_Y, TEXT_MINIMUM) } else { (V1_MARGIN_X, V1_TOP_Y, V1_TEXT_MINIMUM) };

    if plan.states.len() != doc.states.len() {
        sink.problem("State ids must be unique.".to_owned());
    }
    // The three bands are fixed; v2 derives its canvas from the rows, so the floor is v1's.
    if !v2 && area_bottom + 4.0 < V1_AREA_FLOOR {
        sink.problem(format!(
            "viewBox height {} is too short for the fixed band layout — set meta.viewBox[1] to at least {}.",
            num(view_box[1]),
            num(V1_MIN_HEIGHT)
        ));
    }
    let lane_ids: std::collections::HashSet<&str> = doc.lanes.iter().map(|l| l.id.as_str()).collect();
    if lane_ids.len() != doc.lanes.len() {
        sink.problem("Lane ids must be unique.".to_owned());
    }
    if !lane_ids.contains("main") {
        sink.problem(
            if v2 {
                "Lifecycle diagrams need a lane with id \"main\" (the first row). Lane ids \"main\" and \"terminal\" are reserved: \"main\" renders as the first row, \"terminal\" as the last, and every other lane in authored order."
            } else {
                "Lifecycle diagrams need a lane with id \"main\" (the phase rail). Lane ids \"main\" and \"terminal\" are reserved: \"main\" maps to the top phase band, \"terminal\" to the bottom outcome band, and all other lanes share the middle event band."
            }
            .to_owned(),
        );
    }

    for s in &plan.states {
        let state = &doc.states[s.index];
        if !lane_ids.contains(s.lane.as_str()) {
            sink.problem(format!("State \"{}\" uses unknown lane \"{}\".", s.id, s.lane));
            continue;
        }
        if v2 {
            if s.col.fract() != 0.0 || s.col < 0.0 || s.col > 4.0 {
                sink.problem(format!(
                    "State \"{}\" uses invalid column {} — every lifecycle row has integer columns 0..4.",
                    s.id,
                    num(s.col)
                ));
                continue;
            }
        } else {
            let band = BandKind::of(&s.lane);
            let max_col = if band == BandKind::Phase { 5.0 } else { 3.0 };
            if s.col.fract() != 0.0 || s.col < 0.0 || s.col >= max_col {
                sink.problem(format!(
                    "State \"{}\" uses invalid column {} — the {} band has integer columns 0..{}.",
                    s.id,
                    num(s.col),
                    band.name(),
                    num(max_col - 1.0)
                ));
                continue;
            }
        }
        let r = s.rect;
        if !is_finite_point(&[r.x, r.y, s.cx, r.cy()]) {
            sink.problem(format!(
                "State \"{}\" produced non-finite coordinates — check col, width, height, and yOffset are numbers.",
                s.id
            ));
            continue;
        }
        if r.x < margin_x || r.x + r.width > view_box[0] - margin_x {
            sink.problem(format!(
                "State \"{}\" exceeds the horizontal bounds of the diagram — reduce state.width{} or increase meta.viewBox[0].",
                s.id,
                if v2 { ", lower its col," } else { "" }
            ));
        }
        if r.y < top_y || r.y + r.height > area_bottom {
            sink.problem(format!(
                "State \"{}\" exceeds the vertical lifecycle area — keep y between {} and {} (adjust yOffset or increase meta.viewBox[1]).",
                s.id,
                num(top_y),
                num(area_bottom)
            ));
        }
        let estimate = text::units(&state.label) as f64 * 6.2;
        if estimate > r.width + 6.0 {
            sink.problem(format!(
                "Label \"{}\" (~{}px) is wider than state \"{}\" ({}px) — shorten the label or increase state.width.",
                state.label,
                num(js_round(estimate)),
                s.id,
                num(r.width)
            ));
        }
        if state.brand.is_some()
            && let Some((available, required)) = text::brand_rail_problem(&state.label, r.width, BRAND_MINIMUM)
        {
            sink.problem(format!(
                "State \"{}\" brand top rail leaves {}px for its label, but \"{}\" needs ~{}px at the {}px legible minimum — widen the node or shorten the label.",
                s.id,
                num(available.max(0.0)),
                state.label,
                num(required.ceil()),
                num(BRAND_MINIMUM)
            ));
        }
        let available = text::available_width(r.width);
        for (field, value) in [("Sublabel", &state.sublabel), ("Tag", &state.tag)] {
            let Some(value) = value.as_deref().filter(|v| !v.is_empty()) else { continue };
            let needed = text::min_text_width(value, text_minimum);
            if needed > available {
                sink.problem(format!(
                    "{field} \"{value}\" needs ~{}px at the {}px legible minimum, but state \"{}\" provides {}px — shorten the {} or increase state.width.",
                    num(needed.ceil()),
                    num(text_minimum),
                    s.id,
                    num(available),
                    field.to_lowercase()
                ));
            }
        }
    }

    for (i, a) in plan.states.iter().enumerate() {
        for b in &plan.states[i + 1..] {
            if rects_overlap(&a.rect, &b.rect, STATE_GAP) {
                sink.problem(format!(
                    "States \"{}\" and \"{}\" are less than 10px apart — move one to another col or separate them with yOffset{}",
                    a.id,
                    b.id,
                    if v2 { "." } else { " (lanes other than \"main\"/\"terminal\" share one band)." }
                ));
            }
        }
    }

    for (t, route) in doc.transitions.iter().zip(&plan.routes) {
        let label = t.label.as_deref().filter(|l| !l.is_empty());
        if plan.state(&t.from).is_none() {
            sink.problem(format!(
                "Transition \"{}\" references unknown source \"{}\".",
                label.unwrap_or(&t.from),
                t.from
            ));
        }
        if plan.state(&t.to).is_none() {
            sink.problem(format!("Transition \"{}\" references unknown target \"{}\".", label.unwrap_or(&t.to), t.to));
        }
        if let Some(route) = route {
            let (start, end) = (route.points[0], route.points[route.points.len() - 1]);
            let distance = (end[0] - start[0]).hypot(end[1] - start[1]);
            if distance < MIN_TRANSITION {
                let name = label.map_or_else(|| format!("{}->{}", t.from, t.to), str::to_owned);
                sink.problem(format!(
                    "Transition \"{name}\" is too short ({}px; minimum 32px) — route it through a channel or drop its label.",
                    num(js_round(distance))
                ));
            }
        }
    }

    // The relations the shared gates see: transitions whose states both exist.
    let mut planned = Vec::new();
    let rels: Vec<Rel> = doc
        .transitions
        .iter()
        .zip(&plan.routes)
        .enumerate()
        .filter_map(|(index, (t, route))| {
            let route = route.as_ref()?;
            let has_geometry = t.route.is_some_and(|r| r != TransitionRoute::Auto) || t.via.is_some();
            // `shouldCheckRelation: !Array.isArray(via)`; an authored side wins, else the inferred
            // one when the route is not authored geometry.
            let pick = |authored: Option<crate::model::common::Side>, inferred| -> CheckedSide {
                if t.via.is_some() {
                    return None;
                }
                authored
                    .map(|s| (model_side(s), SideOrigin::Authored))
                    .or_else(|| (!has_geometry).then_some((inferred, SideOrigin::Inferred)))
            };
            planned.push(route.planned);
            Some(Rel {
                index,
                id: t.id.clone(),
                from: t.from.clone(),
                to: t.to.clone(),
                label: label_or_note(t).unwrap_or("").to_owned(),
                points: route.points.clone(),
                controls: controls(t),
                arrow_width: t
                    .width
                    .filter(|w| *w != 0.0)
                    .unwrap_or(if t.variant == Some(Variant::Emphasis) { 1.8 } else { 1.5 }),
                from_side: pick(t.from_side, route.from_side),
                to_side: pick(t.to_side, route.to_side),
            })
        })
        .collect();
    let obstacles: Vec<Obstacle> = plan.states.iter().map(|s| Obstacle { id: s.id.clone(), rect: s.rect }).collect();

    endpoint_side(
        &CTX,
        &rels,
        "keep automatic routing, or choose fromSide/toSide and via points whose first and final segments cross state borders perpendicularly",
        &mut sink,
    );
    edge_through_node(
        &CTX,
        &rels,
        &obstacles,
        "state",
        2.0,
        "adjust fromSide/toSide, set route/via or channelX/channelY, or move the state with col/yOffset",
        &mut sink,
    );
    crossing(&rels, &planned, gate, &mut sink);
    ambiguous_corridor(
        &CTX,
        &rels,
        gate,
        "adjust route/via or channelX/channelY so unrelated transitions do not visually merge",
        &mut sink,
    );
    // `cleanBorderRunProblems` runs with an explicit empty frame set: it can find nothing.
    route_rhythm(
        &CTX,
        &rels,
        gate,
        "move route/via or channel coordinates so each lifecycle turn has a readable run-up",
        &mut sink,
    );

    let labels: Vec<LabelRect> = doc
        .transitions
        .iter()
        .zip(&plan.labels)
        .enumerate()
        .filter_map(|(index, (t, label))| {
            let label = label.as_ref()?;
            Some(LabelRect {
                rel: index,
                text: label_or_note(t)?.to_owned(),
                rect: label.rect,
                anchor: label.at,
                authored_at: t.label_at.is_some(),
                authored_dx: t.label_dx.filter(|v| v.is_finite()).unwrap_or(0.0),
                authored_dy: t.label_dy.filter(|v| v.is_finite()).unwrap_or(0.0),
            })
        })
        .collect();

    if plan.showcase() {
        for label in &labels {
            for band in &plan.bands {
                if !rects_overlap(&label.rect, &band.rect, 0.0) {
                    continue;
                }
                let t = &doc.transitions[label.rel];
                let message = format!(
                    "Transition {} label \"{}\" overlaps lifecycle band title \"{}\" — move the label with labelAt/labelDx/labelDy/labelSegment or provide more space.",
                    label.rel, label.text, band.label
                );
                let mut subject = Subject::of("lifecycle");
                subject.collection = Some("transitions".to_owned());
                subject.index = Some(label.rel as i64);
                subject = subject.with_extra("from", t.from.clone().into()).with_extra("to", t.to.clone().into());
                let title = match band.band {
                    Some(kind) => json!({"index": band.index, "band": kind.name(), "label": band.label, "x": band.rect.x,
                        "y": band.rect.y, "width": band.rect.width, "height": band.rect.height, "baseline": band.at[1]}),
                    None => json!({"index": band.index, "label": band.label, "vertical": true, "cx": band.at[0], "cy": band.at[1],
                        "x": band.rect.x, "y": band.rect.y, "width": band.rect.width, "height": band.rect.height}),
                };
                sink.error(
                    Diagnostic::error("composition/label-band-title-overlap", &message)
                        .with_subject(subject)
                        .with_evidence(evidence([("labelRect", rect_json(&label.rect)), ("bandTitle", title)]))
                        .with_fixes(["move the transition label with labelAt/labelDx/labelDy/labelSegment while preserving its text"]),
                );
            }
        }
    }
    for label in &labels {
        for o in &obstacles {
            if rects_overlap(&label.rect, &o.rect, LABEL_GAP) {
                sink.problem(format!(
                    "Label \"{}\" overlaps state \"{}\" — adjust labelDx/labelDy/labelSegment or set labelAt.\n{}",
                    label.text,
                    o.id,
                    label_obstacle_fix(label, o, "state", view_box, &obstacles)
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
        "adjust labelAt, labelDx, labelDy, or labelSegment; otherwise adjust the other relationship route/via/channel",
        &mut sink,
    );
    label_canvas_containment(&CTX, &rels, &labels, view_box, gate, &mut sink);

    let outcome = sink.finish(|message| Subject::of("lifecycle").with_rule(rule_of(message)));
    (outcome, Review { rels, labels, frames: Vec::new(), view_box, planned })
}

/// The success receipt's `composition` block: the shared one, with every proper crossing of two
/// grid-routed (both-halo) transitions moved to `resolvedCrossovers`, as the artifact checker
/// counts them.
pub fn receipt(review: &Review, reported: &str, gate: Gate) -> Value {
    super::composition::receipt_resolving(
        &review.rels,
        &review.labels,
        &review.frames,
        review.view_box,
        reported,
        gate,
        &review.planned,
    )
}
