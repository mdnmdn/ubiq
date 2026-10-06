//! Workflow v2 end to end (P6.3): [`build`] runs `compileWorkflowWithFeedback` over the readable
//! solver ([`super::readable`]) and router ([`crate::route::workflow`]) and paints `renderSvg`.
//!
//! One attempt is: place under the current feedback, the pre-routing geometry check
//! (`non-finite-node-geometry`, `node-overlap`), `validateReadablePinnedGeometry` (every route and
//! pin), the caller's gates (`validateWorkflow`, `gates::workflow_v2`), `finalizeReadableViewBox`, then the legend
//! against the routes. A router [`FeedbackRequest`] re-runs the attempt under
//! [`LayoutFeedback::next`], at most [`MAX_FEEDBACK_ROUNDS`] times, then fails with
//! [`feedback_failure`].
//!
//! Fix discovery recompiles the mutated document ([`Options::accepts`]). [`build`] verifies with
//! this pipeline minus the gates; [`crate::gates::workflow_v2::build`] passes the full one.

use crate::diag::Diagnostic;
use crate::geom::{Pt, Rect};
use crate::layout::workflow::readable::{
    Failure, FeedbackRequest, LayoutFeedback, MAX_FEEDBACK_ROUNDS, PlacedNode, ReadablePlacement, feedback_failure, place,
};
use crate::legend::{self, Layout as LegendLayout, Measured, Placed, Unfit, relationship_obstacles};
use crate::model::common::{NodeIcon, Variant};
use crate::model::workflow::{Edge, LaneVariant, Node, Workflow};
use crate::route::workflow::{
    Acceptor, CompositionFrame, EdgeLabel, LEGEND_STYLE, LegendRect, Router, RoutedEdge, Sides, Throw, ViewBox, kind_name, stroke_width,
};
use crate::scene::{
    Anchor, Bounds, Detail, EdgeRef, Fill, Group, GroupId, GroupKind, Layer, Marker, PolylineShape, RectShape, Scene, SceneBuilder, Shape,
    Stroke, TextShape,
};
use crate::sigils;
use crate::text::{self, DiagramType, LabelBox, Row};
use crate::tokens::{EdgeVariant, Kind, Token};

/// Node rect radius and stroke (`renderNode`: `rx 6`, `stroke-width 1.5`).
pub const NODE_RADIUS: f64 = 6.0;
pub const NODE_STROKE: f64 = 1.5;
/// Node text baselines from the top: label 21, sublabel 38, tag `height - 12`.
pub const LABEL_Y: f64 = 21.0;
pub const SUBLABEL_Y: f64 = 38.0;
pub const TAG_UP: f64 = 12.0;
/// Edge label mask radius and font (`renderEdgeLabel`).
pub const LABEL_RADIUS: f64 = 3.0;
pub const LABEL_FONT: f64 = 8.0;
/// Lane frame (`rx 10`), exception inset (`rx 8`), group (`rx 9`), phase mask (`rx 4`).
pub const LANE_RADIUS: f64 = 10.0;
pub const EXCEPTION_RADIUS: f64 = 8.0;
pub const GROUP_RADIUS: f64 = 9.0;
pub const PHASE_MASK_RADIUS: f64 = 4.0;
/// Phase rule stroke (`stroke-width 1.1`).
pub const PHASE_STROKE: f64 = 1.1;
/// Edge hit rail (screen-independent, vu); not an Archify constant.
pub const EDGE_HIT_HALF_WIDTH: f64 = 6.0;

/// What `validateWorkflow` reads, between routing and the viewBox.
pub struct GateInput<'a> {
    pub placement: &'a ReadablePlacement,
    /// Every routed edge, canonical order.
    pub edges: &'a [RoutedEdge],
    /// The endpoint sides (`edgeSides`) of each of `edges`.
    pub sides: &'a [Sides],
    pub labels: &'a [EdgeLabel],
    /// `workflowCompositionFrames`.
    pub frames: &'a [CompositionFrame],
    /// The canvas before it is finalised (`meta.viewBox`, else the minimum width and auto height).
    pub view_box: [f64; 2],
}

/// Gates run between routing and the viewBox (`validateWorkflow`, [`crate::gates::workflow_v2`]):
/// the warnings they recorded, or the failure they threw.
pub type Gates<'a> = dyn Fn(&GateInput<'_>) -> Result<Vec<Diagnostic>, Failure> + 'a;

