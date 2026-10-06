//! Sequence layout (P4.1): participant columns, lifelines, messages with their label boxes, segment
//! frames, activations and the legend, emitted as a [`Scene`] plus the raw [`SequenceGeometry`] the
//! goldens read. Port of `renderers/sequence/render-sequence.mjs`; `00` §5.5 (Sequence), `03` §2.5.
//!
//! Nothing is solved: a message sits at its authored `y`, a participant at its array index. The
//! one thing that moves is a segment label, bumped up 22 px (at most four times) off the messages.
//! [`Plan`] holds every number both this layout and the gates (`gates::sequence`) need, measured the
//! way `validateSequence` measures: before and independent of a successful [`build`].
//!
//! The legend catalogue lives here ([`catalog`]): it is the only sequence-specific part of the
//! shared legend machinery.
//!
//! The brand badge is drawn (`brand::push_badge`). Not drawn: the `grid` pattern (the classic preset
//! hides it; the others get it from `tokens::restyle`).

use std::collections::{HashMap, HashSet};

use crate::diag::{Diagnostic, Subject, js_round};
use crate::geom::{Pt, Rect, rects_overlap};
use crate::layout::dataflow::EDGE_HIT_HALF_WIDTH;
use crate::i18n::Locale;
use crate::legend::{self, CatalogEntry, Entry, Layout as LegendLayout, Measured, Placed, Unfit};
use crate::model::common::{ComponentType, LegendMode, NodeIcon, QualityProfile};
use crate::model::sequence::{ColumnFit, Message, MessageVariant, Participant, Segment, Sequence, SequenceMeta};
use crate::scene::{
    Anchor, Bounds, Detail, EdgeRef, Fill, Group, GroupKind, Layer, Marker, PolylineShape, RectShape, Scene, SceneBuilder,
    Shape, Stroke, TextShape,
};
use crate::sigils;
use crate::text::{self, DiagramType, Profile};
use crate::tokens::{EdgeVariant, Kind, Token};

/// `layout.topY` (`render-sequence.mjs:73`): the participant header row.
pub const TOP_Y: f64 = 72.0;
/// `participantH` (`:77`), `participantLabelY` (`:78`), `participantSublabelY` (`:79`).
pub const PARTICIPANT_H: f64 = 60.0;
pub const PARTICIPANT_LABEL_Y: f64 = 36.0;
pub const PARTICIPANT_SUBLABEL_Y: f64 = 50.0;
/// `lifelineTop` (`:80`); the bottom is `H - 65`, the legend baseline `H - 54` (`:81-82`).
pub const LIFELINE_TOP: f64 = 142.0;
pub const LIFELINE_BOTTOM_PAD: f64 = 65.0;
pub const LEGEND_BASELINE_UP: f64 = 54.0;
/// `sideMargin` (`:67`); a fixed participant is 86 wide with a 108 gap (`:68-72`).
pub const SIDE_MARGIN: f64 = 62.0;
pub const FIXED_W: f64 = 86.0;
pub const FIXED_GAP: f64 = 108.0;
/// The default canvas (`:63`): `920 x max(760, legendRequired)`.
pub const DEFAULT_WIDTH: f64 = 920.0;
pub const MIN_HEIGHT: f64 = 760.0;
/// The legend sits `12` below the content and needs `86` (`:47-48`).
pub const LEGEND_CONTENT_GAP: f64 = 12.0;
pub const LEGEND_BLOCK_HEIGHT: f64 = 86.0;
pub const LEGEND_X: f64 = 40.0;
pub const LEGEND_SIDE_PAD: f64 = 40.0;
/// A message starts and ends this far inside its lifelines (`:127-135`).
pub const MESSAGE_INSET: f64 = 7.0;
/// Segment frames: `x 48`, `W - 96` wide, radius 10 (`:157-166`, `:380`); dashed `6,6` like every lane.
pub const SEGMENT_X: f64 = 48.0;
pub const SEGMENT_RADIUS: f64 = 10.0;
pub const SEGMENT_DASH: [f64; 2] = [6.0, 6.0];
/// A segment label: `x 56`, 18 tall, `22` above the frame, bumped by 22 up to four times (`:171-182`).
pub const SEGMENT_LABEL_X: f64 = 56.0;
pub const SEGMENT_LABEL_H: f64 = 18.0;
pub const SEGMENT_LABEL_RISE: f64 = 22.0;
pub const SEGMENT_LABEL_BUMPS: usize = 4;
/// Participant box radius 6, stroke 1.5; activation 10 wide, radius 3, stroke 1; label mask radius 3.
pub const PARTICIPANT_RADIUS: f64 = 6.0;
pub const PARTICIPANT_STROKE: f64 = 1.5;
pub const ACTIVATION_W: f64 = 10.0;
pub const ACTIVATION_RADIUS: f64 = 3.0;
pub const LABEL_RADIUS: f64 = 3.0;
/// Lifelines: `a-default`, 0.8 wide, dashed `3,7` (`:407`).
pub const LIFELINE_WIDTH: f64 = 0.8;
pub const LIFELINE_DASH: [f64; 2] = [3.0, 7.0];
/// A lifeline stops this far above the legend title (`:400`).
pub const LIFELINE_LEGEND_GAP: f64 = 22.0;
/// Message strokes (`:440`): 1.8 for `emphasis`, else 1.4.
pub const MESSAGE_WIDTH: f64 = 1.4;
pub const EMPHASIS_WIDTH: f64 = 1.8;
/// A message note: font 7 at `(min(start, end) + 12, y + 18)`, a label font 9 (standard) or 11.
pub const NOTE_FONT: f64 = 7.0;
pub const NOTE_DX: f64 = 12.0;
pub const NOTE_DY: f64 = 18.0;

