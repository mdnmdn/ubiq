//! Workflow v2 router (P6.3): the edge half of `compileWorkflowInternal`
//! (`renderers/workflow/workflow-compiler.mjs:1295-4660`). `00` section 5.6.5 (Workflow v2) and
//! section 3.4c (W); `03` section 3.5.
//!
//! [`Router::new`] starts from a [`ReadablePlacement`] (P6.2) and builds what the compiler derives
//! before routing: the legend footprint and minimum canvas width (`resolveWorkflowLegendFootprint`),
//! the legend rects, the scene label obstacles, the composition frames and the automatic port spread
//! (`automaticPortSpread` with `edgeSides`, which already runs the side search). Then:
//!
//! - [`Router::route_all`] is `validateReadablePinnedGeometry`: it routes every edge in Archify's
//!   order (pinned routes at contested nodes first, then canonical order) and raises the pin
//!   conflicts (`workflow/explicit-pin-conflict`), `workflow/route-preset-conflict`,
//!   `workflow/solver-budget-exhausted`, or a [`FeedbackRequest`] for the solver.
//! - [`Router::finalize_view_box`] is `finalizeReadableViewBox` (`requiredViewBox`, the origin
//!   check, `workflow/viewbox-capacity`). `validateWorkflow` (P6.4) runs between the two in Archify.
//!
//! Every edge is routed by `pathFor`: a controlled route (`via`, `channelX`/`channelY`, a preset)
//! by `readableControlledRoute`, every other one by `readableAutomaticRoute`, which tries the nine
//! candidate families of `readableAutomaticCandidateSet` per side pair and port, keeps the feasible
//! ones (`readableCandidateIsFeasible`) and takes the cheapest under the lexicographic
//! [`COST_PRIORITY`], with the outside-right expansion (`readableAutomaticVia`) and the corridor
//! repair pass.
//!
//! Fix discovery (`acceptsFix`: recompile the mutated document) is the caller's [`Acceptor`]; with
//! none, every verified alternative is empty, as with `discoverFixes: false`.
//!
//! # JS parity
//!
//! - **Edge identity.** Archify keys its caches by edge object; here by canonical edge index
//!   (`workflow.edges.indexOf(edge)`). Paths carry the authored index (`sourceIndexes.edges`).
//! - **Obstacle grid.** `createSpatialGrid` only narrows the candidates of a query to a superset of
//!   what the box can reach; every predicate re-tests the geometry, so a linear scan returns the
//!   same answer. The two sums that depend on order (`labelRouteClearanceDeficit`,
//!   `routeInteractionMetrics`) sort by registration sequence, route before label, which is the
//!   order [`Router`] keeps them in. Items outside the query box contribute `0`.
//! - **Node centres.** A node's `cx` is `colXs[col]`, not `x + width / 2` (which can differ in the
//!   last ulp), so anchors and side defaults read the stored centre ([`anchor`]), not `Rect::cx`.
//! - **`classifyFailedAutomaticCandidatePins`** reads `points` off the raw candidates, which have
//!   none: a labelled edge without `labelAt` crashes there in JS (`TypeError`), a pinned label is
//!   checked at its pin. Both are reproduced (the crash as `internal/unclassified`).
//! - **Legend.** `measureLegend` never fails for v2 (the packing width is at least the widest entry
//!   and the band starts 14 px under the last lane), so the legend rects are computed once.
//! - **Numbers.** `Math.round` is [`js_round`]; `Math.max`/`Math.min` on finite values; sums keep
//!   Archify's operand order.

use std::cmp::Ordering;
use std::collections::HashSet;

use serde_json::{Map, Value, json};

use crate::diag::{Diagnostic, Subject, js_round, json_num};
use crate::gates::clean_flow::route_honors_endpoint_sides;
use crate::geom::{
    BorderSide, Frame, Pt, Rect, Seg, collinear_axis_overlap, cross_product, frame_border_segments, is_finite_point, normalize,
    join_route_points, rects_overlap, segment_intersects_rect, segment_rect_clearance,
};
use crate::geom::Side;
use crate::labels::{Hints, label_point};
use crate::layout::workflow::readable::{
    Failure, FeedbackKind, FeedbackRequest, LabelObstacle, LabelObstacleKind, LANE_X, PlacedNode, ReadablePlacement,
    has_absolute_workflow_pins, js_max, js_min, js_num, stable_value_key, workflow_label_width,
};
use crate::legend::{self, Entry, Layout as LegendLayout, Style as LegendStyle, Unfit};
use crate::model::common::{ComponentType, LegendMode, SchemaVersion, Side as ModelSide, Variant};
use crate::model::workflow::{Edge, EdgeRoute, Role, Workflow, WorkflowMeta};
use crate::text::units;

/// `READABLE_CANDIDATE_COST_PRIORITY`, the lexicographic order of [`Cost`].
pub const COST_PRIORITY: [&str; 11] = [
    "properCrossingCount",
    "sharedCorridorPx",
    "automaticForwardReversePx",
    "labelRouteClearanceDeficit",
    "interiorPreferred28Deficit",
    "bendCount",
    "stretchMilli",
    "canvasGrowthPx",
    "portDisplacementMilli",
    "legacyCoordinateDisplacement",
    "stableCandidateOrdinal",
];

/// The nine automatic candidate families, in ordinal order (`readableAutomaticCandidateSet`).
pub const FAMILIES: [&str; 9] = [
    "facing-straight",
    "horizontal-then-vertical",
    "vertical-then-horizontal",
    "lane-gap-corridor",
    "column-gap-corridor",
    "outside-left",
    "outside-right",
    "top-corridor",
    "bottom-corridor",
];

/// `LEGACY_COLUMN_CENTERS`.
const LEGACY_COLUMN_CENTERS: [f64; 6] = [88.0, 220.0, 300.0, 430.0, 500.0, 625.0];
const EPS: f64 = 0.0001;
const SIDE_ORDER: [Side; 4] = [Side::Right, Side::Bottom, Side::Left, Side::Top];
const PRESETS: [EdgeRoute; 6] = [
    EdgeRoute::Straight,
    EdgeRoute::Drop,
    EdgeRoute::OutsideRight,
    EdgeRoute::ReturnLeft,
    EdgeRoute::BottomChannel,
    EdgeRoute::UpChannel,
];

/// Verifies a fix (`acceptsFix`): does the whole compile of this mutated canonical document pass?
pub type Acceptor<'a> = dyn Fn(&Workflow) -> bool + 'a;

/// Why routing stopped: a solver request (`WorkflowLayoutFeedback`) or a thrown diagnostic error.
#[derive(Debug, Clone, PartialEq)]
pub enum Throw {
    Feedback(Box<FeedbackRequest>),
    Failure(Box<Failure>),
}

type R<T> = Result<T, Throw>;

fn fail(message: String, diagnostic: Diagnostic) -> Throw {
    Throw::Failure(Box::new(Failure::new(message, vec![diagnostic])))
}

/// A resolved endpoint-side pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sides {
    pub from: Side,
    pub to: Side,
}

/// A routed edge (`pathCache`).
#[derive(Debug, Clone, PartialEq)]
pub struct RoutedEdge {
    /// Canonical index.
    pub index: usize,
    /// `pathFor(edge).points`, un-rounded (normalised unless absolutely pinned).
    pub points: Vec<Pt>,
    /// The sides `readableSideCache` settled on.
    pub sides: Option<Sides>,
    /// Registration order (`obstacleSequence`).
    pub sequence: usize,
}

/// An edge label: its anchor (`workflowEdgeLabelPoint`) and mask rect (`labelRectFor`).
#[derive(Debug, Clone, PartialEq)]
pub struct EdgeLabel {
    pub index: usize,
    pub text: String,
    pub at: Pt,
    pub rect: Rect,
}

/// One `workflowLegendRects` rect.
#[derive(Debug, Clone, PartialEq)]
pub struct LegendRect {
    /// `title` or the entry's kind.
    pub kind: String,
    pub rect: Rect,
}

/// One composition frame (`workflowCompositionFrames`).
#[derive(Debug, Clone, PartialEq)]
pub struct CompositionFrame {
    /// `lane-<i>`, `lane-<i>-exception`, `group-<i>`.
    pub id: String,
    /// `lane`, `exception-lane`, `group`.
    pub kind: &'static str,
    pub label: String,
    pub rect: Rect,
    pub radius: f64,
}

/// The `[left, top, right, bottom]` box of `measuredContentBounds` and its contributors.
#[derive(Debug, Clone, PartialEq)]
pub struct ContentBounds {
    pub left: f64,
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
    pub contributors: Vec<String>,
}

/// The finished canvas.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewBox {
    pub view_box: [f64; 2],
    pub required: [f64; 2],
}

/// `resolveWorkflowLegendFootprint` (v2).
#[derive(Debug, Clone, PartialEq)]
pub struct LegendFootprint {
    pub entries: Vec<Entry>,
    pub packing_width: f64,
    pub extra_height: f64,
    pub minimum_canvas_width: f64,
}

/// The workflow legend style (`fontSize: 7, itemGap: 7`).
pub const LEGEND_STYLE: LegendStyle = LegendStyle { font_size: 7.0, item_gap: 7.0 };

fn legend_config(meta: &WorkflowMeta) -> Option<legend::Config> {
    let l = meta.legend.as_ref()?;
    let mut entries = std::collections::HashMap::new();
    if let Some(e) = &l.entries {
        for (kind, entry) in [
            ("frontend", &e.frontend),
            ("backend", &e.backend),
            ("database", &e.database),
            ("cloud", &e.cloud),
            ("security", &e.security),
            ("messagebus", &e.messagebus),
            ("external", &e.external),
        ] {
            if let Some(o) = entry {
                entries.insert(kind.to_owned(), legend::Override { label: o.label.clone(), visible: o.visible });
            }
        }
    }
    Some(legend::Config { mode: l.mode.unwrap_or(LegendMode::Auto), entries })
}

/// The `type` string of a node kind.
pub fn kind_name(kind: ComponentType) -> &'static str {
    match kind {
        ComponentType::Frontend => "frontend",
        ComponentType::Backend => "backend",
        ComponentType::Database => "database",
        ComponentType::Cloud => "cloud",
        ComponentType::Security => "security",
        ComponentType::Messagebus => "messagebus",
        ComponentType::External => "external",
    }
}

/// `resolveWorkflowLegendFootprint` for a v2 document: the resolved entries, the one-row minimum
/// canvas width (`max(defaultViewBoxWidth, minWidth + 40)`), the packing width and the extra rows.
pub fn legend_footprint(w: &Workflow, default_view_box_width: f64) -> LegendFootprint {
    let present: HashSet<&str> = w.nodes.iter().map(|n| kind_name(n.kind)).collect();
    let config = legend_config(&w.meta);
    let entries = legend::resolve(config.as_ref(), &legend::workflow_catalog(&w.meta.locale()), &present);
    let one_row = legend::footprint_styled(&entries, 9_007_199_254_740_991.0, LEGEND_STYLE);
    let minimum_canvas_width = js_max(default_view_box_width, one_row.min_width + 40.0);
    let packing_width = js_max(1.0, minimum_canvas_width - 40.0);
    let packed = legend::footprint_styled(&entries, packing_width, LEGEND_STYLE);
    LegendFootprint { entries, packing_width, extra_height: packed.extra_height, minimum_canvas_width }
}

// ---- edge helpers ---------------------------------------------------------------------------------

fn gside(side: Option<ModelSide>) -> Option<Side> {
    side.map(|s| match s {
        ModelSide::Left => Side::Left,
        ModelSide::Right => Side::Right,
        ModelSide::Top => Side::Top,
        ModelSide::Bottom => Side::Bottom,
    })
}

/// The side name Archify prints.
pub fn side_name(side: Side) -> &'static str {
    match side {
        Side::Left => "left",
        Side::Right => "right",
        Side::Top => "top",
        Side::Bottom => "bottom",
    }
}

/// The preset name Archify prints.
pub fn route_name(route: EdgeRoute) -> &'static str {
    match route {
        EdgeRoute::Auto => "auto",
        EdgeRoute::Straight => "straight",
        EdgeRoute::Drop => "drop",
        EdgeRoute::OutsideRight => "outside-right",
        EdgeRoute::ReturnLeft => "return-left",
        EdgeRoute::BottomChannel => "bottom-channel",
        EdgeRoute::UpChannel => "up-channel",
    }
}

/// `edge.route && edge.route !== 'auto'`: the authored preset.
fn preset_of(e: &Edge) -> Option<EdgeRoute> {
    e.route.filter(|r| *r != EdgeRoute::Auto)
}

/// `hasAbsoluteRoutePins`: `via`, `channelX` or `channelY`.
pub fn has_absolute_route_pins(e: &Edge) -> bool {
    e.via.is_some() || e.channel_x.is_some() || e.channel_y.is_some()
}

/// `edge.label` read for truthiness.
fn label_of(e: &Edge) -> Option<&str> {
    e.label.as_deref().filter(|l| !l.is_empty())
}

/// `edge.width || (variant === 'emphasis' ? 1.8 : 1.4)`.
pub fn stroke_width(e: &Edge) -> f64 {
    match e.width {
        Some(w) if w != 0.0 && !w.is_nan() => w,
        _ => {
            if e.variant == Some(Variant::Emphasis) {
                1.8
            } else {
                1.4
            }
        }
    }
}

fn return_or_error(e: &Edge) -> bool {
    matches!(e.role, Some(Role::Return) | Some(Role::Error))
}

/// `independentAutomaticRoute` (v2): nothing authored about the route, the label or the sides.
pub fn independent_automatic_route(e: &Edge) -> bool {
    !has_absolute_route_pins(e) && e.label_at.is_none() && preset_of(e).is_none() && e.from_side.is_none() && e.to_side.is_none()
}

/// `workflowEdgeName`.
fn edge_name(e: &Edge) -> String {
    match e.id.as_deref() {
        Some(id) if !id.is_empty() => id.to_owned(),
        _ => format!("{}->{}", e.from, e.to),
    }
}

/// `anchor(node, side)` on the measured node (stored `cx`/`cy`).
pub fn anchor(n: &PlacedNode, side: Side) -> Pt {
    match side {
        Side::Left => [n.rect.x, n.cy],
        Side::Right => [n.rect.x + n.rect.width, n.cy],
        Side::Top => [n.cx, n.rect.y],
        Side::Bottom => [n.cx, n.rect.y + n.rect.height],
    }
}

/// `legacyDefaultFromSide` (imported as `defaultFromSide` by the workflow compiler).
fn default_from_side(from: &PlacedNode, to: &PlacedNode) -> Side {
    if to.cx < from.cx {
        Side::Left
    } else if to.cx > from.cx {
        Side::Right
    } else if to.cy > from.cy {
        Side::Bottom
    } else {
        Side::Top
    }
}

/// `legacyDefaultToSide`.
fn default_to_side(from: &PlacedNode, to: &PlacedNode) -> Side {
    if to.cx < from.cx {
        Side::Right
    } else if to.cx > from.cx {
        Side::Left
    } else if to.cy > from.cy {
        Side::Top
    } else {
        Side::Bottom
    }
}

fn is_vertical(side: Side) -> bool {
    matches!(side, Side::Top | Side::Bottom)
}

/// `OUTWARD_SIDE_VECTOR` stub of 16.
fn outward_stub(p: Pt, side: Side) -> Pt {
    match side {
        Side::Left => [p[0] + -16.0, p[1]],
        Side::Right => [p[0] + 16.0, p[1]],
        Side::Top => [p[0], p[1] + -16.0],
        Side::Bottom => [p[0], p[1] + 16.0],
    }
}

fn corridor_via_y(start: Pt, end: Pt, fs: Side, ts: Side, y: f64) -> Vec<Pt> {
    let (a, b) = (outward_stub(start, fs), outward_stub(end, ts));
    vec![a, [a[0], y], [b[0], y], b]
}

fn corridor_via_x(start: Pt, end: Pt, fs: Side, ts: Side, x: f64) -> Vec<Pt> {
    let (a, b) = (outward_stub(start, fs), outward_stub(end, ts));
    vec![a, [x, a[1]], [x, b[1]], b]
}

fn ceil3(x: f64) -> f64 {
    (x * 1000.0).ceil() / 1000.0
}

/// `{minX, minY, maxX, maxY}` of `routeBounds`/`rectToBounds`.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Bounds {
    min_x: f64,
    min_y: f64,
    max_x: f64,
    max_y: f64,
}

fn route_bounds(points: &[Pt]) -> Bounds {
    let mut b = Bounds { min_x: f64::INFINITY, min_y: f64::INFINITY, max_x: f64::NEG_INFINITY, max_y: f64::NEG_INFINITY };
    for p in points {
        if p[0] < b.min_x {
            b.min_x = p[0];
        }
        if p[0] > b.max_x {
            b.max_x = p[0];
        }
        if p[1] < b.min_y {
            b.min_y = p[1];
        }
        if p[1] > b.max_y {
            b.max_y = p[1];
        }
    }
    b
}

fn rect_bounds(r: &Rect) -> Bounds {
    Bounds { min_x: r.x, min_y: r.y, max_x: r.x + r.width, max_y: r.y + r.height }
}

fn bounds_overlap(a: &Bounds, b: &Bounds, margin: f64) -> bool {
    a.min_x <= b.max_x + margin && b.min_x <= a.max_x + margin && a.min_y <= b.max_y + margin && b.min_y <= a.max_y + margin
}

