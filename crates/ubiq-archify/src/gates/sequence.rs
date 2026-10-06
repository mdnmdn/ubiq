//! `validateSequence` (`render-sequence.mjs:196-355`): every problem of one sequence document, in
//! Archify's order, graph rules and geometry rules together. Archify measures participants and
//! messages as authored, so this does too: [`validate`] does not need a successful
//! [`crate::layout::sequence::build`].
//!
//! Rules: the timeline height (`H - 65 - 142 >= 120`), participant label (6.8 per unit) and
//! sublabel fit, message endpoints, the readable `y` range, the 60 px span, then the shared gates
//! (`clean-flow/edge-through-node` against the participant headers, `composition/*` over the
//! messages and the segment frames), messages less than 28 px apart that share horizontal space,
//! label/label overlap, segment `y` range and label width, activation ranges, the last
//! participant against the canvas and the room left for an implicit showcase legend.

use super::clean_flow::edge_through_node;
use super::composition::{
    FrameInfo, ambiguous_corridor, collect_label_overflow, container_border_run, describe_overflow, label_route_clearance,
    proper_crossing, route_rhythm, LABEL_TOLERANCE_PX,
};
use super::{Ctx, Gate, LabelRect, Obstacle, Outcome, Rel, Sink, describe, evidence, format_rect, num, re_plan_hint, rect_json, rel_subject};
use crate::diag::{Diagnostic, Subject, js_round, json_num};
use crate::geom::{Frame, rects_overlap};
use crate::layout::sequence::{
    LIFELINE_BOTTOM_PAD, LIFELINE_TOP, Plan, SEGMENT_RADIUS, SEGMENT_X, TOP_Y,
};
use crate::legend;
use crate::model::common::QualityProfile;
use crate::model::sequence::{ColumnFit, Sequence};
use crate::text::{self, DiagramType};

const CTX: Ctx<'static> = Ctx { diagram: "sequence", collection: "messages" };

/// The least readable timeline (`:215`).
const MIN_TIMELINE: f64 = 120.0;
/// `sublabelMinimum` (`participantTextFit`).
const SUBLABEL_MINIMUM: f64 = 6.0;
/// The brand rail check's minimum font.
const BRAND_MINIMUM: f64 = 8.0;
/// A message stays this far inside the timeline, and spans at least this much.
const TIMELINE_MARGIN: f64 = 18.0;
const MIN_SPAN: f64 = 60.0;
/// Messages closer than this in `y` must not share horizontal space.
const MESSAGE_GAP: f64 = 28.0;
/// `rectsOverlap(label, label, -2)`: labels may touch by 2 px.
const LABEL_GAP: f64 = -2.0;
/// A segment may reach this far below the lifelines.
const SEGMENT_OVERHANG: f64 = 20.0;

/// What the gates measured, for the success receipt and for callers that want the numbers.
#[derive(Debug, Default, Clone)]
pub struct Review {
    pub rels: Vec<Rel>,
    pub labels: Vec<LabelRect>,
    pub frames: Vec<FrameInfo>,
    pub view_box: [f64; 2],
}

/// The rule id (D7) a plain sentence of `validateSequence` carries in `subject.rule`.
fn rule_of(message: &str) -> &'static str {
    let has = |s: &str| message.contains(s);
    if message == "Participant ids must be unique." {
        "graph/duplicate-node-id"
    } else if has("leaves under 120px")
        || has("sits outside the readable timeline")
        || has("less than 28px apart")
        || has("extends outside the canvas")
    {
        "sequence/timeline"
    } else if has("is wider than the") || has("exceeds the segment frame") {
        "geom/label-too-wide"
    } else if has("brand top rail") {
        "geom/brand-rail"
    } else if has("legible minimum") {
        "geom/text-min-fit"
    } else if has("references unknown participant") {
        "graph/unknown-container"
    } else if has("references unknown") {
        "graph/unknown-endpoint"
    } else if has("spans ") {
        "geom/edge-too-short"
    } else if has("invalid y range") || has("invalid time range") {
        "sequence/inverted-range"
    } else if message.starts_with("Labels ") {
        "geom/label-label-overlap"
    } else if has("Participants exceed viewBox width") {
        "geom/node-out-of-bounds"
    } else {
        "legend/vertical-overflow"
    }
}