/// What `legend.sequence.*` says in the document's locale (`i18n.mjs`).
pub fn catalog(l: &Locale) -> Vec<CatalogEntry> {
    ["emphasis", "return", "security", "dashed", "default"]
    .into_iter()
    .map(|kind| CatalogEntry {
        kind,
        label: l.t(&format!("legend.sequence.{kind}")),
        swatch_width: Some(34.0),
        swatch_gap: Some(9.0),
        interactive: false,
    })
    .collect()
}

fn variant_name(v: Option<MessageVariant>) -> &'static str {
    match v {
        None | Some(MessageVariant::Default) => "default",
        Some(MessageVariant::Emphasis) => "emphasis",
        Some(MessageVariant::Security) => "security",
        Some(MessageVariant::Dashed) => "dashed",
        Some(MessageVariant::Return) => "return",
    }
}

fn kind_of(t: ComponentType) -> Kind {
    match t {
        ComponentType::Frontend => Kind::Frontend,
        ComponentType::Backend => Kind::Backend,
        ComponentType::Database => Kind::Database,
        ComponentType::Cloud => Kind::Cloud,
        ComponentType::Security => Kind::Security,
        ComponentType::Messagebus => Kind::Messagebus,
        ComponentType::External => Kind::External,
    }
}

fn icon_name(icon: NodeIcon) -> String {
    serde_json::to_value(icon).ok().and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_default()
}

fn legend_config(meta: &SequenceMeta) -> Option<legend::Config> {
    let l = meta.legend.as_ref()?;
    let mut entries = HashMap::new();
    if let Some(e) = &l.entries {
        for (kind, entry) in [
            ("default", &e.default),
            ("emphasis", &e.emphasis),
            ("security", &e.security),
            ("dashed", &e.dashed),
            ("return", &e.return_),
        ] {
            if let Some(o) = entry {
                entries.insert(kind.to_owned(), legend::Override { label: o.label.clone(), visible: o.visible });
            }
        }
    }
    Some(legend::Config { mode: l.mode.unwrap_or(LegendMode::Auto), entries })
}

/// `legendRequiredHeight(width)`: `0` without entries, else the content, the gap, the block and any
/// wrapped rows.
fn required_height(entries: &[Entry], content_bottom: f64, width: f64) -> f64 {
    if entries.is_empty() {
        return 0.0;
    }
    let footprint = legend::footprint(entries, width - 2.0 * LEGEND_SIDE_PAD);
    (content_bottom + LEGEND_CONTENT_GAP + LEGEND_BLOCK_HEIGHT + footprint.extra_height).ceil()
}

/// A participant as the renderer's `Map` holds it: the last one of a duplicate id, at the first
/// one's position, with its own index.
#[derive(Clone, Debug)]
pub struct Part<'a> {
    pub index: usize,
    pub participant: &'a Participant,
    pub cx: f64,
    pub rect: Rect,
}

