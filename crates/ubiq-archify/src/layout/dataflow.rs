//! Dataflow layout (P2.4): stage frames, node placement, routes, labels, legend and canvas, emitted
//! as a [`Scene`] plus the raw [`DataflowGeometry`] the goldens and the gates read.
//! Port of `renderers/dataflow/render-dataflow.mjs`; `00` §5.5 (Dataflow), `03` §2.3.
//!
//! Nothing here is solved: a node sits at `(stage, row)`, the canvas is the authored `viewBox` (default
//! 940 x 720) and is never grown. The geometry rules of `validateDataflow` (bounds, overlap, short
//! flow, diagonal via, label collisions) are gates (P2.5); only what makes placement impossible is
//! an error here (a duplicate id, an out-of-range stage or row, an unknown endpoint).
//!
//! The brand badge is drawn (`brand::push_badge`). Not drawn: the `grid` pattern (the classic preset
//! hides it; the others get it from `tokens::restyle`) and the source beacon (it needs repository
//! evidence, D19).

use std::collections::{HashMap, HashSet};

use crate::diag::{Diagnostic, Subject};
use crate::geom::{Pt, Rect, Side};
use crate::labels::{self, FlowLabel, Hints};
use crate::legend::{self, Layout as LegendLayout, Measured, Placed, Unfit};
use crate::model::common::{ComponentType, LegendMode, NodeIcon, Variant};
use crate::model::dataflow::{Dataflow, DataflowMeta, Flow, Node};
use crate::route::dataflow::{Routed, route_all};
use crate::scene::{
    Anchor, Bounds, Detail, EdgeRef, Fill, Group, GroupId, GroupKind, Layer, Marker, PolylineShape, RectShape,
    Scene, SceneBuilder, Shape, Stroke, TextShape,
};
use crate::sigils;
use crate::text::{self, DiagramType, LabelBox, Row};
use crate::tokens::{EdgeVariant, Kind, Token};

/// `layout.stageY` (`render-dataflow.mjs:55`): the top of every stage frame.
pub const STAGE_Y: f64 = 46.0;
/// `layout.stageH` (`:56`): the header band, used by the node-bounds gate.
pub const STAGE_H: f64 = 36.0;
/// `layout.stageBottomPad` (`:57`): a frame ends this far above the canvas bottom.
pub const STAGE_BOTTOM_PAD: f64 = 74.0;
/// `layout.leftX` (`:58`): the centre of stage 0.
pub const LEFT_X: f64 = 100.0;
/// `layout.colGap` (`:59`): the distance between stage centres.
pub const COL_GAP: f64 = 215.0;
/// `layout.stageW` (`:60`).
pub const STAGE_W: f64 = 168.0;
/// `layout.nodeW` / `nodeH` (`:61-62`): the node size when the node sets none.
pub const NODE_W: f64 = 112.0;
pub const NODE_H: f64 = 58.0;
/// `layout.rowYs` (`:63`).
pub const ROW_YS: [f64; 5] = [128.0, 242.0, 356.0, 470.0, 584.0];
/// The default `viewBox` (`:53`).
pub const DEFAULT_VIEW_BOX: [f64; 2] = [940.0, 720.0];
/// Frame radius 10 (`:88`), node radius 6 (`:447`), label radius 4 (`:469`).
pub const STAGE_RADIUS: f64 = 10.0;
pub const NODE_RADIUS: f64 = 6.0;
pub const LABEL_RADIUS: f64 = 4.0;
/// Stage frames are dashed `6,6`, 1 wide (`.c-lane`, `03` §4.2).
pub const STAGE_DASH: [f64; 2] = [6.0, 6.0];
/// The stage header: baseline `stageY + 22` (`:384`), font `9 -> 7` (`:382`), weight 600 (`:384`).
pub const HEADER_DY: f64 = 22.0;
pub const HEADER_FONT: (f64, f64) = (9.0, 7.0);
pub const HEADER_WEIGHT: u16 = 600;
/// A wrapped header line advances `font + 4` and must end `4` clear of the first node (`:407-410`).
pub const HEADER_LINE_GAP: f64 = 4.0;
/// Node rows: label baseline `21`, sublabel `37`, tag `height - 11` (`:428-430`).
pub const LABEL_Y: f64 = 21.0;
pub const SUBLABEL_Y: f64 = 37.0;
pub const TAG_UP: f64 = 11.0;
/// Node stroke 1.5 (`:448`); flow strokes 1.4, emphasis 1.8 (`:457`).
pub const NODE_STROKE: f64 = 1.5;
pub const FLOW_WIDTH: f64 = 1.4;
pub const EMPHASIS_WIDTH: f64 = 1.8;
/// Flow label text 8, classification 7 (`:470`, `:466`).
pub const LABEL_FONT: f64 = 8.0;
pub const CLASSIFICATION_FONT: f64 = 7.0;
/// The legend band (`:493-496`): baseline `H - 36`, `x` 40, `W - 80` wide, title no higher than `H - 66`.
pub const LEGEND_X: f64 = 40.0;
pub const LEGEND_BASELINE_UP: f64 = 36.0;
pub const LEGEND_SIDE_PAD: f64 = 40.0;
pub const LEGEND_MIN_TITLE_UP: f64 = 66.0;
/// Edge hit rail (screen-independent, vu) for `Hit::Polyline`; not an Archify constant.
pub const EDGE_HIT_HALF_WIDTH: f64 = 6.0;

