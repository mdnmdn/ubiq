//! The architecture router (P3.3, P3.4): port of `renderers/architecture/routing.mjs`
//! (`createRouter`) with `automaticPortRhythmBridge` of `renderers/shared/geometry.mjs:1497`.
//! `00` §5.6.3, `03` §3.2.
//!
//! [`plan_routes`] is `planRoutes` plus `pathFor`/`connectionSides` for every routable relation:
//!
//! 1. explicit routes (authored `via`/`route`/`channelX`/`channelY`) in document order, then the
//!    automatic ones by centre Manhattan distance (ties by index);
//! 2. per automatic relation, side pairs ordered by deviation from the inferred sides then by anchor
//!    distance; per pair the cheap candidates (direct, rhythm bridge, outside channels, the two
//!    doglegs and the side-aware bridges, the parallel channels) and then the bounded grid search;
//!    the first pair whose route `routeIsClear` wins, else the first pair's route;
//! 3. the default label rect of each routed relation is reserved for the later ones; a route that
//!    honours them but runs past `1.25 * direct + 64` is retried without them;
//! 4. under `preferReadableRoutes`, the reciprocal-pair repair and the one-route readability sweep
//!    (`length + 48 * bends + 160 * crossings`, within the scene bounds, the grid search off).
//!
//! Relations are identified by their index in the slice (`conn` identity in the JS). Boxes are the
//! measured components in `Map` order, ids unique. Callers build the inputs from their own model;
//! architecture's are in `layout::architecture_build`.

use std::cell::Cell;
use std::collections::HashMap;

use crate::gates::clean_flow::route_honors_endpoint_sides;
use crate::geom::{
    End, Frame, Pt, Rect, Seg, Side, SpreadOptions, SpreadPorts, SpreadRelation, anchor, automatic_port_spread,
    chosen_side, collinear_axis_overlap, default_from_side, default_to_side, frame_border_segments, is_finite_point,
    normalize, proper_segment_intersection, rects_overlap, segment_intersects_rect,
};
use crate::route::grid::{GridQuery, shortest_orthogonal_grid_route};

const EPS: f64 = 0.0001;
/// `LABEL_CLEARANCE` (`routing.mjs:48`).
pub const LABEL_CLEARANCE: f64 = 4.0;
/// `maximumGridSearchCount` (`:101`).
pub const MAX_GRID_SEARCHES: usize = 64;
/// `SIDE_ORDER` (`:808`).
const SIDE_ORDER: [Side; 4] = [Side::Right, Side::Bottom, Side::Left, Side::Top];
/// `AUTOMATIC_PORT_CORNER_GUTTER`, `AUTOMATIC_PORT_ALIGNMENT_DELTA` (`:311-312`).
const CORNER_GUTTER: f64 = 16.0;
const ALIGNMENT_DELTA: f64 = 16.0;
/// The grid search's own component clearance (`clearance: 2`, `:564`).
const GRID_CLEARANCE: f64 = 2.0;
/// `endpointStubPx` of the rhythm bridge and the side-aware stubs.
const STUB_PX: f64 = 24.0;
/// `readabilityCost` weights (`:861`).
pub const BEND_COST: f64 = 48.0;
pub const CROSSING_COST: f64 = 160.0;

/// A measured box the router avoids and anchors to (`components.values()`).
#[derive(Clone, Debug, PartialEq)]
pub struct RouterBox {
    pub id: String,
    pub rect: Rect,
}

/// `conn.route`: absent is `Auto`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RouteKind {
    #[default]
    Auto,
    Straight,
    OrthogonalH,
    OrthogonalV,
}

/// A relationship as the router reads it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RouterRel {
    pub id: Option<String>,
    pub from: String,
    pub to: String,
    /// `relation.label || ''` for the port-spread tie-break; `None` or empty is unlabelled.
    pub label: Option<String>,
    pub from_side: Option<Side>,
    pub to_side: Option<Side>,
    pub route: RouteKind,
    pub via: Option<Vec<Pt>>,
    pub channel_x: Option<f64>,
    pub channel_y: Option<f64>,
    pub label_at: Option<Pt>,
    /// Any of `labelAt`, `labelDx`, `labelDy`, `labelSegment` authored (`hasAuthoredLabelPlacement`).
    pub label_placement: bool,
    /// `relation.width || (variant === 'emphasis' ? 1.8 : 1.5)`.
    pub stroke: f64,
}

impl RouterRel {
    /// `hasAuthoredRouteGeometry`.
    pub fn authored_geometry(&self) -> bool {
        self.via.is_some() || self.route != RouteKind::Auto || self.channel_x.is_some() || self.channel_y.is_some()
    }

    fn has_label(&self) -> bool {
        self.label.as_deref().is_some_and(|l| !l.is_empty())
    }
}

/// `createRouter` options.
#[derive(Clone, Debug, PartialEq)]
pub struct RouterOptions {
    /// Structural frames an automatic route may cross but never follow.
    pub frames: Vec<Frame>,
    pub interior_segment_px: f64,
    pub micro_segment_px: f64,
    pub distinct_automatic_ports: bool,
    pub prefer_readable_routes: bool,
}

impl Default for RouterOptions {
    fn default() -> Self {
        Self {
            frames: Vec::new(),
            interior_segment_px: 16.0,
            micro_segment_px: 8.0,
            distinct_automatic_ports: false,
            prefer_readable_routes: false,
        }
    }
}

impl RouterOptions {
    /// What `render-architecture.mjs:314` passes: distinct ports, readable routes, the raw frames.
    pub fn architecture(frames: Vec<Frame>) -> Self {
        Self { frames, distinct_automatic_ports: true, prefer_readable_routes: true, ..Self::default() }
    }
}

/// `labelRectFor(conn, points, { routes, labels })`: the default label rect of relation `rel` on
/// `points`, given every route and reserved label rect so far; `None` for no label (or one the
/// planner does not reserve).
pub type LabelRectFor<'a> = dyn Fn(usize, &[Pt], &[&[Pt]], &[Rect]) -> Option<Rect> + 'a;

/// A planned route and the sides it was planned on (`pathFor(conn).points`, `connectionSides(conn)`).
#[derive(Clone, Debug, PartialEq)]
pub struct Routed {
    pub points: Vec<Pt>,
    pub sides: Sides,
}

/// A side pair.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sides {
    pub from: Side,
    pub to: Side,
}

/// The planner's counters (`routingMetrics()`, the scalar ones).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PlanningMetrics {
    pub route_count: usize,
    pub explicit_route_count: usize,
    pub automatic_route_count: usize,
    pub grid_search_count: usize,
    pub grid_routed_count: usize,
    pub conflict_fallback_count: usize,
    pub grid_budget_exhausted_count: usize,
    pub crossover_routed_count: usize,
    pub readability_candidate_count: usize,
    pub readability_improved_count: usize,
    pub reciprocal_candidate_count: usize,
    pub reciprocal_improved_count: usize,
}

/// The plan: one entry per relation (`None` when an endpoint is not a box), and the counters.
#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    pub routes: Vec<Option<Routed>>,
    pub metrics: PlanningMetrics,
}

/// A route already resolved: the relation index and its points.
type Entry<'r> = (usize, &'r [Pt]);

fn opposite(side: Side) -> Side {
    match side {
        Side::Left => Side::Right,
        Side::Right => Side::Left,
        Side::Top => Side::Bottom,
        Side::Bottom => Side::Top,
    }
}

fn is_vertical(side: Side) -> bool {
    matches!(side, Side::Top | Side::Bottom)
}

fn outward(side: Side) -> Pt {
    match side {
        Side::Left => [-1.0, 0.0],
        Side::Right => [1.0, 0.0],
        Side::Top => [0.0, -1.0],
        Side::Bottom => [0.0, 1.0],
    }
}

fn segs(points: &[Pt]) -> impl Iterator<Item = Seg> + '_ {
    points.windows(2).map(|w| Seg::new(w[0], w[1]))
}