/// The horizontal extent of a message (`messageGeometry`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Span {
    pub start: f64,
    pub end: f64,
    pub center: f64,
}

/// Everything layout measures before anything is drawn.
#[derive(Debug)]
pub struct Plan<'a> {
    pub doc: &'a Sequence,
    pub view_box: [f64; 2],
    pub fit: ColumnFit,
    /// `quality_profile === 'showcase'` *as authored* (`--quality` does not reach the layout).
    pub readable: bool,
    pub participant_w: f64,
    pub col_gap: f64,
    pub left_x: f64,
    pub lifeline_bottom: f64,
    pub label_h: f64,
    /// The lowest content: last message and its note, activations, segments (`contentBottom`).
    pub content_bottom: f64,
    pub entries: Vec<Entry>,
    pub parts: Vec<Part<'a>>,
    at: HashMap<&'a str, usize>,
}

impl<'a> Plan<'a> {
    pub fn new(doc: &'a Sequence) -> Self {
        let messages = &doc.messages;
        let content_bottom = messages
            .iter()
            .map(|m| m.y + if m.note.is_some() { 22.0 } else { 6.0 })
            .chain(doc.activations.iter().flatten().map(|a| a.to))
            .chain(doc.segments.iter().flatten().map(|s| s.to))
            .fold(0.0, f64::max);

        let present: HashSet<&str> = messages.iter().map(|m| variant_name(m.variant)).collect();
        let config = legend_config(&doc.meta);
        let entries = legend::resolve(config.as_ref(), &catalog(&doc.meta.locale()), &present);

        let required = required_height(&entries, content_bottom, DEFAULT_WIDTH);
        let view_box = doc.meta.view_box.unwrap_or([DEFAULT_WIDTH, MIN_HEIGHT.max(required)]);

        let fit = if doc.meta.column_fit == Some(ColumnFit::Spread) { ColumnFit::Spread } else { ColumnFit::Fixed };
        let count = doc.participants.len().max(1) as f64;
        let spread = fit == ColumnFit::Spread;
        let participant_w = if spread {
            FIXED_W.max(190.0_f64.min(js_round((view_box[0] - SIDE_MARGIN * 2.0) / count) - 24.0))
        } else {
            FIXED_W
        };
        let col_gap = if spread && count > 1.0 {
            FIXED_GAP.max((view_box[0] - 40.0 - SIDE_MARGIN - participant_w) / (count - 1.0))
        } else {
            FIXED_GAP
        };
        let left_x = if spread { SIDE_MARGIN + participant_w / 2.0 } else { SIDE_MARGIN };

        let mut parts: Vec<Part<'a>> = Vec::new();
        let mut at: HashMap<&'a str, usize> = HashMap::new();
        for (index, participant) in doc.participants.iter().enumerate() {
            let cx = left_x + index as f64 * col_gap;
            let part = Part {
                index,
                participant,
                cx,
                rect: Rect::new(cx - participant_w / 2.0, TOP_Y, participant_w, PARTICIPANT_H),
            };
            match at.get(participant.id.as_str()) {
                Some(&i) => parts[i] = part,
                None => {
                    at.insert(participant.id.as_str(), parts.len());
                    parts.push(part);
                }
            }
        }

        let readable = doc.meta.quality_profile == Some(QualityProfile::Showcase);
        Plan {
            doc,
            view_box,
            fit,
            readable,
            participant_w,
            col_gap,
            left_x,
            lifeline_bottom: view_box[1] - LIFELINE_BOTTOM_PAD,
            label_h: if readable { 18.0 } else { 16.0 },
            content_bottom,
            entries,
            parts,
            at,
        }
    }