/// A placed node: the box the router and the gates see.
#[derive(Clone, Debug, PartialEq)]
pub struct NodeGeom {
    pub id: String,
    pub rect: Rect,
    pub stage: usize,
    pub row: usize,
}

/// A routed flow with its label: the raw numbers behind `data-composition-points` and the label rect.
#[derive(Clone, Debug, PartialEq)]
pub struct FlowGeom {
    /// The flow's index in `flows` (`data-edge-key`).
    pub key: usize,
    pub from: String,
    pub to: String,
    pub points: Vec<Pt>,
    pub from_side: Side,
    pub to_side: Side,
    pub label: FlowLabel,
}

/// Everything layout measured, un-rounded, in document order.
#[derive(Clone, Debug, PartialEq)]
pub struct DataflowGeometry {
    pub view_box: [f64; 2],
    pub frames: Vec<Rect>,
    pub nodes: Vec<NodeGeom>,
    pub flows: Vec<FlowGeom>,
    /// `None` when no legend is drawn (hidden, no entries, or an implicit one that does not fit).
    pub legend: Option<Measured>,
}

/// `stageX(index)`.
pub fn stage_x(index: usize) -> f64 {
    LEFT_X + index as f64 * COL_GAP
}

/// `stageFrame`: `x = cx - 84`, `y = 46`, `w = 168`, `h = H - 46 - 74`.
pub fn stage_frame(index: usize, view_box: [f64; 2]) -> Rect {
    Rect::new(stage_x(index) - STAGE_W / 2.0, STAGE_Y, STAGE_W, view_box[1] - STAGE_Y - STAGE_BOTTOM_PAD)
}

fn constraint(rule: &str, message: String) -> Diagnostic {
    Diagnostic::error("layout/constraint", &message).with_subject(Subject::of("dataflow").with_rule(rule))
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

fn variant_name(v: Option<Variant>) -> &'static str {
    match v {
        None | Some(Variant::Default) => "default",
        Some(Variant::Emphasis) => "emphasis",
        Some(Variant::Security) => "security",
        Some(Variant::Dashed) => "dashed",
    }
}

fn icon_name(icon: NodeIcon) -> String {
    serde_json::to_value(icon).ok().and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_default()
}

/// `String(n + 1).padStart(2, '0') + " / " + label`.
fn stage_title(index: usize, label: &str) -> String {
    format!("{:02} / {label}", index + 1)
}

/// `measureNode`: `x = cx - width / 2`, `y = rowYs[row] + yOffset`.
fn measure_node(node: &Node, stage: usize, row: usize) -> Rect {
    let (width, height) = (node.width.unwrap_or(NODE_W), node.height.unwrap_or(NODE_H));
    let cx = stage_x(stage);
    Rect::new(cx - width / 2.0, ROW_YS[row] + node.y_offset.unwrap_or(0.0), width, height)
}

fn index_of(value: f64, len: usize) -> Option<usize> {
    (value.is_finite() && value.fract() == 0.0 && value >= 0.0 && (value as usize) < len).then_some(value as usize)
}