/// How [`build_with`] verifies fixes and which gates it runs.
#[derive(Default, Clone, Copy)]
pub struct Options<'a> {
    /// `acceptsFix`; `None` is `discoverFixes: false`.
    pub accepts: Option<&'a Acceptor<'a>>,
    pub gates: Option<&'a Gates<'a>>,
}

/// Everything the receipt (`--layout-json`), the gates and the painter read.
#[derive(Debug, Clone)]
pub struct ReadableGeometry {
    pub placement: ReadablePlacement,
    /// The feedback the router settled on.
    pub feedback: LayoutFeedback,
    /// Every edge with measured endpoints, canonical order (the receipt's `edges`).
    pub edges: Vec<RoutedEdge>,
    /// Every labelled edge, canonical order (the receipt's `labels`).
    pub labels: Vec<EdgeLabel>,
    pub view_box: [f64; 2],
    pub required_view_box: [f64; 2],
    pub frames: Vec<CompositionFrame>,
    pub legend_rects: Vec<LegendRect>,
    /// The legend as painted (measured against the routes); `None` when hidden or absent.
    pub legend: Option<Measured>,
    /// `workflowDiagnostics` of the attempt that laid out: the warnings the gates recorded.
    pub diagnostics: Vec<Diagnostic>,
}

impl ReadableGeometry {
    /// The routed edge with this canonical index.
    pub fn edge(&self, index: usize) -> Option<&RoutedEdge> {
        self.edges.iter().find(|e| e.index == index)
    }

    /// The route's sides.
    pub fn sides(&self, index: usize) -> Option<Sides> {
        self.edge(index).and_then(|e| e.sides)
    }
}

/// One attempt's outcome.
enum Attempt {
    Done(Box<ReadableGeometry>),
    Feedback(FeedbackRequest),
    Failed(Failure),
}

fn attempt(w: &Workflow, feedback: &LayoutFeedback, opts: &Options<'_>) -> Attempt {
    let placement = place(w, feedback);
    let accepts_col = |node: usize, col: usize| {
        opts.accepts.is_some_and(|accepts| {
            let mut doc = placement.workflow().clone();
            doc.nodes[node].col = col as f64;
            accepts(&doc)
        })
    };
    if let Some(f) = placement.geometry_failure(accepts_col) {
        return Attempt::Failed(f);
    }
    let mut router = Router::new(&placement, opts.accepts);
    let routed = router.route_all();
    let to_attempt = |t: Throw| match t {
        Throw::Feedback(r) => Attempt::Feedback(*r),
        Throw::Failure(f) => Attempt::Failed(*f),
    };
    if let Err(t) = routed {
        return to_attempt(t);
    }
    let n = placement.workflow().edges.len();
    let edges: Vec<RoutedEdge> = (0..n).filter_map(|i| router.path(i).cloned()).collect();
    let labels: Vec<EdgeLabel> = (0..n).filter_map(|i| router.label(i).cloned()).collect();
    let mut diagnostics = Vec::new();
    if let Some(gates) = opts.gates {
        let sides: Vec<Sides> = edges.iter().filter_map(|e| router.resolved_sides(e.index)).collect();
        let view_box = placement.workflow().meta.view_box.unwrap_or([router.legend.minimum_canvas_width, router.auto_height]);
        let input = GateInput { placement: &placement, edges: &edges, sides: &sides, labels: &labels, frames: router.frames(), view_box };
        match gates(&input) {
            Ok(warnings) => diagnostics = warnings,
            Err(f) => return Attempt::Failed(f),
        }
    }
    let ViewBox { view_box, required } = match router.finalize_view_box() {
        Ok(v) => v,
        Err(t) => return to_attempt(t),
    };
    let legend = match measure_legend(&placement, &router, &edges, &labels) {
        Ok(m) => m,
        Err(d) => return Attempt::Failed(Failure::new(d.message.clone(), vec![*d])),
    };
    let frames = router.frames().to_vec();
    let legend_rects = router.legend_rects().to_vec();
    drop(router);
    Attempt::Done(Box::new(ReadableGeometry {
        placement,
        feedback: feedback.clone(),
        edges,
        labels,
        view_box,
        required_view_box: required,
        frames,
        legend_rects,
        legend,
        diagnostics,
    }))
}