    pub fn part(&self, id: &str) -> Option<&Part<'a>> {
        self.at.get(id).map(|&i| &self.parts[i])
    }

    /// `legendRequiredHeight(width)`: `0` without entries.
    pub fn legend_required_height(&self, width: f64) -> f64 {
        required_height(&self.entries, self.content_bottom, width)
    }

    /// `legendLayout()`.
    pub fn legend_layout(&self) -> LegendLayout {
        LegendLayout {
            x: LEGEND_X,
            baseline_y: self.view_box[1] - LEGEND_BASELINE_UP,
            width: self.view_box[0] - 2.0 * LEGEND_SIDE_PAD,
            min_title_y: LIFELINE_TOP.max(self.content_bottom + LEGEND_CONTENT_GAP),
            unfit: if self.doc.meta.legend.is_none() { Unfit::Hide } else { Unfit::Error },
            diagram_type: "sequence",
        }
    }

    /// `messageGeometry`: `None` for an unknown participant.
    pub fn span(&self, m: &Message) -> Option<Span> {
        let (from, to) = (self.part(&m.from)?, self.part(&m.to)?);
        let direction = if to.cx > from.cx { 1.0 } else { -1.0 };
        let start = from.cx + direction * MESSAGE_INSET;
        let end = to.cx - direction * MESSAGE_INSET;
        Some(Span { start, end, center: (start + end) / 2.0 })
    }

    /// `[from.cx, y] -> [to.cx, y]` (`messagePath`), empty for an unknown participant.
    pub fn path(&self, m: &Message) -> Vec<Pt> {
        match (self.part(&m.from), self.part(&m.to)) {
            (Some(from), Some(to)) => vec![[from.cx, m.y], [to.cx, m.y]],
            _ => Vec::new(),
        }
    }

    fn profile(&self) -> Profile {
        if self.readable { Profile::Showcase } else { Profile::Standard }
    }

    /// The label box width: `max(34, k * units + 12)`.
    pub fn label_width(&self, m: &Message) -> f64 {
        text::edge_label_width(DiagramType::Sequence, &[m.label.as_str()], self.profile())
    }

    /// `messageLabelBox`: what the gates measure.
    pub fn label_box(&self, m: &Message) -> Option<Rect> {
        let span = self.span(m)?;
        let width = self.label_width(m);
        Some(Rect::new(span.center - width / 2.0, m.y - 20.0, width, self.label_h))
    }

    /// `messageRouteBox`.
    pub fn route_box(&self, m: &Message) -> Option<Rect> {
        let span = self.span(m)?;
        Some(Rect::new(span.start.min(span.end), m.y - 2.0, (span.end - span.start).abs(), 4.0))
    }

    /// `segmentLabelBox`: bumped up 22 px, at most four times, off every message label and route.
    pub fn segment_label(&self, segment: &Segment) -> Rect {
        let width = (text::units(&segment.label) as f64 * 5.2 + 14.0).max(42.0);
        let occupied: Vec<Rect> =
            self.doc.messages.iter().flat_map(|m| [self.label_box(m), self.route_box(m)]).flatten().collect();
        let mut label = Rect::new(SEGMENT_LABEL_X, segment.from - SEGMENT_LABEL_RISE, width, SEGMENT_LABEL_H);
        for _ in 0..SEGMENT_LABEL_BUMPS {
            if !occupied.iter().any(|r| rects_overlap(&label, r, 2.0)) {
                break;
            }
            label.y -= SEGMENT_LABEL_RISE;
        }
        label
    }

    /// The frame of segment `i` (`compositionFrames`, `renderSegment`).
    pub fn segment_frame(&self, segment: &Segment) -> Rect {
        Rect::new(SEGMENT_X, segment.from, self.view_box[0] - 2.0 * SEGMENT_X, segment.to - segment.from)
    }

    /// Where a lifeline stops: above the legend title, never below `lifelineBottom`.
    pub fn lifeline_end(&self, legend: Option<&Measured>) -> f64 {
        legend.map_or(self.lifeline_bottom, |m| self.lifeline_bottom.min(m.title_y - LIFELINE_LEGEND_GAP))
    }
}

/// A placed participant: the box the gates see.
#[derive(Clone, Debug, PartialEq)]
pub struct ParticipantGeom {
    pub id: String,
    pub index: usize,
    pub cx: f64,
    pub rect: Rect,
}

/// A message with its label: the raw numbers behind `data-composition-points` and the label rect.
#[derive(Clone, Debug, PartialEq)]
pub struct MessageGeom {
    /// The message's index in `messages` (`data-edge-key`).
    pub key: usize,
    pub from: String,
    pub to: String,
    pub points: Vec<Pt>,
    /// The rendered mask rect (`messageLabel`), whose `x` is re-derived from the box centre.
    pub label: Rect,
    /// The text baseline: `(centre, y - 10)`.
    pub label_at: Pt,
    pub note_at: Option<Pt>,
    /// 11 for a showcase document, else 9 (`messageFontSize`).
    pub label_font: f64,
}

