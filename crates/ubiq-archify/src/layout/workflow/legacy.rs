//! Workflow v1, the fixed layout (P6.1): `createLegacyLayout`, the lane/column cell math, the legacy
//! routes (presets, the same-lane elbow, the one-bend cross-lane route or a gap channel), the v1
//! canvas, the labels, the legend and the scene. Port of the `schema_version === 1` paths of
//! `renderers/workflow/workflow-compiler.mjs`; `00` section 5.5 (Workflow v1), `03` section 2.6.
//!
//! Nothing is solved: a node sits at `(lane, col)` with `colXs = [88, 220, 300, 430, 500, 625]`, the
//! canvas is the authored `viewBox` or `720 x (52 + 104n + 20(n-1) + 124)`.
//!
//! One deliberate deviation (`_docs/decisions.md` D40): [`LegacyPlan::new`] pads the
//! frames. A lane with a group (or an exception lane) grows to hold its nodes [`FRAME_PAD`] inside
//! their group, under the group's label, and its groups `FRAME_PAD` inside the exception frame; a
//! group widens to its nodes; the lanes below shift down and the canvas (authored or not) grows by the
//! same amount. [`LegacyPlan::archify`] is Archify's fixed geometry, which the parity tests compare.
//! The rules of `validateWorkflow` (bounds, overlap, short edges, clean-flow, composition, the v1
//! canvas, `column-capacity`) are [`crate::gates::workflow_v1`]; only a legend that cannot be placed
//! is an error here, as `measureLegend` says at render time.
//!
//! # API
//!
//! - [`LegacyPlan::new`]: everything measured, un-rounded, in Archify's `Map` order (a duplicated node
//!   id collapses to the last value at the first position). It never fails: a node on an unknown lane
//!   or column measures `NaN`, which the gate reports.
//! - [`build_plan`] / [`build_v1`]: the [`Scene`] and the [`LegacyGeometry`] (the receipt numbers).
//!
//! The brand badge and the locale catalogues are drawn. Not drawn: the `grid` pattern (the classic
//! preset hides it; the others get it from `tokens::restyle`) and the source beacon (needs
//! repository evidence, D19).

use std::collections::{HashMap, HashSet};

use crate::diag::Diagnostic;
use crate::geom::{
    End, Pt, Rect, Seg, Side, SpreadOptions, SpreadRelation, automatic_port_spread, normalize, segment_intersects_rect,
};
use crate::labels::{self, Hints};
use crate::legend::{self, Layout as LegendLayout, Measured, Placed, Style, Unfit};
use crate::model::common::{ComponentType, LegendMode, NodeIcon, SchemaVersion, Variant};
use crate::model::workflow::{Edge, EdgeRoute, Group as WfGroup, LaneVariant, Node, Phase, Workflow, WorkflowMeta};
use crate::route::dataflow::side as model_side;
use crate::scene::{
    Anchor, Bounds, Detail, EdgeRef, Fill, Group, GroupId, GroupKind, Layer, Marker, PolylineShape, RectShape, Scene,
    SceneBuilder, Shape, Stroke, TextShape,
};
use crate::sigils;
use crate::text::{self, DiagramType, LabelBox, Row};
use crate::tokens::{EdgeVariant, Kind, Token};

/// `layout.laneX`, `laneY`, `laneW`, `laneH`, `laneGap`, `laneTitleH` (`createLegacyLayout`).
pub const LANE_X: f64 = 40.0;
pub const LANE_Y: f64 = 52.0;
pub const LANE_W: f64 = 640.0;
pub const LANE_H: f64 = 104.0;
pub const LANE_GAP: f64 = 20.0;
pub const LANE_TITLE_H: f64 = 30.0;
/// `LEGACY_COLUMN_CENTERS`.
pub const COLUMNS: [f64; 6] = [88.0, 220.0, 300.0, 430.0, 500.0, 625.0];
/// `layout.nodeW`, `nodeH` and the taller node that bears a `tag`.
pub const NODE_W: f64 = 92.0;
pub const NODE_H: f64 = 52.0;
pub const TAGGED_NODE_H: f64 = 68.0;
/// `layout.defaultViewBoxWidth`, and the space under the last lane (`+ 124`).
pub const DEFAULT_VIEW_BOX_WIDTH: f64 = 720.0;
pub const BOTTOM_PAD: f64 = 124.0;
/// The legend sits `44` under the last lane; its band starts `8` under it.
pub const LEGEND_GAP: f64 = 44.0;
pub const LEGEND_TOP_PAD: f64 = 8.0;
pub const LEGEND_X: f64 = 20.0;
pub const LEGEND_SIDE_PAD: f64 = 20.0;
/// `{ fontSize: 7, itemGap: 7 }` of `workflowLegendLayout`.
pub const LEGEND_STYLE: Style = Style { font_size: 7.0, item_gap: 7.0 };
/// Node radius 6, label mask radius 3, lane radius 10, exception inset 6 and radius 8, group radius 9.
pub const NODE_RADIUS: f64 = 6.0;
pub const LABEL_RADIUS: f64 = 3.0;
pub const LANE_RADIUS: f64 = 10.0;
pub const EXCEPTION_INSET: f64 = 6.0;
pub const EXCEPTION_RADIUS: f64 = 8.0;
pub const GROUP_RADIUS: f64 = 9.0;
/// A v1 group frame starts `GROUP_FRAME_TOP_INSET` under the title strip and is `laneH - titleH - 16` high.
pub const GROUP_FRAME_TOP_INSET: f64 = 8.0;
pub const GROUP_FRAME_HEIGHT: f64 = LANE_H - LANE_TITLE_H - 16.0;
/// A v1 group label baseline is `14` under its frame top (v2 hangs it above), font 7.
pub const GROUP_LABEL_DOWN: f64 = 14.0;
pub const GROUP_LABEL_FONT: f64 = 7.0;
/// D40 (a deliberate deviation from Archify): the clearance between a node and
/// its group frame (sides, bottom) and between a group and its exception frame (every side).
pub const FRAME_PAD: f64 = 12.0;
/// The top of a group frame down to its nodes: the label baseline (`14`), its descent and a gap.
pub const GROUP_LABEL_BAND: f64 = 20.0;
/// A group frame's top, from its lane's top.
const GROUP_TOP: f64 = LANE_TITLE_H + GROUP_FRAME_TOP_INSET;
/// Phase header: rule at `y 35`, mask `y 27 x 16`, text baseline `39`, font 8.
pub const PHASE_LINE_Y: f64 = 35.0;
pub const PHASE_MASK_Y: f64 = 27.0;
pub const PHASE_MASK_H: f64 = 16.0;
pub const PHASE_TEXT_Y: f64 = 39.0;
/// Phase spans pad each end by `46`, group spans by `50`.
pub const PHASE_PAD: f64 = 46.0;
pub const GROUP_PAD: f64 = 50.0;
/// Lane title: `x + 14`, baseline `y + 22`, font 10, weight 600.
pub const LANE_TITLE_DX: f64 = 14.0;
pub const LANE_TITLE_DY: f64 = 22.0;
/// Node rows: label baseline `21`, sublabel `38`, tag `height - 12`.
pub const LABEL_Y: f64 = 21.0;
pub const SUBLABEL_Y: f64 = 38.0;
pub const TAG_UP: f64 = 12.0;
/// Strokes: node 1.5, edge 1.4, emphasis 1.8, phase rule 1.1; edge label font 8.
pub const NODE_STROKE: f64 = 1.5;
pub const EDGE_WIDTH: f64 = 1.4;
pub const EMPHASIS_WIDTH: f64 = 1.8;
pub const LABEL_FONT: f64 = 8.0;
/// Edge hit rail (screen-independent, vu) for `Hit::Polyline`; not an Archify constant.
pub const EDGE_HIT_HALF_WIDTH: f64 = 6.0;
/// An edge label box is `14` high, its baseline `10` under the top.
pub const LABEL_H: f64 = 14.0;
pub const LABEL_ASCENT: f64 = 10.0;
/// The one-bend route needs both legs at least this long and clears unrelated nodes by `2`.
const ONE_BEND_MIN_LEG: f64 = 8.0;
const ONE_BEND_CLEARANCE: f64 = 2.0;