/// `cleanLabelCanvasContainmentProblems` with the sequence hint (the shared gate words its own).
fn label_canvas(rels: &[Rel], labels: &[LabelRect], view_box: [f64; 2], gate: Gate, sink: &mut Sink) {
    if !gate.showcase() {
        return;
    }
    const HINT: &str = "shorten the label, reorder participants, or enlarge meta.viewBox";
    for hit in collect_label_overflow(labels, view_box, LABEL_TOLERANCE_PX) {
        let label = &labels[hit.label];
        let Some(rel) = rels.iter().find(|r| r.index == label.rel) else { continue };
        let hint = re_plan_hint(&[rel], HINT);
        let message = format!(
            "[composition/label-canvas-containment] showcase {} label \"{}\" on {} extends past the {} (label rect {}; viewBox {}x{}) — {hint}.",
            CTX.diagram,
            label.text,
            describe(&CTX, rel),
            describe_overflow(&hit),
            format_rect(&label.rect),
            num(view_box[0]),
            num(view_box[1]),
        );
        let overflow: serde_json::Map<String, serde_json::Value> =
            hit.sides.iter().map(|(s, px)| ((*s).to_owned(), json_num(*px))).collect();
        let ev = evidence([
            ("label", label.text.clone().into()),
            ("labelRect", rect_json(&label.rect)),
            ("viewBox", serde_json::json!([json_num(view_box[0]), json_num(view_box[1])])),
            ("overflowPx", serde_json::Value::Object(overflow)),
            ("tolerancePx", json_num(LABEL_TOLERANCE_PX)),
        ]);
        sink.error(
            Diagnostic::error("composition/label-canvas-containment", &message)
                .with_subject(rel_subject(&CTX, rel))
                .with_evidence(ev)
                .with_fixes([hint]),
        );
    }
}