/// A segment frame and its (bumped) label box.
#[derive(Clone, Debug, PartialEq)]
pub struct SegmentGeom {
    pub frame: Rect,
    pub label: Rect,
}

/// Everything layout measured, un-rounded, in document order.
#[derive(Clone, Debug, PartialEq)]
pub struct SequenceGeometry {
    pub view_box: [f64; 2],
    pub column_fit: ColumnFit,
    pub participant_w: f64,
    pub col_gap: f64,
    pub participants: Vec<ParticipantGeom>,
    pub messages: Vec<MessageGeom>,
    pub segments: Vec<SegmentGeom>,
    pub activations: Vec<Rect>,
    /// The `y` every lifeline ends at.
    pub lifeline_end: f64,
    /// `None` when no legend is drawn (hidden, no entries, or an implicit one that does not fit).
    pub legend: Option<Measured>,
}

fn constraint(rule: &str, message: String) -> Diagnostic {
    Diagnostic::error("layout/constraint", &message).with_subject(Subject::of("sequence").with_rule(rule))
}

fn text_shape(at: Pt, text: &str, size: f64, weight: u16, anchor: Anchor, token: Token, detail: Detail) -> Shape {
    Shape::Text(TextShape { at, text: text.to_owned(), size, weight, anchor, token, detail })
}

/// The swatch of a sequence legend row: a 34-long line with the variant's arrowhead (`:451-453`).
fn legend_swatch(p: &Placed) -> Vec<Shape> {
    let variant = EdgeVariant::parse(Some(p.entry.kind));
    let width = if variant == EdgeVariant::Emphasis { EMPHASIS_WIDTH } else { MESSAGE_WIDTH };
    let y = p.baseline - 3.0;
    vec![Shape::Polyline(PolylineShape {
        points: vec![[p.x, y], [p.x + p.entry.swatch_w(), y]],
        radius: 0.0,
        stroke: Stroke { token: variant.stroke(), width, dash: variant.dash().to_vec() },
        marker: Some(Marker { fill: variant.stroke() }),
        halo: false,
    })]
}

/// What makes placement impossible: a duplicate participant, a message or activation that names a
/// participant that is not there.
fn check(plan: &Plan<'_>) -> Vec<Diagnostic> {
    let doc = plan.doc;
    let mut errors = Vec::new();
    if plan.parts.len() != doc.participants.len() {
        errors.push(constraint("graph/duplicate-node-id", "Participant ids must be unique.".to_owned()));
    }
    for m in &doc.messages {
        for (id, role) in [(&m.from, "source"), (&m.to, "target")] {
            if plan.part(id).is_none() {
                errors.push(constraint(
                    "graph/unknown-endpoint",
                    format!("Message \"{}\" references unknown {role} \"{id}\".", m.label),
                ));
            }
        }
    }
    for a in doc.activations.iter().flatten() {
        if plan.part(&a.participant).is_none() {
            errors.push(constraint(
                "graph/unknown-container",
                format!("Activation references unknown participant \"{}\".", a.participant),
            ));
        }
    }
    errors
}

fn push_participant(b: &mut SceneBuilder, p: &Part<'_>, w: f64, context: &str) {
    let n = p.participant;
    let kind = kind_of(n.kind);
    let r = p.rect;
    let brand = n.brand.is_some();
    let label_font = text::fit(&n.label, text::label_fit_width(w, brand), 11.0, 8.0);
    let sub = n.sublabel.as_deref().filter(|s| !s.is_empty());

    let mut group = Group::node(&n.id, kind, &n.label, r.into());
    group.sublabel = sub.map(str::to_owned);
    group.context = Some(context.to_owned());
    let id = b.group(group);

    b.push_in(id, Layer::Nodes, Shape::Rect(RectShape::mask(r.into(), PARTICIPANT_RADIUS)));
    b.push_in(
        id,
        Layer::Nodes,
        Shape::Rect(RectShape {
            rect: r.into(),
            radius: PARTICIPANT_RADIUS,
            fill: Some(Fill::new(Token::KindFill(kind))),
            stroke: Some(Stroke::solid(Token::KindStroke(kind), PARTICIPANT_STROKE)),
        }),
    );
    let icon = n.icon.map(icon_name);
    for path in sigils::sigil_paths(
        kind.as_str(),
        icon.as_deref(),
        r.x + sigils::SIGIL_INSET,
        TOP_Y + sigils::SIGIL_INSET,
        sigils::SIGIL_SIZE,
    ) {
        b.push_in(id, Layer::Nodes, Shape::Path(path));
    }
    crate::brand::push_badge(b, id, n.brand.as_ref(), r.x + w, TOP_Y);
    b.push_in(
        id,
        Layer::Nodes,
        text_shape([p.cx, TOP_Y + PARTICIPANT_LABEL_Y], &n.label, label_font, 600, Anchor::Middle, Token::Text, Detail::Anchor),
    );
    if let Some(s) = sub {
        let font = text::fit(s, w, 7.0, 6.0);
        b.push_in(
            id,
            Layer::Nodes,
            text_shape([p.cx, TOP_Y + PARTICIPANT_SUBLABEL_Y], s, font, 400, Anchor::Middle, Token::TextMuted, Detail::Context),
        );
    }
}