/// `renderLegend` (v2): the legend measured against every route and label mask.
fn measure_legend(
    p: &ReadablePlacement,
    router: &Router<'_>,
    edges: &[RoutedEdge],
    labels: &[EdgeLabel],
) -> Result<Option<Measured>, Box<Diagnostic>> {
    let fp = &router.legend;
    if fp.entries.is_empty() {
        return Ok(None);
    }
    let obstacles = relationship_obstacles(edges.iter().map(|e| {
        let label = labels.iter().find(|l| l.index == e.index).map(|l| l.rect);
        (e.points.as_slice(), label)
    }));
    let layout = LegendLayout {
        x: 20.0,
        baseline_y: p.legend_y(fp.extra_height),
        width: fp.packing_width,
        min_title_y: p.last_lane_bottom() + 8.0,
        unfit: if p.workflow().meta.legend.is_none() { Unfit::Hide } else { Unfit::Error },
        diagram_type: "workflow",
    };
    legend::measure_styled(&fp.entries, &layout, &obstacles, LEGEND_STYLE)
}

/// `compileWorkflowWithFeedback` up to the geometry: at most [`MAX_FEEDBACK_ROUNDS`] re-runs.
pub fn compile_geometry(w: &Workflow, opts: &Options<'_>) -> Result<ReadableGeometry, Failure> {
    let mut feedback = LayoutFeedback::default();
    for round in 0..=MAX_FEEDBACK_ROUNDS {
        match attempt(w, &feedback, opts) {
            Attempt::Done(g) => return Ok(*g),
            Attempt::Failed(f) => return Err(f),
            Attempt::Feedback(request) => match feedback.next(&request) {
                Some(next) if round < MAX_FEEDBACK_ROUNDS => feedback = next,
                _ => {
                    let (error, diagnostic) = feedback_failure(&request);
                    return Err(Failure::new(error, vec![diagnostic]));
                }
            },
        }
    }
    unreachable!("the last round always returns")
}

/// The fix verifier [`build`] uses: this pipeline (no gates, no nested fix discovery) passes.
pub fn accepts_without_gates(w: &Workflow) -> bool {
    compile_geometry(w, &Options::default()).is_ok()
}

/// [`compile_geometry`] plus the scene.
pub fn build_with(w: &Workflow, opts: &Options<'_>) -> Result<(Scene, ReadableGeometry), Failure> {
    let g = compile_geometry(w, opts)?;
    Ok((scene(&g), g))
}

/// Lay out, route and paint a v2 workflow, verifying fixes without the gates.
pub fn build(w: &Workflow) -> Result<(Scene, ReadableGeometry), Vec<Diagnostic>> {
    let accepts: &Acceptor<'_> = &accepts_without_gates;
    build_with(w, &Options { accepts: Some(accepts), gates: None }).map_err(|f| f.diagnostics)
}

// ---- the scene ------------------------------------------------------------------------------------

fn variant_name(v: Option<Variant>) -> &'static str {
    match v {
        None | Some(Variant::Default) => "default",
        Some(Variant::Emphasis) => "emphasis",
        Some(Variant::Security) => "security",
        Some(Variant::Dashed) => "dashed",
    }
}

/// `variantAccent`: phase and group text.
fn variant_accent(v: Option<Variant>) -> Token {
    match v {
        Some(Variant::Security) => Token::KindStroke(Kind::Security),
        Some(Variant::Emphasis) => Token::KindStroke(Kind::Backend),
        Some(Variant::Dashed) => Token::KindStroke(Kind::Messagebus),
        _ => Token::TextMuted,
    }
}

fn kind_of(node: &Node) -> Kind {
    Kind::parse(kind_name(node.kind)).unwrap_or(Kind::External)
}

fn icon_name(icon: NodeIcon) -> String {
    serde_json::to_value(icon).ok().and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_default()
}

fn text_shape(at: Pt, text: &str, size: f64, weight: u16, anchor: Anchor, token: Token, detail: Detail) -> Shape {
    Shape::Text(TextShape { at, text: text.to_owned(), size, weight, anchor, token, detail })
}

fn edge_ref(e: &Edge, index: usize) -> EdgeRef {
    EdgeRef { key: index as u32, from: e.from.clone(), to: e.to.clone(), id: e.id.clone() }
}