/// `Math.max` / `Math.min`: a NaN poisons the result (Rust's `f64::max` would skip it).
fn js_max(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() { f64::NAN } else { a.max(b) }
}

/// `layout.colXs[col]`: `NaN` (`undefined`) for a column that is not an integer in `0..=5`.
pub fn col_x(col: f64) -> f64 {
    if col.is_finite() && col.fract() == 0.0 && col >= 0.0 && (col as usize) < COLUMNS.len() { COLUMNS[col as usize] } else { f64::NAN }
}

/// A measured node (`measureNode`): the rect the gates and the router see, and the centre JS reads
/// from the node itself (`cx` is the column centre, not `x + width / 2`).
#[derive(Clone, Debug, PartialEq)]
pub struct NodeBox {
    /// The index in `doc.nodes` of the value this entry holds (the last of a duplicated id).
    pub index: usize,
    pub id: String,
    pub lane: String,
    pub col: f64,
    pub rect: Rect,
    pub cx: f64,
    pub cy: f64,
}

/// A routed edge: `pathFor` plus the sides it was drawn with.
#[derive(Clone, Debug, PartialEq)]
pub struct RoutedEdge {
    pub points: Vec<Pt>,
    pub from_side: Side,
    pub to_side: Side,
}

/// An edge label (`labelRectFor`): the mask rect, the text anchor and the text.
#[derive(Clone, Debug, PartialEq)]
pub struct LabelPlate {
    pub text: String,
    pub rect: Rect,
    pub at: Pt,
}

/// A phase or group span (`spanForCols`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Span {
    pub x: f64,
    pub width: f64,
    pub cx: f64,
}

/// A structural frame as `workflowCompositionFrames` lists it.
#[derive(Clone, Debug, PartialEq)]
pub struct FrameDef {
    pub id: String,
    pub label: String,
    pub kind: &'static str,
    pub rect: Rect,
    pub radius: f64,
}