/// The grapheme clusters of `word`, approximated: a mark, variation selector, ZWJ joiner, skin-tone
/// modifier or tag attaches to the previous cluster, and a regional-indicator pair is one. (Archify
/// uses `Intl.Segmenter`; no segmentation crate is in the lock.)
fn graphemes(word: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut joined = false;
    let mut flag_open = false;
    for c in word.chars() {
        let cp = c as u32;
        let extends = matches!(cp, 0x0300..=0x036F | 0x1AB0..=0x1AFF | 0x1DC0..=0x1DFF | 0x20D0..=0x20FF
            | 0xFE00..=0xFE0F | 0x200D | 0x1F3FB..=0x1F3FF | 0xE0020..=0xE007F);
        let regional = (0x1F1E6..=0x1F1FF).contains(&cp);
        let attach = !out.is_empty() && (extends || joined || (regional && flag_open));
        if attach {
            out.last_mut().expect("non-empty").push(c);
        } else {
            out.push(c.to_string());
        }
        flag_open = regional && !(attach && flag_open);
        joined = cp == 0x200D;
    }
    out
}

/// The lines of a stage header (`renderStageHeader`): one line when it fits at its fitted font, else
/// wrapped on word boundaries (whitespace runs are kept), splitting oversized words by grapheme; the
/// single line again when the wrapped block would run into the first node of the stage.
fn header_lines(title: &str, font: f64, first_node_y: f64) -> Vec<String> {
    let available = text::available_width(STAGE_W);
    if text::min_text_width(title, font) <= available {
        return vec![title.to_owned()];
    }
    let overflows = |s: &str| text::min_text_width(s, font) > available;
    let mut lines: Vec<String> = Vec::new();
    let mut line = String::new();
    let mut words: Vec<String> = Vec::new();
    for c in title.chars() {
        match words.last_mut() {
            Some(w) if w.chars().next().is_some_and(char::is_whitespace) == c.is_whitespace() => w.push(c),
            _ => words.push(c.to_string()),
        }
    }
    for word in &words {
        if !line.is_empty() && overflows(&format!("{line}{word}")) {
            lines.push(std::mem::take(&mut line));
        }
        for segment in graphemes(word) {
            if !line.is_empty() && overflows(&format!("{line}{segment}")) {
                lines.push(std::mem::take(&mut line));
            }
            line.push_str(&segment);
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    let last_bottom = STAGE_Y + HEADER_DY + (lines.len() - 1) as f64 * (font + HEADER_LINE_GAP) + font * 0.3;
    if last_bottom > first_node_y - HEADER_LINE_GAP {
        return vec![title.to_owned()];
    }
    lines
}

fn text_shape(at: Pt, text: &str, size: f64, weight: u16, token: Token, detail: Detail) -> Shape {
    Shape::Text(TextShape { at, text: text.to_owned(), size, weight, anchor: Anchor::Middle, token, detail })
}

fn legend_config(meta: &DataflowMeta) -> Option<legend::Config> {
    let l = meta.legend.as_ref()?;
    let mut entries = HashMap::new();
    if let Some(e) = &l.entries {
        for (kind, entry) in [
            ("default", &e.default),
            ("emphasis", &e.emphasis),
            ("security", &e.security),
            ("dashed", &e.dashed),
            ("database", &e.database),
        ] {
            if let Some(o) = entry {
                entries.insert(kind.to_owned(), legend::Override { label: o.label.clone(), visible: o.visible });
            }
        }
    }
    Some(legend::Config { mode: l.mode.unwrap_or(LegendMode::Auto), entries })
}

/// The swatch of a dataflow legend row: a `14 x 9` kind rect for `database`, else a 34-long line
/// with the variant's arrowhead (`render-dataflow.mjs:501-503`).
fn legend_swatch(p: &Placed) -> Vec<Shape> {
    if p.entry.kind == "database" {
        return vec![Shape::Rect(RectShape {
            rect: Bounds::new(p.x, p.baseline - 8.0, 14.0, 9.0),
            radius: 2.0,
            fill: Some(Fill::new(Token::KindFill(Kind::Database))),
            stroke: Some(Stroke::solid(Token::KindStroke(Kind::Database), 1.0)),
        })];
    }
    let variant = EdgeVariant::parse(Some(p.entry.kind));
    let width = if variant == EdgeVariant::Emphasis { EMPHASIS_WIDTH } else { FLOW_WIDTH };
    let y = p.baseline - 3.0;
    vec![Shape::Polyline(PolylineShape {
        points: vec![[p.x, y], [p.x + p.entry.swatch_w(), y]],
        radius: 0.0,
        stroke: Stroke { token: variant.stroke(), width, dash: variant.dash().to_vec() },
        marker: Some(Marker { fill: variant.stroke() }),
        halo: false,
    })]
}

struct Placement {
    nodes: Vec<NodeGeom>,
    by_id: HashMap<String, Rect>,
}

/// Place the nodes, or say why they cannot be placed.
fn place(doc: &Dataflow) -> Result<Placement, Vec<Diagnostic>> {
    let mut errors = Vec::new();
    let mut by_id: HashMap<String, Rect> = HashMap::new();
    let mut nodes = Vec::with_capacity(doc.nodes.len());
    let mut seen = HashSet::new();
    if doc.nodes.iter().any(|n| !seen.insert(n.id.as_str())) {
        errors.push(constraint("graph/duplicate-node-id", "Node ids must be unique.".to_owned()));
    }
    for n in &doc.nodes {
        let stage = index_of(n.stage, doc.stages.len());
        let row = index_of(n.row, ROW_YS.len());
        if stage.is_none() {
            errors.push(constraint(
                "dataflow/stage-range",
                format!("Node \"{}\" uses invalid stage {} — valid stages are 0..{}.", n.id, n.stage, doc.stages.len() as f64 - 1.0),
            ));
        }
        if row.is_none() {
            errors.push(constraint(
                "dataflow/row-range",
                format!("Node \"{}\" uses invalid row {} — valid rows are 0..{}.", n.id, n.row, ROW_YS.len() - 1),
            ));
        }
        if let (Some(stage), Some(row)) = (stage, row) {
            let rect = measure_node(n, stage, row);
            by_id.insert(n.id.clone(), rect);
            nodes.push(NodeGeom { id: n.id.clone(), rect, stage, row });
        }
    }
    for f in &doc.flows {
        for (end, role) in [(&f.from, "source"), (&f.to, "target")] {
            if !by_id.contains_key(end) && doc.nodes.iter().all(|n| &n.id != end) {
                let name = if f.label.is_empty() { end } else { &f.label };
                errors.push(constraint(
                    "graph/unknown-endpoint",
                    format!("Flow \"{name}\" references unknown {role} \"{end}\"."),
                ));
            }
        }
    }
    if errors.is_empty() { Ok(Placement { nodes, by_id }) } else { Err(errors) }
}

fn push_stage(b: &mut SceneBuilder, doc: &Dataflow, p: &Placement, index: usize, view_box: [f64; 2]) {
    let frame = stage_frame(index, view_box);
    let label = &doc.stages[index].label;
    let mut group = Group::new(GroupKind::Frame, format!("frame-stage-{index}"), frame.into());
    group.label = label.clone();
    let id = b.group(group);
    b.push_in(
        id,
        Layer::Frames,
        Shape::Rect(RectShape {
            rect: frame.into(),
            radius: STAGE_RADIUS,
            fill: Some(Fill::new(Token::LaneFill)),
            stroke: Some(Stroke::dashed(Token::LaneStroke, 1.0, &STAGE_DASH)),
        }),
    );
    // The header sits with its frame, under the edges, as Archify draws it.
    let title = stage_title(index, label);
    let font = text::fit(&title, STAGE_W, HEADER_FONT.0, HEADER_FONT.1);
    let first_node_y = p
        .nodes
        .iter()
        .filter(|n| n.stage == index)
        .map(|n| n.rect.y)
        .fold(view_box[1] - STAGE_BOTTOM_PAD, f64::min);
    let (cx, mut y) = (stage_x(index), STAGE_Y + HEADER_DY);
    for (i, line) in header_lines(&title, font, first_node_y).iter().enumerate() {
        if i > 0 {
            y += font + HEADER_LINE_GAP;
        }
        b.push_in(id, Layer::Frames, text_shape([cx, y], line, font, HEADER_WEIGHT, Token::TextDim, Detail::Anchor));
    }
}

fn push_node(b: &mut SceneBuilder, doc: &Dataflow, n: &Node, g: &NodeGeom) {
    let kind = kind_of(n.kind);
    let r = g.rect;
    let fonts = text::node_fonts(DiagramType::Dataflow, 1);
    let brand = n.brand.is_some();
    let label_font = text::fit(&n.label, text::label_fit_width(r.width, brand), fonts.label.0, fonts.label.1);
    let sub = n.sublabel.as_deref().filter(|s| !s.is_empty());
    let tag = n.tag.as_deref().filter(|s| !s.is_empty());
    let (sub_pref, sub_min) = fonts.sublabel;
    let (tag_pref, tag_min) = fonts.tag.unwrap_or((7.0, 6.0));

    let mut rows = vec![Row { text: &n.label, font: label_font, y: LABEL_Y }];
    let sub_font = sub.map(|s| text::fit(s, r.width, sub_pref, sub_min));
    if let (Some(s), Some(font)) = (sub, sub_font) {
        rows.push(Row { text: s, font, y: SUBLABEL_Y });
    }
    let tag_font = tag.map(|t| text::fit(t, r.width, tag_pref, tag_min));
    if let (Some(t), Some(font)) = (tag, tag_font) {
        rows.push(Row { text: t, font, y: r.height - TAG_UP });
    }
    let layout = text::node_label_layout(
        &LabelBox { width: r.width, height: r.height, brand, ..LabelBox::default() },
        &rows,
    );

    let mut group = Group::node(&n.id, kind, &n.label, r.into());
    group.sublabel = sub.map(str::to_owned);
    group.context = Some(match doc.stages.get(g.stage) {
        Some(s) => stage_title(g.stage, &s.label),
        None => doc.meta.locale().t("node.context.dataflow"),
    });
    group.tags = tag.map(str::to_owned).into_iter().collect();
    let id: GroupId = b.group(group);

    b.push_in(id, Layer::Nodes, Shape::Rect(RectShape::mask(r.into(), NODE_RADIUS)));
    b.push_in(
        id,
        Layer::Nodes,
        Shape::Rect(RectShape {
            rect: r.into(),
            radius: NODE_RADIUS,
            fill: Some(Fill::new(Token::KindFill(kind))),
            stroke: Some(Stroke::solid(Token::KindStroke(kind), NODE_STROKE)),
        }),
    );
    let icon = n.icon.map(icon_name);
    for path in sigils::sigil_paths(kind.as_str(), icon.as_deref(), r.x + sigils::SIGIL_INSET, r.y + layout.sigil_y, layout.sigil_size) {
        b.push_in(id, Layer::Nodes, Shape::Path(path));
    }
    crate::brand::push_badge(b, id, n.brand.as_ref(), r.x + r.width, r.y);
    b.push_in(id, Layer::Nodes, text_shape([r.x + layout.x, r.y + layout.ys[0]], &n.label, label_font, 600, Token::Text, Detail::Anchor));
    let cx = stage_x(g.stage);
    let mut next = 1;
    if let (Some(s), Some(font)) = (sub, sub_font) {
        b.push_in(id, Layer::Nodes, text_shape([cx, r.y + layout.ys[next]], s, font, 400, Token::TextMuted, Detail::Context));
        next += 1;
    }
    if let (Some(t), Some(font)) = (tag, tag_font) {
        b.push_in(id, Layer::Nodes, text_shape([cx, r.y + layout.ys[next]], t, font, 400, Token::KindStroke(kind), Detail::Fine));
    }
}

fn push_flow(b: &mut SceneBuilder, flow: &Flow, geom: &FlowGeom) {
    let variant = EdgeVariant::parse(Some(variant_name(flow.variant)));
    let edge = EdgeRef { key: geom.key as u32, from: flow.from.clone(), to: flow.to.clone(), id: flow.id.clone() };
    let width = flow.width.filter(|w| *w != 0.0).unwrap_or(if flow.variant == Some(Variant::Emphasis) { EMPHASIS_WIDTH } else { FLOW_WIDTH });
    let id = b.group(Group::edge(edge, &flow.label, &geom.points, EDGE_HIT_HALF_WIDTH.max(width / 2.0)));
    b.push_in(
        id,
        Layer::Edges,
        Shape::Polyline(PolylineShape {
            points: geom.points.clone(),
            radius: 0.0,
            stroke: Stroke { token: variant.stroke(), width, dash: variant.dash().to_vec() },
            marker: Some(Marker { fill: variant.stroke() }),
            halo: false,
        }),
    );
}

fn push_label(b: &mut SceneBuilder, flow: &Flow, geom: &FlowGeom) {
    let variant = EdgeVariant::parse(Some(variant_name(flow.variant)));
    let edge = EdgeRef { key: geom.key as u32, from: flow.from.clone(), to: flow.to.clone(), id: flow.id.clone() };
    let mut group = Group::new(GroupKind::Label, format!("label-{}", geom.key), geom.label.rect.into());
    group.edge = Some(edge);
    group.label = flow.label.clone();
    let id = b.group(group);
    let [lx, ly] = geom.label.at;
    b.push_in(id, Layer::EdgeLabels, Shape::Rect(RectShape::mask(geom.label.rect.into(), LABEL_RADIUS)));
    b.push_in(id, Layer::EdgeLabels, text_shape([lx, ly], &flow.label, LABEL_FONT, 400, variant.label(), Detail::Context));
    if let Some(c) = flow.classification.as_deref().filter(|c| !c.is_empty()) {
        b.push_in(
            id,
            Layer::EdgeLabels,
            text_shape([lx, ly + labels::DATAFLOW_LABEL_RISE], c, CLASSIFICATION_FONT, 400, Token::TextDim, Detail::Fine),
        );
    }
}

/// Lay out a dataflow document: the scene to paint and the raw geometry.
///
/// `Err` carries what makes the document unplaceable (a duplicate id, a stage or row out of range, an
/// unknown endpoint) and, for an authored `meta.legend` that does not fit, `legend/label-too-wide`
/// or `legend/vertical-overflow`. The geometry gates are not run here (P2.5).
pub fn build(doc: &Dataflow) -> Result<(Scene, DataflowGeometry), Vec<Diagnostic>> {
    let view_box = doc.meta.view_box.unwrap_or(DEFAULT_VIEW_BOX);
    let placement = place(doc)?;

    let routed: Vec<Option<Routed>> = route_all(&doc.flows, &placement.by_id);
    let mut flows = Vec::with_capacity(doc.flows.len());
    for (key, (flow, routed)) in doc.flows.iter().zip(routed).enumerate() {
        let routed = routed.expect("`place` checked every endpoint");
        let hints = Hints { at: flow.label_at, dx: flow.label_dx, dy: flow.label_dy, segment: flow.label_segment };
        let label = labels::dataflow_label(&hints, &routed.points, &flow.label, flow.classification.as_deref());
        flows.push(FlowGeom {
            key,
            from: flow.from.clone(),
            to: flow.to.clone(),
            points: routed.points,
            from_side: routed.from_side,
            to_side: routed.to_side,
            label,
        });
    }

    let present: HashSet<&str> = doc
        .flows
        .iter()
        .map(|f| variant_name(f.variant))
        .chain(doc.nodes.iter().any(|n| n.kind == ComponentType::Database).then_some("database"))
        .collect();
    let config = legend_config(&doc.meta);
    let entries = legend::resolve(config.as_ref(), &legend::dataflow_catalog(&doc.meta.locale()), &present);
    let legend = legend::measure(
        &entries,
        &LegendLayout {
            x: LEGEND_X,
            baseline_y: view_box[1] - LEGEND_BASELINE_UP,
            width: view_box[0] - 2.0 * LEGEND_SIDE_PAD,
            min_title_y: view_box[1] - LEGEND_MIN_TITLE_UP,
            unfit: if doc.meta.legend.is_none() { Unfit::Hide } else { Unfit::Error },
            diagram_type: "dataflow",
        },
    )
    .map_err(|d| vec![*d])?;

    let mut b = SceneBuilder::new(view_box[0], view_box[1]);
    b.push(
        Layer::Background,
        Shape::Rect(RectShape {
            rect: Bounds::new(0.0, 0.0, view_box[0], view_box[1]),
            radius: 0.0,
            fill: Some(Fill::new(Token::Bg)),
            stroke: None,
        }),
    );
    for index in 0..doc.stages.len() {
        push_stage(&mut b, doc, &placement, index, view_box);
    }
    for (flow, geom) in doc.flows.iter().zip(&flows) {
        push_flow(&mut b, flow, geom);
    }
    for (node, geom) in doc.nodes.iter().zip(&placement.nodes) {
        push_node(&mut b, doc, node, geom);
    }
    for (flow, geom) in doc.flows.iter().zip(&flows) {
        push_label(&mut b, flow, geom);
    }
    if let Some(m) = &legend {
        legend::push_scene(&mut b, m, &doc.meta.locale(), &legend_swatch);
    }

    let geometry = DataflowGeometry {
        view_box,
        frames: (0..doc.stages.len()).map(|i| stage_frame(i, view_box)).collect(),
        nodes: placement.nodes,
        flows,
        legend,
    };
    Ok((b.build(), geometry))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn doc(value: serde_json::Value) -> Dataflow {
        serde_json::from_value(value).expect("a valid dataflow")
    }

    fn base() -> serde_json::Value {
        json!({
            "schema_version": 1, "diagram_type": "dataflow",
            "meta": { "title": "t", "output": "t.html" },
            "stages": [{ "label": "In" }, { "label": "Out" }],
            "nodes": [
                { "id": "a", "type": "frontend", "label": "A", "stage": 0, "row": 0 },
                { "id": "b", "type": "database", "label": "B", "stage": 1, "row": 0 }
            ],
            "flows": [{ "from": "a", "to": "b", "label": "go" }]
        })
    }

    #[test]
    fn the_default_canvas_and_stage_frames() {
        let (scene, g) = build(&doc(base())).unwrap();
        assert_eq!(scene.view_box, [940.0, 720.0]);
        assert_eq!(g.frames[1], Rect::new(231.0, 46.0, 168.0, 600.0));
        assert_eq!(g.nodes[1].rect, Rect::new(259.0, 128.0, 112.0, 58.0));
        assert_eq!(g.flows[0].points, vec![[156.0, 157.0], [259.0, 157.0]]);
        // `database` is present, so the legend lists data flow + data store, wrapped to one row.
        let legend = g.legend.unwrap();
        assert_eq!(legend.entries.iter().map(|p| p.entry.kind).collect::<Vec<_>>(), ["database", "default"]);
        assert_eq!(legend.entries[0].baseline, 684.0);
    }

    #[test]
    fn what_cannot_be_placed_is_an_error() {
        let mut v = base();
        v["nodes"][1]["stage"] = json!(7);
        v["nodes"][0]["row"] = json!(5);
        v["flows"][0]["to"] = json!("nope");
        let errs = build(&doc(v)).unwrap_err();
        let rules: Vec<_> = errs.iter().map(|d| d.subject.rule.clone().unwrap()).collect();
        assert_eq!(rules, ["dataflow/row-range", "dataflow/stage-range", "graph/unknown-endpoint"]);
    }

    #[test]
    fn an_authored_legend_that_does_not_fit_is_an_error_and_an_implicit_one_is_dropped() {
        let mut v = base();
        v["meta"]["viewBox"] = json!([100, 720]);
        // Implicit: dropped (and the nodes are outside the narrow canvas, which a gate reports).
        let (_, g) = build(&doc(v.clone())).unwrap();
        assert!(g.legend.is_none());
        v["meta"]["legend"] = json!({ "mode": "all" });
        let errs = build(&doc(v)).unwrap_err();
        assert_eq!(errs[0].code, "legend/label-too-wide");
    }

    #[test]
    fn a_long_stage_title_wraps_by_word() {
        let lines = header_lines("01 / A very long stage title that cannot fit", 7.0, 128.0);
        assert!(lines.len() > 1);
        assert_eq!(lines.concat(), "01 / A very long stage title that cannot fit");
        // Too close to the first node: the single overflowing line stays.
        assert_eq!(header_lines("01 / A very long stage title that cannot fit", 7.0, 70.0).len(), 1);
    }

    #[test]
    fn graphemes_keep_sequences_together() {
        assert_eq!(graphemes("e\u{301}x"), ["e\u{301}", "x"]);
        assert_eq!(graphemes("\u{1F468}\u{200D}\u{1F469}!"), ["\u{1F468}\u{200D}\u{1F469}", "!"]);
        assert_eq!(graphemes("\u{1F1EF}\u{1F1F5}\u{1F1FA}\u{1F1F8}").len(), 2);
    }

    #[test]
    fn scene_is_z_ordered_with_a_group_per_part() {
        let (scene, _) = build(&doc(base())).unwrap();
        let layers: Vec<Layer> = scene.items.iter().map(|i| i.layer).collect();
        assert!(layers.windows(2).all(|w| w[0] <= w[1]));
        assert!(scene.node("a").is_some());
        assert_eq!(scene.edges_of("a").len(), 1);
        let a = scene.group(scene.node("a").unwrap()).unwrap();
        assert_eq!(a.tooltip(), "A · 01 / In");
    }
}