fn push_message(b: &mut SceneBuilder, m: &Message, g: &MessageGeom) {
    let variant = EdgeVariant::parse(Some(variant_name(m.variant)));
    let width = if m.variant == Some(MessageVariant::Emphasis) { EMPHASIS_WIDTH } else { MESSAGE_WIDTH };
    let edge = EdgeRef { key: g.key as u32, from: m.from.clone(), to: m.to.clone(), id: m.id.clone() };
    let id = b.group(Group::edge(edge.clone(), &m.label, &g.points, EDGE_HIT_HALF_WIDTH.max(width / 2.0)));
    b.push_in(
        id,
        Layer::Edges,
        Shape::Polyline(PolylineShape {
            points: g.points.clone(),
            radius: 0.0,
            stroke: Stroke { token: variant.stroke(), width, dash: variant.dash().to_vec() },
            marker: Some(Marker { fill: variant.stroke() }),
            halo: false,
        }),
    );

    // A coloured line gets a label in its colour; the grey ones (default, return) stay muted.
    let token = match m.variant {
        Some(MessageVariant::Emphasis | MessageVariant::Security | MessageVariant::Dashed) => variant.label(),
        _ => Token::TextMuted,
    };
    let mut group = Group::new(GroupKind::Label, format!("label-{}", g.key), g.label.into());
    group.edge = Some(edge);
    group.label = m.label.clone();
    let label = b.group(group);
    b.push_in(label, Layer::EdgeLabels, Shape::Rect(RectShape::mask(g.label.into(), LABEL_RADIUS)));
    b.push_in(label, Layer::EdgeLabels, text_shape(g.label_at, &m.label, g.label_font, 400, Anchor::Middle, token, Detail::Context));
    if let (Some(note), Some(at)) = (m.note.as_deref().filter(|n| !n.is_empty()), g.note_at) {
        b.push_in(label, Layer::EdgeLabels, text_shape(at, note, NOTE_FONT, 400, Anchor::Start, Token::TextDim, Detail::Fine));
    }
}