/// `routeLength`: the Manhattan length, summed left to right as the JS `reduce` does.
pub fn manhattan_length(points: &[Pt]) -> f64 {
    let mut total = 0.0;
    for w in points.windows(2) {
        total = total + (w[1][0] - w[0][0]).abs() + (w[1][1] - w[0][1]).abs();
    }
    total
}

fn endpoint_distance(start: Pt, end: Pt) -> f64 {
    (end[0] - start[0]).abs() + (end[1] - start[1]).abs()
}

/// The local `collinearOverlapLength` of `routing.mjs:142` (not the gates' `collinearAxisOverlap`).
fn collinear_overlap_length(a: Pt, b: Pt, c: Pt, d: Pt) -> f64 {
    if (a[0] - b[0]).abs() <= EPS && (c[0] - d[0]).abs() <= EPS && (a[0] - c[0]).abs() <= EPS {
        return 0f64.max(a[1].max(b[1]).min(c[1].max(d[1])) - a[1].min(b[1]).max(c[1].min(d[1])));
    }
    if (a[1] - b[1]).abs() <= EPS && (c[1] - d[1]).abs() <= EPS && (a[1] - c[1]).abs() <= EPS {
        return 0f64.max(a[0].max(b[0]).min(c[0].max(d[0])) - a[0].min(b[0]).max(c[0].min(d[0])));
    }
    0.0
}

/// `orthogonalTouchOnResolvedInterior` (`:161`).
fn orthogonal_touch_on_resolved_interior(start: Pt, end: Pt, rs: Pt, re: Pt) -> bool {
    let candidate_horizontal = (start[1] - end[1]).abs() <= EPS;
    let candidate_vertical = (start[0] - end[0]).abs() <= EPS;
    let resolved_horizontal = (rs[1] - re[1]).abs() <= EPS;
    let resolved_vertical = (rs[0] - re[0]).abs() <= EPS;
    if candidate_horizontal && resolved_vertical {
        let (x, y) = (rs[0], start[1]);
        return x >= start[0].min(end[0]) - EPS
            && x <= start[0].max(end[0]) + EPS
            && y > rs[1].min(re[1]) + EPS
            && y < rs[1].max(re[1]) - EPS;
    }
    if candidate_vertical && resolved_horizontal {
        let (x, y) = (start[0], rs[1]);
        return y >= start[1].min(end[1]) - EPS
            && y <= start[1].max(end[1]) + EPS
            && x > rs[0].min(re[0]) + EPS
            && x < rs[0].max(re[0]) - EPS;
    }
    false
}

/// `collinearBacktrack(a, b, c)` (`:251`).
fn collinear_backtrack(a: Pt, b: Pt, c: Pt) -> bool {
    let first = [b[0] - a[0], b[1] - a[1]];
    let second = [c[0] - b[0], c[1] - b[1]];
    let cross = first[0] * second[1] - first[1] * second[0];
    let dot = first[0] * second[0] + first[1] * second[1];
    cross.abs() <= EPS && dot < -EPS
}

/// `portHasCornerClearance` (`:314`).
fn port_has_corner_clearance(rect: &Rect, side: Side, point: Pt) -> bool {
    if is_vertical(side) {
        let inset = CORNER_GUTTER.min(rect.width / 2.0);
        point[0] >= rect.x + inset && point[0] <= rect.x + rect.width - inset
    } else {
        let inset = CORNER_GUTTER.min(rect.height / 2.0);
        point[1] >= rect.y + inset && point[1] <= rect.y + rect.height - inset
    }
}

/// `collectRouteRhythmIssues(...).length === 0` for one route.
fn rhythm_ok(points: &[Pt], interior_px: f64, micro_px: f64) -> bool {
    let points = normalize(points);
    if points.len() < 2 {
        return true;
    }
    let count = points.len() - 1;
    for (index, w) in points.windows(2).enumerate() {
        let length = (w[1][0] - w[0][0]).abs() + (w[1][1] - w[0][1]).abs();
        if length <= EPS {
            continue;
        }
        if length < micro_px - EPS {
            return false;
        }
        let interior = index != 0 && index != count - 1;
        if interior && length < interior_px - EPS {
            return false;
        }
    }
    true
}

/// `automaticPortRhythmBridge(start, end, fromSide, toSide, { accept })` with the router's defaults
/// (`endpointStubPx` 24, `interiorSegmentPx` 16): a full outside-channel route when parallel ports
/// sit closer than the interior floor, or `None`.
pub fn automatic_port_rhythm_bridge(
    start: Pt,
    end: Pt,
    from_side: Side,
    to_side: Side,
    accept: &mut dyn FnMut(&[Pt]) -> bool,
) -> Option<Vec<Pt>> {
    const INTERIOR: f64 = 16.0;
    if !is_finite_point(&[start[0], start[1], end[0], end[1]]) {
        return None;
    }
    let (fv, tv) = (outward(from_side), outward(to_side));
    let opposed_facing_gap = if (from_side == Side::Right && to_side == Side::Left && end[0] > start[0])
        || (from_side == Side::Left && to_side == Side::Right && start[0] > end[0])
    {
        Some((end[0] - start[0]).abs())
    } else if (from_side == Side::Bottom && to_side == Side::Top && end[1] > start[1])
        || (from_side == Side::Top && to_side == Side::Bottom && start[1] > end[1])
    {
        Some((end[1] - start[1]).abs())
    } else {
        None
    };
    let bounded = opposed_facing_gap
        .filter(|gap| gap - STUB_PX * 2.0 < INTERIOR)
        .map(|gap| 8f64.max(STUB_PX.min((gap - INTERIOR) / 2.0)));
    let mut stubs = vec![STUB_PX];
    if let Some(b) = bounded
        && b >= 8.0
        && b != STUB_PX
    {
        stubs.push(b);
    }
    let mut candidates: Vec<[Pt; 6]> = Vec::new();
    for stub in stubs {
        let ss = [start[0] + fv[0] * stub, start[1] + fv[1] * stub];
        let es = [end[0] + tv[0] * stub, end[1] + tv[1] * stub];
        if is_vertical(from_side) && is_vertical(to_side) && (start[0] - end[0]).abs() < INTERIOR {
            for cx in [start[0].max(end[0]) + INTERIOR, start[0].min(end[0]) - INTERIOR] {
                candidates.push([start, ss, [cx, ss[1]], [cx, es[1]], es, end]);
            }
        }
        if !is_vertical(from_side) && !is_vertical(to_side) && (start[1] - end[1]).abs() < INTERIOR {
            for cy in [start[1].max(end[1]) + INTERIOR, start[1].min(end[1]) - INTERIOR] {
                candidates.push([start, ss, [ss[0], cy], [es[0], cy], es, end]);
            }
        }
    }
    candidates.into_iter().map(|c| normalize(&c)).find(|points| {
        route_honors_endpoint_sides(points, from_side, to_side) && rhythm_ok(points, INTERIOR, 8.0) && accept(points)
    })
}

/// `connectionGeometry(conn, sides)`.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Geometry {
    from: Rect,
    to: Rect,
    start: Pt,
    end: Pt,
    sides: Sides,
    crowded: bool,
}

struct Router<'a> {
    boxes: &'a [RouterBox],
    by_id: HashMap<&'a str, usize>,
    rels: &'a [RouterRel],
    opts: &'a RouterOptions,
    label_rect_for: Option<&'a LabelRectFor<'a>>,
    frame_borders: Vec<Seg>,
    /// `reservedLabels`: (relation, rect).
    reserved: Vec<(usize, Rect)>,
    honour_reserved: bool,
    allow_grid: bool,
    metrics: Cell<PlanningMetrics>,
    /// `neighbourRects`: box -> the boxes it connects to (with repeats).
    neighbours: HashMap<usize, Vec<usize>>,
    /// `incidentEndpoints`: box -> (relation, end), in relation order.
    incident: HashMap<usize, Vec<(usize, End)>>,
    ports: Vec<SpreadPorts>,
    path_cache: HashMap<usize, Vec<Pt>>,
    selected: HashMap<usize, Sides>,
}