/// Every problem of `doc` under `gate`, plus what was measured.
pub fn validate(doc: &Sequence, gate: Gate) -> (Outcome, Review) {
    let plan = Plan::new(doc);
    let view_box = plan.view_box;
    let mut sink = Sink::default();

    if plan.parts.len() != doc.participants.len() {
        sink.problem("Participant ids must be unique.".to_owned());
    }
    if plan.lifeline_bottom - LIFELINE_TOP < MIN_TIMELINE {
        sink.problem(format!(
            "viewBox height {} leaves under 120px of timeline — set meta.viewBox[1] to at least {}.",
            num(view_box[1]),
            num(LIFELINE_TOP + MIN_TIMELINE + LIFELINE_BOTTOM_PAD)
        ));
    }

    let width = plan.participant_w;
    let width_note = if plan.fit == ColumnFit::Spread {
        format!("participant boxes are {}px for this viewBox width and {} participants", num(width), doc.participants.len().max(1))
    } else {
        format!("participant boxes are a fixed {}px unless meta.column_fit is \"spread\"", num(width))
    };
    for part in &plan.parts {
        let p = part.participant;
        if text::label_too_wide(DiagramType::Sequence, &p.label, width) {
            let estimate = text::units(&p.label) as f64 * 6.8;
            sink.problem(format!(
                "Label \"{}\" (~{}px) is wider than the {}px participant box — shorten it.",
                p.label,
                num(js_round(estimate)),
                num(width)
            ));
        }
        if p.brand.is_some()
            && let Some((available, required)) = text::brand_rail_problem(&p.label, width, BRAND_MINIMUM)
        {
            sink.problem(format!(
                "Participant \"{}\" brand top rail leaves {}px for its label, but \"{}\" needs ~{}px at the {}px legible minimum — widen the node or shorten the label.",
                p.id,
                num(available.max(0.0)),
                p.label,
                num(required.ceil()),
                num(BRAND_MINIMUM)
            ));
        }
        if let Some(sub) = p.sublabel.as_deref().filter(|s| !s.is_empty()) {
            let available = text::available_width(width);
            let needed = text::min_text_width(sub, SUBLABEL_MINIMUM);
            if needed > available {
                sink.problem(format!(
                    "Sublabel \"{sub}\" needs ~{}px at the {}px legible minimum, but participant \"{}\" provides {}px — shorten the sublabel ({width_note}).",
                    num(needed.ceil()),
                    num(SUBLABEL_MINIMUM),
                    p.id,
                    num(available)
                ));
            }
        }
    }

    for m in &doc.messages {
        let (from, to) = (plan.part(&m.from), plan.part(&m.to));
        if from.is_none() {
            sink.problem(format!("Message \"{}\" references unknown source \"{}\".", m.label, m.from));
        }
        if to.is_none() {
            sink.problem(format!("Message \"{}\" references unknown target \"{}\".", m.label, m.to));
        }
        let (low, high) = (LIFELINE_TOP + TIMELINE_MARGIN, plan.lifeline_bottom - TIMELINE_MARGIN);
        if m.y < low || m.y > high {
            sink.problem(format!(
                "Message \"{}\" sits outside the readable timeline — keep y between {} and {}.",
                m.label,
                num(low),
                num(high)
            ));
        }
        if let (Some(from), Some(to)) = (from, to) {
            let distance = (to.cx - from.cx).abs();
            if distance < MIN_SPAN {
                sink.problem(format!(
                    "Message \"{}\" spans {}px (minimum 60px) — give its participants more column distance.",
                    m.label,
                    num(js_round(distance))
                ));
            }
        }
    }

    // The relations the shared gates see: messages whose participants both exist.
    let rels: Vec<Rel> = doc
        .messages
        .iter()
        .enumerate()
        .filter_map(|(index, m)| {
            let points = plan.path(m);
            (!points.is_empty()).then(|| Rel {
                index,
                id: m.id.clone(),
                from: m.from.clone(),
                to: m.to.clone(),
                label: m.label.clone(),
                points,
                controls: Vec::new(),
                arrow_width: if m.variant == Some(crate::model::sequence::MessageVariant::Emphasis) { 1.8 } else { 1.5 },
                from_side: None,
                to_side: None,
            })
        })
        .collect();
    let obstacles: Vec<Obstacle> =
        plan.parts.iter().map(|p| Obstacle { id: p.participant.id.clone(), rect: p.rect }).collect();
    let frames: Vec<FrameInfo> = doc
        .segments
        .iter()
        .flatten()
        .enumerate()
        .map(|(i, s)| FrameInfo {
            frame: Frame::Rect { rect: plan.segment_frame(s), radius: SEGMENT_RADIUS },
            kind: "segment".to_owned(),
            id: i.into(),
            label: Some(s.label.clone()),
        })
        .collect();

    // Participant headers are opaque; lifelines, activations and segment bands are pass-through.
    edge_through_node(
        &CTX,
        &rels,
        &obstacles,
        "participant header",
        0.0,
        "move the message y below the participant headers or reorder participants",
        &mut sink,
    );
    proper_crossing(&CTX, &rels, gate, false, "separate the message y values; lifeline crossings remain allowed", &mut sink);
    ambiguous_corridor(&CTX, &rels, gate, "separate the message y values so unrelated messages do not visually merge", &mut sink);
    container_border_run(
        &CTX,
        &rels,
        &frames,
        gate,
        "move the message y so it crosses a segment boundary perpendicularly or stays clearly inside the segment",
        &mut sink,
    );
    route_rhythm(
        &CTX,
        &rels,
        gate,
        "increase participant spacing or simplify message routing so every turn has room to read",
        &mut sink,
    );

    // Vertical crowding only matters when the arrows share horizontal space.
    let mut placed: Vec<(&str, f64, f64, f64)> = doc
        .messages
        .iter()
        .filter_map(|m| {
            let (from, to) = (plan.part(&m.from)?, plan.part(&m.to)?);
            Some((m.label.as_str(), m.y, from.cx.min(to.cx), from.cx.max(to.cx)))
        })
        .collect();
    placed.sort_by(|a, b| a.1.total_cmp(&b.1));
    for (i, a) in placed.iter().enumerate() {
        for b in placed[i + 1..].iter().take_while(|b| b.1 - a.1 < MESSAGE_GAP) {
            if a.2 < b.3 && b.2 < a.3 {
                sink.problem(format!(
                    "Messages \"{}\" and \"{}\" are less than 28px apart and share horizontal space — spread their y values.",
                    a.0, b.0
                ));
            }
        }
    }

    // Label masks can extend well past the arrow span: check the rectangles too.
    let labels: Vec<LabelRect> = doc
        .messages
        .iter()
        .enumerate()
        .filter_map(|(index, m)| {
            let rect = plan.label_box(m)?;
            Some(LabelRect {
                rel: index,
                text: m.label.clone(),
                rect,
                anchor: [rect.x + rect.width / 2.0, m.y - 10.0],
                authored_at: false,
                authored_dx: 0.0,
                authored_dy: 0.0,
            })
        })
        .collect();
    for (i, a) in labels.iter().enumerate() {
        for b in &labels[i + 1..] {
            if rects_overlap(&a.rect, &b.rect, LABEL_GAP) {
                sink.problem(format!(
                    "Labels \"{}\" and \"{}\" overlap — spread their message y values or shorten the labels.",
                    a.text, b.text
                ));
            }
        }
    }
    label_route_clearance(
        &CTX,
        &rels,
        &labels,
        gate,
        "spread the message y values, shorten the label, or reorder participants so the adjacent route stays visible",
        &mut sink,
    );
    label_canvas(&rels, &labels, view_box, gate, &mut sink);

    for s in doc.segments.iter().flatten() {
        if s.to <= s.from {
            sink.problem(format!(
                "Segment \"{}\" has invalid y range (from {} to {}) — \"to\" must be greater than \"from\".",
                s.label,
                num(s.from),
                num(s.to)
            ));
        }
        let limit = plan.lifeline_bottom + SEGMENT_OVERHANG;
        if s.from < TOP_Y || s.to > limit {
            sink.problem(format!(
                "Segment \"{}\" extends outside the canvas — keep its y range between {} and {}.",
                s.label,
                num(TOP_Y),
                num(limit)
            ));
        }
        let label = plan.segment_label(s);
        let right = view_box[0] - SEGMENT_X;
        if label.x + label.width > right {
            sink.problem(format!(
                "Segment \"{}\" label (~{}px) exceeds the segment frame's available width ({}px) — shorten the label or increase meta.viewBox[0] to at least {}.",
                s.label,
                num(js_round(label.width)),
                num((right - label.x).max(0.0)),
                num((label.x + label.width + SEGMENT_X).ceil())
            ));
        }
    }

    for a in doc.activations.iter().flatten() {
        if plan.part(&a.participant).is_none() {
            sink.problem(format!("Activation references unknown participant \"{}\".", a.participant));
        }
        if a.to <= a.from {
            sink.problem(format!(
                "Activation for \"{}\" has invalid time range — \"to\" must be greater than \"from\".",
                a.participant
            ));
        }
    }

    if let Some(last) = doc.participants.last().and_then(|p| plan.part(&p.id))
        && last.cx + width / 2.0 > view_box[0] - 40.0
    {
        sink.problem(format!(
            "Participants exceed viewBox width — set meta.viewBox[0] to at least {} or remove a participant.",
            num((last.cx + width / 2.0 + 40.0).ceil())
        ));
    }

    // Showcase must not silently drop the implicit legend because late content leaves no room.
    let legend_height = plan.legend_required_height(view_box[0]);
    if doc.meta.quality_profile == Some(QualityProfile::Showcase)
        && doc.meta.legend.is_none()
        && legend_height > view_box[1]
        && matches!(legend::measure(&plan.entries, &plan.legend_layout()), Ok(None))
    {
        sink.problem(format!(
            "Sequence content ends at y={}, leaving no room for the legend below it — set meta.viewBox[1] to at least {} or omit meta.viewBox so the canvas grows.",
            num(plan.content_bottom),
            num(legend_height)
        ));
    }

    let outcome = sink.finish(|message| Subject::of("sequence").with_rule(rule_of(message)));
    (outcome, Review { rels, labels, frames, view_box })
}