/// Lay out a sequence document: the scene to paint and the raw geometry.
///
/// `Err` carries what makes the document unplaceable (a duplicate participant, an unknown
/// participant) and, for an authored `meta.legend` that does not fit, `legend/label-too-wide` or
/// `legend/vertical-overflow`. The geometry gates are not run here (`gates::sequence`).
pub fn build(doc: &Sequence) -> Result<(Scene, SequenceGeometry), Vec<Diagnostic>> {
    let plan = Plan::new(doc);
    let errors = check(&plan);
    if !errors.is_empty() {
        return Err(errors);
    }
    let [width, height] = plan.view_box;
    let legend = legend::measure(&plan.entries, &plan.legend_layout()).map_err(|d| vec![*d])?;
    let lifeline_end = plan.lifeline_end(legend.as_ref());

    let participants: Vec<ParticipantGeom> = plan
        .parts
        .iter()
        .map(|p| ParticipantGeom { id: p.participant.id.clone(), index: p.index, cx: p.cx, rect: p.rect })
        .collect();
    let messages: Vec<MessageGeom> = doc
        .messages
        .iter()
        .enumerate()
        .map(|(key, m)| {
            let span = plan.span(m).expect("`check` found every participant");
            // `messageLabel`: the box is re-centred from its own `x`, so the rect `x` is derived twice.
            let label_w = plan.label_width(m);
            let center = (span.center - label_w / 2.0) + label_w / 2.0;
            let baseline = m.y - 10.0;
            MessageGeom {
                key,
                from: m.from.clone(),
                to: m.to.clone(),
                points: vec![[span.start, m.y], [span.end, m.y]],
                label: Rect::new(center - label_w / 2.0, baseline - 10.0, label_w, plan.label_h),
                label_at: [center, baseline],
                note_at: m.note.is_some().then(|| [span.start.min(span.end) + NOTE_DX, m.y + NOTE_DY]),
                label_font: if plan.readable { 11.0 } else { 9.0 },
            }
        })
        .collect();
    let segments: Vec<SegmentGeom> = doc
        .segments
        .iter()
        .flatten()
        .map(|s| SegmentGeom { frame: plan.segment_frame(s), label: plan.segment_label(s) })
        .collect();
    let activations: Vec<Rect> = doc
        .activations
        .iter()
        .flatten()
        .map(|a| {
            let cx = plan.part(&a.participant).expect("`check` found every participant").cx;
            Rect::new(cx - ACTIVATION_W / 2.0, a.from, ACTIVATION_W, a.to - a.from)
        })
        .collect();

    let mut b = SceneBuilder::new(width, height);
    b.push(
        Layer::Background,
        Shape::Rect(RectShape {
            rect: Bounds::new(0.0, 0.0, width, height),
            radius: 0.0,
            fill: Some(Fill::new(Token::Bg)),
            stroke: None,
        }),
    );
    // Segment frames, their labels with them. A label is painted above the messages and below the
    // participants (it may brush a header), hence `Edges` and not `FrameTitles`.
    for (i, (segment, g)) in doc.segments.iter().flatten().zip(&segments).enumerate() {
        let mut group = Group::new(GroupKind::Frame, format!("frame-segment-{i}"), g.frame.into());
        group.label = segment.label.clone();
        let id = b.group(group);
        b.push_in(
            id,
            Layer::Frames,
            Shape::Rect(RectShape {
                rect: g.frame.into(),
                radius: SEGMENT_RADIUS,
                fill: Some(Fill::new(Token::LaneFill)),
                stroke: Some(Stroke::dashed(Token::LaneStroke, 1.0, &SEGMENT_DASH)),
            }),
        );
        b.push_in(id, Layer::Edges, Shape::Rect(RectShape::mask(g.label.into(), LABEL_RADIUS)));
        b.push_in(
            id,
            Layer::Edges,
            text_shape([g.label.x + 6.0, g.label.y + 13.0], &segment.label, 9.0, 600, Anchor::Start, Token::TextDim, Detail::Anchor),
        );
    }
    for p in &plan.parts {
        b.push(
            Layer::Frames,
            Shape::Polyline(PolylineShape {
                points: vec![[p.cx, LIFELINE_TOP], [p.cx, lifeline_end]],
                radius: 0.0,
                stroke: Stroke::dashed(Token::Arrow, LIFELINE_WIDTH, &LIFELINE_DASH),
                marker: None,
                halo: false,
            }),
        );
    }
    for (a, rect) in doc.activations.iter().flatten().zip(&activations) {
        let part = plan.part(&a.participant).expect("`check` found every participant");
        let kind = kind_of(a.kind.unwrap_or(part.participant.kind));
        b.push(Layer::Frames, Shape::Rect(RectShape::mask((*rect).into(), ACTIVATION_RADIUS)));
        b.push(
            Layer::Frames,
            Shape::Rect(RectShape {
                rect: (*rect).into(),
                radius: ACTIVATION_RADIUS,
                fill: Some(Fill::new(Token::KindFill(kind))),
                stroke: Some(Stroke::solid(Token::KindStroke(kind), 1.0)),
            }),
        );
    }
    for (m, g) in doc.messages.iter().zip(&messages) {
        push_message(&mut b, m, g);
    }
    let locale = doc.meta.locale();
    let context = locale.t("node.context.sequence");
    for p in &plan.parts {
        push_participant(&mut b, p, plan.participant_w, &context);
    }
    if let Some(m) = &legend {
        legend::push_scene(&mut b, m, &locale, &legend_swatch);
    }

    let geometry = SequenceGeometry {
        view_box: plan.view_box,
        column_fit: plan.fit,
        participant_w: plan.participant_w,
        col_gap: plan.col_gap,
        participants,
        messages,
        segments,
        activations,
        lifeline_end,
        legend,
    };
    Ok((b.build(), geometry))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn doc(value: serde_json::Value) -> Sequence {
        serde_json::from_value(value).expect("a valid sequence")
    }

    fn base() -> serde_json::Value {
        json!({
            "schema_version": 1, "diagram_type": "sequence",
            "meta": { "title": "t", "output": "t.html" },
            "participants": [
                { "id": "a", "type": "frontend", "label": "A" },
                { "id": "b", "type": "backend", "label": "B", "sublabel": "svc" }
            ],
            "messages": [
                { "from": "a", "to": "b", "y": 180, "label": "go" },
                { "from": "b", "to": "a", "y": 240, "label": "ok", "variant": "return", "note": "n" }
            ]
        })
    }

    #[test]
    fn fixed_columns_and_the_default_canvas() {
        let (scene, g) = build(&doc(base())).unwrap();
        assert_eq!(scene.view_box, [920.0, 760.0]);
        assert_eq!(g.participants[0].rect, Rect::new(19.0, 72.0, 86.0, 60.0));
        assert_eq!(g.participants[1].cx, 170.0);
        assert_eq!(g.messages[0].points, vec![[69.0, 180.0], [163.0, 180.0]]);
        assert_eq!(g.messages[1].points, vec![[163.0, 240.0], [69.0, 240.0]]);
        // Standard labels: 9px, 5.2 per unit: max(34, 2 * 5.2 + 12) = 34 wide, 16 tall, at y - 20.
        assert_eq!(g.messages[0].label, Rect::new(99.0, 160.0, 34.0, 16.0));
        assert_eq!(g.messages[1].note_at, Some([81.0, 258.0]));
        // `default` and `return` are present: the legend lists those two, one row.
        let legend = g.legend.unwrap();
        assert_eq!(legend.entries.iter().map(|p| p.entry.kind).collect::<Vec<_>>(), ["return", "default"]);
        assert_eq!(legend.entries[0].baseline, 706.0);
        // The legend title sits at 706 - 20; lifelines stop 22 above it.
        assert_eq!(g.lifeline_end, 664.0);
    }

    #[test]
    fn spread_widens_the_columns_with_the_canvas() {
        let mut v = base();
        v["meta"]["viewBox"] = json!([1080, 580]);
        v["meta"]["column_fit"] = json!("spread");
        let (_, g) = build(&doc(v)).unwrap();
        // round((1080 - 124) / 2) - 24 = 454, clamped to 190; gap = (1080 - 102 - 190) / 1.
        assert_eq!(g.participant_w, 190.0);
        assert_eq!(g.col_gap, 788.0);
        assert_eq!(g.participants[0].cx, 157.0);
    }

    #[test]
    fn a_segment_label_is_bumped_off_a_message_label() {
        let mut v = base();
        v["segments"] = json!([{ "from": 200, "to": 300, "label": "S" }]);
        let (_, g) = build(&doc(v)).unwrap();
        // The label box would sit at y 178..196, on the first message's route; at 156..174 it is
        // still within 2 px of that message's label (x 99..133); at 134..152 it is clear.
        assert_eq!(g.segments[0].label, Rect::new(56.0, 134.0, 42.0, 18.0));
    }

    #[test]
    fn what_cannot_be_placed_is_an_error() {
        let mut v = base();
        v["messages"][0]["to"] = json!("ghost");
        v["activations"] = json!([{ "participant": "nobody", "from": 150, "to": 200 }]);
        let errs = build(&doc(v)).unwrap_err();
        let rules: Vec<_> = errs.iter().map(|d| d.subject.rule.clone().unwrap()).collect();
        assert_eq!(rules, ["graph/unknown-endpoint", "graph/unknown-container"]);
    }

    #[test]
    fn scene_is_z_ordered_with_a_group_per_part() {
        let (scene, _) = build(&doc(base())).unwrap();
        let layers: Vec<Layer> = scene.items.iter().map(|i| i.layer).collect();
        assert!(layers.windows(2).all(|w| w[0] <= w[1]));
        assert!(scene.node("b").is_some());
        assert_eq!(scene.edges_of("a").len(), 2);
        let b = scene.group(scene.node("b").unwrap()).unwrap();
        assert_eq!(b.tooltip(), "B · svc · Sequence participant");
    }
}