impl<'a> Router<'a> {
    fn new(boxes: &'a [RouterBox], rels: &'a [RouterRel], opts: &'a RouterOptions, label_rect_for: Option<&'a LabelRectFor<'a>>) -> Self {
        let mut by_id = HashMap::new();
        for (i, b) in boxes.iter().enumerate() {
            by_id.entry(b.id.as_str()).or_insert(i);
        }
        let frame_borders = opts.frames.iter().flat_map(frame_border_segments).map(|b| Seg::new(b.start, b.end)).collect();
        let mut router = Router {
            boxes,
            by_id,
            rels,
            opts,
            label_rect_for,
            frame_borders,
            reserved: Vec::new(),
            honour_reserved: true,
            allow_grid: true,
            metrics: Cell::new(PlanningMetrics::default()),
            neighbours: HashMap::new(),
            incident: HashMap::new(),
            ports: vec![SpreadPorts::default(); rels.len()],
            path_cache: HashMap::new(),
            selected: HashMap::new(),
        };
        for rel in rels {
            let (Some(f), Some(t)) = (router.box_of(&rel.from), router.box_of(&rel.to)) else { continue };
            if f == t {
                continue;
            }
            router.neighbours.entry(f).or_default().push(t);
            router.neighbours.entry(t).or_default().push(f);
        }
        for (i, rel) in rels.iter().enumerate() {
            let (Some(f), Some(t)) = (router.box_of(&rel.from), router.box_of(&rel.to)) else { continue };
            router.incident.entry(f).or_default().push((i, End::From));
            router.incident.entry(t).or_default().push((i, End::To));
        }
        router.ports = router.spread_ports();
        router
    }

    fn box_of(&self, id: &str) -> Option<usize> {
        self.by_id.get(id).copied()
    }

    fn rect(&self, index: usize) -> &Rect {
        &self.boxes[index].rect
    }

    fn ends(&self, rel: usize) -> (usize, usize) {
        let r = &self.rels[rel];
        (self.by_id[r.from.as_str()], self.by_id[r.to.as_str()])
    }

    fn bump(&self, f: impl FnOnce(&mut PlanningMetrics)) {
        let mut m = self.metrics.take();
        f(&mut m);
        self.metrics.set(m);
    }

    fn distinct(&self) -> bool {
        self.opts.distinct_automatic_ports
    }

    /// `markerSpacing` / `portSpacing` (`:671-672`).
    fn marker_spacing(&self, a: usize, b: usize) -> f64 {
        3.5 * (self.rels[a].stroke + self.rels[b].stroke)
    }

    fn port_spacing(&self, a: usize, b: usize) -> f64 {
        14f64.max(self.marker_spacing(a, b) + 3.5)
    }

    fn spread_ports(&self) -> Vec<SpreadPorts> {
        let relations: Vec<SpreadRelation> = self
            .rels
            .iter()
            .map(|r| SpreadRelation {
                id: r.id.clone().unwrap_or_default(),
                from: r.from.clone(),
                to: r.to.clone(),
                label: r.label.clone().unwrap_or_default(),
                from_side: r.from_side,
                to_side: r.to_side,
                auto: !r.authored_geometry() && r.label_at.is_none(),
            })
            .collect();
        let boxes: HashMap<String, Rect> = self.by_id.iter().map(|(id, &i)| ((*id).to_owned(), self.boxes[i].rect)).collect();
        let side_for = |i: usize, end: End| {
            let (f, t) = self.ends(i);
            self.row_fan_out_sides(f, t).map(|s| if end == End::From { s.from } else { s.to })
        };
        let spacing_for = |a: usize, b: usize| if self.marker_spacing(a, b) > 14.0 { self.port_spacing(a, b) } else { 14.0 };
        automatic_port_spread(
            &relations,
            &boxes,
            SpreadOptions::default(),
            Some(&side_for),
            if self.distinct() { Some(&spacing_for) } else { None },
        )
    }

    /// `rowFanOutSides(from, to)` (`:638`).
    fn row_fan_out_sides(&self, from: usize, to: usize) -> Option<Sides> {
        if !self.opts.prefer_readable_routes {
            return None;
        }
        if let Some(s) = self.vertical_fan_side(from, to) {
            return Some(Sides { from: s, to: opposite(s) });
        }
        self.vertical_fan_side(to, from).map(|s| Sides { from: opposite(s), to: s })
    }

    fn vertical_fan_side(&self, node: usize, other: usize) -> Option<Side> {
        let (n, o) = (self.rect(node), self.rect(other));
        let below = o.y >= n.y + n.height;
        if !below && o.y + o.height > n.y {
            return None;
        }
        if is_vertical(default_from_side(n, o)) {
            return None;
        }
        let want = if below { Side::Bottom } else { Side::Top };
        let empty = Vec::new();
        let neighbours = self.neighbours.get(&node).unwrap_or(&empty);
        let row_sibling = neighbours.iter().any(|&s| {
            s != other && (self.rect(s).cy() - o.cy()).abs() < 1.0 && default_from_side(n, self.rect(s)) == want
        });
        let blocked = |target: usize| {
            let t = self.rect(target);
            let (gap_start, gap_end) = if t.cx() < n.cx() { (t.x + t.width, n.x) } else { (n.x + n.width, t.x) };
            self.boxes.iter().enumerate().any(|(k, b)| {
                let r = &b.rect;
                k != node
                    && k != target
                    && r.y < t.y + t.height
                    && r.y + r.height > t.y
                    && r.x < gap_end
                    && r.x + r.width > gap_start
            })
        };
        let side_of_other = default_from_side(n, o);
        let same_side_blocked = neighbours
            .iter()
            .filter(|&&s| (self.rect(s).cy() - o.cy()).abs() < 1.0 && default_from_side(n, self.rect(s)) == side_of_other)
            .any(|&s| blocked(s));
        (row_sibling && same_side_blocked).then_some(want)
    }

    /// `inferredConnectionSides(conn)`.
    fn inferred_sides(&self, rel: usize) -> Sides {
        let (f, t) = self.ends(rel);
        let (from, to) = (self.rect(f), self.rect(t));
        let fan = self.row_fan_out_sides(f, t);
        let r = &self.rels[rel];
        Sides {
            from: chosen_side(r.from_side, fan.map_or_else(|| default_from_side(from, to), |s| s.from)),
            to: chosen_side(r.to_side, fan.map_or_else(|| default_to_side(from, to), |s| s.to)),
        }
    }

    /// `selectedSides.get(conn) || inferredConnectionSides(conn)`.
    fn current_sides(&self, rel: usize) -> Sides {
        self.selected.get(&rel).copied().unwrap_or_else(|| self.inferred_sides(rel))
    }

    // ---- predicates ------------------------------------------------------------------------------

    fn share_endpoint(&self, a: usize, b: usize) -> bool {
        let (l, r) = (&self.rels[a], &self.rels[b]);
        l.from == r.from || l.from == r.to || l.to == r.from || l.to == r.to
    }

    /// A shared endpoint exempts a pair from the corridor and crossing tests unless distinct
    /// automatic ports are on and neither side is authored geometry or a pinned label.
    fn shared_exempt(&self, rel: usize, other: usize) -> bool {
        let (r, o) = (&self.rels[rel], &self.rels[other]);
        self.share_endpoint(rel, other)
            && (!self.distinct() || o.authored_geometry() || o.label_at.is_some() || r.label_at.is_some())
    }

    /// `routeClearsReservedLabels`.
    fn clears_reserved(&self, rel: usize, points: &[Pt]) -> bool {
        if !self.honour_reserved {
            return true;
        }
        self.reserved
            .iter()
            .filter(|(owner, _)| *owner != rel)
            .all(|(_, rect)| segs(points).all(|s| !segment_intersects_rect(&s, rect, LABEL_CLEARANCE)))
    }

    /// `reservedLabelObstacles(2)`: the reserved rects grown so the grid's own 2 px makes 4.
    fn reserved_obstacles(&self) -> Vec<Rect> {
        if !self.honour_reserved {
            return Vec::new();
        }
        let grow = LABEL_CLEARANCE - GRID_CLEARANCE;
        self.reserved
            .iter()
            .map(|(_, r)| Rect::new(r.x - grow, r.y - grow, r.width + grow * 2.0, r.height + grow * 2.0))
            .collect()
    }