/// `renderSvg` (v2) into the scene: ground, lanes, phases, groups, edges, nodes, labels, legend.
pub fn scene(g: &ReadableGeometry) -> Scene {
    let p = &g.placement;
    let w = p.workflow();
    let [width, height] = g.view_box;
    let mut b = SceneBuilder::new(width, height);
    b.push(
        Layer::Background,
        Shape::Rect(RectShape { rect: Bounds::new(0.0, 0.0, width, height), radius: 0.0, fill: Some(Fill::new(Token::Bg)), stroke: None }),
    );
    for lane in &p.lanes {
        let exception = w.lanes[lane.index].variant == Some(LaneVariant::Exception);
        let mut group = Group::new(GroupKind::Frame, format!("lane-{}", lane.index), lane.rect.into());
        group.label = lane.label.clone();
        let id = b.group(group);
        b.push_in(id, Layer::Frames, frame_rect(lane.rect, LANE_RADIUS, Token::LaneStroke, true));
        if let Some(ex) = lane.exception {
            b.push_in(id, Layer::Frames, frame_rect(ex, EXCEPTION_RADIUS, Token::KindStroke(Kind::Security), false));
        }
        let token = if exception { Token::KindStroke(Kind::Security) } else { Token::TextDim };
        let at = [lane.rect.x + 14.0, lane.rect.y + 22.0];
        b.push_in(id, Layer::Frames, text_shape(at, &lane.header, 10.0, 600, Anchor::Start, token, Detail::Anchor));
    }
    for phase in &p.phases {
        let source = &w.phases.as_ref().expect("phases")[phase.index];
        let variant = EdgeVariant::parse(Some(variant_name(source.variant)));
        let s = phase.span;
        let mut group = Group::new(GroupKind::Frame, format!("phase-{}", phase.index), Bounds::new(s.x, 27.0, s.width, 16.0));
        group.label = phase.label.clone();
        let id = b.group(group);
        b.push_in(
            id,
            Layer::Frames,
            Shape::Polyline(PolylineShape {
                points: vec![[s.x, 35.0], [s.x + s.width, 35.0]],
                radius: 0.0,
                stroke: Stroke { token: variant.stroke(), width: PHASE_STROKE, dash: variant.dash().to_vec() },
                marker: None,
                halo: false,
            }),
        );
        b.push_in(id, Layer::Frames, Shape::Rect(RectShape::mask(Bounds::new(s.x, 27.0, s.width, 16.0), PHASE_MASK_RADIUS)));
        b.push_in(id, Layer::Frames, text_shape([s.cx, 39.0], &phase.label, 8.0, 600, Anchor::Middle, variant_accent(source.variant), Detail::Anchor));
    }
    for gf in &p.groups {
        let source = &w.groups.as_ref().expect("groups")[gf.index];
        let security = source.variant == Some(Variant::Security);
        let mut group = Group::new(GroupKind::Frame, format!("group-{}", gf.index), gf.rect.into());
        group.label = gf.label.clone();
        let id = b.group(group);
        let stroke = if security { Token::KindStroke(Kind::Security) } else { Token::LaneStroke };
        b.push_in(id, Layer::Frames, frame_rect(gf.rect, GROUP_RADIUS, stroke, !security));
        b.push_in(
            id,
            Layer::Frames,
            text_shape([gf.label_x, gf.label_y], &gf.label, 7.0, 600, Anchor::Start, variant_accent(source.variant), Detail::Anchor),
        );
    }
    for routed in &g.edges {
        let e = &w.edges[routed.index];
        let variant = EdgeVariant::parse(Some(variant_name(e.variant)));
        let width = stroke_width(e);
        let label = e.label.clone().unwrap_or_default();
        let id = b.group(Group::edge(edge_ref(e, routed.index), &label, &routed.points, EDGE_HIT_HALF_WIDTH.max(width / 2.0)));
        b.push_in(
            id,
            Layer::Edges,
            Shape::Polyline(PolylineShape {
                points: routed.points.clone(),
                radius: 0.0,
                stroke: Stroke { token: variant.stroke(), width, dash: variant.dash().to_vec() },
                marker: Some(Marker { fill: variant.stroke() }),
                halo: false,
            }),
        );
    }
    for node in &p.nodes {
        push_node(&mut b, p, &w.nodes[node.index], node);
    }
    for l in &g.labels {
        let e = &w.edges[l.index];
        let variant = EdgeVariant::parse(Some(variant_name(e.variant)));
        let mut group = Group::new(GroupKind::Label, format!("label-{}", l.index), l.rect.into());
        group.edge = Some(edge_ref(e, l.index));
        group.label = l.text.clone();
        let id = b.group(group);
        b.push_in(id, Layer::EdgeLabels, Shape::Rect(RectShape::mask(l.rect.into(), LABEL_RADIUS)));
        b.push_in(id, Layer::EdgeLabels, text_shape(l.at, &l.text, LABEL_FONT, 400, Anchor::Middle, variant.label(), Detail::Context));
    }
    if let Some(m) = &g.legend {
        legend::push_scene(&mut b, m, &w.meta.locale(), &legend_swatch);
    }
    b.build()
}