fn seg_hits(a: Pt, b: Pt, rect: &Rect, gap: f64) -> bool {
    segment_intersects_rect(&Seg::new(a, b), rect, gap)
}

fn clearance(a: Pt, b: Pt, rect: &Rect) -> Option<f64> {
    segment_rect_clearance(&Seg::new(a, b), rect)
}

fn any_segment_hits(points: &[Pt], rect: &Rect) -> bool {
    points.windows(2).any(|w| seg_hits(w[0], w[1], rect, 0.0))
}

/// `segmentOrientation`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Orientation {
    Vertical,
    Horizontal,
    Diagonal,
}

fn orientation(a: Pt, b: Pt) -> Orientation {
    if (a[0] - b[0]).abs() <= EPS {
        Orientation::Vertical
    } else if (a[1] - b[1]).abs() <= EPS {
        Orientation::Horizontal
    } else {
        Orientation::Diagonal
    }
}

/// `orthogonalRoute`.
fn orthogonal_route(points: &[Pt]) -> bool {
    points.iter().enumerate().all(|(i, p)| {
        if !is_finite_point(p) {
            return false;
        }
        if i == 0 {
            return true;
        }
        let q = points[i - 1];
        ((p[0] - q[0]).abs() <= EPS) != ((p[1] - q[1]).abs() <= EPS)
    })
}

/// `routeMeetsHardRhythm`: a direct route at least 28, endpoint stubs 8, interior legs 16.
fn route_meets_hard_rhythm(points: &[Pt]) -> bool {
    if points.len() == 2 {
        return (points[1][0] - points[0][0]).hypot(points[1][1] - points[0][1]) + EPS >= 28.0;
    }
    let last = points.len() - 2;
    points.windows(2).enumerate().all(|(i, w)| {
        let length = (w[1][0] - w[0][0]).abs() + (w[1][1] - w[0][1]).abs();
        length + EPS >= if i == 0 || i == last { 8.0 } else { 16.0 }
    })
}

/// `properAxisCrossing`.
fn proper_axis_crossing(a: Pt, b: Pt, c: Pt, d: Pt) -> bool {
    let first_h = (a[1] - b[1]).abs() <= EPS;
    let second_h = (c[1] - d[1]).abs() <= EPS;
    if first_h == second_h {
        return false;
    }
    let (h, v) = if first_h { ([a, b], [c, d]) } else { ([c, d], [a, b]) };
    let x = v[0][0];
    let y = h[0][1];
    x > js_min(h[0][0], h[1][0]) + EPS
        && x < js_max(h[0][0], h[1][0]) - EPS
        && y > js_min(v[0][1], v[1][1]) + EPS
        && y < js_max(v[0][1], v[1][1]) - EPS
}

/// `endpointSideIsHonored` (raw points).
fn endpoint_side_is_honored(points: &[Pt], side: Side, source: bool) -> bool {
    if points.len() < 2 {
        return true;
    }
    let n = points.len();
    let (from, to) = if source { (points[0], points[1]) } else { (points[n - 2], points[n - 1]) };
    let dx = to[0] - from[0];
    let dy = to[1] - from[1];
    let s = if source { 1.0 } else { -1.0 };
    match side {
        Side::Right => dx * s > 0.0 && dy.abs() <= EPS,
        Side::Left => dx * s < 0.0 && dy.abs() <= EPS,
        Side::Bottom => dy * s > 0.0 && dx.abs() <= EPS,
        Side::Top => dy * s < 0.0 && dx.abs() <= EPS,
    }
}

/// `routeContainsChannelPin`: some leg runs along the pinned line (exact coordinate).
fn route_contains_channel_pin(points: &[Pt], x_axis: bool, value: f64) -> bool {
    points.windows(2).any(|w| {
        let (s, e) = (w[0], w[1]);
        if x_axis {
            s[0] == value && e[0] == value && (e[1] - s[1]).abs() > EPS
        } else {
            s[1] == value && e[1] == value && (e[0] - s[0]).abs() > EPS
        }
    })
}

fn orientations(points: &[Pt]) -> Vec<Orientation> {
    points.windows(2).map(|w| orientation(w[0], w[1])).collect()
}

/// `corridorTopologyMatches`.
fn corridor_topology_matches(points: &[Pt], x_axis: bool, coordinate: f64) -> bool {
    let collapsed = normalize(points);
    let (Some(&start), Some(&end)) = (collapsed.first(), collapsed.last()) else { return false };
    let via = if x_axis { [[coordinate, start[1]], [coordinate, end[1]]] } else { [[start[0], coordinate], [end[0], coordinate]] };
    let expected = join_route_points(start, &via, end);
    orientations(&collapsed) == orientations(&expected) && route_contains_channel_pin(&collapsed, x_axis, coordinate)
}

/// `routeMatchesPresetFamily`.
fn route_matches_preset_family(preset: EdgeRoute, points: &[Pt], from: &PlacedNode, to: &PlacedNode) -> bool {
    let collapsed = normalize(points);
    let segments: Vec<(Pt, Orientation)> = collapsed.windows(2).map(|w| (w[0], orientation(w[0], w[1]))).collect();
    match preset {
        EdgeRoute::Straight => collapsed.len() == 2,
        EdgeRoute::Drop => {
            if from.lane == to.lane {
                return false;
            }
            if collapsed.len() == 2 && segments.first().map(|s| s.1) == Some(Orientation::Vertical) {
                return true;
            }
            let (upper, lower) = if from.cy <= to.cy { (from, to) } else { (to, from) };
            segments.iter().any(|(start, o)| {
                *o == Orientation::Horizontal
                    && start[1] >= upper.rect.y + upper.rect.height - EPS
                    && start[1] <= lower.rect.y + EPS
                    && corridor_topology_matches(points, false, start[1])
            })
        }
        EdgeRoute::OutsideRight | EdgeRoute::ReturnLeft => {
            let right = preset == EdgeRoute::OutsideRight;
            let boundary = if right {
                js_max(from.rect.x + from.rect.width, to.rect.x + to.rect.width)
            } else {
                js_min(from.rect.x, to.rect.x)
            };
            segments.iter().any(|(start, o)| {
                *o == Orientation::Vertical
                    && (if right { start[0] > boundary + EPS } else { start[0] < boundary - EPS })
                    && corridor_topology_matches(points, true, start[0])
            })
        }
        EdgeRoute::BottomChannel | EdgeRoute::UpChannel => {
            let bottom = preset == EdgeRoute::BottomChannel;
            let boundary = if bottom {
                js_max(from.rect.y + from.rect.height, to.rect.y + to.rect.height)
            } else {
                js_min(from.rect.y, to.rect.y)
            };
            segments.iter().any(|(start, o)| {
                *o == Orientation::Horizontal
                    && (if bottom { start[1] > boundary + EPS } else { start[1] < boundary - EPS })
                    && corridor_topology_matches(points, false, start[1])
            })
        }
        EdgeRoute::Auto => false,
    }
}

/// `READABLE_PRESET_PIN_FIELDS`.
fn preset_pin_fields(preset: EdgeRoute) -> &'static [&'static str] {
    match preset {
        EdgeRoute::Straight | EdgeRoute::Auto => &[],
        EdgeRoute::Drop | EdgeRoute::BottomChannel | EdgeRoute::UpChannel => &["channelY"],
        EdgeRoute::OutsideRight | EdgeRoute::ReturnLeft => &["channelX"],
    }
}

/// `presentChannelPins`.
fn present_channel_pins(e: &Edge) -> Vec<&'static str> {
    let mut out = Vec::new();
    if e.channel_x.is_some() {
        out.push("channelX");
    }
    if e.channel_y.is_some() {
        out.push("channelY");
    }
    out
}

fn channel_pin_value(e: &Edge, field: &str) -> Option<f64> {
    if field == "channelX" { e.channel_x } else { e.channel_y }
}

/// `authoredRouteAssertionFields`.
fn route_assertion_fields(e: &Edge) -> Vec<&'static str> {
    let mut out = Vec::new();
    if e.via.is_some() {
        out.push("via");
    }
    if e.channel_x.is_some() {
        out.push("channelX");
    }
    if e.channel_y.is_some() {
        out.push("channelY");
    }
    if preset_of(e).is_some() {
        out.push("route");
    }
    if e.from_side.is_some() {
        out.push("fromSide");
    }
    if e.to_side.is_some() {
        out.push("toSide");
    }
    out
}

/// `presentRouteGeometryFields`.
fn route_geometry_fields(e: &Edge) -> Vec<&'static str> {
    route_assertion_fields(e).into_iter().filter(|f| matches!(*f, "via" | "channelX" | "channelY")).collect()
}

fn field_present(e: &Edge, field: &str) -> bool {
    match field {
        "via" => e.via.is_some(),
        "channelX" => e.channel_x.is_some(),
        "channelY" => e.channel_y.is_some(),
        "route" => e.route.is_some(),
        "fromSide" => e.from_side.is_some(),
        "toSide" => e.to_side.is_some(),
        "labelAt" => e.label_at.is_some(),
        _ => false,
    }
}

fn delete_field(e: &mut Edge, field: &str) {
    match field {
        "via" => e.via = None,
        "channelX" => e.channel_x = None,
        "channelY" => e.channel_y = None,
        "route" => e.route = None,
        "fromSide" => e.from_side = None,
        "toSide" => e.to_side = None,
        "labelAt" => e.label_at = None,
        _ => {}
    }
}

/// A JSON value with JS number text (whole floats as integers).
fn js_json(v: Value) -> Value {
    match v {
        Value::Number(n) => n.as_f64().map_or(Value::Number(n), json_num),
        Value::Array(items) => Value::Array(items.into_iter().map(js_json).collect()),
        Value::Object(map) => Value::Object(map.into_iter().map(|(k, v)| (k, js_json(v))).collect()),
        other => other,
    }
}

fn field_value(e: &Edge, field: &str) -> Value {
    let value = serde_json::to_value(e).ok().and_then(|v| v.get(field).cloned()).unwrap_or(Value::Null);
    js_json(value)
}

fn pt_json(p: Pt) -> Value {
    json!([json_num(p[0]), json_num(p[1])])
}

fn points_json(points: &[Pt]) -> Value {
    Value::Array(points.iter().map(|p| pt_json(*p)).collect())
}

fn rect_json(r: &Rect) -> Value {
    json!({ "x": json_num(r.x), "y": json_num(r.y), "width": json_num(r.width), "height": json_num(r.height) })
}

fn obj(pairs: Vec<(&str, Value)>) -> Map<String, Value> {
    pairs.into_iter().map(|(k, v)| (k.to_owned(), v)).collect()
}

/// `combinations(values, size)`: lexicographic index combinations.
fn combinations(n: usize, size: usize) -> Vec<Vec<usize>> {
    fn go(n: usize, size: usize, start: usize, prefix: &mut Vec<usize>, out: &mut Vec<Vec<usize>>) {
        if prefix.len() == size {
            out.push(prefix.clone());
            return;
        }
        let mut i = start;
        while i + (size - prefix.len()) <= n {
            prefix.push(i);
            go(n, size, i + 1, prefix, out);
            prefix.pop();
            i += 1;
        }
    }
    let mut out = Vec::new();
    go(n, size, 0, &mut Vec::new(), &mut out);
    out
}

// ---- cost -----------------------------------------------------------------------------------------

/// `readableCandidateCost`, in [`COST_PRIORITY`] order.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Cost(pub [f64; 11]);

impl Cost {
    fn shared_corridor_px(&self) -> f64 {
        self.0[1]
    }

    fn ordinal(&self) -> f64 {
        self.0[10]
    }
}

/// `compareCost`: the first dimension that differs (`|| 0`, so `NaN` reads `0`).
pub fn compare_cost(left: &Cost, right: &Cost) -> Ordering {
    let z = |x: f64| if x.is_nan() { 0.0 } else { x };
    for i in 0..11 {
        let (l, r) = (z(left.0[i]), z(right.0[i]));
        if l != r {
            return (l - r).partial_cmp(&0.0).unwrap_or(Ordering::Equal);
        }
    }
    Ordering::Equal
}

/// A candidate route of an automatic plan.
#[derive(Debug, Clone)]
struct Plan {
    points: Vec<Pt>,
    sides: Sides,
    cost: Cost,
}

/// `readableAutomaticCandidateSet`'s result.
struct CandidateSet {
    raw: Vec<Vec<Pt>>,
    candidates: Vec<Plan>,
    outside_right: f64,
}

// ---- the router -----------------------------------------------------------------------------------

/// The routing state of one compile attempt (`compileWorkflowInternal` from the obstacle grid on).
pub struct Router<'a> {
    p: &'a ReadablePlacement,
    accepts: Option<&'a Acceptor<'a>>,
    pub legend: LegendFootprint,
    /// `autoHeight` of `createWorkflowLaneGeometry`.
    pub auto_height: f64,
    legend_rects: Vec<LegendRect>,
    scene_obstacles: Vec<LabelObstacle>,
    frames: Vec<CompositionFrame>,
    /// Per canonical edge, the measured endpoint nodes (positions in `p.nodes`).
    ends: Vec<Option<(usize, usize)>>,
    paths: Vec<Option<RoutedEdge>>,
    labels: Vec<Option<EdgeLabel>>,
    /// `readableSideCache`: absent, `null`, or sides.
    side_cache: Vec<Option<Option<Sides>>>,
    /// Registered edges in sequence order.
    order: Vec<usize>,
    rightmost_node_edge: f64,
    rightmost_routed_edge: f64,
    /// `automaticPorts`.
    ports: Vec<(Option<Pt>, Option<Pt>)>,
}

impl<'a> Router<'a> {
    /// Everything `compileWorkflowInternal` sets up before `validateReadableInputsBeforeRouting`,
    /// including the automatic port spread (which runs the side search over the bare nodes).
    pub fn new(p: &'a ReadablePlacement, accepts: Option<&'a Acceptor<'a>>) -> Self {
        let w = p.workflow();
        let legend = legend_footprint(w, p.layout.default_view_box_width);
        let auto_height = p.auto_height(legend.extra_height);
        let mut legend_rects = Vec::new();
        if !legend.entries.is_empty() {
            let layout = LegendLayout {
                x: 20.0,
                baseline_y: p.legend_y(legend.extra_height),
                width: legend.packing_width,
                min_title_y: p.last_lane_bottom() + 8.0,
                unfit: if w.meta.legend.is_none() { Unfit::Hide } else { Unfit::Error },
                diagram_type: "workflow",
            };
            if let Ok(Some(m)) = legend::measure_styled(&legend.entries, &layout, &[], LEGEND_STYLE) {
                legend_rects.push(LegendRect { kind: "title".into(), rect: Rect::new(20.0, m.title_y - 10.0, 48.0, 14.0) });
                for e in &m.entries {
                    legend_rects
                        .push(LegendRect { kind: e.entry.kind.to_owned(), rect: Rect::new(e.x, e.baseline - 10.0, e.width, 14.0) });
                }
            }
        }
        let mut frames = Vec::new();
        for lane in &p.lanes {
            frames.push(CompositionFrame {
                id: format!("lane-{}", lane.index),
                kind: "lane",
                label: lane.label.clone(),
                rect: lane.rect,
                radius: 10.0,
            });
            if let Some(ex) = lane.exception {
                frames.push(CompositionFrame {
                    id: format!("lane-{}-exception", lane.index),
                    kind: "exception-lane",
                    label: format!("{} exception", lane.label),
                    rect: ex,
                    radius: 8.0,
                });
            }
        }
        for g in &p.groups {
            frames.push(CompositionFrame {
                id: format!("group-{}", g.index),
                kind: "group",
                label: g.label.clone(),
                rect: g.rect,
                radius: 9.0,
            });
        }
        let position = |id: &str| p.nodes.iter().position(|n| n.id == id);
        let ends = w.edges.iter().map(|e| Some((position(&e.from)?, position(&e.to)?))).collect();
        let rightmost_node_edge = p.nodes.iter().fold(0.0, |m, n| js_max(m, n.rect.x + n.rect.width));
        let n = w.edges.len();
        let mut router = Router {
            p,
            accepts,
            legend,
            auto_height,
            legend_rects,
            scene_obstacles: p.label_obstacles(),
            frames,
            ends,
            paths: vec![None; n],
            labels: vec![None; n],
            side_cache: vec![None; n],
            order: Vec::new(),
            rightmost_node_edge,
            rightmost_routed_edge: f64::NEG_INFINITY,
            ports: vec![(None, None); n],
        };
        router.ports = router.automatic_port_spread();
        router
    }