    /// `routeClearsComponents(conn, points)` (clearance 2), then the reserved labels.
    fn clears_components(&self, rel: usize, points: &[Pt]) -> bool {
        let r = &self.rels[rel];
        for b in self.boxes {
            if b.id == r.from || b.id == r.to {
                continue;
            }
            if segs(points).any(|s| segment_intersects_rect(&s, &b.rect, GRID_CLEARANCE)) {
                return false;
            }
        }
        self.clears_reserved(rel, points)
    }

    /// `routeClearsEndpointComponents`: only the first segment may touch `from`, only the last `to`.
    fn clears_endpoints(points: &[Pt], from: &Rect, to: &Rect) -> bool {
        let last = points.len().saturating_sub(2);
        for (index, s) in segs(points).enumerate() {
            if index > 0 && segment_intersects_rect(&s, from, 0.0) {
                return false;
            }
            if index < last && segment_intersects_rect(&s, to, 0.0) {
                return false;
            }
        }
        true
    }

    /// `routeMeetsCompositionFloors`: the rhythm floors and no frame border run.
    fn meets_floors(&self, points: &[Pt]) -> bool {
        if !rhythm_ok(points, self.opts.interior_segment_px, self.opts.micro_segment_px) {
            return false;
        }
        if self.opts.frames.is_empty() || points.len() < 2 || !points.iter().all(|p| is_finite_point(p)) {
            return true;
        }
        !self.frame_borders.iter().any(|border| {
            points.windows(2).any(|w| collinear_axis_overlap(w[0], w[1], border.start, border.end).is_some_and(|o| o.length > EPS))
        })
    }

    /// `routeConflictsWithResolved`: a proper crossing, a touch on another route's interior or a
    /// shared corridor of 8+ with a route that shares no endpoint.
    fn conflicts(&self, rel: usize, points: &[Pt], resolved: &[Entry]) -> bool {
        resolved.iter().filter(|(other, _)| !self.share_endpoint(rel, *other)).any(|(_, op)| {
            points.windows(2).any(|a| {
                op.windows(2).any(|b| {
                    proper_segment_intersection(a[0], a[1], b[0], b[1]).is_some()
                        || orthogonal_touch_on_resolved_interior(a[0], a[1], b[0], b[1])
                        || collinear_overlap_length(a[0], a[1], b[0], b[1]) >= 8.0
                })
            })
        })
    }

    /// `routeOverlapsResolved`: a shared corridor of 8+ with any route not exempt.
    fn overlaps(&self, rel: usize, points: &[Pt], resolved: &[Entry]) -> bool {
        resolved.iter().filter(|(other, _)| !self.shared_exempt(rel, *other)).any(|(_, op)| {
            points.windows(2).any(|a| op.windows(2).any(|b| collinear_overlap_length(a[0], a[1], b[0], b[1]) >= 8.0))
        })
    }

    /// `crossingCount`: the resolved routes this one properly crosses (shared endpoints exempt as above).
    fn crossing_count(&self, rel: usize, points: &[Pt], resolved: &[Entry]) -> usize {
        resolved
            .iter()
            .filter(|(other, op)| {
                !self.shared_exempt(rel, *other)
                    && points.windows(2).any(|a| op.windows(2).any(|b| proper_segment_intersection(a[0], a[1], b[0], b[1]).is_some()))
            })
            .count()
    }

    /// `reciprocal(conn)`: some relation (of all of them) runs the other way.
    fn reciprocal(&self, rel: usize) -> bool {
        let r = &self.rels[rel];
        self.rels.iter().any(|o| o.from == r.to && o.to == r.from)
    }

    /// `siblingCrossings`: crossings with non-reciprocal routes, 0 for a reciprocal relation.
    fn sibling_crossings(&self, rel: usize, points: &[Pt], resolved: &[Entry]) -> usize {
        if self.reciprocal(rel) {
            return 0;
        }
        let kept: Vec<Entry> = resolved.iter().copied().filter(|(o, _)| !self.reciprocal(*o)).collect();
        self.crossing_count(rel, points, &kept)
    }

    /// `readabilityCost`: `length + 48 * bends + 160 * crossings`.
    fn readability_cost(&self, rel: usize, points: &[Pt], resolved: &[Entry]) -> f64 {
        manhattan_length(points)
            + points.len().saturating_sub(2) as f64 * BEND_COST
            + self.crossing_count(rel, points, resolved) as f64 * CROSSING_COST
    }

    fn route_is_clear(&self, rel: usize, points: &[Pt], g: &Geometry, resolved: &[Entry]) -> bool {
        !g.crowded
            && points.len() >= 2
            && route_honors_endpoint_sides(points, g.sides.from, g.sides.to)
            && Self::clears_endpoints(points, &g.from, &g.to)
            && self.clears_components(rel, points)
            && self.meets_floors(points)
            && !self.overlaps(rel, points, resolved)
    }

    // ---- endpoints ---------------------------------------------------------------------------

    /// `alignFacingPorts` (`:326`): move the unshared endpoint of a nearly aligned facing pair onto
    /// the other's axis.
    #[allow(clippy::too_many_arguments)]
    fn align_facing_ports(&self, rel: usize, from: &Rect, to: &Rect, start: Pt, end: Pt, sides: Sides, spread: (bool, bool)) -> (Pt, Pt) {
        let r = &self.rels[rel];
        let (fs, ts) = (sides.from, sides.to);
        let horizontally = (fs == Side::Right && ts == Side::Left) || (fs == Side::Left && ts == Side::Right);
        let vertically = (fs == Side::Bottom && ts == Side::Top) || (fs == Side::Top && ts == Side::Bottom);
        if r.authored_geometry() || r.label_at.is_some() || (!horizontally && !vertically) {
            return (start, end);
        }
        let (from_spread, to_spread) = spread;
        if from_spread && to_spread {
            return (start, end);
        }
        if !from_spread && !to_spread && (r.from_side.is_some() || r.to_side.is_some()) {
            return (start, end);
        }
        let delta = if horizontally { (start[1] - end[1]).abs() } else { (start[0] - end[0]).abs() };
        if delta >= ALIGNMENT_DELTA {
            return (start, end);
        }
        let end_to_start = if horizontally { (start, [end[0], start[1]]) } else { (start, [start[0], end[1]]) };
        let start_to_end = if horizontally { ([start[0], end[1]], end) } else { ([end[0], start[1]], end) };
        let candidates: &[(Pt, Pt)] = if from_spread {
            &[end_to_start]
        } else if to_spread {
            &[start_to_end]
        } else {
            &[end_to_start, start_to_end]
        };
        for &(s, e) in candidates {
            let points = [s, e];
            if port_has_corner_clearance(from, fs, s)
                && port_has_corner_clearance(to, ts, e)
                && route_honors_endpoint_sides(&points, fs, ts)
                && Self::clears_endpoints(&points, from, to)
                && self.clears_components(rel, &points)
            {
                return (s, e);
            }
        }
        (start, end)
    }

    fn spread_port(&self, rel: usize, end: End) -> Option<Pt> {
        match end {
            End::From => self.ports[rel].from,
            End::To => self.ports[rel].to,
        }
    }