/// `meta.legend` as the legend resolver reads it (`legendEntry` overrides per kind).
fn legend_config(meta: &WorkflowMeta) -> Option<legend::Config> {
    let l = meta.legend.as_ref()?;
    let mut entries = HashMap::new();
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

pub fn kind_of(t: ComponentType) -> Kind {
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

pub fn variant_name(v: Option<Variant>) -> &'static str {
    match v {
        None | Some(Variant::Default) => "default",
        Some(Variant::Emphasis) => "emphasis",
        Some(Variant::Security) => "security",
        Some(Variant::Dashed) => "dashed",
    }
}

/// A non-empty string field (JS truthiness of `edge.label`, `node.tag`).
fn truthy(s: &Option<String>) -> Option<&str> {
    s.as_deref().filter(|s| !s.is_empty())
}

/// `edge.width || (variant === 'emphasis' ? 1.8 : 1.4)`.
pub fn edge_width(edge: &Edge) -> f64 {
    edge.width.filter(|w| *w != 0.0).unwrap_or(if edge.variant == Some(Variant::Emphasis) { EMPHASIS_WIDTH } else { EDGE_WIDTH })
}

/// Whether the router chooses everything (`!via`, `route` absent or `auto`): the one-bend sides.
fn automatic_route(edge: &Edge) -> bool {
    edge.via.is_none() && matches!(edge.route, None | Some(EdgeRoute::Auto))
}

/// The eligibility test of `automaticPortSpread`: no `via` (an empty one counts), `route` absent or
/// `auto`, no `channelX`/`channelY`, no `labelAt`.
fn spread_eligible(edge: &Edge) -> bool {
    automatic_route(edge) && edge.channel_x.is_none() && edge.channel_y.is_none() && edge.label_at.is_none()
}

/// `legacyDefaultFromSide` on the node centres JS reads (`node.cx`, `node.cy`).
fn default_from_side(from: &NodeBox, to: &NodeBox) -> Side {
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

fn default_to_side(from: &NodeBox, to: &NodeBox) -> Side {
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

/// `anchor(node, side)`: the side midpoint through the node's own `cx`/`cy`.
fn anchor(n: &NodeBox, side: Side) -> Pt {
    let r = &n.rect;
    match side {
        Side::Left => [r.x, n.cy],
        Side::Right => [r.x + r.width, n.cy],
        Side::Top => [n.cx, r.y],
        Side::Bottom => [n.cx, r.y + r.height],
    }
}

/// `sameLaneAutoVia`: a mid-x elbow, nothing for an already straight pair.
fn same_lane_via(start: Pt, end: Pt) -> Vec<Pt> {
    if start[0] == end[0] || start[1] == end[1] {
        return Vec::new();
    }
    let mid = (start[0] + end[0]) / 2.0;
    vec![[mid, start[1]], [mid, end[1]]]
}

/// `spanForCols(from, to, pad, 0)`.
fn span_for_cols(from_col: f64, to_col: f64, pad: f64) -> Span {
    let start = col_x(from_col) - pad;
    let end = col_x(to_col) + pad;
    let width = js_max(end - start, 0.0);
    if from_col == to_col && width > end - start {
        return Span { x: start, width, cx: start + width / 2.0 };
    }
    let cx = (start + end) / 2.0;
    Span { x: cx - width / 2.0, width, cx }
}

/// `(width, height, y from the lane top)` of a node as Archify measures it: centred in the `74` under
/// the title strip, plus `yOffset`.
fn node_size(node: &Node) -> (f64, f64, f64) {
    let width = node.width.filter(|w| *w != 0.0).unwrap_or(NODE_W);
    let height = node.height.filter(|h| *h != 0.0).unwrap_or(if truthy(&node.tag).is_some() { TAGGED_NODE_H } else { NODE_H });
    let content_h = LANE_H - LANE_TITLE_H;
    (width, height, LANE_TITLE_H + (content_h - height) / 2.0 + node.y_offset.filter(|o| *o != 0.0).unwrap_or(0.0))
}

/// The per-lane fit, in lane-relative numbers.
struct LaneFit {
    heights: Vec<f64>,
    dy: Vec<f64>,
    /// Per `doc.groups`: `(x, width, bottom from the lane top)`.
    groups: Vec<(f64, f64, f64)>,
}

/// The frame padding (D40). Unpadded it is Archify's geometry: every lane
/// `LANE_H`, no shift, the column spans `58` high. Padded, per lane: the nodes move down (all of them,
/// so a lane keeps one centre line) until each grouped node is [`GROUP_LABEL_BAND`] under its group's
/// top; a group grows to its nodes plus [`FRAME_PAD`] (and, in an exception lane, is first kept
/// `FRAME_PAD` inside the exception frame); the lane grows to hold its groups (`8` under, as Archify,
/// or `FRAME_PAD` inside the exception frame) and, once shifted, its nodes. A lane with no group is
/// Archify's.
fn fit_lanes(doc: &Workflow, padded: bool) -> LaneFit {
    let groups: Vec<&WfGroup> = doc.groups.iter().flatten().collect();
    let archify_group = |g: &WfGroup| {
        let span = span_for_cols(g.from_col, g.to_col, GROUP_PAD);
        (span.x, span.width, GROUP_TOP + GROUP_FRAME_HEIGHT)
    };
    let mut fit = LaneFit {
        heights: vec![LANE_H; doc.lanes.len()],
        dy: vec![0.0; doc.lanes.len()],
        groups: groups.iter().map(|g| archify_group(g)).collect(),
    };
    if !padded {
        return fit;
    }
    // The value a duplicated id collapses to is its last.
    let last: HashMap<&str, usize> = doc.nodes.iter().enumerate().map(|(i, n)| (n.id.as_str(), i)).collect();
    for (li, lane) in doc.lanes.iter().enumerate() {
        let exception = lane.variant == Some(LaneVariant::Exception);
        // `(col, x, width, y, height)` of the lane's nodes, lane-relative and unshifted.
        let nodes: Vec<(f64, f64, f64, f64, f64)> = doc
            .nodes
            .iter()
            .enumerate()
            .filter(|(i, n)| n.lane == lane.id && last.get(n.id.as_str()) == Some(i) && col_x(n.col).is_finite())
            .map(|(_, n)| {
                let (w, h, y) = node_size(n);
                (n.col, col_x(n.col) - w / 2.0, w, y, h)
            })
            .collect();
        let in_lane: Vec<usize> = (0..groups.len()).filter(|&gi| groups[gi].lane == lane.id).collect();
        let members = |g: &WfGroup| {
            let (from, to) = (g.from_col, g.to_col);
            nodes.iter().filter(move |n| n.0 >= from && n.0 <= to)
        };
        let dy = in_lane
            .iter()
            .flat_map(|&gi| members(groups[gi]).map(|n| GROUP_TOP + GROUP_LABEL_BAND - n.3))
            .fold(0.0_f64, f64::max);
        fit.dy[li] = dy;
        let mut height = LANE_H;
        for &gi in &in_lane {
            let g = groups[gi];
            let (mut left, width, _) = archify_group(g);
            let mut right = left + width;
            let mut bottom = GROUP_TOP + GROUP_FRAME_HEIGHT;
            if left.is_finite() && right.is_finite() {
                if exception {
                    left = left.max(LANE_X + EXCEPTION_INSET + FRAME_PAD);
                    right = right.min(LANE_X + LANE_W - EXCEPTION_INSET - FRAME_PAD);
                }
                for n in members(g) {
                    left = left.min(n.1 - FRAME_PAD);
                    right = right.max(n.1 + n.2 + FRAME_PAD);
                    bottom = bottom.max(n.3 + dy + n.4 + FRAME_PAD);
                }
            }
            fit.groups[gi] = (left, right - left, bottom);
            height = height.max(bottom + if exception { FRAME_PAD + EXCEPTION_INSET } else { GROUP_FRAME_TOP_INSET });
        }
        if dy > 0.0 {
            let under = if exception { FRAME_PAD + EXCEPTION_INSET } else { FRAME_PAD };
            height = nodes.iter().map(|n| n.3 + dy + n.4 + under).fold(height, f64::max);
        }
        fit.heights[li] = height;
    }
    fit
}

/// Whether the frame padding moves anything in `doc` (a lane, a node, a group frame), i.e. whether
/// [`LegacyPlan::new`] and [`LegacyPlan::archify`] differ. The golden tests compare such a document
/// with our own recorded output instead of Archify's.
pub fn padding_changes(doc: &Workflow) -> bool {
    let (a, b) = (fit_lanes(doc, false), fit_lanes(doc, true));
    let bits = |v: &mut dyn Iterator<Item = f64>| v.map(f64::to_bits).collect::<Vec<_>>();
    let groups = |f: &LaneFit| bits(&mut f.groups.iter().flat_map(|g| [g.0, g.1, g.2]));
    bits(&mut a.heights.iter().copied()) != bits(&mut b.heights.iter().copied())
        || bits(&mut a.dy.iter().copied()) != bits(&mut b.dy.iter().copied())
        || groups(&a) != groups(&b)
}

/// Everything laid out for one v1 document.
#[derive(Clone, Debug)]
pub struct LegacyPlan<'a> {
    pub doc: &'a Workflow,
    /// `meta.viewBox`, else `[720, autoHeight]`; also `requiredViewBox` in v1. Padded, an authored
    /// height grows by what the lanes grew.
    pub view_box: [f64; 2],
    /// `52 + 104n + 20(n - 1) + 124` (with the lanes' real heights when padded).
    pub auto_height: f64,
    /// Per `doc.lanes`: the top and the height (`LANE_H` unless padding grew it).
    lane_tops: Vec<f64>,
    lane_heights: Vec<f64>,
    /// Per `doc.lanes`: how far padding moved the lane's nodes down from Archify's centre line.
    lane_dy: Vec<f64>,
    /// Per `doc.groups`: the frame.
    group_rects: Vec<Rect>,
    /// The `nodes` Map in order (the render order).
    pub nodes: Vec<NodeBox>,
    /// Per `doc.edges`: `None` when an endpoint is not a node (the gate reports it).
    pub routes: Vec<Option<RoutedEdge>>,
    /// Per `doc.edges`: the label box, for a labelled routed edge.
    pub labels: Vec<Option<LabelPlate>>,
    /// The legend rows that survived `resolveLegend`.
    pub entries: Vec<legend::Entry>,
    lane_index: HashMap<String, usize>,
    by_id: HashMap<String, usize>,
}

impl<'a> LegacyPlan<'a> {
    /// Measure, route and label `doc` (`schema_version` is taken as 1: the caller chooses), with the
    /// frames padded (the module's deviation).
    pub fn new(doc: &'a Workflow) -> LegacyPlan<'a> {
        Self::plan(doc, true)
    }

    /// Archify's own fixed geometry, unpadded: what the parity tests compare with Archify.
    pub fn archify(doc: &'a Workflow) -> LegacyPlan<'a> {
        Self::plan(doc, false)
    }

    fn plan(doc: &'a Workflow, padded: bool) -> LegacyPlan<'a> {
        let mut lane_index = HashMap::new();
        for (i, lane) in doc.lanes.iter().enumerate() {
            lane_index.insert(lane.id.clone(), i);
        }
        let fit = fit_lanes(doc, padded);
        let mut lane_tops = Vec::with_capacity(doc.lanes.len());
        let mut y = LANE_Y;
        for h in &fit.heights {
            lane_tops.push(y);
            y += h + LANE_GAP;
        }
        let growth: f64 = fit.heights.iter().map(|h| h - LANE_H).sum();
        let lane_count = if doc.lanes.is_empty() { 1.0 } else { doc.lanes.len() as f64 };
        let auto_height = LANE_Y + lane_count * LANE_H + (lane_count - 1.0) * LANE_GAP + growth + BOTTOM_PAD;
        let view_box = doc.meta.view_box.map_or([DEFAULT_VIEW_BOX_WIDTH, auto_height], |[w, h]| [w, h + growth]);
        let group_rects = doc
            .groups
            .iter()
            .flatten()
            .zip(&fit.groups)
            .map(|(g, &(x, width, bottom))| {
                let top = lane_index.get(&g.lane).map_or(f64::NAN, |&i| lane_tops[i]);
                Rect::new(x, top + GROUP_TOP, width, bottom - GROUP_TOP)
            })
            .collect();
        let mut plan = LegacyPlan {
            doc,
            view_box,
            auto_height,
            lane_tops,
            lane_heights: fit.heights,
            lane_dy: fit.dy,
            group_rects,
            nodes: Vec::new(),
            routes: Vec::new(),
            labels: Vec::new(),
            entries: Vec::new(),
            lane_index,
            by_id: HashMap::new(),
        };
        for (index, node) in doc.nodes.iter().enumerate() {
            let measured = plan.measure_node(index, node);
            match plan.by_id.get(&node.id) {
                Some(&at) => plan.nodes[at] = measured,
                None => {
                    plan.by_id.insert(node.id.clone(), plan.nodes.len());
                    plan.nodes.push(measured);
                }
            }
        }
        plan.route_all();
        plan.resolve_legend();
        plan
    }

    /// `laneTop(id)`: `NaN` for a lane that does not exist.
    pub fn lane_top(&self, lane: &str) -> f64 {
        self.lane_index.get(lane).map_or(f64::NAN, |&i| self.lane_tops[i])
    }

    /// A lane's height: `LANE_H` unless padding grew it; `NaN` for a lane that does not exist.
    pub fn lane_height(&self, lane: &str) -> f64 {
        self.lane_index.get(lane).map_or(f64::NAN, |&i| self.lane_heights[i])
    }

    /// `lastLaneBottom()`.
    pub fn last_lane_bottom(&self) -> f64 {
        match (self.lane_tops.last(), self.lane_heights.last()) {
            (Some(top), Some(h)) => top + h,
            _ => LANE_Y - LANE_GAP,
        }
    }

    /// The frame of the `index`-th lane and its inner `exception-lane` frame (found by id, as
    /// `laneTop` does: a duplicated id draws at its last position).
    fn lane_rects(&self, index: usize) -> (Rect, Option<Rect>) {
        let lane = &self.doc.lanes[index];
        let (y, h) = (self.lane_top(&lane.id), self.lane_height(&lane.id));
        let exception = (lane.variant == Some(LaneVariant::Exception))
            .then(|| Rect::new(LANE_X + EXCEPTION_INSET, y + EXCEPTION_INSET, LANE_W - 2.0 * EXCEPTION_INSET, h - 2.0 * EXCEPTION_INSET));
        (Rect::new(LANE_X, y, LANE_W, h), exception)
    }

    /// `legendY()`: the legend baseline (no extra rows in v1).
    pub fn legend_y(&self) -> f64 {
        self.last_lane_bottom() + LEGEND_GAP
    }

    pub fn node(&self, id: &str) -> Option<&NodeBox> {
        self.by_id.get(id).map(|&i| &self.nodes[i])
    }

    fn measure_node(&self, index: usize, node: &Node) -> NodeBox {
        let (width, height, rel_y) = node_size(node);
        let cx = col_x(node.col);
        let dy = self.lane_index.get(&node.lane).map_or(0.0, |&i| self.lane_dy[i]);
        let y = self.lane_top(&node.lane) + rel_y + dy;
        NodeBox {
            index,
            id: node.id.clone(),
            lane: node.lane.clone(),
            col: node.col,
            rect: Rect::new(cx - width / 2.0, y, width, height),
            cx,
            cy: y + height / 2.0,
        }
    }

    /// `gapYBetween(fromLane, toLane, bias)`.
    fn gap_y(&self, from_lane: &str, to_lane: &str, bias: f64) -> f64 {
        let a = self.lane_top(from_lane) + self.lane_height(from_lane);
        let b = self.lane_top(to_lane);
        a + (b - a) * bias
    }

    /// `routeClearsUnrelatedNodes(edge, points)`: no segment comes within `2` of a node that is not an
    /// endpoint.
    fn clears_unrelated_nodes(&self, edge: &Edge, points: &[Pt]) -> bool {
        self.nodes.iter().filter(|n| n.id != edge.from && n.id != edge.to).all(|n| {
            points.windows(2).all(|w| !segment_intersects_rect(&Seg::new(w[0], w[1]), &n.rect, ONE_BEND_CLEARANCE))
        })
    }

    /// `oneBendCrossLaneVia`: the corner of an L route that leaves and enters perpendicularly, both legs
    /// at least `8` long, clear of every unrelated node.
    fn one_bend_via(&self, edge: &Edge, start: Pt, end: Pt, from_side: Side, to_side: Side) -> Option<Pt> {
        if from_side.is_vertical_edge() == to_side.is_vertical_edge() {
            return None;
        }
        // `isVerticalEdge` is true for left/right (the side is a vertical edge of the box): a side that
        // leaves vertically is top/bottom.
        let from_vertical = !from_side.is_vertical_edge();
        let corner = if from_vertical { [start[0], end[1]] } else { [end[0], start[1]] };
        let points = normalize(&[start, corner, end]);
        if points.len() != 3 || !crate::gates::clean_flow::route_honors_endpoint_sides(&points, from_side, to_side) {
            return None;
        }
        let readable = points.windows(2).all(|w| (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]) >= ONE_BEND_MIN_LEG);
        (readable && self.clears_unrelated_nodes(edge, &points)).then(|| points[1])
    }

    /// `legacyAutomaticOneBendSides`: for an automatic route between lanes, the first of the two L
    /// shapes whose corner is free.
    fn one_bend_sides(&self, edge: &Edge, from: &NodeBox, to: &NodeBox) -> Option<(Side, Side)> {
        let automatic_sides = edge.from_side.is_none() && edge.to_side.is_none();
        if !automatic_route(edge) || !automatic_sides || from.lane == to.lane {
            return None;
        }
        if from.cx == to.cx || from.cy == to.cy {
            return None;
        }
        let up = to.cy < from.cy;
        let left = to.cx < from.cx;
        let vertical_from = if up { Side::Top } else { Side::Bottom };
        let horizontal_to = if left { Side::Right } else { Side::Left };
        let horizontal_from = if left { Side::Left } else { Side::Right };
        let vertical_to = if up { Side::Bottom } else { Side::Top };
        [(vertical_from, horizontal_to), (horizontal_from, vertical_to)]
            .into_iter()
            .find(|&(f, t)| self.one_bend_via(edge, anchor(from, f), anchor(to, t), f, t).is_some())
    }

    /// `edgeSides`.
    fn edge_sides(&self, edge: &Edge, from: &NodeBox, to: &NodeBox) -> (Side, Side) {
        if let Some(sides) = self.one_bend_sides(edge, from, to) {
            return sides;
        }
        (
            edge.from_side.map_or_else(|| default_from_side(from, to), model_side),
            edge.to_side.map_or_else(|| default_to_side(from, to), model_side),
        )
    }

    /// An authored `y` (`via`, `channelY`, `labelAt`) is in Archify's unpadded space: the same place
    /// relative to its lane (or the gap it sits in), after padding moved the lanes and their nodes.
    /// The identity when nothing grew.
    fn remap_y(&self, y: f64) -> f64 {
        for (i, (&top, &h)) in self.lane_tops.iter().zip(&self.lane_heights).enumerate() {
            let archify_top = LANE_Y + i as f64 * (LANE_H + LANE_GAP);
            if y < archify_top {
                return y + top - archify_top;
            }
            let d = y - archify_top;
            if d <= LANE_H {
                return top + if d >= LANE_TITLE_H { (d + self.lane_dy[i]).min(h) } else { d };
            }
        }
        y + self.last_lane_bottom() - (LANE_Y + self.lane_tops.len() as f64 * (LANE_H + LANE_GAP) - LANE_GAP)
    }

    /// `routeVia` (v1): an authored `via` verbatim, a preset, or the automatic route.
    fn route_via(&self, edge: &Edge, from: &NodeBox, to: &NodeBox, start: Pt, end: Pt, sides: (Side, Side)) -> Vec<Pt> {
        if let Some(via) = &edge.via {
            return via.iter().map(|p| [p[0], self.remap_y(p[1])]).collect();
        }
        let channel_y = edge.channel_y.map(|y| self.remap_y(y));
        match edge.route.unwrap_or(EdgeRoute::Auto) {
            EdgeRoute::Straight => Vec::new(),
            EdgeRoute::Drop => {
                let y = channel_y.unwrap_or_else(|| self.gap_y(&from.lane, &to.lane, edge.bias.unwrap_or(0.5)));
                vec![[start[0], y], [end[0], y]]
            }
            EdgeRoute::OutsideRight => {
                let x = edge.channel_x.unwrap_or(LANE_X + LANE_W + 12.0);
                vec![[x, start[1]], [x, end[1]]]
            }
            EdgeRoute::ReturnLeft => {
                let x = edge.channel_x.unwrap_or(from.rect.x.min(to.rect.x) - 28.0);
                vec![[x, start[1]], [x, end[1]]]
            }
            EdgeRoute::BottomChannel => {
                let y = channel_y.unwrap_or((from.rect.y + from.rect.height).max(to.rect.y + to.rect.height) + 32.0);
                vec![[start[0], y], [end[0], y]]
            }
            EdgeRoute::UpChannel => {
                let y = channel_y.unwrap_or(from.rect.y.min(to.rect.y) - 28.0);
                vec![[start[0], y], [end[0], y]]
            }
            EdgeRoute::Auto => {
                if from.lane == to.lane {
                    return same_lane_via(start, end);
                }
                if let Some(corner) = self.one_bend_via(edge, start, end, sides.0, sides.1) {
                    return vec![corner];
                }
                let y = self.gap_y(&from.lane, &to.lane, edge.bias.unwrap_or(0.5));
                vec![[start[0], y], [end[0], y]]
            }
        }
    }

    /// `automaticPortSpread` over every edge, then `pathFor` and `labelRectFor` for each.
    fn route_all(&mut self) {
        let doc = self.doc;
        let sides: Vec<Option<(Side, Side)>> = doc
            .edges
            .iter()
            .map(|e| Some(self.edge_sides(e, self.node(&e.from)?, self.node(&e.to)?)))
            .collect();
        let relations: Vec<SpreadRelation> = doc
            .edges
            .iter()
            .map(|e| SpreadRelation {
                id: e.id.clone().unwrap_or_default(),
                from: e.from.clone(),
                to: e.to.clone(),
                label: e.label.clone().unwrap_or_default(),
                from_side: e.from_side.map(model_side),
                to_side: e.to_side.map(model_side),
                auto: spread_eligible(e),
            })
            .collect();
        let boxes: HashMap<String, Rect> = self.nodes.iter().map(|n| (n.id.clone(), n.rect)).collect();
        let side_for = |i: usize, end: End| sides[i].map(|(f, t)| if end == End::From { f } else { t });
        let ports = automatic_port_spread(&relations, &boxes, SpreadOptions::default(), Some(&side_for), None);

        let mut routes = Vec::with_capacity(doc.edges.len());
        let mut labels = Vec::with_capacity(doc.edges.len());
        for (i, edge) in doc.edges.iter().enumerate() {
            let (Some(from), Some(to), Some((from_side, to_side))) = (self.node(&edge.from), self.node(&edge.to), sides[i]) else {
                routes.push(None);
                labels.push(None);
                continue;
            };
            let start = ports[i].from.unwrap_or_else(|| anchor(from, from_side));
            let end = ports[i].to.unwrap_or_else(|| anchor(to, to_side));
            let via = self.route_via(edge, from, to, start, end, (from_side, to_side));
            let points: Vec<Pt> = std::iter::once(start).chain(via).chain(std::iter::once(end)).collect();
            let label_at = edge.label_at.map(|p| [p[0], self.remap_y(p[1])]);
            labels.push(truthy(&edge.label).map(|text| label_plate(edge, label_at, text, &points)));
            routes.push(Some(RoutedEdge { points, from_side, to_side }));
        }
        self.routes = routes;
        self.labels = labels;
    }

    /// `resolveLegend` over the node kinds present (every `workflow.nodes` entry, not the Map).
    fn resolve_legend(&mut self) {
        let meta = &self.doc.meta;
        let present: HashSet<&str> = self.doc.nodes.iter().map(|n| kind_of(n.kind).as_str()).collect();
        self.entries = legend::resolve(legend_config(meta).as_ref(), &legend::workflow_catalog(&meta.locale()), &present);
    }

    /// The `legend::measure` band (`workflowLegendLayout`).
    pub fn legend_layout(&self) -> LegendLayout {
        LegendLayout {
            x: LEGEND_X,
            baseline_y: self.legend_y(),
            width: self.view_box[0] - 2.0 * LEGEND_SIDE_PAD,
            min_title_y: self.last_lane_bottom() + LEGEND_TOP_PAD,
            unfit: if self.doc.meta.legend.is_none() { Unfit::Hide } else { Unfit::Error },
            diagram_type: "workflow",
        }
    }

    /// `phaseSpan` (v1: pad 46, no minimum).
    pub fn phase_span(&self, phase: &Phase) -> Span {
        span_for_cols(phase.from_col, phase.to_col, PHASE_PAD)
    }

    /// `groupSpan` (v1: pad 50, no minimum): Archify's span, before padding widens it.
    pub fn group_span(&self, group: &WfGroup) -> Span {
        span_for_cols(group.from_col, group.to_col, GROUP_PAD)
    }

    /// The frame rect of the `index`-th group (Archify: `laneTop + titleH + 8`, `58` high, the column
    /// span; padded, grown to its nodes).
    pub fn group_rect(&self, index: usize) -> Rect {
        self.group_rects[index]
    }

    /// `workflowCompositionFrames`: the lanes (and their exception frames), then the groups.
    pub fn frames(&self) -> Vec<FrameDef> {
        let mut frames = Vec::new();
        for (index, lane) in self.doc.lanes.iter().enumerate() {
            let (rect, exception) = self.lane_rects(index);
            frames.push(FrameDef { id: format!("lane-{index}"), label: lane.label.clone(), kind: "lane", rect, radius: LANE_RADIUS });
            if let Some(rect) = exception {
                frames.push(FrameDef {
                    id: format!("lane-{index}-exception"),
                    label: format!("{} exception", lane.label),
                    kind: "exception-lane",
                    rect,
                    radius: EXCEPTION_RADIUS,
                });
            }
        }
        for (index, group) in self.doc.groups.iter().flatten().enumerate() {
            frames.push(FrameDef {
                id: format!("group-{index}"),
                label: group.label.clone(),
                kind: "group",
                rect: self.group_rect(index),
                radius: GROUP_RADIUS,
            });
        }
        frames
    }

    /// `nodeContext(node)`: lane, then the first group and the first phase that hold the column.
    pub fn node_context(&self, node: &NodeBox) -> String {
        let doc = self.doc;
        let group = doc
            .groups
            .iter()
            .flatten()
            .find(|g| g.lane == node.lane && node.col >= g.from_col && node.col <= g.to_col);
        let phase = doc.phases.iter().flatten().find(|p| node.col >= p.from_col && node.col <= p.to_col);
        let lane = doc.lanes.iter().find(|l| l.id == node.lane).map(|l| l.label.as_str());
        let parts: Vec<&str> = [lane, group.map(|g| g.label.as_str()), phase.map(|p| p.label.as_str())]
            .into_iter()
            .flatten()
            .filter(|s| !s.is_empty())
            .collect();
        if parts.is_empty() { doc.meta.locale().t("node.context.workflow") } else { parts.join(" \u{203a} ") }
    }
}

/// `workflowEdgeLabelPoint` (v1): the default anchor of `labelPoint`, except that a 3-point route puts
/// the label on its longer leg (the first on a tie), `10` lower on a vertical one.
/// `at` is `edge.labelAt` as the plan places it ([`LegacyPlan`] remaps its `y`).
pub fn label_point(edge: &Edge, at: Option<Pt>, points: &[Pt]) -> Pt {
    let mut hints = Hints { at, dx: edge.label_dx, dy: edge.label_dy, segment: edge.label_segment };
    let integer_segment = edge.label_segment.is_some_and(|s| s.is_finite() && s.fract() == 0.0);
    if edge.label_at.is_some() || integer_segment || points.len() != 3 {
        return labels::label_point(&hints, points);
    }
    let length = |i: usize| (points[i + 1][0] - points[i][0]).hypot(points[i + 1][1] - points[i][1]);
    let segment = usize::from(length(0) < length(1));
    hints.segment = Some(segment as f64);
    let mut at = labels::label_point(&hints, points);
    if points[segment][0] == points[segment + 1][0] {
        at[1] += 10.0;
    }
    at
}

/// `workflowLabelWidth(label)`: `max(30, units * 4.8 + 10)`.
pub fn label_width(label: &str) -> f64 {
    text::edge_label_width(DiagramType::Workflow, &[label], text::Profile::Standard)
}

fn label_plate(edge: &Edge, label_at: Option<Pt>, text: &str, points: &[Pt]) -> LabelPlate {
    let at = label_point(edge, label_at, points);
    let width = label_width(text);
    LabelPlate { text: text.to_owned(), rect: Rect::new(at[0] - width / 2.0, at[1] - LABEL_ASCENT, width, LABEL_H), at }
}

/// One lane as drawn.
#[derive(Clone, Debug, PartialEq)]
pub struct LaneGeom {
    pub rect: Rect,
    /// The inner `exception-lane` frame.
    pub exception: Option<Rect>,
}

/// One edge as drawn (the receipt's `edges[]` and its label).
#[derive(Clone, Debug, PartialEq)]
pub struct EdgeGeom {
    /// The index in `edges` (`data-edge-key`).
    pub key: usize,
    pub id: Option<String>,
    pub from: String,
    pub to: String,
    pub points: Vec<Pt>,
    pub from_side: Side,
    pub to_side: Side,
    pub width: f64,
    pub label: Option<LabelPlate>,
}

/// Everything layout measured, un-rounded, in document order: the numbers of the `fixed-v1` receipt.
#[derive(Clone, Debug, PartialEq)]
pub struct LegacyGeometry {
    pub view_box: [f64; 2],
    /// `requiredViewBox`: the canvas itself in v1.
    pub required_view_box: [f64; 2],
    pub columns: [f64; 6],
    pub lanes: Vec<LaneGeom>,
    /// The `nodes` Map in order.
    pub nodes: Vec<NodeBox>,
    /// Edges whose endpoints both exist, in order.
    pub edges: Vec<EdgeGeom>,
    /// `None` when no legend is drawn (hidden, no entries, or an implicit one that does not fit).
    pub legend: Option<Measured>,
}

fn text_shape(at: Pt, text: &str, size: f64, weight: u16, anchor: Anchor, token: Token, detail: Detail) -> Shape {
    Shape::Text(TextShape { at, text: text.to_owned(), size, weight, anchor, token, detail })
}

fn icon_name(icon: NodeIcon) -> String {
    serde_json::to_value(icon).ok().and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_default()
}

/// `variantAccent`: the text token of a phase or group variant.
fn variant_accent(v: Option<Variant>) -> Token {
    match v {
        Some(Variant::Security) => Token::KindStroke(Kind::Security),
        Some(Variant::Emphasis) => Token::KindStroke(Kind::Backend),
        Some(Variant::Dashed) => Token::KindStroke(Kind::Messagebus),
        _ => Token::TextMuted,
    }
}

/// The frame style of `c-lane` (filled, dashed `6,6`) and `c-security-group` (open, dashed `4,4`).
fn frame_shape(rect: Rect, radius: f64, security: bool) -> Shape {
    let (fill, stroke) = if security {
        (None, Stroke::dashed(Token::KindStroke(Kind::Security), 1.0, &crate::layout::architecture::SECURITY_GROUP_DASH))
    } else {
        (Some(Fill::new(Token::LaneFill)), Stroke::dashed(Token::LaneStroke, 1.0, &[6.0, 6.0]))
    };
    Shape::Rect(RectShape { rect: rect.into(), radius, fill, stroke: Some(stroke) })
}

fn lane_prefix(index: usize, exception: bool) -> String {
    if exception { "EX".to_owned() } else { format!("{:02}", index + 1) }
}

fn push_lane(b: &mut SceneBuilder, plan: &LegacyPlan<'_>, index: usize) {
    let lane = &plan.doc.lanes[index];
    let exception = lane.variant == Some(LaneVariant::Exception);
    let (rect, inner) = plan.lane_rects(index);
    let y = rect.y;
    let mut group = Group::new(GroupKind::Frame, format!("frame-lane-{index}"), rect.into());
    group.label = lane.label.clone();
    let id = b.group(group);
    b.push_in(id, Layer::Frames, frame_shape(rect, LANE_RADIUS, false));
    if let Some(inner) = inner {
        b.push_in(id, Layer::Frames, frame_shape(inner, EXCEPTION_RADIUS, true));
    }
    let token = if exception { Token::KindStroke(Kind::Security) } else { Token::TextDim };
    let title = format!("{} / {}", lane_prefix(index, exception), lane.label);
    b.push_in(
        id,
        Layer::Frames,
        text_shape([LANE_X + LANE_TITLE_DX, y + LANE_TITLE_DY], &title, 10.0, 600, Anchor::Start, token, Detail::Anchor),
    );
}

fn push_phase(b: &mut SceneBuilder, plan: &LegacyPlan<'_>, phase: &Phase) {
    let span = plan.phase_span(phase);
    let variant = EdgeVariant::parse(Some(variant_name(phase.variant)));
    b.push(
        Layer::Frames,
        Shape::Polyline(PolylineShape {
            points: vec![[span.x, PHASE_LINE_Y], [span.x + span.width, PHASE_LINE_Y]],
            radius: 0.0,
            stroke: Stroke { token: variant.stroke(), width: 1.1, dash: variant.dash().to_vec() },
            marker: None,
            halo: false,
        }),
    );
    b.push(Layer::Frames, Shape::Rect(RectShape::mask(Bounds::new(span.x, PHASE_MASK_Y, span.width, PHASE_MASK_H), 4.0)));
    b.push(
        Layer::Frames,
        text_shape([span.cx, PHASE_TEXT_Y], &phase.label, 8.0, 600, Anchor::Middle, variant_accent(phase.variant), Detail::Anchor),
    );
}

fn push_group(b: &mut SceneBuilder, plan: &LegacyPlan<'_>, index: usize, group: &WfGroup) {
    let rect = plan.group_rect(index);
    let mut g = Group::new(GroupKind::Frame, format!("frame-group-{index}"), rect.into());
    g.label = group.label.clone();
    let id = b.group(g);
    b.push_in(id, Layer::Frames, frame_shape(rect, GROUP_RADIUS, group.variant == Some(Variant::Security)));
    b.push_in(
        id,
        Layer::Frames,
        text_shape(
            [rect.x + 10.0, rect.y + GROUP_LABEL_DOWN],
            &group.label,
            GROUP_LABEL_FONT,
            600,
            Anchor::Start,
            variant_accent(group.variant),
            Detail::Anchor,
        ),
    );
}

fn push_node(b: &mut SceneBuilder, plan: &LegacyPlan<'_>, n: &NodeBox) {
    let node = &plan.doc.nodes[n.index];
    let kind = kind_of(node.kind);
    let r = n.rect;
    let fonts = text::node_fonts(DiagramType::Workflow, 1);
    let brand = node.brand.is_some();
    let label_font = text::fit(&node.label, text::label_fit_width(r.width, brand), fonts.label.0, fonts.label.1);
    let sub = truthy(&node.sublabel);
    let tag = truthy(&node.tag);
    let (sub_pref, sub_min) = fonts.sublabel;
    let (tag_pref, tag_min) = fonts.tag.unwrap_or((7.0, 6.0));

    let mut rows = vec![Row { text: &node.label, font: label_font, y: LABEL_Y }];
    let sub_font = sub.map(|s| text::fit(s, r.width, sub_pref, sub_min));
    if let (Some(s), Some(font)) = (sub, sub_font) {
        rows.push(Row { text: s, font, y: SUBLABEL_Y });
    }
    let tag_font = tag.map(|t| text::fit(t, r.width, tag_pref, tag_min));
    if let (Some(t), Some(font)) = (tag, tag_font) {
        rows.push(Row { text: t, font, y: r.height - TAG_UP });
    }
    let layout = text::node_label_layout(&LabelBox { width: r.width, height: r.height, brand, ..LabelBox::default() }, &rows);

    let mut group = Group::node(&node.id, kind, &node.label, r.into());
    group.sublabel = sub.map(str::to_owned);
    group.context = Some(plan.node_context(n));
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
    let icon = node.icon.map(icon_name);
    for path in sigils::sigil_paths(kind.as_str(), icon.as_deref(), r.x + sigils::SIGIL_INSET, r.y + layout.sigil_y, layout.sigil_size) {
        b.push_in(id, Layer::Nodes, Shape::Path(path));
    }
    crate::brand::push_badge(b, id, node.brand.as_ref(), r.x + r.width, r.y);
    b.push_in(
        id,
        Layer::Nodes,
        text_shape([r.x + layout.x, r.y + layout.ys[0]], &node.label, label_font, 600, Anchor::Middle, Token::Text, Detail::Anchor),
    );
    let mut next = 1;
    if let (Some(s), Some(font)) = (sub, sub_font) {
        b.push_in(id, Layer::Nodes, text_shape([n.cx, r.y + layout.ys[next]], s, font, 400, Anchor::Middle, Token::TextMuted, Detail::Context));
        next += 1;
    }
    if let (Some(t), Some(font)) = (tag, tag_font) {
        b.push_in(
            id,
            Layer::Nodes,
            text_shape([n.cx, r.y + layout.ys[next]], t, font, 400, Anchor::Middle, Token::KindStroke(kind), Detail::Fine),
        );
    }
}

/// The swatch of a workflow legend row: a `14 x 9` kind rect (`class c-<kind>`, `c-external` if unknown).
fn legend_swatch(p: &Placed) -> Vec<Shape> {
    let kind = Kind::parse(p.entry.kind).unwrap_or(Kind::External);
    vec![Shape::Rect(RectShape {
        rect: Bounds::new(p.x, p.baseline - 8.0, 14.0, 9.0),
        radius: 2.0,
        fill: Some(Fill::new(Token::KindFill(kind))),
        stroke: Some(Stroke::solid(Token::KindStroke(kind), 1.0)),
    })]
}

/// The scene and geometry of a planned v1 document. `Err` is an authored `meta.legend` that does not
/// fit (`legend/label-too-wide`, `legend/vertical-overflow`), which Archify throws at render time,
/// after `validateWorkflow` passed.
pub fn build_plan(plan: &LegacyPlan<'_>) -> Result<(Scene, LegacyGeometry), Vec<Diagnostic>> {
    let doc = plan.doc;
    let view_box = plan.view_box;
    let legend = legend::measure_styled(&plan.entries, &plan.legend_layout(), &[], LEGEND_STYLE).map_err(|d| vec![*d])?;

    let mut b = SceneBuilder::new(view_box[0], view_box[1]);
    b.push(
        Layer::Background,
        Shape::Rect(RectShape { rect: Bounds::new(0.0, 0.0, view_box[0], view_box[1]), radius: 0.0, fill: Some(Fill::new(Token::Bg)), stroke: None }),
    );
    for index in 0..doc.lanes.len() {
        push_lane(&mut b, plan, index);
    }
    for phase in doc.phases.iter().flatten() {
        push_phase(&mut b, plan, phase);
    }
    for (index, group) in doc.groups.iter().flatten().enumerate() {
        push_group(&mut b, plan, index, group);
    }

    let mut edges = Vec::new();
    for (key, edge) in doc.edges.iter().enumerate() {
        let Some(route) = &plan.routes[key] else { continue };
        let variant = EdgeVariant::parse(Some(variant_name(edge.variant)));
        let width = edge_width(edge);
        let edge_ref = EdgeRef { key: key as u32, from: edge.from.clone(), to: edge.to.clone(), id: edge.id.clone() };
        let id = b.group(Group::edge(edge_ref, edge.label.as_deref().unwrap_or(""), &route.points, EDGE_HIT_HALF_WIDTH.max(width / 2.0)));
        b.push_in(
            id,
            Layer::Edges,
            Shape::Polyline(PolylineShape {
                points: route.points.clone(),
                radius: 0.0,
                stroke: Stroke { token: variant.stroke(), width, dash: variant.dash().to_vec() },
                marker: Some(Marker { fill: variant.stroke() }),
                halo: false,
            }),
        );
        edges.push(EdgeGeom {
            key,
            id: edge.id.clone(),
            from: edge.from.clone(),
            to: edge.to.clone(),
            points: route.points.clone(),
            from_side: route.from_side,
            to_side: route.to_side,
            width,
            label: plan.labels[key].clone(),
        });
    }
    for node in &plan.nodes {
        push_node(&mut b, plan, node);
    }
    for g in &edges {
        let Some(label) = &g.label else { continue };
        let edge = &doc.edges[g.key];
        let variant = EdgeVariant::parse(Some(variant_name(edge.variant)));
        let edge_ref = EdgeRef { key: g.key as u32, from: g.from.clone(), to: g.to.clone(), id: g.id.clone() };
        let mut group = Group::new(GroupKind::Label, format!("label-{}", g.key), label.rect.into());
        group.edge = Some(edge_ref);
        group.label = label.text.clone();
        let id = b.group(group);
        b.push_in(id, Layer::EdgeLabels, Shape::Rect(RectShape::mask(label.rect.into(), LABEL_RADIUS)));
        b.push_in(id, Layer::EdgeLabels, text_shape(label.at, &label.text, LABEL_FONT, 400, Anchor::Middle, variant.label(), Detail::Context));
    }
    if let Some(m) = &legend {
        legend::push_scene(&mut b, m, &doc.meta.locale(), &legend_swatch);
    }

    let lanes = (0..doc.lanes.len())
        .map(|index| {
            let (rect, exception) = plan.lane_rects(index);
            LaneGeom { rect, exception }
        })
        .collect();
    let geometry = LegacyGeometry {
        view_box,
        required_view_box: view_box,
        columns: COLUMNS,
        lanes,
        nodes: plan.nodes.clone(),
        edges,
        legend,
    };
    Ok((b.build(), geometry))
}

/// Lay out a v1 workflow: the scene to paint and the raw geometry. The rules of `validateWorkflow` are
/// [`crate::gates::workflow_v1::validate_v1`]; call it first when the caller wants Archify's order
/// (the legend error only surfaces once the validation passed).
pub fn build_v1(doc: &Workflow) -> Result<(Scene, LegacyGeometry), Vec<Diagnostic>> {
    debug_assert!(doc.schema_version == SchemaVersion::V1, "build_v1 lays out schema_version 1");
    build_plan(&LegacyPlan::new(doc))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn doc(value: serde_json::Value) -> Workflow {
        serde_json::from_value(value).expect("a valid workflow")
    }

    fn base() -> serde_json::Value {
        json!({
            "schema_version": 1, "diagram_type": "workflow",
            "meta": { "title": "t", "output": "t.html" },
            "lanes": [{ "id": "a", "label": "A" }, { "id": "b", "label": "B" }],
            "nodes": [
                { "id": "x", "lane": "a", "col": 0, "type": "frontend", "label": "X" },
                { "id": "y", "lane": "a", "col": 2, "type": "backend", "label": "Y" },
                { "id": "z", "lane": "b", "col": 3, "type": "database", "label": "Z", "tag": "t" }
            ],
            "edges": [
                { "from": "x", "to": "y", "label": "go" },
                { "from": "y", "to": "z" }
            ]
        })
    }

    #[test]
    fn the_default_canvas_lanes_and_nodes() {
        let d = doc(base());
        let plan = LegacyPlan::new(&d);
        // 52 + 2 * 104 + 20 + 124.
        assert_eq!(plan.view_box, [720.0, 404.0]);
        assert_eq!(plan.lane_top("b"), 176.0);
        assert_eq!(plan.node("x").unwrap().rect, Rect::new(42.0, 93.0, 92.0, 52.0));
        // A tagged node is 68 tall: y = 176 + 30 + (74 - 68) / 2.
        assert_eq!(plan.node("z").unwrap().rect, Rect::new(384.0, 209.0, 92.0, 68.0));
        assert_eq!(plan.legend_y(), 52.0 + 208.0 + 20.0 + 44.0 - 0.0);
    }

    #[test]
    fn a_same_lane_edge_is_straight_when_level_and_the_label_rides_above_it() {
        let d = doc(base());
        let plan = LegacyPlan::new(&d);
        assert_eq!(plan.routes[0].as_ref().unwrap().points, vec![[134.0, 119.0], [254.0, 119.0]]);
        let label = plan.labels[0].as_ref().unwrap();
        assert_eq!(label.at, [194.0, 109.0]);
        assert_eq!(label.rect, Rect::new(179.0, 99.0, 30.0, 14.0));
    }

    #[test]
    fn a_cross_lane_edge_takes_the_one_bend_route() {
        let d = doc(base());
        let plan = LegacyPlan::new(&d);
        let route = plan.routes[1].as_ref().unwrap();
        // y (col 2, lane a) -> z (col 3, lane b): leaves the bottom, enters the left side.
        assert_eq!((route.from_side, route.to_side), (Side::Bottom, Side::Left));
        assert_eq!(route.points, vec![[300.0, 145.0], [300.0, 243.0], [384.0, 243.0]]);
    }

    #[test]
    fn a_duplicated_node_id_collapses_to_the_last_value_at_the_first_position() {
        let mut v = base();
        v["nodes"].as_array_mut().unwrap().push(json!({ "id": "x", "lane": "b", "col": 5, "type": "cloud", "label": "X2" }));
        let d = doc(v);
        let plan = LegacyPlan::new(&d);
        assert_eq!(plan.nodes.len(), 3);
        assert_eq!(plan.nodes[0].lane, "b");
        assert_eq!(plan.nodes[0].index, 3);
    }

    #[test]
    fn a_node_on_an_unknown_lane_measures_nan_and_the_edge_still_routes_without_panicking() {
        let mut v = base();
        v["nodes"][0]["lane"] = json!("nope");
        let d = doc(v);
        let plan = LegacyPlan::new(&d);
        assert!(plan.nodes[0].rect.y.is_nan());
        assert!(plan.routes[0].is_some());
    }

    #[test]
    fn the_scene_is_z_ordered_with_a_group_per_part() {
        let d = doc(base());
        let (scene, g) = build_v1(&d).unwrap();
        let layers: Vec<Layer> = scene.items.iter().map(|i| i.layer).collect();
        assert!(layers.windows(2).all(|w| w[0] <= w[1]));
        assert_eq!(g.edges.len(), 2);
        assert_eq!(scene.edges_of("y").len(), 2);
        assert_eq!(scene.group(scene.node("x").unwrap()).unwrap().tooltip(), "X \u{b7} A");
    }

    #[test]
    fn padding_keeps_nodes_inside_their_groups_and_groups_inside_the_exception_frame() {
        let mut v = base();
        v["lanes"][1]["variant"] = json!("exception");
        v["meta"]["viewBox"] = json!([720, 404]);
        v["groups"] = json!([
            { "id": "g", "label": "G", "lane": "a", "fromCol": 0, "toCol": 2 },
            { "id": "h", "label": "H", "lane": "b", "fromCol": 3, "toCol": 5 }
        ]);
        let d = doc(v);
        assert!(padding_changes(&d));
        let plan = LegacyPlan::new(&d);
        let frames = plan.frames();
        let rect = |id: &str| frames.iter().find(|f| f.id == id).unwrap().rect;
        let inside = |outer: Rect, inner: Rect, pad: f64| {
            inner.x - outer.x >= pad
                && outer.x + outer.width - inner.x - inner.width >= pad
                && inner.y - outer.y >= pad
                && outer.y + outer.height - inner.y - inner.height >= pad
        };
        for (node, group) in [("x", "group-0"), ("y", "group-0"), ("z", "group-1")] {
            let (n, g) = (plan.node(node).unwrap().rect, rect(group));
            assert!(inside(g, n, FRAME_PAD), "{node} in {group}: {n:?} {g:?}");
            assert!(n.y - g.y >= GROUP_LABEL_BAND, "{node} clears the label of {group}");
        }
        assert!(inside(rect("lane-1-exception"), rect("group-1"), FRAME_PAD));
        // The lanes below a grown lane follow it, and the authored canvas grows by the same amount.
        let grown = plan.lane_height("a") - LANE_H + plan.lane_height("b") - LANE_H;
        assert!(grown > 0.0);
        assert_eq!(plan.lane_top("b"), LANE_Y + plan.lane_height("a") + LANE_GAP);
        assert_eq!(plan.view_box, [720.0, 404.0 + grown]);
        // Unpadded, it is Archify's geometry.
        let archify = LegacyPlan::archify(&d);
        assert_eq!(archify.lane_top("b"), 176.0);
        assert_eq!(archify.group_rect(0), Rect::new(38.0, 90.0, 312.0, GROUP_FRAME_HEIGHT));
        assert!(!padding_changes(&doc(base())));
    }

    #[test]
    fn an_authored_legend_that_does_not_fit_is_an_error_and_an_implicit_one_is_dropped() {
        let mut v = base();
        v["meta"]["viewBox"] = json!([100, 404]);
        let (_, g) = build_v1(&doc(v.clone())).unwrap();
        assert!(g.legend.is_none());
        v["meta"]["legend"] = json!({ "mode": "all" });
        assert!(build_v1(&doc(v)).is_err());
    }
}