    fn w(&self) -> &'a Workflow {
        self.p.workflow()
    }

    fn edge(&self, i: usize) -> &'a Edge {
        &self.p.workflow().edges[i]
    }

    fn nodes_of(&self, i: usize) -> Option<(&'a PlacedNode, &'a PlacedNode)> {
        let p: &'a ReadablePlacement = self.p;
        self.ends[i].map(|(a, b)| (&p.nodes[a], &p.nodes[b]))
    }

    /// The legend rects (`workflowLegendRects`).
    pub fn legend_rects(&self) -> &[LegendRect] {
        &self.legend_rects
    }

    /// The composition frames (`workflowCompositionFrames`).
    pub fn frames(&self) -> &[CompositionFrame] {
        &self.frames
    }

    /// The routed edge, once [`Router::route_all`] (or a pin check) reached it.
    pub fn path(&self, i: usize) -> Option<&RoutedEdge> {
        self.paths[i].as_ref()
    }

    /// The edge label of a routed edge (`labelRectFor`).
    pub fn label(&self, i: usize) -> Option<&EdgeLabel> {
        self.labels[i].as_ref()
    }

    /// `edgeSides` of a routed edge as `validateWorkflow` reads it: the cached sides, else the
    /// authored or default pair.
    pub fn resolved_sides(&mut self, i: usize) -> Option<Sides> {
        self.ends[i].map(|_| self.edge_sides(i))
    }

    // ---- labels ---------------------------------------------------------------------------------

    /// `workflowEdgeLabelPoint` (v2).
    pub fn label_point(&self, e: &Edge, points: &[Pt]) -> Pt {
        let hints = Hints { at: e.label_at, dx: e.label_dx, dy: e.label_dy, segment: e.label_segment };
        let integer_segment = e.label_segment.is_some_and(|s| s.is_finite() && s.fract() == 0.0);
        if e.label_at.is_none()
            && !integer_segment
            && e.label_dx.is_none()
            && e.label_dy.is_none()
            && points.len() == 2
            && points[0][0] == points[1][0]
        {
            return [points[0][0], (points[0][1] + points[1][1]) / 2.0];
        }
        if e.label_at.is_some() || integer_segment || points.len() <= 2 {
            return label_point(&hints, points);
        }
        let mut segments: Vec<(usize, bool, f64)> = points
            .windows(2)
            .enumerate()
            .map(|(i, w)| (i, (w[1][1] - w[0][1]).abs() <= EPS, (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1])))
            .collect();
        segments.sort_by(|l, r| {
            (r.1 as i32 - l.1 as i32)
                .cmp(&0)
                .then_with(|| (r.2 - l.2).partial_cmp(&0.0).unwrap_or(Ordering::Equal))
                .then_with(|| l.0.cmp(&r.0))
        });
        let segment = segments.first().map_or(0, |s| s.0);
        let mut point = label_point(&Hints { segment: Some(segment as f64), ..hints }, points);
        if points[segment][0] == points[segment + 1][0] {
            point[1] += 10.0;
        }
        point
    }

    /// `candidateLabelRect`.
    fn candidate_label_rect(&self, e: &Edge, points: &[Pt]) -> Option<Rect> {
        let label = label_of(e)?;
        let [lx, ly] = self.label_point(e, points);
        let width = workflow_label_width(label);
        Some(Rect::new(lx - width / 2.0, ly - 10.0, width, 14.0))
    }

    // ---- feasibility ----------------------------------------------------------------------------

    /// `routeClearsUnrelatedNodes`.
    fn route_clears_unrelated_nodes(&self, e: &Edge, points: &[Pt], clearance: f64) -> bool {
        let extent = route_bounds(points);
        self.p.nodes.iter().all(|node| {
            if node.id == e.from || node.id == e.to || !bounds_overlap(&extent, &rect_bounds(&node.rect), clearance) {
                return true;
            }
            !points.windows(2).any(|w| seg_hits(w[0], w[1], &node.rect, clearance))
        })
    }

    /// `routeClearsEndpointNodes`.
    fn route_clears_endpoint_nodes(points: &[Pt], from: &PlacedNode, to: &PlacedNode) -> bool {
        let last = points.len() - 2;
        points.windows(2).enumerate().all(|(i, w)| {
            !(i > 0 && seg_hits(w[0], w[1], &from.rect, 0.0)) && !(i < last && seg_hits(w[0], w[1], &to.rect, 0.0))
        })
    }

    /// `routeLabelClearsNodes`.
    fn route_label_clears_nodes(&self, e: &Edge, points: &[Pt]) -> bool {
        if label_of(e).is_none() || e.label_at.is_some() {
            return true;
        }
        let Some(rect) = self.candidate_label_rect(e, points) else { return true };
        !self.p.nodes.iter().any(|n| rects_overlap(&rect, &n.rect, -2.0))
    }

    /// The label rects of the registered edges, in sequence order.
    fn placed_labels(&self) -> impl Iterator<Item = (usize, &EdgeLabel)> {
        self.order.iter().filter_map(|&k| self.labels[k].as_ref().map(|l| (k, l)))
    }

    /// `labelRouteClearanceDeficit` (threshold 8).
    fn label_route_clearance_deficit(&self, e: &Edge, points: &[Pt]) -> f64 {
        let threshold = 8.0;
        let candidate = self.candidate_label_rect(e, points);
        let mut deficit = 0.0;
        for &k in &self.order {
            if let (Some(label), Some(other)) = (candidate, self.paths[k].as_ref()) {
                for w in other.points.windows(2) {
                    if let Some(c) = clearance(w[0], w[1], &label) {
                        deficit += js_max(0.0, threshold - c);
                    }
                }
            }
            if let Some(l) = &self.labels[k] {
                for w in points.windows(2) {
                    if let Some(c) = clearance(w[0], w[1], &l.rect) {
                        deficit += js_max(0.0, threshold - c);
                    }
                }
            }
        }
        deficit
    }

    /// `routeClearsPlacedLabels` (4 px).
    fn route_clears_placed_labels(&self, e: &Edge, points: &[Pt]) -> bool {
        let candidate = self.candidate_label_rect(e, points);
        for (_, l) in self.placed_labels() {
            if candidate.is_some_and(|c| rects_overlap(&c, &l.rect, -2.0)) {
                return false;
            }
            if points.windows(2).any(|w| clearance(w[0], w[1], &l.rect).is_some_and(|c| c + EPS < 4.0)) {
                return false;
            }
        }
        if let Some(c) = candidate {
            for &k in &self.order {
                let Some(other) = &self.paths[k] else { continue };
                if other.points.windows(2).any(|w| clearance(w[0], w[1], &c).is_some_and(|d| d + EPS < 4.0)) {
                    return false;
                }
            }
        }
        true
    }

    /// `routeClearsLegend`.
    fn route_clears_legend(&self, e: &Edge, points: &[Pt]) -> bool {
        if self.legend.entries.is_empty() {
            return true;
        }
        let label = self.candidate_label_rect(e, points);
        !self.legend_rects.iter().any(|r| any_segment_hits(points, &r.rect) || label.is_some_and(|l| rects_overlap(&l, &r.rect, 0.0)))
    }

    /// `routeClearsSceneLabelObstacles`.
    fn route_clears_scene_label_obstacles(&self, e: &Edge, points: &[Pt]) -> bool {
        let label = self.candidate_label_rect(e, points);
        !self
            .scene_obstacles
            .iter()
            .any(|o| any_segment_hits(points, &o.rect) || label.is_some_and(|l| rects_overlap(&l, &o.rect, 0.0)))
    }

    /// `routeClearsFrameBorders`.
    fn route_clears_frame_borders(&self, points: &[Pt]) -> bool {
        first_border_run(points, &self.frames).is_none()
    }

    /// `routeFitsCanvasOrigin` over `routeExtentCoordinates`.
    fn route_fits_canvas_origin(&self, e: &Edge, points: &[Pt]) -> bool {
        let ok = |p: &Pt| p[0] >= 0.0 && p[1] >= 0.0;
        if !points.iter().all(ok) {
            return false;
        }
        if e.label_at.is_none()
            && let Some(l) = self.candidate_label_rect(e, points)
        {
            return ok(&[l.x, l.y]) && ok(&[l.x + l.width, l.y + l.height]);
        }
        true
    }

    /// `readableCandidateIsFeasible`.
    fn feasible(&self, e: &Edge, points: &[Pt], from: &PlacedNode, to: &PlacedNode, sides: Sides) -> bool {
        points.len() >= 2
            && orthogonal_route(points)
            && route_meets_hard_rhythm(points)
            && route_honors_endpoint_sides(points, sides.from, sides.to)
            && Self::route_clears_endpoint_nodes(points, from, to)
            && self.route_label_clears_nodes(e, points)
            && self.route_clears_scene_label_obstacles(e, points)
            && self.route_clears_unrelated_nodes(e, points, 2.0)
            && self.route_clears_placed_labels(e, points)
            && self.route_fits_canvas_origin(e, points)
            && self.route_clears_frame_borders(points)
            && self.route_clears_legend(e, points)
    }

    // ---- cost -----------------------------------------------------------------------------------

    /// `routeInteractionMetrics`: proper crossings and shared corridor length against every routed
    /// edge (v2 judges shared endpoints too, with the short-trunk exemption).
    fn interaction(&self, e: &Edge, points: &[Pt]) -> (f64, f64) {
        let mut crossings = 0.0;
        let mut shared = 0.0;
        for &k in &self.order {
            let other = self.edge(k);
            let Some(routed) = &self.paths[k] else { continue };
            shared += corridor_overlap(e, points, other, &routed.points, true, true).map_or(0.0, |c| c.length);
            for l in points.windows(2) {
                for r in routed.points.windows(2) {
                    if proper_axis_crossing(l[0], l[1], r[0], r[1]) {
                        crossings += 1.0;
                    }
                }
            }
        }
        (crossings, shared)
    }

    /// `automaticForwardReversePx`.
    fn forward_reverse_px(e: &Edge, points: &[Pt], from: &PlacedNode, to: &PlacedNode) -> f64 {
        if return_or_error(e) || to.col <= from.col {
            return 0.0;
        }
        points.windows(2).fold(0.0, |total, w| total + js_max(0.0, w[0][0] - w[1][0]))
    }

    /// `readableCandidateCost`.
    fn cost(&self, e: &Edge, points: &[Pt], ordinal: f64, natural: Sides, from: &PlacedNode, to: &PlacedNode) -> Cost {
        let (crossings, shared) = self.interaction(e, points);
        let lengths: Vec<f64> = points.windows(2).map(|w| (w[1][0] - w[0][0]).abs() + (w[1][1] - w[0][1]).abs()).collect();
        let route_length = lengths.iter().fold(0.0, |t, l| t + l);
        let (first, last) = (points[0], points[points.len() - 1]);
        let direct = (last[0] - first[0]).abs() + (last[1] - first[1]).abs();
        let interior = if lengths.len() > 2 { &lengths[1..lengths.len() - 1] } else { &[][..] };
        let interior_deficit = interior.iter().fold(0.0, |t, l| t + js_max(0.0, 28.0 - l));
        let (mut min_x, mut max_x, mut min_y, mut max_y) = (f64::INFINITY, f64::NEG_INFINITY, f64::INFINITY, f64::NEG_INFINITY);
        for p in points {
            min_x = js_min(min_x, p[0]);
            max_x = js_max(max_x, p[0]);
            min_y = js_min(min_y, p[1]);
            max_y = js_max(max_y, p[1]);
        }
        let growth = js_max(0.0, -min_x)
            + js_max(0.0, max_x - self.legend.minimum_canvas_width)
            + js_max(0.0, -min_y)
            + js_max(0.0, max_y - self.auto_height);
        let ns = anchor(from, natural.from);
        let ne = anchor(to, natural.to);
        let port = (first[0] - ns[0]).abs() + (first[1] - ns[1]).abs() + (last[0] - ne[0]).abs() + (last[1] - ne[1]).abs();
        let legacy = (from.cx - LEGACY_COLUMN_CENTERS[from.col as usize]).abs() + (to.cx - LEGACY_COLUMN_CENTERS[to.col as usize]).abs();
        Cost([
            crossings,
            shared,
            Self::forward_reverse_px(e, points, from, to),
            self.label_route_clearance_deficit(e, points),
            interior_deficit,
            js_max(0.0, points.len() as f64 - 2.0),
            js_round((if direct > 0.0 { route_length / direct } else { 1.0 }) * 1000.0),
            growth,
            js_round(port * 1000.0),
            legacy,
            ordinal,
        ])
    }

    // ---- candidates -----------------------------------------------------------------------------

    /// `readableAutomaticCandidateSet`.
    #[allow(clippy::too_many_arguments)]
    fn candidate_set(
        &self,
        e: &Edge,
        from: &PlacedNode,
        to: &PlacedNode,
        start: Pt,
        end: Pt,
        sides: Sides,
        ordinal_offset: f64,
        natural: Sides,
    ) -> CandidateSet {
        let p = self.p;
        let mid_x = (start[0] + end[0]) / 2.0;
        let lane_gap_y = if from.lane == to.lane {
            p.lane_top(&from.lane) - 16.0
        } else {
            p.gap_y_between(&from.lane, &to.lane, e.bias.unwrap_or(0.5))
        };
        let top_y = js_max(8.0, js_min(p.lane_top(&from.lane), p.lane_top(&to.lane)) - 16.0);
        let bottom_y =
            js_max(p.lane_top(&from.lane) + p.lane_height(&from.lane), p.lane_top(&to.lane) + p.lane_height(&to.lane)) + 16.0;
        let outside_left = LANE_X - 20.0;
        let outside_right = LANE_X + p.layout.lane_w + 12.0;
        let (fs, ts) = (sides.from, sides.to);
        let raw: Vec<Vec<Pt>> = vec![
            vec![],
            vec![[end[0], start[1]]],
            vec![[start[0], end[1]]],
            corridor_via_y(start, end, fs, ts, lane_gap_y),
            corridor_via_x(start, end, fs, ts, mid_x),
            corridor_via_x(start, end, fs, ts, outside_left),
            corridor_via_x(start, end, fs, ts, outside_right),
            corridor_via_y(start, end, fs, ts, top_y),
            corridor_via_y(start, end, fs, ts, bottom_y),
        ];
        let mut candidates: Vec<Plan> = raw
            .iter()
            .enumerate()
            .map(|(ordinal, via)| (ordinal, join_route_points(start, via, end)))
            .filter(|(_, points)| self.feasible(e, points, from, to, sides))
            .map(|(ordinal, points)| {
                let cost = self.cost(e, &points, ordinal_offset + ordinal as f64, natural, from, to);
                Plan { points, sides, cost }
            })
            .collect();
        candidates.sort_by(|l, r| compare_cost(&l.cost, &r.cost));
        CandidateSet { raw, candidates, outside_right }
    }

    // ---- sides and ports ------------------------------------------------------------------------

    /// `oneBendCrossLaneVia`: the single corner between perpendicular sides, if readable and clear.
    fn one_bend_cross_lane_via(&self, e: &Edge, start: Pt, end: Pt, sides: Sides) -> Option<Vec<Pt>> {
        let (fv, tv) = (is_vertical(sides.from), is_vertical(sides.to));
        if fv == tv {
            return None;
        }
        let corner = if fv { [start[0], end[1]] } else { [end[0], start[1]] };
        let points = normalize(&[start, corner, end]);
        if points.len() != 3 || !route_honors_endpoint_sides(&points, sides.from, sides.to) {
            return None;
        }
        let readable = points.windows(2).all(|w| (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]) >= 8.0);
        if !readable || !self.route_clears_unrelated_nodes(e, &points, 2.0) {
            return None;
        }
        Some(points[1..2].to_vec())
    }

    /// `legacyAutomaticOneBendSides`.
    fn legacy_one_bend_sides(&self, e: &Edge, from: &PlacedNode, to: &PlacedNode) -> Option<Sides> {
        if e.via.is_some() || preset_of(e).is_some() || e.from_side.is_some() || e.to_side.is_some() || from.lane == to.lane {
            return None;
        }
        if from.cx == to.cx || from.cy == to.cy {
            return None;
        }
        let vertical_from = if to.cy < from.cy { Side::Top } else { Side::Bottom };
        let horizontal_to = if to.cx < from.cx { Side::Right } else { Side::Left };
        let horizontal_from = if to.cx < from.cx { Side::Left } else { Side::Right };
        let vertical_to = if to.cy < from.cy { Side::Bottom } else { Side::Top };
        [Sides { from: vertical_from, to: horizontal_to }, Sides { from: horizontal_from, to: vertical_to }]
            .into_iter()
            .find(|s| self.one_bend_cross_lane_via(e, anchor(from, s.from), anchor(to, s.to), *s).is_some())
    }

    /// The side pairs both side searches walk: the preferred ones, then every authored-compatible
    /// pair in `right, bottom, left, top` order, deduplicated.
    fn side_pairs(preferred: Vec<Sides>, authored_from: Option<Side>, authored_to: Option<Side>) -> Vec<Sides> {
        let mut all = preferred;
        for fs in authored_from.map_or(SIDE_ORDER.to_vec(), |s| vec![s]) {
            for ts in authored_to.map_or(SIDE_ORDER.to_vec(), |s| vec![s]) {
                all.push(Sides { from: fs, to: ts });
            }
        }
        let mut out: Vec<Sides> = Vec::new();
        for s in all {
            if authored_from.is_some_and(|a| a != s.from) || authored_to.is_some_and(|a| a != s.to) {
                continue;
            }
            if !out.contains(&s) {
                out.push(s);
            }
        }
        out
    }

    /// `readableAutomaticSides`: the side pair whose candidate set is non-empty (the primary pair
    /// first, else the cheapest over every pair), cached; `None` (uncached) for a controlled route or
    /// two authored sides.
    fn readable_automatic_sides(&mut self, i: usize, from: &PlacedNode, to: &PlacedNode) -> Option<Sides> {
        let e = self.edge(i);
        let automatic = !has_absolute_route_pins(e) && preset_of(e).is_none();
        let (af, at) = (gside(e.from_side), gside(e.to_side));
        if !automatic || (af.is_some() && at.is_some()) {
            return None;
        }
        if let Some(cached) = self.side_cache[i] {
            return cached;
        }
        let mut preferred = Vec::new();
        if let Some(s) = self.legacy_one_bend_sides(e, from, to) {
            preferred.push(s);
        }
        let natural = Sides { from: af.unwrap_or(default_from_side(from, to)), to: at.unwrap_or(default_to_side(from, to)) };
        preferred.push(natural);
        let pairs = Self::side_pairs(preferred, af, at);
        let plan = |s: &Sides, ordinal: usize| {
            self.candidate_set(e, from, to, anchor(from, s.from), anchor(to, s.to), *s, ordinal as f64 * 9.0, natural)
        };
        let mut selected = None;
        if let Some(primary) = pairs.first()
            && !plan(primary, 0).candidates.is_empty()
        {
            selected = Some(*primary);
        }
        if selected.is_none() {
            let mut all: Vec<Plan> = Vec::new();
            for (ordinal, s) in pairs.iter().enumerate() {
                all.extend(plan(s, ordinal).candidates);
            }
            all.sort_by(|l, r| compare_cost(&l.cost, &r.cost));
            selected = all.first().map(|c| c.sides);
        }
        self.side_cache[i] = Some(selected);
        selected
    }

    /// `edgeSides` (v2).
    fn edge_sides(&mut self, i: usize) -> Sides {
        if let Some(Some(s)) = self.side_cache[i] {
            return s;
        }
        let (from, to) = self.nodes_of(i).expect("routed edges have measured endpoints");
        if let Some(s) = self.readable_automatic_sides(i, from, to) {
            return s;
        }
        let e = self.edge(i);
        if self.p.layout.channel_label_edges.get(i).copied().unwrap_or(false) && e.from_side.is_none() && e.to_side.is_none() {
            return Sides { from: Side::Top, to: Side::Top };
        }
        Sides {
            from: gside(e.from_side).unwrap_or(default_from_side(from, to)),
            to: gside(e.to_side).unwrap_or(default_to_side(from, to)),
        }
    }

    /// `automaticPortSpread(workflow.edges, nodes, { sideFor: edgeSides })`: endpoints sharing a
    /// `(node, side)` spread 14 px apart (at most `(extent - 32) / (n - 1)`), sorted by the
    /// counterpart's centre along the side, ties by `id\0from\0to\0label`.
    fn automatic_port_spread(&mut self) -> Vec<(Option<Pt>, Option<Pt>)> {
        struct Item {
            edge: usize,
            source: bool,
            node: usize,
            side: Side,
            counterpart: usize,
        }
        let w = self.w();
        let mut groups: Vec<((usize, Side), Vec<Item>)> = Vec::new();
        for (i, e) in w.edges.iter().enumerate() {
            if preset_of(e).is_some() || has_absolute_route_pins(e) || e.label_at.is_some() {
                continue;
            }
            let Some((a, b)) = self.ends[i] else { continue };
            let from_side = self.edge_sides(i).from;
            let to_side = self.edge_sides(i).to;
            for item in [
                Item { edge: i, source: true, node: a, side: from_side, counterpart: b },
                Item { edge: i, source: false, node: b, side: to_side, counterpart: a },
            ] {
                let key = (item.node, item.side);
                match groups.iter_mut().find(|(k, _)| *k == key) {
                    Some((_, list)) => list.push(item),
                    None => groups.push((key, vec![item])),
                }
            }
        }
        let nodes = &self.p.nodes;
        let mut spread = vec![(None, None); w.edges.len()];
        let key_of = |e: &Edge| {
            format!("{}\0{}\0{}\0{}", e.id.clone().unwrap_or_default(), e.from, e.to, e.label.clone().unwrap_or_default())
        };
        for (_, mut items) in groups {
            if items.len() < 2 {
                continue;
            }
            let vertical_side = !is_vertical(items[0].side);
            items.sort_by(|a, b| {
                let coord = |it: &Item| if vertical_side { nodes[it.counterpart].cy } else { nodes[it.counterpart].cx };
                let (ac, bc) = (coord(a), coord(b));
                if ac != bc {
                    return (ac - bc).partial_cmp(&0.0).unwrap_or(Ordering::Equal);
                }
                crate::layout::workflow::readable::js_cmp(&key_of(&w.edges[a.edge]), &key_of(&w.edges[b.edge]))
            });
            let rect = nodes[items[0].node].rect;
            let extent = if vertical_side { rect.height } else { rect.width };
            let usable = js_max(0.0, extent - 32.0);
            let n = items.len() as f64;
            let spacing = js_min(14.0, usable / (n - 1.0));
            if spacing.is_nan() || spacing <= 0.0 {
                continue;
            }
            for (index, item) in items.iter().enumerate() {
                let offset = (index as f64 - (n - 1.0) / 2.0) * spacing;
                let mut point = anchor(&nodes[item.node], item.side);
                if vertical_side {
                    point[1] += offset;
                } else {
                    point[0] += offset;
                }
                if item.source {
                    spread[item.edge].0 = Some(point);
                } else {
                    spread[item.edge].1 = Some(point);
                }
            }
        }
        spread
    }

    /// `automaticPortCandidates`: the preferred port when no routed endpoint on that side is too
    /// close, else up to two free ports stepping 12 px from the centre toward the counterpart.
    fn port_candidates(&self, i: usize, node: &PlacedNode, side: Side, preferred: Pt, counterpart: &PlacedNode) -> Vec<Pt> {
        let e = self.edge(i);
        if !independent_automatic_route(e) {
            return vec![preferred];
        }
        let vertical_side = !is_vertical(side);
        let axis = usize::from(vertical_side);
        let center = anchor(node, side);
        let mut occupied: Vec<(f64, f64)> = Vec::new();
        for &k in &self.order {
            let other = self.edge(k);
            let Some(routed) = &self.paths[k] else { continue };
            for source in [true, false] {
                let id = if source { &other.from } else { &other.to };
                if *id != node.id {
                    continue;
                }
                let point = if source { routed.points[0] } else { routed.points[routed.points.len() - 1] };
                if (point[1 - axis] - center[1 - axis]).abs() < EPS {
                    occupied.push((point[axis], js_max(12.0, 3.5 * (stroke_width(e) + stroke_width(other)))));
                }
            }
        }
        let clear = |p: &Pt| occupied.iter().all(|(c, cl)| (c - p[axis]).abs() >= cl - EPS);
        if clear(&preferred) {
            return vec![preferred];
        }
        let mut candidates: Vec<Pt> = Vec::new();
        let extent = if vertical_side { node.rect.height } else { node.rect.width };
        let direction = if (if vertical_side { counterpart.cy } else { counterpart.cx }) >= center[axis] { 1.0 } else { -1.0 };
        let mut offset = 0.0;
        while offset <= extent / 2.0 - 16.0 {
            for sign in [direction, -direction] {
                let mut point = center;
                point[axis] += sign * offset;
                if clear(&point) && !candidates.iter().any(|c| c[axis] == point[axis]) {
                    candidates.push(point);
                }
            }
            if candidates.len() >= 2 {
                candidates.truncate(2);
                return candidates;
            }
            offset += 12.0;
        }
        if candidates.is_empty() { vec![preferred] } else { candidates }
    }

    // ---- automatic via --------------------------------------------------------------------------

    fn feedback_request(&self, e: &Edge, kind: FeedbackKind) -> Box<FeedbackRequest> {
        Box::new(FeedbackRequest {
            kind,
            edge: e.id.clone(),
            from: e.from.clone(),
            to: e.to.clone(),
            attempted_candidate_families: FAMILIES.iter().map(|f| (*f).to_owned()).collect(),
            candidate_count: FAMILIES.len(),
        })
    }

    /// `readableAutomaticVia`: the cheapest feasible family; else the outside-right corridor pushed
    /// right past the label's blockers and refined by bisection; else the pin classification, a
    /// solver request (rank gap, lane gap) or `workflow/solver-budget-exhausted`.
    fn automatic_via(&mut self, i: usize, from: &PlacedNode, to: &PlacedNode, start: Pt, end: Pt, sides: Sides) -> R<Vec<Pt>> {
        let e = self.edge(i);
        let set = self.candidate_set(e, from, to, start, end, sides, 0.0, sides);
        if let Some(best) = set.candidates.first() {
            return Ok(best.points[1..best.points.len() - 1].to_vec());
        }
        let outside_right = set.outside_right;
        let mut via = vec![start];
        via.extend(set.raw[6].iter().copied());
        via.push(end);
        let current = normalize(&via);
        let label = self.candidate_label_rect(e, &current);
        let mut min_x = outside_right;
        if let Some(l) = label {
            let mut push = |deficit: f64| {
                if deficit > 0.0 {
                    min_x = js_max(min_x, outside_right + deficit * 2.0);
                }
            };
            for n in &self.p.nodes {
                if rects_overlap(&l, &n.rect, -2.0) {
                    push(n.rect.x + n.rect.width - 2.0 - l.x);
                }
            }
            for &k in &self.order {
                if let Some(pl) = &self.labels[k]
                    && rects_overlap(&l, &pl.rect, -2.0)
                {
                    push(pl.rect.x + pl.rect.width - 2.0 - l.x);
                }
                if let Some(routed) = &self.paths[k] {
                    for w in routed.points.windows(2) {
                        match clearance(w[0], w[1], &l) {
                            Some(c) if c + EPS < 4.0 => push(js_max(w[0][0], w[1][0]) + 4.0 - l.x),
                            _ => {}
                        }
                    }
                }
            }
        }
        min_x = ceil3(min_x);
        let rightmost = js_max(js_max(outside_right, self.rightmost_node_edge), self.rightmost_routed_edge);
        let mut growth = js_max(js_max(32.0, label.map_or(0.0, |l| l.width)), rightmost + 16.0 - min_x);
        let mut last_infeasible = outside_right;
        let expanded_at = |x: f64| {
            let mut pts = vec![start];
            pts.extend(corridor_via_x(start, end, sides.from, sides.to, x));
            pts.push(end);
            normalize(&pts)
        };
        for _ in 0..7 {
            if min_x > outside_right + EPS {
                let expanded = expanded_at(min_x);
                if self.feasible(e, &expanded, from, to, sides) {
                    let (mut feasible_x, mut feasible_points, mut infeasible_x) = (min_x, expanded, last_infeasible);
                    let mut refinement = 0;
                    while refinement < 53 && feasible_x - infeasible_x > 0.001 {
                        let mid = ceil3((infeasible_x + feasible_x) / 2.0);
                        if mid >= feasible_x - EPS {
                            break;
                        }
                        let mid_points = expanded_at(mid);
                        if self.feasible(e, &mid_points, from, to, sides) {
                            feasible_x = mid;
                            feasible_points = mid_points;
                        } else {
                            infeasible_x = mid;
                        }
                        refinement += 1;
                    }
                    return Ok(feasible_points[1..feasible_points.len() - 1].to_vec());
                }
                last_infeasible = min_x;
            }
            min_x = ceil3(min_x + growth);
            growth *= 2.0;
        }
        let relevant_pin = e.label_at.is_some()
            || self.order.iter().any(|&k| {
                let other = self.edge(k);
                other.label_at.is_some() || has_absolute_route_pins(other)
            });
        if relevant_pin {
            self.classify_failed_candidate_pins(i)?;
        }
        let horizontally_facing = (sides.from == Side::Right && sides.to == Side::Left && end[0] > start[0])
            || (sides.from == Side::Left && sides.to == Side::Right && start[0] > end[0]);
        if horizontally_facing && from.col != to.col {
            let from_col = js_min(from.col, to.col);
            let to_col = js_max(from.col, to.col);
            let required = from.rect.width / 2.0 + 32.0 + to.rect.width / 2.0;
            let actual = self.p.layout.col_xs[to_col as usize] - self.p.layout.col_xs[from_col as usize];
            if actual + EPS < required {
                return Err(Throw::Feedback(
                    self.feedback_request(e, FeedbackKind::RankGapMinimum { from_col, to_col, minimum: ceil3(required) }),
                ));
            }
        }
        if from.lane != to.lane && self.p.layout.lane_gap < 32.0 {
            return Err(Throw::Feedback(self.feedback_request(e, FeedbackKind::LaneGapMinimum { minimum: 32.0 })));
        }
        let message = format!("Workflow edge \"{}\" has no feasible readable-v2 automatic route.", edge_name(e));
        Err(fail(
            message.clone(),
            Diagnostic::error("workflow/solver-budget-exhausted", &message)
                .with_subject(edge_subject(e, "workflow/solver-budget-exhausted"))
                .with_evidence(obj(vec![
                    ("attemptedCandidateFamilies", json!(FAMILIES)),
                    ("candidateCount", json!(FAMILIES.len())),
                ])),
        ))
    }

    /// `readablePresetVia`: the preset's geometry if it is feasible and still that preset, else
    /// `workflow/route-preset-conflict` (with the verified alternative presets).
    fn preset_via(&mut self, i: usize, from: &PlacedNode, to: &PlacedNode, start: Pt, end: Pt, sides: Sides) -> R<Vec<Pt>> {
        let e = self.edge(i);
        let p = self.p;
        let Some(preset) = preset_of(e) else { return self.automatic_via(i, from, to, start, end, sides) };
        let via: Vec<Pt> = match preset {
            EdgeRoute::Straight | EdgeRoute::Auto => vec![],
            EdgeRoute::Drop => {
                let y = p.gap_y_between(&from.lane, &to.lane, e.bias.unwrap_or(0.5));
                vec![[start[0], y], [end[0], y]]
            }
            EdgeRoute::OutsideRight => {
                let x = LANE_X + p.layout.lane_w + 12.0;
                vec![[x, start[1]], [x, end[1]]]
            }
            EdgeRoute::ReturnLeft => {
                let x = js_min(from.rect.x, to.rect.x) - 28.0;
                vec![[x, start[1]], [x, end[1]]]
            }
            EdgeRoute::BottomChannel => {
                let y = js_max(from.rect.y + from.rect.height, to.rect.y + to.rect.height) + 32.0;
                vec![[start[0], y], [end[0], y]]
            }
            EdgeRoute::UpChannel => {
                let y = js_min(from.rect.y, to.rect.y) - 28.0;
                vec![[start[0], y], [end[0], y]]
            }
        };
        let points = join_route_points(start, &via, end);
        if self.feasible(e, &points, from, to, sides) && route_matches_preset_family(preset, &points, from, to) {
            return Ok(points[1..points.len() - 1].to_vec());
        }
        let name = edge_name(e);
        let message = format!(
            "Workflow edge \"{name}\" cannot satisfy route preset \"{}\" under readable-v2 constraints (minimum 8px endpoint stubs, 16px interior turns, and 28px direct clearance).",
            route_name(preset)
        );
        let mut fixes = Vec::new();
        for candidate in PRESETS {
            if candidate == preset {
                continue;
            }
            if self.accepts_edge(i, |edge| edge.route = Some(candidate)) {
                fixes.push(format!("set edge \"{name}\" route to verified preset \"{}\"", route_name(candidate)));
            }
        }
        if self.accepts_edge(i, |edge| edge.route = None) {
            fixes.push(format!("remove route from edge \"{name}\" so readable-v2 can use its verified automatic candidate"));
        }
        Err(fail(
            message.clone(),
            Diagnostic::error("workflow/route-preset-conflict", &message)
                .with_subject(edge_subject(e, "workflow/route-preset-conflict").with_extra("route", json!(route_name(preset))))
                .with_evidence(obj(vec![
                    ("attemptedCandidateFamily", json!(route_name(preset))),
                    ("points", points_json(&points)),
                    ("fromSide", json!(side_name(sides.from))),
                    ("toSide", json!(side_name(sides.to))),
                    ("requiredEndpointStubPx", json!(8)),
                    ("requiredInteriorSegmentPx", json!(16)),
                    ("requiredDirectClearancePx", json!(28)),
                ]))
                .with_fixes(fixes),
        ))
    }

    /// `routeVia` (v2).
    #[allow(clippy::too_many_arguments)]
    fn route_via(
        &mut self,
        i: usize,
        from: &PlacedNode,
        to: &PlacedNode,
        start: Pt,
        end: Pt,
        sides: Sides,
        validate_preset: bool,
    ) -> R<Vec<Pt>> {
        let e = self.edge(i);
        let p = self.p;
        if let Some(via) = &e.via {
            return Ok(via.clone());
        }
        let coordinate_pins = e.channel_x.is_some() || e.channel_y.is_some();
        if preset_of(e).is_some() && !coordinate_pins && validate_preset {
            return self.preset_via(i, from, to, start, end, sides);
        }
        Ok(match e.route.unwrap_or(EdgeRoute::Auto) {
            EdgeRoute::Straight => vec![],
            EdgeRoute::Drop => {
                let y = e.channel_y.unwrap_or_else(|| p.gap_y_between(&from.lane, &to.lane, e.bias.unwrap_or(0.5)));
                vec![[start[0], y], [end[0], y]]
            }
            EdgeRoute::OutsideRight => {
                let x = e.channel_x.unwrap_or(LANE_X + p.layout.lane_w + 12.0);
                vec![[x, start[1]], [x, end[1]]]
            }
            EdgeRoute::ReturnLeft => {
                let x = e.channel_x.unwrap_or(js_min(from.rect.x, to.rect.x) - 28.0);
                vec![[x, start[1]], [x, end[1]]]
            }
            EdgeRoute::BottomChannel => {
                let y = e.channel_y.unwrap_or(js_max(from.rect.y + from.rect.height, to.rect.y + to.rect.height) + 32.0);
                vec![[start[0], y], [end[0], y]]
            }
            EdgeRoute::UpChannel => {
                let y = e.channel_y.unwrap_or(js_min(from.rect.y, to.rect.y) - 28.0);
                vec![[start[0], y], [end[0], y]]
            }
            EdgeRoute::Auto => match (e.channel_x, e.channel_y) {
                (Some(x), Some(y)) => vec![[x, start[1]], [x, y], [end[0], y]],
                (Some(x), None) => vec![[x, start[1]], [x, end[1]]],
                (None, Some(y)) => vec![[start[0], y], [end[0], y]],
                (None, None) => return self.automatic_via(i, from, to, start, end, sides),
            },
        })
    }

    // ---- automatic and controlled routes --------------------------------------------------------

    /// `readableAutomaticRoute`: every side pair and port alternative, the aligned facing pair, the
    /// outside-right expansions, then the corridor repair of the best seeds.
    fn automatic_route(&mut self, i: usize, from: &PlacedNode, to: &PlacedNode, primary: Sides, ports: (Option<Pt>, Option<Pt>)) -> R<(Vec<Pt>, Sides)> {
        let e = self.edge(i);
        let (af, at) = (gside(e.from_side), gside(e.to_side));
        let natural = Sides { from: af.unwrap_or(default_from_side(from, to)), to: at.unwrap_or(default_to_side(from, to)) };
        let pairs = Self::side_pairs(vec![primary], af, at);
        let mut plans: Vec<Plan> = Vec::new();
        let mut feedback: Vec<(Box<FeedbackRequest>, usize)> = Vec::new();
        let mut first_failure: Option<Throw> = None;

        let aligned_start = self.port_candidates(i, from, natural.from, anchor(from, natural.from), to)[0];
        let aligned_end = self.port_candidates(i, to, natural.to, anchor(to, natural.to), from)[0];
        let aligned = vec![aligned_start, aligned_end];
        if to.col > from.col
            && !return_or_error(e)
            && e.from_side.is_none()
            && e.to_side.is_none()
            && e.label_at.is_none()
            && self.feasible(e, &aligned, from, to, natural)
        {
            let cost = self.cost(e, &aligned, -1.0, natural, from, to);
            plans.push(Plan { points: aligned, sides: natural, cost });
        }

        for (pair_ordinal, sides) in pairs.iter().copied().enumerate() {
            let is_primary = pair_ordinal == 0;
            let preferred_start = match ports.0 {
                Some(p) if is_primary => p,
                _ => anchor(from, sides.from),
            };
            let preferred_end = match ports.1 {
                Some(p) if is_primary => p,
                _ => anchor(to, sides.to),
            };
            let starts = self.port_candidates(i, from, sides.from, preferred_start, to);
            let ends = self.port_candidates(i, to, sides.to, preferred_end, from);
            let (start, end) = (starts[0], ends[0]);
            let base = pair_ordinal as f64 * 9.0;
            let planned = self.candidate_set(e, from, to, start, end, sides, base, natural);
            let planned_empty = planned.candidates.is_empty();
            plans.extend(planned.candidates);
            let mut port_ordinal = 0.0;
            for (si, &alt_start) in starts.iter().enumerate() {
                for (ei, &alt_end) in ends.iter().enumerate() {
                    if si == 0 && ei == 0 {
                        continue;
                    }
                    port_ordinal += 1.0;
                    let offset = base + port_ordinal * 144.0;
                    let alternative = self.candidate_set(e, from, to, alt_start, alt_end, sides, offset, natural);
                    let empty = alternative.candidates.is_empty();
                    plans.extend(alternative.candidates);
                    if empty {
                        match self.automatic_via(i, from, to, alt_start, alt_end, sides) {
                            Ok(via) => {
                                let points = join_route_points(alt_start, &via, alt_end);
                                let cost = self.cost(e, &points, offset + 6.0, natural, from, to);
                                plans.push(Plan { points, sides, cost });
                            }
                            Err(Throw::Feedback(request)) => feedback.push((request, pair_ordinal)),
                            Err(other) => {
                                first_failure.get_or_insert(other);
                            }
                        }
                    }
                }
            }
            if !planned_empty {
                continue;
            }
            match self.automatic_via(i, from, to, start, end, sides) {
                Ok(via) => {
                    let points = join_route_points(start, &via, end);
                    let cost = self.cost(e, &points, base + 6.0, natural, from, to);
                    plans.push(Plan { points, sides, cost });
                }
                Err(Throw::Feedback(request)) => feedback.push((request, pair_ordinal)),
                Err(other) => {
                    first_failure.get_or_insert(other);
                }
            }
        }

        plans.sort_by(|l, r| compare_cost(&l.cost, &r.cost));
        if !plans.is_empty() {
            let mut seeds: Vec<Plan> = Vec::new();
            if plans[0].cost.shared_corridor_px() > 0.0 {
                let key = |p: &Plan| {
                    let (a, b) = (p.points[0], p.points[p.points.len() - 1]);
                    (p.sides, format!("{},{}", js_num(a[0]), js_num(a[1])), format!("{},{}", js_num(b[0]), js_num(b[1])))
                };
                let mut seen = Vec::new();
                for plan in &plans {
                    let k = key(plan);
                    if !seen.contains(&k) {
                        seen.push(k);
                        seeds.push(plan.clone());
                    }
                }
                seeds.truncate(8);
            }
            for selected in seeds {
                if selected.cost.shared_corridor_px() == 0.0 {
                    continue;
                }
                let mut alternatives: Vec<Plan> = Vec::new();
                let n = selected.points.len();
                let mut segment = 1;
                while segment + 2 < n {
                    let (a, b) = (selected.points[segment], selected.points[segment + 1]);
                    let axis = if a[0] == b[0] { 0 } else { 1 };
                    for offset in [-16.0, 16.0, -32.0, 32.0] {
                        let mut points = selected.points.clone();
                        points[segment][axis] += offset;
                        points[segment + 1][axis] += offset;
                        if !self.feasible(e, &points, from, to, selected.sides) {
                            continue;
                        }
                        let cost = self.cost(e, &points, selected.cost.ordinal(), natural, from, to);
                        if compare_cost(&cost, &selected.cost) == Ordering::Less {
                            alternatives.push(Plan { points, sides: selected.sides, cost });
                        }
                    }
                    segment += 1;
                }
                alternatives.sort_by(|l, r| compare_cost(&l.cost, &r.cost));
                if let Some(best) = alternatives.into_iter().next() {
                    plans.push(best);
                }
            }
            plans.sort_by(|l, r| compare_cost(&l.cost, &r.cost));
            let best = plans.swap_remove(0);
            return Ok((best.points, best.sides));
        }

        let priority = |r: &FeedbackRequest| match r.kind {
            FeedbackKind::RankGapMinimum { .. } => 0,
            FeedbackKind::LaneGapMinimum { .. } => 1,
        };
        feedback.sort_by(|l, r| priority(&l.0).cmp(&priority(&r.0)).then(l.1.cmp(&r.1)));
        if let Some((request, _)) = feedback.into_iter().next() {
            return Err(Throw::Feedback(request));
        }
        let mut side_fields = Vec::new();
        if af.is_some() {
            side_fields.push("fromSide");
        }
        if at.is_some() {
            side_fields.push("toSide");
        }
        if !side_fields.is_empty() {
            let (removal_sets, fixes) =
                self.pin_removal_alternatives(i, &side_fields, "so readable-v2 can replan the remaining endpoint-side pins");
            let source = ports.0.unwrap_or_else(|| anchor(from, primary.from));
            let target = ports.1.unwrap_or_else(|| anchor(to, primary.to));
            let mut evidence = obj(vec![
                ("conflictingPins", self.conflict_pins_from_removal_sets(i, &removal_sets, &side_fields)),
                ("actualCoordinates", json!({ "sourceAnchor": pt_json(source), "targetAnchor": pt_json(target) })),
                ("fromSide", json!(side_name(primary.from))),
                ("toSide", json!(side_name(primary.to))),
            ]);
            if let Some(Throw::Failure(f)) = &first_failure
                && let Some(d) = f.diagnostics.first()
            {
                for key in ["attemptedCandidateFamilies", "candidateCount"] {
                    if let Some(v) = d.evidence.get(key) {
                        evidence.insert(key.to_owned(), v.clone());
                    }
                }
            }
            return Err(self.explicit_pin_conflict(i, "readable route feasibility with authored endpoint sides", evidence, fixes));
        }
        Err(first_failure.unwrap_or_else(|| internal("readable-v2 automatic route enumeration produced no result")))
    }

    /// `readableControlledRoute`: the authored geometry under every compatible side pair, the
    /// cheapest feasible one that keeps its preset and channel pins; else the best diagnosable
    /// candidate (absolute pins) or the preset's typed conflict.
    fn controlled_route(&mut self, i: usize, from: &PlacedNode, to: &PlacedNode) -> R<Option<(Vec<Pt>, Sides)>> {
        let e = self.edge(i);
        let (af, at) = (gside(e.from_side), gside(e.to_side));
        let natural = Sides { from: af.unwrap_or(default_from_side(from, to)), to: at.unwrap_or(default_to_side(from, to)) };
        let pairs = Self::side_pairs(vec![natural], af, at);
        let absolute = has_absolute_route_pins(e);
        let mut candidates: Vec<Plan> = Vec::new();
        let mut diagnostic: Option<(Vec<Pt>, Sides)> = None;
        let mut materialized: Vec<(Vec<Pt>, Sides)> = Vec::new();
        for (ordinal, sides) in pairs.iter().copied().enumerate() {
            let start = anchor(from, sides.from);
            let end = anchor(to, sides.to);
            let via = self.route_via(i, from, to, start, end, sides, false)?;
            let mut authored = vec![start];
            authored.extend(via);
            authored.push(end);
            let points = if absolute { authored } else { normalize(&authored) };
            materialized.push((points.clone(), sides));
            if diagnostic.is_none()
                && points.len() >= 2
                && points.iter().all(|p| is_finite_point(p))
                && route_honors_endpoint_sides(&points, sides.from, sides.to)
            {
                diagnostic = Some((points.clone(), sides));
            }
            if !self.feasible(e, &points, from, to, sides) {
                continue;
            }
            if let Some(preset) = preset_of(e)
                && !route_matches_preset_family(preset, &points, from, to)
            {
                continue;
            }
            if present_channel_pins(e)
                .iter()
                .any(|f| !route_contains_channel_pin(&points, *f == "channelX", channel_pin_value(e, f).unwrap_or(f64::NAN)))
            {
                continue;
            }
            let cost = self.cost(e, &points, ordinal as f64, natural, from, to);
            candidates.push(Plan { points, sides, cost });
        }
        candidates.sort_by(|l, r| compare_cost(&l.cost, &r.cost));
        if let Some(best) = candidates.into_iter().next() {
            return Ok(Some((best.points, best.sides)));
        }
        if absolute {
            return Ok(diagnostic.or_else(|| materialized.first().cloned()));
        }
        let Some((points, sides)) = materialized.into_iter().next() else { return Ok(None) };
        let (start, end) = (points[0], points[points.len() - 1]);
        let via = self.preset_via(i, from, to, start, end, sides)?;
        let mut all = vec![start];
        all.extend(via);
        all.push(end);
        Ok(Some((normalize(&all), sides)))
    }

    /// `pathFor`: the cached route, or route the edge now and register it.
    fn path_for(&mut self, i: usize) -> R<Vec<Pt>> {
        if let Some(routed) = &self.paths[i] {
            return Ok(routed.points.clone());
        }
        let e = self.edge(i);
        let (from, to) = self.nodes_of(i).expect("routed edges have measured endpoints");
        let controlled = has_absolute_route_pins(e) || preset_of(e).is_some();
        if controlled && let Some((points, sides)) = self.controlled_route(i, from, to)? {
            self.side_cache[i] = Some(Some(sides));
            self.register(i, points.clone(), Some(sides));
            return Ok(points);
        }
        let ports = self.ports[i];
        let sides = self.edge_sides(i);
        if !controlled {
            let (points, sides) = self.automatic_route(i, from, to, sides, ports)?;
            self.side_cache[i] = Some(Some(sides));
            self.register(i, points.clone(), Some(sides));
            return Ok(points);
        }
        let start = ports.0.unwrap_or_else(|| anchor(from, sides.from));
        let end = ports.1.unwrap_or_else(|| anchor(to, sides.to));
        let mut authored = vec![start];
        authored.extend(self.route_via(i, from, to, start, end, sides, true)?);
        authored.push(end);
        let points = if has_absolute_route_pins(e) { authored } else { normalize(&authored) };
        self.register(i, points.clone(), None);
        Ok(points)
    }

    /// `registerRouted`: cache the route, publish it and its label box to the obstacle list.
    fn register(&mut self, i: usize, points: Vec<Pt>, sides: Option<Sides>) {
        for p in &points {
            self.rightmost_routed_edge = js_max(self.rightmost_routed_edge, p[0]);
        }
        let e = self.edge(i);
        let label = label_of(e).map(|text| {
            let at = self.label_point(e, &points);
            let width = workflow_label_width(text);
            EdgeLabel { index: i, text: text.to_owned(), at, rect: Rect::new(at[0] - width / 2.0, at[1] - 10.0, width, 14.0) }
        });
        if let Some(l) = &label {
            self.rightmost_routed_edge = js_max(self.rightmost_routed_edge, l.rect.x + l.rect.width);
        }
        self.paths[i] = Some(RoutedEdge { index: i, points, sides, sequence: self.order.len() });
        self.labels[i] = label;
        self.order.push(i);
    }

    /// `labelRectFor`: routes the edge first.
    fn label_rect_for(&mut self, i: usize) -> R<Option<EdgeLabel>> {
        if label_of(self.edge(i)).is_none() || self.ends[i].is_none() {
            return Ok(None);
        }
        self.path_for(i)?;
        Ok(self.labels[i].clone())
    }

    // ---- fix discovery and pin conflicts --------------------------------------------------------

    /// `acceptsFix` over a mutated clone of the canonical document.
    fn accepts_doc(&self, mutate: impl FnOnce(&mut Workflow)) -> bool {
        let Some(accepts) = self.accepts else { return false };
        let mut doc = self.w().clone();
        mutate(&mut doc);
        accepts(&doc)
    }

    /// `verifiedEdgeFix` without the message.
    fn accepts_edge(&self, i: usize, mutate: impl FnOnce(&mut Edge)) -> bool {
        self.accepts_doc(|doc| mutate(&mut doc.edges[i]))
    }

    fn verified_edge_fix(&self, i: usize, message: String, mutate: impl FnOnce(&mut Edge)) -> Option<String> {
        self.accepts_edge(i, mutate).then_some(message)
    }

    /// `verifiedAutomaticRouteFix`.
    fn verified_automatic_route_fix(&self, i: usize, clear_sides: bool) -> Option<String> {
        let name = edge_name(self.edge(i));
        let message = if clear_sides {
            format!("remove explicit route geometry and endpoint sides from edge \"{name}\" so readable-v2 can use its verified automatic candidate")
        } else {
            format!("remove explicit route geometry from edge \"{name}\" so readable-v2 can use its verified automatic candidate")
        };
        self.verified_edge_fix(i, message, |e| {
            e.via = None;
            e.channel_x = None;
            e.channel_y = None;
            e.route = None;
            if clear_sides {
                e.from_side = None;
                e.to_side = None;
            }
        })
    }

    /// `authoredPinEvidence`.
    fn pin_evidence(&self, i: usize, field: &str) -> Value {
        let e = self.edge(i);
        let source = self.p.canonical.edge_source[i];
        json!({
            "edge": edge_name(e),
            "field": field,
            "path": format!("/edges/{source}/{field}"),
            "value": field_value(e, field),
        })
    }

    /// `verifiedPinRemovalAlternatives`: the smallest field sets whose removal verifies, and their fixes.
    fn pin_removal_alternatives(&self, i: usize, fields: &[&'static str], reason: &str) -> (Vec<Vec<&'static str>>, Vec<String>) {
        if self.accepts.is_none() {
            return (Vec::new(), Vec::new());
        }
        let e = self.edge(i);
        let mut unique: Vec<&'static str> = Vec::new();
        for f in fields {
            if field_present(e, f) && !unique.contains(f) {
                unique.push(f);
            }
        }
        for size in 1..=unique.len() {
            let sets: Vec<Vec<&'static str>> = combinations(unique.len(), size)
                .into_iter()
                .map(|c| c.into_iter().map(|k| unique[k]).collect::<Vec<_>>())
                .filter(|set| {
                    self.accepts_edge(i, |edge| {
                        for f in set {
                            delete_field(edge, f);
                        }
                    })
                })
                .collect();
            if sets.is_empty() {
                continue;
            }
            let name = edge_name(e);
            let fixes = sets.iter().map(|set| format!("remove {} from edge \"{name}\" {reason}", set.join(" and "))).collect();
            return (sets, fixes);
        }
        (Vec::new(), Vec::new())
    }

    /// `conflictPinsFromRemovalSets`.
    fn conflict_pins_from_removal_sets(&self, i: usize, sets: &[Vec<&'static str>], fallback: &[&'static str]) -> Value {
        let mut fields: Vec<&str> = Vec::new();
        let source: Vec<&str> = if sets.is_empty() { fallback.to_vec() } else { sets.iter().flatten().copied().collect() };
        for f in source {
            if !fields.contains(&f) {
                fields.push(f);
            }
        }
        Value::Array(fields.iter().map(|f| self.pin_evidence(i, f)).collect())
    }

    /// `verifiedPinReferenceAlternatives` over `(edge, field)` references.
    fn pin_reference_alternatives(&self, refs: &[(usize, &'static str)], reason: &str) -> PinAlternatives {
        let mut unique: Vec<(usize, &'static str)> = Vec::new();
        for r in refs {
            if field_present(self.edge(r.0), r.1) && !unique.contains(r) {
                unique.push(*r);
            }
        }
        let fallback = PinAlternatives {
            refs: unique.clone(),
            pins: unique.iter().map(|(i, f)| self.pin_evidence(*i, f)).collect(),
            repairs: Vec::new(),
        };
        if self.accepts.is_none() {
            return fallback;
        }
        for size in 1..=unique.len() {
            let sets: Vec<Vec<(usize, &'static str)>> = combinations(unique.len(), size)
                .into_iter()
                .map(|c| c.into_iter().map(|k| unique[k]).collect::<Vec<_>>())
                .filter(|set| {
                    self.accepts_doc(|doc| {
                        for (i, f) in set {
                            delete_field(&mut doc.edges[*i], f);
                        }
                    })
                })
                .collect();
            if sets.is_empty() {
                continue;
            }
            let mut seen: Vec<(usize, &'static str)> = Vec::new();
            for set in &sets {
                for r in set {
                    if !seen.contains(r) {
                        seen.push(*r);
                    }
                }
            }
            let repairs = sets
                .iter()
                .map(|set| {
                    let mut grouped: Vec<(usize, Vec<&str>)> = Vec::new();
                    for (i, f) in set {
                        match grouped.iter_mut().find(|(e, _)| e == i) {
                            Some((_, fields)) => fields.push(f),
                            None => grouped.push((*i, vec![f])),
                        }
                    }
                    let removals: Vec<String> = grouped
                        .iter()
                        .map(|(i, fields)| format!("remove {} from edge \"{}\"", fields.join(" and "), edge_name(self.edge(*i))))
                        .collect();
                    (set.clone(), format!("{} {reason}", removals.join(" and ")))
                })
                .collect();
            return PinAlternatives { pins: seen.iter().map(|(i, f)| self.pin_evidence(*i, f)).collect(), refs: seen, repairs };
        }
        fallback
    }

    /// `verifiedRouteGeometryPinAlternatives`.
    fn route_geometry_pin_alternatives(&self, i: usize) -> PinAlternatives {
        let refs: Vec<(usize, &'static str)> = route_assertion_fields(self.edge(i)).into_iter().map(|f| (i, f)).collect();
        self.pin_reference_alternatives(&refs, "so readable-v2 can replan the remaining explicit route assertions")
    }

    /// `verifiedLabelAtAlternatives`: the 24/48 px nudges that verify.
    fn label_at_alternatives(&self, i: usize) -> Vec<String> {
        let e = self.edge(i);
        let Some([x, y]) = e.label_at else { return Vec::new() };
        let name = edge_name(e);
        [[0.0, 24.0], [0.0, -24.0], [24.0, 0.0], [-24.0, 0.0], [0.0, 48.0], [0.0, -48.0], [48.0, 0.0], [-48.0, 0.0]]
            .into_iter()
            .filter_map(|[dx, dy]| {
                let next = [x + dx, y + dy];
                self.verified_edge_fix(i, format!("set labelAt on edge \"{name}\" to [{}, {}]", js_num(next[0]), js_num(next[1])), |c| {
                    c.label_at = Some(next)
                })
            })
            .collect()
    }

    /// `verifiedLabelAtNudge`.
    fn label_at_nudge(&self, i: usize) -> Option<String> {
        if let Some(first) = self.label_at_alternatives(i).into_iter().next() {
            return Some(first);
        }
        let name = edge_name(self.edge(i));
        self.verified_edge_fix(
            i,
            format!("remove labelAt from edge \"{name}\" so readable-v2 can use verified automatic label placement"),
            |c| c.label_at = None,
        )
    }

    /// `verifiedRepairsWithLabelNudges`.
    fn repairs_with_label_nudges(&self, alternatives: &PinAlternatives) -> Vec<String> {
        alternatives
            .repairs
            .iter()
            .flat_map(|(set, message)| {
                if set.len() == 1 && set[0].1 == "labelAt" {
                    let nudges = self.label_at_alternatives(set[0].0);
                    if !nudges.is_empty() {
                        return nudges;
                    }
                }
                vec![message.clone()]
            })
            .collect()
    }

    /// `throwExplicitPinConflict`.
    fn explicit_pin_conflict(&self, i: usize, invariant: &str, evidence: Map<String, Value>, fixes: Vec<String>) -> Throw {
        let e = self.edge(i);
        let message = format!("Workflow edge \"{}\" has explicit geometry that violates {invariant}.", edge_name(e));
        let mut subject = edge_subject(e, "workflow/explicit-pin-conflict");
        if let Some(Value::Array(pins)) = evidence.get("conflictingPins")
            && pins.len() == 1
            && let Some(field) = pins[0].get("field").and_then(Value::as_str)
        {
            let path = match pins[0].get("path").and_then(Value::as_str) {
                Some(p) if !p.is_empty() => p.to_owned(),
                _ => format!("/edges/{}/{field}", self.p.canonical.edge_source[i]),
            };
            subject = subject.with_path(path);
        }
        let mut full = obj(vec![("invariant", json!(invariant))]);
        full.extend(evidence);
        fail(
            message.clone(),
            Diagnostic::error("workflow/explicit-pin-conflict", &message).with_subject(subject).with_evidence(full).with_fixes(fixes),
        )
    }

    /// `throwReadableLabelRoutePinConflict`; `None` when neither side is pinned.
    fn label_route_pin_conflict(&self, hit: &ClearanceHit, route_points: Option<&[Pt]>) -> Option<Throw> {
        let (le, re) = (hit.label_edge, hit.other_edge);
        let label_edge = self.edge(le);
        let route_edge = self.edge(re);
        let label_pinned = label_edge.label_at.is_some();
        if !label_pinned && route_assertion_fields(route_edge).is_empty() {
            return None;
        }
        let mut refs = Vec::new();
        if label_pinned {
            refs.push((le, "labelAt"));
        }
        refs.extend(route_assertion_fields(route_edge).into_iter().map(|f| (re, f)));
        let reason = if label_pinned {
            "so readable-v2 can replan the remaining authored label-route pins"
        } else {
            "so readable-v2 can replan the remaining authored route assertions"
        };
        let alternatives = self.pin_reference_alternatives(&refs, reason);
        let points: Vec<Pt> = match route_points {
            Some(p) => p.to_vec(),
            None => self.paths[re].as_ref().map(|r| r.points.clone()).unwrap_or_default(),
        };
        let diagnostic_edge = alternatives.refs.first().map_or(if label_pinned { le } else { re }, |r| r.0);
        let mut evidence = obj(vec![("conflictingPins", Value::Array(alternatives.pins.clone()))]);
        if let Some(at) = label_edge.label_at {
            evidence.insert("labelAt".into(), pt_json(at));
        }
        evidence.insert("labelRect".into(), rect_json(&hit.rect));
        evidence.insert(
            "collidedRoute".into(),
            json!({ "edge": edge_name(route_edge), "from": route_edge.from, "to": route_edge.to, "points": points_json(&points) }),
        );
        evidence.insert("routeSegmentIndex".into(), json!(hit.segment));
        evidence.insert("routeSegment".into(), json!({ "from": pt_json(hit.start), "to": pt_json(hit.end) }));
        evidence.insert("clearancePx".into(), json_num(js_round(hit.clearance * 10.0) / 10.0));
        evidence.insert("minimumPx".into(), json_num(hit.threshold));
        let fixes = self.repairs_with_label_nudges(&alternatives);
        Some(self.explicit_pin_conflict(diagnostic_edge, "explicit label-route clearance", evidence, fixes))
    }

    /// `throwReadableLabelLabelPinConflict`; `None` when neither label is pinned.
    fn label_label_pin_conflict(&self, left: &EdgeLabel, right: &EdgeLabel) -> Option<Throw> {
        let pinned: Vec<usize> = [left.index, right.index].into_iter().filter(|&k| self.edge(k).label_at.is_some()).collect();
        if pinned.is_empty() {
            return None;
        }
        let refs: Vec<(usize, &'static str)> = pinned.iter().map(|&k| (k, "labelAt")).collect();
        let alternatives = self.pin_reference_alternatives(&refs, "so readable-v2 can replan the remaining authored label pins");
        let diagnostic_edge = alternatives.refs.first().map_or(pinned[0], |r| r.0);
        let rect_of = |l: &EdgeLabel| {
            json!({
                "edge": self.edge(l.index).id,
                "x": json_num(l.rect.x), "y": json_num(l.rect.y),
                "width": json_num(l.rect.width), "height": json_num(l.rect.height),
            })
        };
        let evidence = obj(vec![
            ("conflictingPins", Value::Array(alternatives.pins.clone())),
            ("labelRects", json!([rect_of(left), rect_of(right)])),
            ("minimumGapPx", json!(-2)),
        ]);
        let fixes = self.repairs_with_label_nudges(&alternatives);
        Some(self.explicit_pin_conflict(diagnostic_edge, "explicit label-label clearance", evidence, fixes))
    }

    /// `classifyFailedAutomaticCandidatePins`. The raw candidates carry no points (see the module
    /// doc): only a pinned label is checked, at its pin; an unpinned label crashes in Archify.
    fn classify_failed_candidate_pins(&self, i: usize) -> R<()> {
        let prior: Vec<usize> = self.order.iter().copied().filter(|&k| k != i).collect();
        if prior.is_empty() {
            return Ok(());
        }
        let e = self.edge(i);
        let Some(text) = label_of(e) else { return Ok(()) };
        let Some(at) = e.label_at else {
            return Err(internal("Cannot read properties of undefined (reading 'length')"));
        };
        let width = workflow_label_width(text);
        let candidate = EdgeLabel { index: i, text: text.to_owned(), at, rect: Rect::new(at[0] - width / 2.0, at[1] - 10.0, width, 14.0) };
        let prior_labels: Vec<&EdgeLabel> = prior.iter().filter_map(|&k| self.labels[k].as_ref()).collect();
        if let Some(other) = prior_labels.iter().find(|o| rects_overlap(&candidate.rect, &o.rect, -2.0))
            && let Some(t) = self.label_label_pin_conflict(&candidate, other)
        {
            return Err(t);
        }
        let routes: Vec<(usize, Vec<Pt>)> =
            prior.iter().filter_map(|&k| self.paths[k].as_ref().map(|r| (k, r.points.clone()))).collect();
        let hits = self.collect_label_route_clearance(&[(i, candidate.rect)], &routes, 4.0);
        if let Some(hit) = hits.iter().find(|h| has_absolute_route_pins(self.edge(h.other_edge)) || e.label_at.is_some()) {
            let points = routes.iter().find(|(k, _)| *k == hit.other_edge).map(|(_, p)| p.as_slice());
            if let Some(t) = self.label_route_pin_conflict(hit, points) {
                return Err(t);
            }
        }
        Ok(())
    }

    /// `relationshipIdentity` of a canonical edge.
    fn identity(&self, k: usize) -> String {
        let e = self.edge(k);
        match e.id.as_deref() {
            Some(id) if !id.is_empty() => format!("id:{}\0{}\0{id}", e.from, e.to),
            _ => format!("index:{k}"),
        }
    }

    /// `sameRelationship` for two distinct canonical edges.
    fn same_relationship(&self, a: usize, b: usize) -> bool {
        if a == b {
            return true;
        }
        let (l, r) = (self.edge(a), self.edge(b));
        matches!((&l.id, &r.id), (Some(x), Some(y)) if !x.is_empty() && x == y) && l.from == r.from && l.to == r.to
    }

    /// `collectLabelRouteClearance` over normalised routes: per label and foreign route, the nearest
    /// segment when closer than `threshold`.
    fn collect_label_route_clearance(&self, labels: &[(usize, Rect)], routes: &[(usize, Vec<Pt>)], threshold: f64) -> Vec<ClearanceHit> {
        let mut seen = HashSet::new();
        let routes: Vec<(usize, Vec<Pt>)> = routes
            .iter()
            .filter_map(|(k, p)| {
                let points = normalize(p);
                (points.len() >= 2 && seen.insert(self.identity(*k))).then_some((*k, points))
            })
            .collect();
        let mut seen_labels = HashSet::new();
        let mut hits = Vec::new();
        for &(k, rect) in labels {
            if !rect.is_finite() || rect.width < 0.0 || rect.height < 0.0 || !seen_labels.insert(self.identity(k)) {
                continue;
            }
            for (other, points) in &routes {
                if self.same_relationship(k, *other) {
                    continue;
                }
                let mut nearest: Option<ClearanceHit> = None;
                for (si, w) in points.windows(2).enumerate() {
                    let Some(c) = clearance(w[0], w[1], &rect) else { continue };
                    if nearest.as_ref().is_none_or(|n| c < n.clearance) {
                        nearest = Some(ClearanceHit {
                            label_edge: k,
                            other_edge: *other,
                            rect,
                            clearance: c,
                            segment: si,
                            start: w[0],
                            end: w[1],
                            threshold,
                        });
                    }
                }
                if let Some(n) = nearest
                    && n.clearance + EPS < threshold
                {
                    hits.push(n);
                }
            }
        }
        hits
    }

    // ---- validateReadablePinnedGeometry ---------------------------------------------------------

    /// `validateReadableRouteControls`: a preset with a channel pin it does not own.
    fn validate_route_controls(&self, i: usize) -> R<()> {
        let e = self.edge(i);
        let Some(preset) = preset_of(e) else { return Ok(()) };
        let allowed = preset_pin_fields(preset);
        let conflicting: Vec<&'static str> = present_channel_pins(e).into_iter().filter(|f| !allowed.contains(f)).collect();
        if conflicting.is_empty() {
            return Ok(());
        }
        let mut refs = vec![(i, "route")];
        refs.extend(conflicting.iter().map(|f| (i, *f)));
        let alternatives = self.pin_reference_alternatives(&refs, "and keep the remaining verified route assertions");
        let evidence = obj(vec![
            ("route", json!(route_name(preset))),
            ("allowedPins", json!(allowed)),
            ("conflictingPins", Value::Array(alternatives.pins.clone())),
        ]);
        Err(self.explicit_pin_conflict(i, "route preset compatibility", evidence, alternatives.fixes()))
    }

    /// `validateReadablePinnedGeometry`: route every edge (absolute pins at contested nodes first,
    /// then canonical order) and check every authored pin, then the pairwise pin conflicts.
    pub fn route_all(&mut self) -> R<()> {
        let w = self.w();
        for (i, e) in w.edges.iter().enumerate() {
            if !has_absolute_route_pins(e) || self.ends[i].is_none() {
                continue;
            }
            let contested = w.edges.iter().any(|o| {
                !has_absolute_route_pins(o) && [&e.from, &e.to].iter().any(|id| **id == o.from || **id == o.to)
            });
            if !contested {
                continue;
            }
            self.validate_route_controls(i)?;
            self.path_for(i)?;
        }
        for (i, e) in w.edges.iter().enumerate() {
            if self.ends[i].is_none() {
                continue;
            }
            self.check_edge_pins(i, e)?;
        }
        self.pairwise_pin_conflicts()
    }

    fn check_edge_pins(&mut self, i: usize, e: &Edge) -> R<()> {
        self.validate_route_controls(i)?;
        let name = edge_name(e);
        let source = self.p.canonical.edge_source[i];
        if let Some(at) = e.label_at
            && let Some(l) = self.label_rect_for(i)?
            && (l.rect.x < 0.0 || l.rect.y < 0.0)
        {
            let evidence = obj(vec![
                (
                    "conflictingPins",
                    json!([{ "edge": name, "field": "labelAt", "path": format!("/edges/{source}/labelAt"), "value": pt_json(at) }]),
                ),
                ("offendingRect", rect_json(&l.rect)),
                ("minimumCoordinate", json!(0)),
            ]);
            return Err(self.explicit_pin_conflict(i, "viewBox-origin containment", evidence, self.label_at_nudge(i).into_iter().collect()));
        }
        let negative = if let Some(k) = e.via.iter().flatten().position(|p| p[0] < 0.0 || p[1] < 0.0) {
            Some(json!({ "edge": name, "field": "via", "path": format!("/edges/{source}/via/{k}"), "value": pt_json(e.via.as_ref().expect("via")[k]) }))
        } else if let Some(x) = e.channel_x.filter(|x| *x < 0.0) {
            Some(json!({ "edge": name, "field": "channelX", "path": format!("/edges/{source}/channelX"), "value": json_num(x) }))
        } else {
            e.channel_y
                .filter(|y| *y < 0.0)
                .map(|y| json!({ "edge": name, "field": "channelY", "path": format!("/edges/{source}/channelY"), "value": json_num(y) }))
        };
        if let Some(pin) = negative {
            let evidence = obj(vec![("conflictingPins", json!([pin])), ("minimumCoordinate", json!(0))]);
            let fixes = [self.verified_automatic_route_fix(i, false), self.verified_automatic_route_fix(i, true)];
            return Err(self.explicit_pin_conflict(i, "viewBox-origin containment", evidence, fixes.into_iter().flatten().collect()));
        }
        let points = self.path_for(i)?;
        if has_absolute_route_pins(e) {
            self.check_pinned_route(i, e, &points)?;
        }
        if e.label_at.is_some()
            && let Some(l) = self.label_rect_for(i)?
        {
            let at = pt_json(e.label_at.expect("labelAt"));
            let remove = |me: &Self, message: String| me.verified_edge_fix(i, message, |c| c.label_at = None).into_iter().collect::<Vec<_>>();
            if let Some(node) = self.p.nodes.iter().find(|n| rects_overlap(&l.rect, &n.rect, -2.0)) {
                let evidence = obj(vec![
                    ("conflictingPins", json!([self.pin_evidence(i, "labelAt")])),
                    ("labelAt", at),
                    ("labelRect", rect_json(&l.rect)),
                    ("obstacleNode", json!(node.id)),
                ]);
                let fixes = remove(self, "remove labelAt so readable-v2 can use its verified automatic label placement".into());
                return Err(self.explicit_pin_conflict(i, "edge-label node clearance", evidence, fixes));
            }
            if let Some(legend) = self.legend_rects.iter().find(|r| rects_overlap(&l.rect, &r.rect, 0.0)) {
                let evidence = obj(vec![
                    ("conflictingPins", json!([self.pin_evidence(i, "labelAt")])),
                    ("labelAt", at),
                    ("labelRect", rect_json(&l.rect)),
                    ("legendObstacle", legend_rect_json(legend)),
                ]);
                let fixes = remove(self, "remove labelAt so readable-v2 can use its verified automatic label placement".into());
                return Err(self.explicit_pin_conflict(i, "edge-label legend clearance", evidence, fixes));
            }
            if let Some(o) = self.scene_obstacles.iter().find(|o| rects_overlap(&l.rect, &o.rect, 0.0)) {
                let evidence = obj(vec![
                    ("conflictingPins", json!([self.pin_evidence(i, "labelAt")])),
                    ("labelAt", at),
                    ("labelRect", rect_json(&l.rect)),
                    ("compositionObstacle", obstacle_json(o)),
                ]);
                let fixes = remove(
                    self,
                    format!("remove labelAt from edge \"{name}\" so readable-v2 can use its verified automatic label placement"),
                );
                return Err(self.explicit_pin_conflict(i, "edge-label lane/phase/group clearance", evidence, fixes));
            }
        }
        Ok(())
    }

    /// The `hasPinnedRoute` half of the per-edge checks.
    fn check_pinned_route(&mut self, i: usize, e: &Edge, points: &[Pt]) -> R<()> {
        let geometry = |me: &Self, invariant: &str, mut evidence: Map<String, Value>| {
            let alternatives = me.route_geometry_pin_alternatives(i);
            let mut full = obj(vec![("conflictingPins", Value::Array(alternatives.pins.clone()))]);
            full.append(&mut evidence);
            me.explicit_pin_conflict(i, invariant, full, alternatives.fixes())
        };
        if let Some(k) = points.iter().position(|p| !is_finite_point(p)) {
            return Err(geometry(self, "finite route coordinates", obj(vec![("pointIndex", json!(k)), ("point", pt_json(points[k]))])));
        }
        let n = points.len();
        for si in 0..n.saturating_sub(1) {
            let (start, end) = (points[si], points[si + 1]);
            let dx = (end[0] - start[0]).abs();
            let dy = (end[1] - start[1]).abs();
            if dx <= EPS && dy <= EPS {
                let via = e.via.as_deref();
                let duplicate_fix = match via {
                    Some(v) if !v.is_empty() => {
                        let k = si.min(v.len() - 1);
                        self.verified_edge_fix(
                            i,
                            format!("remove duplicate via[{k}] and keep the remaining authored pins unchanged"),
                            |c| {
                                if let Some(v) = c.via.as_mut() {
                                    v.remove(k);
                                }
                            },
                        )
                    }
                    _ => self.verified_automatic_route_fix(i, false),
                };
                let segment = obj(vec![("segmentIndex", json!(si)), ("from", pt_json(start)), ("to", pt_json(end))]);
                if via.is_some() {
                    let mut evidence = obj(vec![("conflictingPins", json!([self.pin_evidence(i, "via")]))]);
                    evidence.extend(segment);
                    return Err(self.explicit_pin_conflict(i, "non-zero route segments", evidence, duplicate_fix.into_iter().collect()));
                }
                return Err(geometry(self, "non-zero route segments", segment));
            }
            if dx > EPS && dy > EPS {
                let segment = obj(vec![("segmentIndex", json!(si)), ("from", pt_json(start)), ("to", pt_json(end))]);
                return Err(geometry(self, "orthogonal route segments", segment));
            }
            let endpoint = si == 0 || si == n - 2;
            let minimum = if n == 2 { 28.0 } else if endpoint { 8.0 } else { 16.0 };
            let length = dx + dy;
            if length + EPS < minimum {
                let position = if si == 0 {
                    "source-stub"
                } else if si == n - 2 {
                    "target-stub"
                } else {
                    "interior"
                };
                let evidence = obj(vec![
                    ("segmentIndex", json!(si)),
                    ("position", json!(position)),
                    ("from", pt_json(start)),
                    ("to", pt_json(end)),
                    ("lengthPx", json_num(length)),
                    ("minimumPx", json_num(minimum)),
                ]);
                let invariant = if endpoint { "8px endpoint stub clearance" } else { "16px interior turn clearance" };
                return Err(geometry(self, invariant, evidence));
            }
        }
        let sides = self.edge_sides(i);
        if e.via.is_some() {
            let missing: Vec<&'static str> = present_channel_pins(e)
                .into_iter()
                .filter(|f| !route_contains_channel_pin(points, *f == "channelX", channel_pin_value(e, f).unwrap_or(f64::NAN)))
                .collect();
            if !missing.is_empty() {
                let mut fields = vec!["via"];
                fields.extend(missing);
                let (sets, fixes) = self.pin_removal_alternatives(i, &fields, "and replan the remaining explicit route assertions");
                let evidence = obj(vec![
                    ("route", json!(route_name(e.route.unwrap_or(EdgeRoute::Auto)))),
                    ("conflictingPins", self.conflict_pins_from_removal_sets(i, &sets, &fields)),
                    ("points", points_json(points)),
                ]);
                return Err(self.explicit_pin_conflict(i, "channel pin preservation", evidence, fixes));
            }
        }
        let (from, to) = self.nodes_of(i).expect("routed edges have measured endpoints");
        if let Some(preset) = preset_of(e)
            && !route_matches_preset_family(preset, points, from, to)
        {
            let mut refs = vec![(i, "route")];
            refs.extend(route_geometry_fields(e).into_iter().map(|f| (i, f)));
            let alternatives = self.pin_reference_alternatives(&refs, "and keep the remaining verified route assertions");
            let evidence = obj(vec![
                ("route", json!(route_name(preset))),
                ("conflictingPins", Value::Array(alternatives.pins.clone())),
                ("points", points_json(points)),
            ]);
            return Err(self.explicit_pin_conflict(i, "route preset compatibility", evidence, alternatives.fixes()));
        }
        if !route_honors_endpoint_sides(points, sides.from, sides.to) {
            let mut fields: Vec<&'static str> = Vec::new();
            if e.from_side.is_some() && !endpoint_side_is_honored(points, sides.from, true) {
                fields.push("fromSide");
            }
            if e.to_side.is_some() && !endpoint_side_is_honored(points, sides.to, false) {
                fields.push("toSide");
            }
            fields.extend(route_geometry_fields(e));
            let (sets, fixes) = self.pin_removal_alternatives(i, &fields, "and replan the remaining explicit pins");
            let evidence = obj(vec![
                ("conflictingPins", self.conflict_pins_from_removal_sets(i, &sets, &fields)),
                ("points", points_json(points)),
                ("fromSide", json!(side_name(sides.from))),
                ("toSide", json!(side_name(sides.to))),
            ]);
            return Err(self.explicit_pin_conflict(i, "perpendicular endpoint-side direction", evidence, fixes));
        }
        if let Some(collision) = self.first_route_node_collision(e, points) {
            return Err(geometry(self, "node clearance", collision));
        }
        if let Some(legend) = self.legend_rects.iter().find(|r| any_segment_hits(points, &r.rect)) {
            let evidence = obj(vec![("points", points_json(points)), ("legendObstacle", legend_rect_json(legend))]);
            return Err(geometry(self, "legend clearance", evidence));
        }
        if let Some(o) = self.scene_obstacles.iter().find(|o| any_segment_hits(points, &o.rect)) {
            let evidence = obj(vec![("points", points_json(points)), ("compositionObstacle", obstacle_json(o))]);
            return Err(geometry(self, "lane/phase/group label clearance", evidence));
        }
        if let Some((frame, side, length)) = first_border_run(points, &self.frames) {
            let evidence = obj(vec![
                ("points", points_json(points)),
                ("frame", json!(self.frames[frame].id)),
                ("side", json!(border_side_name(side))),
                ("overlapLengthPx", json_num(length)),
            ]);
            return Err(geometry(self, "structural-frame border clearance", evidence));
        }
        Ok(())
    }

    /// `firstRouteNodeCollision`: the first node (map order) a segment enters; an endpoint's own
    /// stub is exempt, unrelated nodes keep 2 px.
    fn first_route_node_collision(&self, e: &Edge, points: &[Pt]) -> Option<Map<String, Value>> {
        let last = points.len().checked_sub(2)?;
        for node in &self.p.nodes {
            let role = if node.id == e.from {
                "source-endpoint"
            } else if node.id == e.to {
                "target-endpoint"
            } else {
                "unrelated"
            };
            for si in 0..=last {
                if (role == "source-endpoint" && si == 0) || (role == "target-endpoint" && si == last) {
                    continue;
                }
                let gap = if role == "unrelated" { 2.0 } else { 0.0 };
                let (a, b) = (points[si], points[si + 1]);
                if seg_hits(a, b, &node.rect, gap) {
                    return Some(obj(vec![
                        ("obstacleNode", json!(node.id)),
                        ("obstacleRole", json!(role)),
                        ("segmentIndex", json!(si)),
                        ("from", pt_json(a)),
                        ("to", pt_json(b)),
                        ("clearancePx", json!(gap as i64)),
                    ]));
                }
            }
        }
        None
    }

    /// `validateReadablePairwisePinConflicts`: label/route, label/label, then (showcase) route/route
    /// crossings and corridors where one side is authored.
    fn pairwise_pin_conflicts(&mut self) -> R<()> {
        let w = self.w();
        let mut labels: Vec<EdgeLabel> = Vec::new();
        for i in 0..w.edges.len() {
            if let Some(l) = self.label_rect_for(i)? {
                labels.push(l);
            }
        }
        let mut routed: Vec<(usize, Vec<Pt>)> = Vec::new();
        for i in 0..w.edges.len() {
            if self.ends[i].is_some() {
                routed.push((i, self.path_for(i)?));
            }
        }
        let label_rects: Vec<(usize, Rect)> = labels.iter().map(|l| (l.index, l.rect)).collect();
        let hits = self.collect_label_route_clearance(&label_rects, &routed, 4.0);
        if let Some(hit) = hits
            .iter()
            .find(|h| self.edge(h.label_edge).label_at.is_some() || !route_assertion_fields(self.edge(h.other_edge)).is_empty())
            && let Some(t) = self.label_route_pin_conflict(hit, None)
        {
            return Err(t);
        }
        for (li, left) in labels.iter().enumerate() {
            for right in &labels[li + 1..] {
                if rects_overlap(&left.rect, &right.rect, -2.0)
                    && let Some(t) = self.label_label_pin_conflict(left, right)
                {
                    return Err(t);
                }
            }
        }
        if w.meta.quality_profile != Some(crate::model::common::QualityProfile::Showcase) {
            return Ok(());
        }
        for (li, (left, lp)) in routed.iter().enumerate() {
            for (right, rp) in &routed[li + 1..] {
                let (le, re) = (self.edge(*left), self.edge(*right));
                let left_pinned = !route_assertion_fields(le).is_empty();
                if !left_pinned && route_assertion_fields(re).is_empty() {
                    continue;
                }
                if [&le.from, &le.to].iter().any(|id| **id == re.from || **id == re.to) {
                    continue;
                }
                for ls in forward_collinear_segments(lp) {
                    for rs in forward_collinear_segments(rp) {
                        let Some(point) = proper_orthogonal_intersection(ls.start, ls.end, rs.start, rs.end) else { continue };
                        let lsi = ls.source_index_at(point);
                        let rsi = rs.source_index_at(point);
                        let mut refs: Vec<(usize, &'static str)> = route_assertion_fields(le).into_iter().map(|f| (*left, f)).collect();
                        refs.extend(route_assertion_fields(re).into_iter().map(|f| (*right, f)));
                        let alternatives =
                            self.pin_reference_alternatives(&refs, "so readable-v2 can replan the remaining authored route assertions");
                        let diagnostic_edge = if left_pinned { *left } else { *right };
                        let seg = |e: &Edge, a: Pt, b: Pt| json!({ "edge": e.id, "from": pt_json(a), "to": pt_json(b) });
                        let evidence = obj(vec![
                            ("conflictingPins", Value::Array(alternatives.pins.clone())),
                            ("point", pt_json(point)),
                            ("segmentIndex", json!(lsi)),
                            ("otherSegmentIndex", json!(rsi)),
                            ("routeSegments", json!([seg(le, ls.start, ls.end), seg(re, rs.start, rs.end)])),
                            (
                                "sourceRouteSegments",
                                json!([seg(le, lp[lsi], lp[lsi + 1]), seg(re, rp[rsi], rp[rsi + 1])]),
                            ),
                        ]);
                        return Err(self.explicit_pin_conflict(diagnostic_edge, "explicit route-route crossing", evidence, alternatives.fixes()));
                    }
                }
            }
        }
        for (li, (left, lp)) in routed.iter().enumerate() {
            for (right, rp) in &routed[li + 1..] {
                let (le, re) = (self.edge(*left), self.edge(*right));
                let Some(hit) = corridor_overlap(le, lp, re, rp, false, false) else { continue };
                let left_pinned = !route_assertion_fields(le).is_empty();
                if !left_pinned && route_assertion_fields(re).is_empty() {
                    continue;
                }
                let mut refs: Vec<(usize, &'static str)> = route_assertion_fields(le).into_iter().map(|f| (*left, f)).collect();
                refs.extend(route_assertion_fields(re).into_iter().map(|f| (*right, f)));
                let alternatives = self.pin_reference_alternatives(&refs, "so readable-v2 can replan the remaining authored route assertions");
                let diagnostic_edge = if left_pinned { *left } else { *right };
                let (ln, rn) = (normalize(lp), normalize(rp));
                let seg = |e: &Edge, pts: &[Pt], k: usize| json!({ "edge": e.id, "from": pt_json(pts[k]), "to": pt_json(pts[k + 1]) });
                let evidence = obj(vec![
                    ("conflictingPins", Value::Array(alternatives.pins.clone())),
                    ("segmentIndex", json!(hit.left_segment)),
                    ("otherSegmentIndex", json!(hit.right_segment)),
                    ("routeSegments", json!([seg(le, &ln, hit.left_segment), seg(re, &rn, hit.right_segment)])),
                    ("overlapStart", pt_json(hit.start)),
                    ("overlapEnd", pt_json(hit.end)),
                    ("overlapLengthPx", json_num(hit.length)),
                    ("minimumClearancePx", json!(8)),
                ]);
                return Err(self.explicit_pin_conflict(
                    diagnostic_edge,
                    "explicit route-route corridor clearance",
                    evidence,
                    alternatives.fixes(),
                ));
            }
        }
        Ok(())
    }

    // ---- the canvas -----------------------------------------------------------------------------

    /// `measuredContentBounds`: lanes, nodes, routes, label masks, phase and group frames and
    /// labels, the legend; each side remembers what pushed it last.
    pub fn content_bounds(&mut self) -> R<ContentBounds> {
        let p = self.p;
        let w = self.w();
        let mut b = [LANE_X, 27.0, LANE_X + p.layout.lane_w, p.legend_y(self.legend.extra_height) + 18.0];
        let mut owners: [String; 4] = [
            "workflow lanes".into(),
            if w.phases.as_ref().is_some_and(|ph| !ph.is_empty()) { "phase header band" } else { "workflow top padding" }.into(),
            "workflow lanes".into(),
            if self.legend.entries.is_empty() { "workflow lanes and bottom padding" } else { "legend" }.into(),
        ];
        let mut include = |pt: Pt, who: &str| {
            if pt[0] < b[0] {
                b[0] = pt[0];
                owners[0] = who.to_owned();
            }
            if pt[1] < b[1] {
                b[1] = pt[1];
                owners[1] = who.to_owned();
            }
            if pt[0] > b[2] {
                b[2] = pt[0];
                owners[2] = who.to_owned();
            }
            if pt[1] > b[3] {
                b[3] = pt[1];
                owners[3] = who.to_owned();
            }
        };
        let mut rects: Vec<(Rect, String)> = Vec::new();
        for n in &p.nodes {
            rects.push((n.rect, format!("node {}", n.id)));
        }
        let mut route_points: Vec<(usize, Vec<Pt>, Option<Rect>)> = Vec::new();
        for i in 0..w.edges.len() {
            if self.ends[i].is_none() {
                continue;
            }
            let points = self.path_for(i)?;
            let label = self.label_rect_for(i)?.map(|l| l.rect);
            route_points.push((i, points, label));
        }
        for r in &rects {
            include([r.0.x, r.0.y], &r.1);
            include([r.0.x + r.0.width, r.0.y + r.0.height], &r.1);
        }
        for (i, points, label) in &route_points {
            let e = &w.edges[*i];
            let key = match e.id.as_deref() {
                Some(id) if !id.is_empty() => id.to_owned(),
                _ => i.to_string(),
            };
            for pt in points {
                include(*pt, &format!("edge {key}"));
            }
            if let Some(l) = label {
                let who = format!("edge {key} label mask");
                include([l.x, l.y], &who);
                include([l.x + l.width, l.y + l.height], &who);
            }
        }
        let mut frame_rects: Vec<(Rect, String)> = Vec::new();
        for o in &self.scene_obstacles {
            match o.kind {
                LabelObstacleKind::PhaseHeader => {
                    frame_rects.push((o.rect, format!("phase {}", o.id.clone().unwrap_or_default())));
                }
                LabelObstacleKind::GroupLabel | LabelObstacleKind::LaneHeader => {}
            }
        }
        for (g, group) in p.groups.iter().zip(w.groups.iter().flatten()) {
            let valid = |c: f64| c.is_finite() && c.fract() == 0.0 && c >= 0.0 && c < p.layout.col_xs.len() as f64;
            if p.lane_index(&group.lane).is_none() || !valid(group.from_col) || !valid(group.to_col) || group.from_col > group.to_col {
                continue;
            }
            frame_rects.push((g.rect, format!("group {}", group.id)));
            let label = Rect::new(
                g.label_x,
                g.label_y - crate::layout::workflow::readable::GROUP_LABEL_MASK_ASCENT,
                group_label_width(&group.label),
                crate::layout::workflow::readable::GROUP_LABEL_MASK_H,
            );
            frame_rects.push((label, format!("group {} label", group.id)));
        }
        for (r, who) in &frame_rects {
            include([r.x, r.y], who);
            include([r.x + r.width, r.y + r.height], who);
        }
        if !self.legend.entries.is_empty() {
            for r in &self.legend_rects {
                let who = format!("legend {}", r.kind);
                include([r.rect.x, r.rect.y], &who);
                include([r.rect.x + r.rect.width, r.rect.y + r.rect.height], &who);
            }
        }
        let mut contributors: Vec<String> = Vec::new();
        for c in owners.iter().chain(&p.layout.width_contributors).chain(&p.layout.height_contributors) {
            if !contributors.contains(c) {
                contributors.push(c.clone());
            }
        }
        Ok(ContentBounds { left: b[0], top: b[1], right: b[2], bottom: b[3], contributors })
    }

    /// `finalizeReadableViewBox`: `requiredViewBox` from the content bounds; geometry above or left
    /// of the origin fails; an authored viewBox too small is `workflow/viewbox-capacity`; otherwise
    /// the canvas is the authored one, or the required one when none is authored.
    pub fn finalize_view_box(&mut self) -> R<ViewBox> {
        let bounds = self.content_bounds()?;
        let required = [
            js_max(self.legend.minimum_canvas_width, (bounds.right + 16.0).ceil()),
            js_max(self.auto_height, (bounds.bottom + 18.0).ceil()),
        ];
        let w = self.w();
        let initial = w.meta.view_box.unwrap_or([self.legend.minimum_canvas_width, self.auto_height]);
        let evidence = |actual: [f64; 2]| {
            obj(vec![
                ("actualViewBox", pt_json(actual)),
                ("requiredViewBox", pt_json(required)),
                ("contentBounds", json!([json_num(bounds.left), json_num(bounds.top), json_num(bounds.right), json_num(bounds.bottom)])),
                ("contributors", json!(bounds.contributors)),
            ])
        };
        if bounds.left < 0.0 || bounds.top < 0.0 {
            let code = if has_absolute_workflow_pins(w) { "workflow/explicit-pin-conflict" } else { "workflow/solver-budget-exhausted" };
            let message = format!(
                "Workflow geometry extends above or left of the viewBox origin ({}, {}).",
                js_num(js_round(bounds.left)),
                js_num(js_round(bounds.top))
            );
            return Err(fail(
                message.clone(),
                Diagnostic::error(code, &message)
                    .with_subject(Subject::of("workflow").with_rule(code).with_path("/meta/viewBox"))
                    .with_evidence(evidence(initial)),
            ));
        }
        let Some(view_box) = w.meta.view_box else { return Ok(ViewBox { view_box: required, required }) };
        if view_box[0] >= required[0] && view_box[1] >= required[1] {
            return Ok(ViewBox { view_box, required });
        }
        let message = format!(
            "Workflow viewBox {}\u{d7}{} cannot contain the readable-v2 layout; minimum {}\u{d7}{}.",
            js_num(view_box[0]),
            js_num(view_box[1]),
            js_num(required[0]),
            js_num(required[1])
        );
        let mut fixes = Vec::new();
        if self.accepts_doc(|d| d.meta.view_box = Some(required)) {
            fixes.push(format!("set meta.viewBox to at least [{}, {}]", js_num(required[0]), js_num(required[1])));
        }
        if self.accepts_doc(|d| d.meta.view_box = None) {
            fixes.push("omit meta.viewBox so the compiler can use its measured intrinsic canvas".to_owned());
        }
        Err(fail(
            message.clone(),
            Diagnostic::error("workflow/viewbox-capacity", &message)
                .with_subject(Subject::of("workflow").with_rule("workflow/viewbox-capacity").with_path("/meta/viewBox"))
                .with_evidence(evidence(view_box))
                .with_fixes(fixes),
        ))
    }
}

// ---- free helpers ---------------------------------------------------------------------------------

/// The verified removal sets of `verifiedPinReferenceAlternatives`.
#[derive(Debug, Clone, Default)]
struct PinAlternatives {
    /// `conflictingRefs`.
    refs: Vec<(usize, &'static str)>,
    /// `conflictingPins`.
    pins: Vec<Value>,
    /// `(removalSet, message)`.
    repairs: Vec<(Vec<(usize, &'static str)>, String)>,
}

impl PinAlternatives {
    fn fixes(&self) -> Vec<String> {
        self.repairs.iter().map(|(_, m)| m.clone()).collect()
    }
}

/// One `collectLabelRouteClearance` hit.
#[derive(Debug, Clone)]
struct ClearanceHit {
    label_edge: usize,
    other_edge: usize,
    rect: Rect,
    clearance: f64,
    segment: usize,
    start: Pt,
    end: Pt,
    threshold: f64,
}

fn edge_subject(e: &Edge, rule: &str) -> Subject {
    Subject::of("workflow").with_rule(rule).with_extra("edge", json!(e.id)).with_extra("from", json!(e.from)).with_extra("to", json!(e.to))
}

/// A thrown non-diagnostic error (`internal/unclassified`, as the CLI's fallback reports it).
fn internal(message: &str) -> Throw {
    fail(
        message.to_owned(),
        Diagnostic::error("internal/unclassified", message)
            .with_subject(Subject::default())
            .with_evidence(obj(vec![("errorName", json!("TypeError"))])),
    )
}

fn legend_rect_json(r: &LegendRect) -> Value {
    json!({ "kind": r.kind, "x": json_num(r.rect.x), "y": json_num(r.rect.y), "width": json_num(r.rect.width), "height": json_num(r.rect.height) })
}

fn obstacle_json(o: &LabelObstacle) -> Value {
    let kind = match o.kind {
        LabelObstacleKind::LaneHeader => "lane-header",
        LabelObstacleKind::PhaseHeader => "phase-header",
        LabelObstacleKind::GroupLabel => "group-label",
    };
    json!({
        "kind": kind, "id": o.id,
        "x": json_num(o.rect.x), "y": json_num(o.rect.y), "width": json_num(o.rect.width), "height": json_num(o.rect.height),
    })
}

fn border_side_name(side: BorderSide) -> &'static str {
    match side {
        BorderSide::Top => "top",
        BorderSide::Right => "right",
        BorderSide::Bottom => "bottom",
        BorderSide::Left => "left",
        BorderSide::Line => "line",
    }
}

/// `collectBorderRuns` for one raw route, first hit only: `(frame, side, merged overlap length)`.
fn first_border_run(points: &[Pt], frames: &[CompositionFrame]) -> Option<(usize, BorderSide, f64)> {
    if points.len() < 2 || !points.iter().all(|p| is_finite_point(p)) {
        return None;
    }
    let extent = route_bounds(points);
    for (fi, frame) in frames.iter().enumerate() {
        if !bounds_overlap(&rect_bounds(&frame.rect), &extent, 0.0) {
            continue;
        }
        for border in frame_border_segments(&Frame::Rect { rect: frame.rect, radius: frame.radius }) {
            let horizontal = (border.start[1] - border.end[1]).abs() <= EPS;
            let axis = usize::from(!horizontal);
            let mut intervals: Vec<(f64, f64)> = points
                .windows(2)
                .filter_map(|w| collinear_axis_overlap(w[0], w[1], border.start, border.end))
                .filter(|o| o.length > EPS)
                .map(|o| (js_min(o.start[axis], o.end[axis]), js_max(o.start[axis], o.end[axis])))
                .collect();
            if intervals.is_empty() {
                continue;
            }
            intervals.sort_by(|a, b| (a.0 - b.0).partial_cmp(&0.0).unwrap_or(Ordering::Equal).then((a.1 - b.1).partial_cmp(&0.0).unwrap_or(Ordering::Equal)));
            let mut merged: Vec<(f64, f64)> = Vec::new();
            for (low, high) in intervals {
                match merged.last_mut() {
                    Some(prev) if low <= prev.1 + EPS => prev.1 = js_max(prev.1, high),
                    _ => merged.push((low, high)),
                }
            }
            let total = merged.iter().fold(0.0, |t, (l, h)| t + (h - l));
            return Some((fi, border.side, total));
        }
    }
    None
}

/// The longest collinear overlap of two relations (`collectAmbiguousCorridors` for one pair).
#[derive(Debug, Clone, Copy)]
struct Corridor {
    left_segment: usize,
    right_segment: usize,
    length: f64,
    start: Pt,
    end: Pt,
}

/// `shortWorkflowTrunk`: a shared terminal trunk of at most 24 px between same-styled edges.
fn short_workflow_trunk(a: &Edge, b: &Edge, lp: &[Pt], rp: &[Pt], ls: usize, rs: usize, length: f64) -> bool {
    if length > 24.0 + EPS {
        return false;
    }
    let variant = |e: &Edge| e.variant.unwrap_or(Variant::Default);
    let role = |e: &Edge| e.role;
    if variant(a) != variant(b) || stroke_width(a) != stroke_width(b) || role(a) != role(b) {
        return false;
    }
    let same = |p: Pt, q: Pt| (p[0] - q[0]).abs() < EPS && (p[1] - q[1]).abs() < EPS;
    let source = a.from == b.from && ls == 0 && rs == 0 && same(lp[0], rp[0]);
    let target = a.to == b.to && ls == lp.len() - 2 && rs == rp.len() - 2 && same(lp[lp.len() - 1], rp[rp.len() - 1]);
    if !source && !target {
        return false;
    }
    let (p, q, r, s) = (lp[ls], lp[ls + 1], rp[rs], rp[rs + 1]);
    (q[0] - p[0]) * (s[0] - r[0]) + (q[1] - p[1]) * (s[1] - r[1]) > 0.0
}

/// `collectAmbiguousCorridors` over two relations (normalised routes, at least 8 px).
fn corridor_overlap(a: &Edge, ap: &[Pt], b: &Edge, bp: &[Pt], include_shared: bool, short_trunks: bool) -> Option<Corridor> {
    let lp = normalize(ap);
    let rp = normalize(bp);
    if lp.len() < 2 || rp.len() < 2 {
        return None;
    }
    let shared = [&a.from, &a.to].iter().any(|id| **id == b.from || **id == b.to);
    if shared && !include_shared {
        return None;
    }
    let mut longest: Option<Corridor> = None;
    for ls in 0..lp.len() - 1 {
        for rs in 0..rp.len() - 1 {
            let Some(o) = collinear_axis_overlap(lp[ls], lp[ls + 1], rp[rs], rp[rs + 1]) else { continue };
            if o.length + EPS < 8.0 {
                continue;
            }
            if short_trunks && short_workflow_trunk(a, b, &lp, &rp, ls, rs, o.length) {
                continue;
            }
            if longest.as_ref().is_none_or(|l| o.length > l.length + EPS) {
                longest = Some(Corridor { left_segment: ls, right_segment: rs, length: o.length, start: o.start, end: o.end });
            }
        }
    }
    longest
}

/// A `forwardCollinearAnalysisSegments` segment: merged forward-collinear legs of a raw route.
pub(crate) struct AnalysisSegment {
    pub(crate) start: Pt,
    pub(crate) end: Pt,
    segment_index: usize,
    sources: Vec<(Pt, Pt, usize)>,
}

impl AnalysisSegment {
    /// `sourceSegmentIndexAtPoint`.
    pub(crate) fn source_index_at(&self, point: Pt) -> usize {
        self.sources.iter().find(|(s, e, _)| point_lies_on_segment(point, *s, *e)).map_or(self.segment_index, |s| s.2)
    }
}

fn segments_continue_forward(fs: Pt, fe: Pt, ss: Pt, se: Pt) -> bool {
    if !is_finite_point(&[fs[0], fs[1], fe[0], fe[1], ss[0], ss[1], se[0], se[1]]) {
        return false;
    }
    if (fe[0] - ss[0]).abs() > EPS || (fe[1] - ss[1]).abs() > EPS {
        return false;
    }
    let f = [fe[0] - fs[0], fe[1] - fs[1]];
    let s = [se[0] - ss[0], se[1] - ss[1]];
    if f[0].hypot(f[1]) <= EPS || s[0].hypot(s[1]) <= EPS {
        return false;
    }
    if (f[0] * s[1] - f[1] * s[0]).abs() > EPS {
        return false;
    }
    f[0] * s[0] + f[1] * s[1] > EPS
}

fn point_lies_on_segment(p: Pt, s: Pt, e: Pt) -> bool {
    if !is_finite_point(&[p[0], p[1], s[0], s[1], e[0], e[1]]) {
        return false;
    }
    let length = (e[0] - s[0]).hypot(e[1] - s[1]);
    if length <= EPS {
        return (p[0] - s[0]).hypot(p[1] - s[1]) <= EPS;
    }
    if cross_product(s, e, p).abs() > EPS * length {
        return false;
    }
    p[0] >= js_min(s[0], e[0]) - EPS && p[0] <= js_max(s[0], e[0]) + EPS && p[1] >= js_min(s[1], e[1]) - EPS && p[1] <= js_max(s[1], e[1]) + EPS
}

/// `authoredAnalysisSegments`: every authored segment on its own.
pub(crate) fn authored_segments(points: &[Pt]) -> Vec<AnalysisSegment> {
    points
        .windows(2)
        .enumerate()
        .map(|(k, w)| AnalysisSegment { start: w[0], end: w[1], segment_index: k, sources: vec![(w[0], w[1], k)] })
        .collect()
}

pub(crate) fn forward_collinear_segments(points: &[Pt]) -> Vec<AnalysisSegment> {
    let mut out: Vec<AnalysisSegment> = Vec::new();
    for (k, w) in points.windows(2).enumerate() {
        let (start, end) = (w[0], w[1]);
        if let Some(prev) = out.last_mut()
            && segments_continue_forward(prev.start, prev.end, start, end)
        {
            prev.end = end;
            prev.sources.push((start, end, k));
            continue;
        }
        out.push(AnalysisSegment { start, end, segment_index: k, sources: vec![(start, end, k)] });
    }
    out
}

/// `properOrthogonalIntersection`.
fn proper_orthogonal_intersection(ls: Pt, le: Pt, rs: Pt, re: Pt) -> Option<Pt> {
    let (lo, ro) = (orientation(ls, le), orientation(rs, re));
    if lo == ro || lo == Orientation::Diagonal || ro == Orientation::Diagonal {
        return None;
    }
    let (hs, he) = if lo == Orientation::Horizontal { (ls, le) } else { (rs, re) };
    let (vs, ve) = if lo == Orientation::Vertical { (ls, le) } else { (rs, re) };
    let point = [vs[0], hs[1]];
    let inside_h = point[0] > js_min(hs[0], he[0]) + EPS && point[0] < js_max(hs[0], he[0]) - EPS;
    let inside_v = point[1] > js_min(vs[1], ve[1]) + EPS && point[1] < js_max(vs[1], ve[1]) - EPS;
    (inside_h && inside_v).then_some(point)
}

/// `stableValueKey` of an edge (the channel-label lookup key), re-exported for the build.
pub fn edge_key(e: &Edge) -> String {
    serde_json::to_value(e).map(|v| stable_value_key(&v)).unwrap_or_default()
}

/// Whether a document is laid out by this router (`schema_version: 2`).
pub fn is_readable(w: &Workflow) -> bool {
    w.schema_version == SchemaVersion::V2
}

/// `textUnits(label) * 5.6`, the group label mask width.
pub fn group_label_width(label: &str) -> f64 {
    units(label) as f64 * 5.6
}