    /// `automaticEndpoint` (`:728`): the spread slot on the inferred side, else the side midpoint,
    /// moved off every slot another automatic route already holds on that side. `(point, spread,
    /// crowded)`.
    fn automatic_endpoint(&self, rel: usize, end: End, at: usize, side: Side, inferred: Side) -> (Pt, bool, bool) {
        let rect = self.rect(at);
        let initial = if side == inferred { self.spread_port(rel, end) } else { None };
        let preferred = initial.unwrap_or_else(|| anchor(rect, side));
        let r = &self.rels[rel];
        if r.authored_geometry() || r.label_at.is_some() {
            return (preferred, initial.is_some(), false);
        }
        let axis = if is_vertical(side) { 0 } else { 1 };
        let mut occupied: Vec<(f64, f64)> = Vec::new();
        for &(other, field) in self.incident.get(&at).map(Vec::as_slice).unwrap_or(&[]) {
            let o = &self.rels[other];
            if other == rel || o.authored_geometry() || o.label_at.is_some() {
                continue;
            }
            let sides = self.current_sides(other);
            if (if field == End::From { sides.from } else { sides.to }) != side {
                continue;
            }
            let point = match self.path_cache.get(&other) {
                Some(points) => {
                    if field == End::From {
                        points[0]
                    } else {
                        points[points.len() - 1]
                    }
                }
                None => self.spread_port(other, field).unwrap_or_else(|| anchor(rect, side)),
            };
            occupied.push((point[axis], self.port_spacing(rel, other)));
        }
        if occupied.is_empty() {
            return (preferred, initial.is_some(), false);
        }
        let p = preferred[axis];
        let mut candidates: Vec<f64> = std::iter::once(p).chain(occupied.iter().flat_map(|&(v, s)| [v - s, v + s])).collect();
        candidates.sort_by(|a, b| js_order((a - p).abs() - (b - p).abs()).then_with(|| js_order(a - b)));
        for value in candidates {
            let mut point = preferred;
            point[axis] = value;
            if port_has_corner_clearance(rect, side, point) && occupied.iter().all(|&(v, s)| (value - v).abs() >= s - EPS) {
                return (point, true, false);
            }
        }
        (preferred, true, true)
    }

    /// `connectionGeometry(conn, sides)`.
    fn geometry(&self, rel: usize, sides: Sides) -> Geometry {
        let (f, t) = self.ends(rel);
        let (from, to) = (*self.rect(f), *self.rect(t));
        let inferred = self.inferred_sides(rel);
        let (source, target) = if self.distinct() {
            (
                self.automatic_endpoint(rel, End::From, f, sides.from, inferred.from),
                self.automatic_endpoint(rel, End::To, t, sides.to, inferred.to),
            )
        } else {
            let legacy = if sides == inferred { self.ports[rel] } else { SpreadPorts::default() };
            (
                (legacy.from.unwrap_or_else(|| anchor(&from, sides.from)), legacy.from.is_some(), false),
                (legacy.to.unwrap_or_else(|| anchor(&to, sides.to)), legacy.to.is_some(), false),
            )
        };
        let (start, end) = self.align_facing_ports(rel, &from, &to, source.0, target.0, sides, (source.1, target.1));
        Geometry { from, to, start, end, sides, crowded: source.2 || target.2 }
    }

    // ---- candidates --------------------------------------------------------------------------

    /// `routeVia` (`:395`): the interior points of the route on `g`.
    fn route_via(&self, rel: usize, g: &Geometry, resolved: &[Entry]) -> Vec<Pt> {
        let r = &self.rels[rel];
        if let Some(via) = &r.via {
            return via.clone();
        }
        let (start, end) = (g.start, g.end);
        match r.route {
            RouteKind::Straight => Vec::new(),
            RouteKind::OrthogonalH => {
                let mid_x = (start[0] + end[0]) / 2.0;
                vec![[mid_x, start[1]], [mid_x, end[1]]]
            }
            RouteKind::OrthogonalV => {
                let mid_y = (start[1] + end[1]) / 2.0;
                vec![[start[0], mid_y], [end[0], mid_y]]
            }
            RouteKind::Auto => self.auto_via(rel, g, resolved),
        }
    }

    fn auto_via(&self, rel: usize, g: &Geometry, resolved: &[Entry]) -> Vec<Pt> {
        let (from, to, start, end) = (&g.from, &g.to, g.start, g.end);
        let (fs, ts) = (g.sides.from, g.sides.to);
        let with = |c: &[Pt]| -> Vec<Pt> { std::iter::once(start).chain(c.iter().copied()).chain(std::iter::once(end)).collect() };
        let dx = (start[0] - end[0]).abs();
        let dy = (start[1] - end[1]).abs();
        if dx < 4.0 || dy < 4.0 {
            let direct = [start, end];
            if route_honors_endpoint_sides(&direct, fs, ts)
                && Self::clears_endpoints(&direct, from, to)
                && self.clears_components(rel, &direct)
                && self.meets_floors(&direct)
                && !self.conflicts(rel, &direct, resolved)
            {
                return Vec::new();
            }
        }

        let bridge = automatic_port_rhythm_bridge(start, end, fs, ts, &mut |points| {
            Self::clears_endpoints(points, from, to)
                && self.clears_components(rel, points)
                && self.meets_floors(points)
                && !self.conflicts(rel, points, resolved)
        });
        if let Some(points) = bridge {
            return interior(&points);
        }

        const MIN_STUB: f64 = 8.0;
        let from_vertical_side = start[1] == from.y || start[1] == from.y + from.height;
        let to_vertical_side = end[1] == to.y || end[1] == to.y + to.height;
        if from_vertical_side && to_vertical_side && dx < MIN_STUB * 2.0 {
            for cx in [start[0].max(end[0]) + MIN_STUB * 2.0, start[0].min(end[0]) - MIN_STUB * 2.0] {
                let candidate = vec![[cx, start[1]], [cx, end[1]]];
                let points = with(&candidate);
                if route_honors_endpoint_sides(&points, fs, ts)
                    && self.clears_components(rel, &points)
                    && self.meets_floors(&points)
                    && !self.conflicts(rel, &points, resolved)
                {
                    return candidate;
                }
            }
        }
        let from_horizontal_side = start[0] == from.x || start[0] == from.x + from.width;
        let to_horizontal_side = end[0] == to.x || end[0] == to.x + to.width;
        if from_horizontal_side && to_horizontal_side && dy < MIN_STUB * 2.0 {
            for cy in [start[1].max(end[1]) + MIN_STUB * 2.0, start[1].min(end[1]) - MIN_STUB * 2.0] {
                let candidate = vec![[start[0], cy], [end[0], cy]];
                let points = with(&candidate);
                if route_honors_endpoint_sides(&points, fs, ts)
                    && self.clears_components(rel, &points)
                    && self.meets_floors(&points)
                    && !self.conflicts(rel, &points, resolved)
                {
                    return candidate;
                }
            }
        }

        let mid_x = (start[0] + end[0]) / 2.0;
        let horizontal_first = vec![[mid_x, start[1]], [mid_x, end[1]]];
        let mid_y = (start[1] + end[1]) / 2.0;
        let vertical_first = vec![[start[0], mid_y], [end[0], mid_y]];
        let side_safe: Vec<Vec<Pt>> = [horizontal_first.clone(), vertical_first]
            .into_iter()
            .filter(|c| route_honors_endpoint_sides(&with(c), fs, ts))
            .collect();
        let side_aware = side_aware_bridge_candidates(start, end, fs, ts);
        let near_parallel = (is_vertical(fs) && is_vertical(ts) && dx < MIN_STUB * 2.0)
            || (!is_vertical(fs) && !is_vertical(ts) && dy < MIN_STUB * 2.0);
        let ordered: Vec<&Vec<Pt>> =
            if near_parallel { side_aware.iter().chain(&side_safe).collect() } else { side_safe.iter().chain(&side_aware).collect() };
        for candidate in ordered {
            let points = with(candidate);
            if Self::clears_endpoints(&points, from, to)
                && self.clears_components(rel, &points)
                && self.meets_floors(&points)
                && !self.conflicts(rel, &points, resolved)
                && (!self.distinct()
                    || (!self.overlaps(rel, &points, resolved) && self.sibling_crossings(rel, &points, resolved) == 0))
            {
                return candidate.clone();
            }
        }

        if self.distinct() {
            let channels = |a: f64, b: f64, mid: f64| {
                let (low, high) = (a.min(b) + 24.0, a.max(b) - 24.0);
                let mut values = Vec::new();
                let mut offset = 16.0;
                while mid - offset >= low || mid + offset <= high {
                    values.extend([mid + offset, mid - offset].into_iter().filter(|v| *v >= low && *v <= high));
                    offset += 16.0;
                }
                values
            };
            let candidates = channels(start[0], end[0], mid_x)
                .into_iter()
                .map(|x| vec![[x, start[1]], [x, end[1]]])
                .chain(channels(start[1], end[1], mid_y).into_iter().map(|y| vec![[start[0], y], [end[0], y]]));
            for candidate in candidates {
                let points = with(&candidate);
                if route_honors_endpoint_sides(&points, fs, ts)
                    && Self::clears_endpoints(&points, from, to)
                    && self.clears_components(rel, &points)
                    && self.meets_floors(&points)
                    && !self.conflicts(rel, &points, resolved)
                    && !self.overlaps(rel, &points, resolved)
                    && self.sibling_crossings(rel, &points, resolved) == 0
                {
                    return candidate;
                }
            }
        }

        let fallback = side_safe.first().or(side_aware.first()).unwrap_or(&horizontal_first).clone();
        if !self.allow_grid {
            return fallback;
        }
        if self.metrics_get().grid_search_count >= MAX_GRID_SEARCHES {
            self.bump(|m| {
                m.grid_budget_exhausted_count += 1;
                m.conflict_fallback_count += 1;
            });
            return fallback;
        }
        let r = &self.rels[rel];
        let avoided: Vec<Seg> = resolved
            .iter()
            .filter(|(other, _)| {
                let o = &self.rels[*other];
                self.distinct()
                    && self.share_endpoint(rel, *other)
                    && !o.authored_geometry()
                    && o.label_at.is_none()
                    && r.label_at.is_none()
            })
            .flat_map(|(_, op)| segs(op).collect::<Vec<_>>())
            .collect();
        self.bump(|m| m.grid_search_count += 1);
        let obstacles: Vec<Rect> = self.boxes.iter().map(|b| b.rect).chain(self.reserved_obstacles()).collect();
        let endpoints = [start, end];
        let query = GridQuery::architecture(
            start,
            end,
            &obstacles,
            Some(fs),
            Some(ts),
            &endpoints,
            &avoided,
            &self.frame_borders,
            self.opts.interior_segment_px,
        );
        if let Some(searched) = shortest_orthogonal_grid_route(&query) {
            let points = &searched.points;
            let accepted = Self::clears_endpoints(points, from, to)
                && self.clears_components(rel, points)
                && !self.overlaps(rel, points, resolved)
                && self.meets_floors(points);
            if accepted {
                let crossover = self.conflicts(rel, points, resolved);
                self.bump(|m| {
                    m.grid_routed_count += 1;
                    if crossover {
                        m.crossover_routed_count += 1;
                    }
                });
                return interior(points);
            }
        }
        self.bump(|m| m.conflict_fallback_count += 1);
        fallback
    }