fn frame_rect(rect: Rect, radius: f64, stroke: Token, fill: bool) -> Shape {
    Shape::Rect(RectShape {
        rect: rect.into(),
        radius,
        fill: fill.then(|| Fill::new(Token::LaneFill)),
        stroke: Some(Stroke::solid(stroke, 1.0)),
    })
}

/// The workflow legend swatch: a `14 x 9` kind rect, its top `8` above the baseline, `rx 2`.
fn legend_swatch(p: &Placed) -> Vec<Shape> {
    let kind = Kind::parse(p.entry.kind).unwrap_or(Kind::External);
    vec![Shape::Rect(RectShape {
        rect: Bounds::new(p.x, p.baseline - 8.0, 14.0, 9.0),
        radius: 2.0,
        fill: Some(Fill::new(Token::KindFill(kind))),
        stroke: Some(Stroke::solid(Token::KindStroke(kind), 1.0)),
    })]
}

/// `nodeContext`: lane, group and phase labels joined by ` › `, else `Workflow node`.
fn node_context(p: &ReadablePlacement, node: &Node) -> String {
    let w = p.workflow();
    let lane = w.lanes.iter().rev().find(|l| l.id == node.lane).map(|l| l.label.as_str());
    let group = w
        .groups
        .iter()
        .flatten()
        .find(|g| g.lane == node.lane && node.col >= g.from_col && node.col <= g.to_col)
        .map(|g| g.label.as_str());
    let phase = w.phases.iter().flatten().find(|ph| node.col >= ph.from_col && node.col <= ph.to_col).map(|ph| ph.label.as_str());
    let parts: Vec<&str> = [lane, group, phase].into_iter().flatten().filter(|s| !s.is_empty()).collect();
    if parts.is_empty() { w.meta.locale().t("node.context.workflow") } else { parts.join(" \u{203a} ") }
}

/// `renderNode`: mask, kind fill, sigil, label, sublabel, tag.
fn push_node(b: &mut SceneBuilder, p: &ReadablePlacement, n: &Node, placed: &PlacedNode) {
    let kind = kind_of(n);
    let r = placed.rect;
    let fonts = text::node_fonts(DiagramType::Workflow, 2);
    let brand = n.brand.is_some();
    let label_font = text::fit(&n.label, text::label_fit_width(r.width, brand), fonts.label.0, fonts.label.1);
    let sub = n.sublabel.as_deref().filter(|s| !s.is_empty());
    let tag = n.tag.as_deref().filter(|s| !s.is_empty());
    let (tag_pref, tag_min) = fonts.tag.unwrap_or((7.0, 6.0));
    let mut rows = vec![Row { text: &n.label, font: label_font, y: LABEL_Y }];
    let sub_font = sub.map(|s| text::fit(s, r.width, fonts.sublabel.0, fonts.sublabel.1));
    if let (Some(s), Some(font)) = (sub, sub_font) {
        rows.push(Row { text: s, font, y: SUBLABEL_Y });
    }
    let tag_font = tag.map(|t| text::fit(t, r.width, tag_pref, tag_min));
    if let (Some(t), Some(font)) = (tag, tag_font) {
        rows.push(Row { text: t, font, y: r.height - TAG_UP });
    }
    let layout = text::node_label_layout(&LabelBox { width: r.width, height: r.height, brand, ..LabelBox::default() }, &rows);

    let mut group = Group::node(&n.id, kind, &n.label, r.into());
    group.sublabel = sub.map(str::to_owned);
    group.context = Some(node_context(p, n));
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
    b.push_in(
        id,
        Layer::Nodes,
        text_shape([r.x + layout.x, r.y + layout.ys[0]], &n.label, label_font, 600, Anchor::Middle, Token::Text, Detail::Anchor),
    );
    let mut next = 1;
    if let (Some(s), Some(font)) = (sub, sub_font) {
        b.push_in(id, Layer::Nodes, text_shape([placed.cx, r.y + layout.ys[next]], s, font, 400, Anchor::Middle, Token::TextMuted, Detail::Context));
        next += 1;
    }
    if let (Some(t), Some(font)) = (tag, tag_font) {
        b.push_in(id, Layer::Nodes, text_shape([placed.cx, r.y + layout.ys[next]], t, font, 400, Anchor::Middle, Token::KindStroke(kind), Detail::Fine));
    }
}