    fn metrics_get(&self) -> PlanningMetrics {
        let m = self.metrics.take();
        self.metrics.set(m.clone());
        m
    }

    /// `routedForGeometry`: authored geometry keeps every waypoint; automatic routes are normalised.
    fn routed_for_geometry(&self, rel: usize, resolved: &[Entry], g: &Geometry) -> Vec<Pt> {
        let mut points = vec![g.start];
        points.extend(self.route_via(rel, g, resolved));
        points.push(g.end);
        if self.rels[rel].authored_geometry() { points } else { normalize(&points) }
    }

    /// `candidateSidePairs`: the authored side or the inferred one first then `SIDE_ORDER`, ordered
    /// by deviation from the inferred pair, then by the anchors' Manhattan distance (stable).
    fn candidate_side_pairs(&self, rel: usize) -> Vec<Sides> {
        let inferred = self.inferred_sides(rel);
        let r = &self.rels[rel];
        let options = |authored: Option<Side>, inferred: Side| -> Vec<Side> {
            match authored {
                Some(s) => vec![s],
                None => std::iter::once(inferred).chain(SIDE_ORDER.into_iter().filter(|s| *s != inferred)).collect(),
            }
        };
        let mut pairs: Vec<(usize, f64, Sides)> = Vec::new();
        for from in options(r.from_side, inferred.from) {
            for to in options(r.to_side, inferred.to) {
                let sides = Sides { from, to };
                let deviation = usize::from(from != inferred.from) + usize::from(to != inferred.to);
                let g = self.geometry(rel, sides);
                pairs.push((deviation, endpoint_distance(g.start, g.end), sides));
            }
        }
        pairs.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| js_order(a.1 - b.1)));
        pairs.into_iter().map(|p| p.2).collect()
    }

    fn cache_path(&mut self, rel: usize, points: Vec<Pt>, sides: Sides) {
        self.path_cache.insert(rel, points);
        self.selected.insert(rel, sides);
    }

    /// One pass of `clearRoute`: the first side pair whose route is clear; records the very first
    /// candidate as the fallback.
    fn clear_route(&self, rel: usize, resolved: &[Entry], fallback: &mut Option<(Vec<Pt>, Sides)>) -> Option<(Vec<Pt>, Sides)> {
        for sides in self.candidate_side_pairs(rel) {
            let g = self.geometry(rel, sides);
            let routed = self.routed_for_geometry(rel, resolved, &g);
            if fallback.is_none() {
                *fallback = Some((routed.clone(), sides));
            }
            if self.route_is_clear(rel, &routed, &g, resolved) {
                return Some((routed, sides));
            }
        }
        None
    }

    /// `computePath(conn, resolvedRoutes)`, with the reserved-label retry.
    fn compute_path(&mut self, rel: usize, resolved: &[Entry]) -> Vec<Pt> {
        if self.rels[rel].authored_geometry() {
            let sides = self.inferred_sides(rel);
            let g = self.geometry(rel, sides);
            let points = self.routed_for_geometry(rel, resolved, &g);
            self.cache_path(rel, points.clone(), sides);
            return points;
        }
        let mut fallback = None;
        self.honour_reserved = true;
        let mut chosen = self.clear_route(rel, resolved, &mut fallback);
        if !self.reserved.is_empty() {
            self.honour_reserved = false;
            match &chosen {
                None => chosen = self.clear_route(rel, resolved, &mut fallback),
                Some((points, _)) => {
                    let length = manhattan_length(points);
                    let direct = endpoint_distance(points[0], points[points.len() - 1]);
                    if length > direct * 1.25 + 64.0
                        && let Some(plain) = self.clear_route(rel, resolved, &mut fallback)
                        && manhattan_length(&plain.0) * 1.25 + 64.0 < length
                    {
                        chosen = Some(plain);
                    }
                }
            }
            self.honour_reserved = true;
        }
        let (points, sides) = chosen.or(fallback).expect("at least one side pair");
        self.cache_path(rel, points.clone(), sides);
        points
    }

    fn label_rect(&self, rel: usize, points: &[Pt], routes: &[&[Pt]], labels: &[Rect]) -> Option<Rect> {
        self.label_rect_for.and_then(|f| f(rel, points, routes, labels))
    }

    // ---- planning ----------------------------------------------------------------------------

    /// `planRoutes`.
    fn plan(mut self) -> Plan {
        let indexed: Vec<usize> =
            (0..self.rels.len()).filter(|&i| self.box_of(&self.rels[i].from).is_some() && self.box_of(&self.rels[i].to).is_some()).collect();
        let explicit: Vec<usize> = indexed.iter().copied().filter(|&i| self.rels[i].authored_geometry()).collect();
        let mut automatic: Vec<usize> = indexed.iter().copied().filter(|&i| !self.rels[i].authored_geometry()).collect();
        let distance = |i: usize| {
            let (f, t) = self.ends(i);
            let (from, to) = (self.rect(f), self.rect(t));
            (to.cx() - from.cx()).abs() + (to.cy() - from.cy()).abs()
        };
        automatic.sort_by(|&a, &b| js_order(distance(a) - distance(b)).then(a.cmp(&b)));

        let mut resolved: Vec<(usize, Vec<Pt>)> = Vec::new();
        self.reserved.clear();
        for &rel in explicit.iter().chain(&automatic) {
            let points = {
                let entries: Vec<Entry> = resolved.iter().map(|(r, p)| (*r, p.as_slice())).collect();
                self.compute_path(rel, &entries)
            };
            resolved.push((rel, points.clone()));
            let routes: Vec<&[Pt]> = resolved.iter().map(|(_, p)| p.as_slice()).collect();
            let labels: Vec<Rect> = self.reserved.iter().map(|(_, r)| *r).collect();
            if let Some(rect) = self.label_rect(rel, &points, &routes, &labels) {
                self.reserved.push((rel, rect));
            }
        }
        if self.opts.prefer_readable_routes {
            self.readability_sweep(&mut resolved);
        }
        self.bump(|m| {
            m.route_count = resolved.len();
            m.explicit_route_count = explicit.len();
            m.automatic_route_count = automatic.len();
        });
        let routes = (0..self.rels.len())
            .map(|i| self.path_cache.get(&i).map(|points| Routed { points: points.clone(), sides: self.current_sides(i) }))
            .collect();
        Plan { routes, metrics: self.metrics.take() }
    }

    /// The bounded sweep after the greedy pass: the reciprocal-pair repair, then one route at a
    /// time against every other route and label, the grid search off, inside the scene bounds.
    fn readability_sweep(&mut self, resolved: &mut [(usize, Vec<Pt>)]) {
        let mut scene: Vec<Pt> = resolved.iter().flat_map(|(_, p)| p.iter().copied()).collect();
        let frame_rects = self.opts.frames.iter().map(|f| match *f {
            Frame::Rect { rect, .. } => rect,
            // A line frame has no `x`/`y`: the JS bounds go NaN.
            Frame::Line { .. } => Rect::new(f64::NAN, f64::NAN, f64::NAN, f64::NAN),
        });
        for r in self.boxes.iter().map(|b| b.rect).chain(frame_rects).chain(self.reserved.iter().map(|(_, r)| *r)) {
            scene.push([r.x, r.y]);
            scene.push([r.x + r.width, r.y + r.height]);
        }
        let fold = |init: f64, pick: fn(f64, f64) -> f64, axis: usize| {
            scene.iter().fold(init, |acc, p| if acc.is_nan() || p[axis].is_nan() { f64::NAN } else { pick(acc, p[axis]) })
        };
        let bounds = [fold(f64::INFINITY, f64::min, 0), fold(f64::NEG_INFINITY, f64::max, 0), fold(f64::INFINITY, f64::min, 1), fold(f64::NEG_INFINITY, f64::max, 1)];
        let within = move |p: Pt| p[0] >= bounds[0] && p[0] <= bounds[1] && p[1] >= bounds[2] && p[1] <= bounds[3];

        self.allow_grid = false;
        let mut jointly: Vec<usize> = Vec::new();
        self.improve_reciprocal_pairs(resolved, &within, &mut jointly);
        for i in 0..resolved.len() {
            let rel = resolved[i].0;
            let r = &self.rels[rel];
            if jointly.contains(&rel) || r.authored_geometry() || r.label_at.is_some() {
                continue;
            }
            let best = {
                let others: Vec<Entry> = resolved.iter().enumerate().filter(|(k, _)| *k != i).map(|(_, (o, p))| (*o, p.as_slice())).collect();
                let current = &resolved[i].1;
                if current.len() <= 4 && self.crossing_count(rel, current, &others) == 0 {
                    continue;
                }
                let mut best: Option<(Vec<Pt>, Sides, Option<Rect>)> = None;
                let mut best_cost = self.readability_cost(rel, current, &others);
                for sides in self.candidate_side_pairs(rel) {
                    let g = self.geometry(rel, sides);
                    let routed = self.routed_for_geometry(rel, &others, &g);
                    self.bump(|m| m.readability_candidate_count += 1);
                    if !routed.iter().all(|p| within(*p)) || !self.route_is_clear(rel, &routed, &g, &others) {
                        continue;
                    }
                    let cost = self.readability_cost(rel, &routed, &others);
                    if cost < best_cost {
                        let routes: Vec<&[Pt]> = others.iter().map(|(_, p)| *p).chain(std::iter::once(routed.as_slice())).collect();
                        let labels: Vec<Rect> = self.reserved.iter().filter(|(o, _)| *o != rel).map(|(_, r)| *r).collect();
                        let rect = self.label_rect(rel, &routed, &routes, &labels);
                        if let Some(r) = rect
                            && (!within([r.x, r.y]) || !within([r.x + r.width, r.y + r.height]))
                        {
                            continue;
                        }
                        best = Some((routed, sides, rect));
                        best_cost = cost;
                    }
                }
                best
            };
            let Some((routed, sides, rect)) = best else { continue };
            self.cache_path(rel, routed.clone(), sides);
            resolved[i].1 = routed;
            self.bump(|m| m.readability_improved_count += 1);
            self.reserved.retain(|(o, _)| *o != rel);
            if let Some(rect) = rect {
                self.reserved.push((rel, rect));
            }
        }
        self.allow_grid = true;
    }

    /// `improveReciprocalPairs`: a facing reciprocal pair tries both direct lanes together.
    fn improve_reciprocal_pairs(&mut self, resolved: &mut [(usize, Vec<Pt>)], within: &dyn Fn(Pt) -> bool, jointly: &mut Vec<usize>) {
        let free = |r: &RouterRel| !r.authored_geometry() && !r.label_placement && r.from_side.is_none() && r.to_side.is_none();
        let mut paired: Vec<usize> = Vec::new();
        for fi in 0..resolved.len() {
            let rel = resolved[fi].0;
            let r = &self.rels[rel];
            if paired.contains(&rel) || !free(r) {
                continue;
            }
            let Some(si) = (0..resolved.len()).find(|&k| {
                let o = &self.rels[resolved[k].0];
                k != fi && o.from == r.to && o.to == r.from && free(o)
            }) else {
                continue;
            };
            let second = resolved[si].0;
            paired.push(rel);
            paired.push(second);
            let fs = self.inferred_sides(rel);
            let ss = self.inferred_sides(second);
            let is = |s: Sides, from: Side, to: Side| s.from == from && s.to == to;
            let horizontal = (is(fs, Side::Right, Side::Left) && is(ss, Side::Left, Side::Right))
                || (is(fs, Side::Left, Side::Right) && is(ss, Side::Right, Side::Left));
            let vertical = (is(fs, Side::Bottom, Side::Top) && is(ss, Side::Top, Side::Bottom))
                || (is(fs, Side::Top, Side::Bottom) && is(ss, Side::Bottom, Side::Top));
            if !horizontal && !vertical {
                continue;
            }
            if resolved[fi].1.len() <= 2 && resolved[si].1.len() <= 2 {
                continue;
            }
            let axis = if horizontal { 1 } else { 0 };
            let fg = self.geometry(rel, fs);
            let sg = self.geometry(second, ss);
            if fg.crowded || sg.crowded {
                continue;
            }
            let previous = self.reserved.clone();
            self.reserved.retain(|(o, _)| *o != rel && *o != second);
            let best = {
                let others: Vec<Entry> = resolved
                    .iter()
                    .enumerate()
                    .filter(|(k, _)| *k != fi && *k != si)
                    .map(|(_, (o, p))| (*o, p.as_slice()))
                    .collect();
                let lanes = |g: &Geometry| -> Vec<f64> {
                    let (a, b) = (g.start[axis], g.end[axis]);
                    if a == b { vec![a] } else { vec![a, b] }
                };
                let direct = |g: &Geometry, lane: f64| -> Vec<Pt> {
                    let (mut s, mut e) = (g.start, g.end);
                    s[axis] = lane;
                    e[axis] = lane;
                    vec![s, e]
                };
                let sides_of = |o: usize| if o == rel { fs } else if o == second { ss } else { self.current_sides(o) };
                let slots_clear = |candidates: &[Entry]| -> bool {
                    let entries: Vec<Entry> = others.iter().chain(candidates).copied().collect();
                    for &(crel, cp) in candidates {
                        let cs = sides_of(crel);
                        let c = &self.rels[crel];
                        for (id, side, point) in [(&c.from, cs.from, cp[0]), (&c.to, cs.to, cp[cp.len() - 1])] {
                            let coord = if is_vertical(side) { 0 } else { 1 };
                            for &(orel, op) in &entries {
                                if orel == crel {
                                    continue;
                                }
                                let os = sides_of(orel);
                                let o = &self.rels[orel];
                                for (oid, oside, opoint) in [(&o.from, os.from, op[0]), (&o.to, os.to, op[op.len() - 1])] {
                                    if id == oid
                                        && side == oside
                                        && (point[coord] - opoint[coord]).abs() < self.port_spacing(crel, orel) - EPS
                                    {
                                        return false;
                                    }
                                }
                            }
                        }
                    }
                    true
                };
                let label_clears = |rect: Option<Rect>, owner: usize, entries: &[Entry], labels: &[(usize, Rect)]| -> bool {
                    let Some(rect) = rect else { return !self.rels[owner].has_label() };
                    if !within([rect.x, rect.y])
                        || !within([rect.x + rect.width, rect.y + rect.height])
                        || labels.iter().any(|(_, other)| rects_overlap(&rect, other, 2.0))
                    {
                        return false;
                    }
                    entries
                        .iter()
                        .all(|&(e, p)| e == owner || segs(p).all(|s| !segment_intersects_rect(&s, &rect, LABEL_CLEARANCE)))
                };
                type Best = (Vec<Pt>, Vec<Pt>, Option<Rect>, Option<Rect>);
                let mut best: Option<Best> = None;
                let (first_points, second_points) = (resolved[fi].1.as_slice(), resolved[si].1.as_slice());
                let mut best_cost = self.readability_cost(rel, first_points, &plus(&others, (second, second_points)))
                    + self.readability_cost(second, second_points, &plus(&others, (rel, first_points)));
                for first_lane in lanes(&fg) {
                    for second_lane in lanes(&sg) {
                        self.bump(|m| m.reciprocal_candidate_count += 1);
                        let fr = direct(&fg, first_lane);
                        let sr = direct(&sg, second_lane);
                        if !fr.iter().chain(&sr).all(|p| within(*p)) {
                            continue;
                        }
                        let candidates = [(rel, fr.as_slice()), (second, sr.as_slice())];
                        if !slots_clear(&candidates) {
                            continue;
                        }
                        let clear = |entry: Entry, g: &Geometry, other: Entry| {
                            port_has_corner_clearance(&g.from, g.sides.from, entry.1[0])
                                && port_has_corner_clearance(&g.to, g.sides.to, entry.1[entry.1.len() - 1])
                                && self.route_is_clear(entry.0, entry.1, g, &plus(&others, other))
                        };
                        if !clear(candidates[0], &fg, candidates[1]) || !clear(candidates[1], &sg, candidates[0]) {
                            continue;
                        }
                        let routes: Vec<&[Pt]> = others.iter().map(|(_, p)| *p).chain([fr.as_slice(), sr.as_slice()]).collect();
                        let labels: Vec<Rect> = self.reserved.iter().map(|(_, r)| *r).collect();
                        let first_rect = self.label_rect(rel, &fr, &routes, &labels);
                        if !label_clears(first_rect, rel, &plus(&others, (second, &sr)), &self.reserved) {
                            continue;
                        }
                        let labels: Vec<Rect> = labels.into_iter().chain(first_rect).collect();
                        let second_rect = self.label_rect(second, &sr, &routes, &labels);
                        let mut reserved_then = self.reserved.clone();
                        reserved_then.extend(first_rect.map(|r| (rel, r)));
                        if !label_clears(second_rect, second, &plus(&others, (rel, &fr)), &reserved_then) {
                            continue;
                        }
                        let cost = self.readability_cost(rel, &fr, &plus(&others, (second, &sr))) + self.readability_cost(second, &sr, &plus(&others, (rel, &fr)));
                        if cost < best_cost {
                            best = Some((fr.clone(), sr.clone(), first_rect, second_rect));
                            best_cost = cost;
                        }
                    }
                }
                best
            };
            let Some((fr, sr, first_rect, second_rect)) = best else {
                self.reserved = previous;
                continue;
            };
            self.cache_path(rel, fr.clone(), fs);
            self.cache_path(second, sr.clone(), ss);
            resolved[fi].1 = fr;
            resolved[si].1 = sr;
            self.bump(|m| m.reciprocal_improved_count += 1);
            jointly.push(rel);
            jointly.push(second);
            self.reserved.extend(first_rect.map(|r| (rel, r)));
            self.reserved.extend(second_rect.map(|r| (second, r)));
        }
    }
}

/// Plan every route of `rels` among `boxes` (`createRouter(...)` then `pathFor` per relation).
pub fn plan_routes(boxes: &[RouterBox], rels: &[RouterRel], opts: &RouterOptions, label_rect_for: Option<&LabelRectFor<'_>>) -> Plan {
    Router::new(boxes, rels, opts, label_rect_for).plan()
}

/// `[...others, extra]`.
fn plus<'x>(others: &[Entry<'x>], extra: Entry<'x>) -> Vec<Entry<'x>> {
    others.iter().copied().chain(std::iter::once(extra)).collect()
}

/// JS comparator result to an ordering: NaN and 0 are equal.
fn js_order(diff: f64) -> std::cmp::Ordering {
    if diff < 0.0 {
        std::cmp::Ordering::Less
    } else if diff > 0.0 {
        std::cmp::Ordering::Greater
    } else {
        std::cmp::Ordering::Equal
    }
}

/// `points.slice(1, -1)`.
fn interior(points: &[Pt]) -> Vec<Pt> {
    if points.len() < 2 { Vec::new() } else { points[1..points.len() - 1].to_vec() }
}

/// `sideAwareBridgeCandidates` (`:259`): outward stubs on both sides joined by one or two bends
/// (an outside channel for near-parallel ports), keeping only routes that honour both sides.
fn side_aware_bridge_candidates(start: Pt, end: Pt, fs: Side, ts: Side) -> Vec<Vec<Pt>> {
    const MINIMUM_BRIDGE: f64 = 16.0;
    let (fv, tv) = (outward(fs), outward(ts));
    let ss = [start[0] + fv[0] * STUB_PX, start[1] + fv[1] * STUB_PX];
    let es = [end[0] + tv[0] * STUB_PX, end[1] + tv[1] * STUB_PX];
    let mut raw: Vec<Vec<Pt>> = Vec::new();
    if is_vertical(fs) && is_vertical(ts) && (start[0] - end[0]).abs() < MINIMUM_BRIDGE {
        for cx in [start[0].max(end[0]) + MINIMUM_BRIDGE, start[0].min(end[0]) - MINIMUM_BRIDGE] {
            raw.push(vec![ss, [cx, ss[1]], [cx, es[1]], es]);
        }
    }
    if !is_vertical(fs) && !is_vertical(ts) && (start[1] - end[1]).abs() < MINIMUM_BRIDGE {
        for cy in [start[1].max(end[1]) + MINIMUM_BRIDGE, start[1].min(end[1]) - MINIMUM_BRIDGE] {
            raw.push(vec![ss, [ss[0], cy], [es[0], cy], es]);
        }
    }
    raw.push(vec![ss, [es[0], ss[1]], es]);
    raw.push(vec![ss, [ss[0], es[1]], es]);
    raw.into_iter()
        .map(|c| {
            let mut full = vec![start];
            full.extend(c);
            full.push(end);
            normalize(&full)
        })
        .filter(|p| p.len() >= 2)
        .filter(|p| !collinear_backtrack(p[0], p[1], if p.len() >= 3 { p[2] } else { p[1] }))
        .filter(|p| {
            let n = p.len();
            !collinear_backtrack(if n >= 3 { p[n - 3] } else { p[n - 2] }, p[n - 2], p[n - 1])
        })
        .filter(|p| route_honors_endpoint_sides(p, fs, ts))
        .map(|p| interior(&p))
        .collect()
}
