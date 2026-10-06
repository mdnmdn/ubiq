//! Bounded orthogonal grid search and the detour oracle.
//!
//! Port of `shortestOrthogonalGridRoute` and `cleanRouteDetourProblems` in
//! `renderers/shared/route-quality.mjs` (`00` 5.6.4, `03` 3.3). The graph is the grid lines through
//! stubs, obstacle edges, avoided segments and frame borders; the search is Dijkstra over
//! `(node, incoming direction)` with no reversals and a bend penalty.
//!
//! Parity notes, because routes must be bit-identical to the JS (P3.7):
//!
//! - Grid coordinates are deduplicated in insertion order (a JS `Set`), then sorted with a *stable*
//!   sort; coalescing compares each value with its predecessor in the **unfiltered** sorted list,
//!   not with the last kept one, and never drops a stub line.
//! - Nodes are the x-major / y-minor cross product (JS `Map` insertion order). A node's neighbour
//!   list is built horizontal lines first (left, then right), then vertical lines (up, then down):
//!   the relaxation order, and so the tie-break between equal-cost paths, depends on it. Lines
//!   hold only *usable* nodes, so an edge may skip over a node removed by an avoided segment.
//! - The heap is the JS binary heap, ported line by line: a pushed entry stays below an equal-key
//!   parent, and a pop sifts the displaced last entry down while a child is strictly smaller.
//!   `std::collections::BinaryHeap` would break ties differently.
//! - Strict `<` when relaxing (`candidate >= best` skips), so the first path found at a cost wins.
//! - Frame-border and avoided-segment overlap use the exact-equality `collinearOverlap` of this
//!   file, not the `1e-4` one in `geom`.
//!
//! There is no state or expansion budget in the JS: the only budgets are the obstacle count and the
//! candidate-node count. The "at most 64 searches per scene" counter belongs to the cascade (P3.3).

use std::collections::HashSet;

use crate::diag::js_round;
use crate::geom::{
    Pt, Rect, Seg, Side, normalize, proper_segment_intersection, segment_intersects_rect,
};

const EPS: f64 = 0.0001;

/// The search inputs (`shortestOrthogonalGridRoute`'s options). [`GridQuery::new`] carries the JS
/// defaults for everything optional.
#[derive(Clone, Debug)]
pub struct GridQuery<'a> {
    pub start: Pt,
    pub end: Pt,
    /// Extra grid coordinates (the authored route's points for the oracle, `[start, end]` for the router).
    pub points: &'a [Pt],
    pub obstacles: &'a [Rect],
    /// `None` is an unsupported side: the search fails with [`GridStatus::UnsupportedEndpointSide`].
    pub from_side: Option<Side>,
    pub to_side: Option<Side>,
    pub clearance: f64,
    pub maximum_obstacle_count: usize,
    /// `None` is `clearance + 2`.
    pub endpoint_stub_px: Option<f64>,
    /// `None` is `Infinity`.
    pub maximum_grid_nodes: Option<usize>,
    pub avoided_segments: &'a [Seg],
    pub allow_avoided_crossings: bool,
    pub minimum_avoided_overlap_px: f64,
    pub route_separation_px: f64,
    pub minimum_segment_px: f64,
    pub border_segments: &'a [Seg],
    pub bend_penalty_px: f64,
}

impl<'a> GridQuery<'a> {
    // The JS takes one options object; the positional form mirrors its required keys.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        start: Pt,
        end: Pt,
        points: &'a [Pt],
        obstacles: &'a [Rect],
        from_side: Option<Side>,
        to_side: Option<Side>,
        clearance: f64,
        maximum_obstacle_count: usize,
    ) -> Self {
        Self {
            start,
            end,
            points,
            obstacles,
            from_side,
            to_side,
            clearance,
            maximum_obstacle_count,
            endpoint_stub_px: None,
            maximum_grid_nodes: None,
            avoided_segments: &[],
            allow_avoided_crossings: false,
            minimum_avoided_overlap_px: 8.0,
            route_separation_px: 8.0,
            minimum_segment_px: 8.0,
            border_segments: &[],
            bend_penalty_px: 0.0,
        }
    }

    /// The options the architecture router passes (`routing.mjs:557`): clearance 2, at most 80
    /// obstacles, stub 24, 4096 nodes, crossings allowed, bend penalty 48.
    #[allow(clippy::too_many_arguments)]
    pub fn architecture(
        start: Pt,
        end: Pt,
        obstacles: &'a [Rect],
        from_side: Option<Side>,
        to_side: Option<Side>,
        points: &'a [Pt],
        avoided_segments: &'a [Seg],
        border_segments: &'a [Seg],
        interior_segment_px: f64,
    ) -> Self {
        let mut q = Self::new(start, end, points, obstacles, from_side, to_side, 2.0, 80);
        q.endpoint_stub_px = Some(24.0);
        q.maximum_grid_nodes = Some(4096);
        q.avoided_segments = avoided_segments;
        q.allow_avoided_crossings = true;
        q.border_segments = border_segments;
        q.minimum_segment_px = interior_segment_px;
        q.bend_penalty_px = 48.0;
        q
    }
}

/// How a search ended (the JS `metrics.status` strings).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GridStatus {
    Initializing,
    UnsupportedEndpointSide,
    ObstacleBudgetExceeded,
    NodeBudgetExceeded,
    EndpointBlocked,
    NoRoute,
    BrokenPredecessorChain,
    Routed,
}

impl GridStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            GridStatus::Initializing => "initializing",
            GridStatus::UnsupportedEndpointSide => "unsupported-endpoint-side",
            GridStatus::ObstacleBudgetExceeded => "obstacle-budget-exceeded",
            GridStatus::NodeBudgetExceeded => "node-budget-exceeded",
            GridStatus::EndpointBlocked => "endpoint-blocked",
            GridStatus::NoRoute => "no-route",
            GridStatus::BrokenPredecessorChain => "broken-predecessor-chain",
            GridStatus::Routed => "routed",
        }
    }
}

/// The counters the JS writes into its `metrics` object.
#[derive(Clone, Debug, PartialEq)]
pub struct GridMetrics {
    pub status: GridStatus,
    pub maximum_grid_nodes: Option<usize>,
    pub obstacle_count: usize,
    pub avoided_segment_count: usize,
    pub coordinate_count: usize,
    pub candidate_node_count: usize,
    pub usable_node_count: usize,
    pub graph_edge_count: usize,
    pub visited_node_count: usize,
}

/// A found route: normalised, bracketed by the true endpoints.
#[derive(Clone, Debug, PartialEq)]
pub struct GridRoute {
    pub points: Vec<Pt>,
    /// `None` when a segment is not axis-aligned (JS `null`); cannot happen for a grid route.
    pub length: Option<f64>,
    pub obstacle_count: usize,
}

#[derive(Clone, Debug)]
pub struct GridOutcome {
    pub route: Option<GridRoute>,
    pub metrics: GridMetrics,
}

/// `shortestOrthogonalGridRoute` without the metrics.
pub fn shortest_orthogonal_grid_route(q: &GridQuery) -> Option<GridRoute> {
    shortest_orthogonal_grid_route_with_metrics(q).route
}

// Directions are `R L D U` = 0 1 2 3 so that `d ^ 1` is the opposite.
const DIR_R: u8 = 0;
const DIR_D: u8 = 2;
const DIR_U: u8 = 3;

fn opposite(dir: u8) -> u8 {
    dir ^ 1
}

fn outward(side: Side) -> (f64, f64) {
    match side {
        Side::Left => (-1.0, 0.0),
        Side::Right => (1.0, 0.0),
        Side::Top => (0.0, -1.0),
        Side::Bottom => (0.0, 1.0),
    }
}

fn direction_of((dx, dy): (f64, f64)) -> u8 {
    if dx > 0.0 {
        DIR_R
    } else if dx < 0.0 {
        1
    } else if dy > 0.0 {
        DIR_D
    } else {
        DIR_U
    }
}

fn move_outward(point: Pt, side: Side, distance: f64) -> Pt {
    let (dx, dy) = outward(side);
    [point[0] + dx * distance, point[1] + dy * distance]
}

/// `orthogonalLength`: `None` when any segment is neither horizontal nor vertical (exact compare).
pub fn orthogonal_length(points: &[Pt]) -> Option<f64> {
    let mut total = 0.0;
    for w in points.windows(2) {
        let (a, b) = (w[0], w[1]);
        if a[0] != b[0] && a[1] != b[1] {
            return None;
        }
        total += (b[0] - a[0]).abs() + (b[1] - a[1]).abs();
    }
    Some(total)
}

/// This file's `collinearOverlap`: exact equality of the shared coordinate, overlap length or 0.
fn collinear_overlap(ls: Pt, le: Pt, rs: Pt, re: Pt) -> f64 {
    if ls[0] == le[0] && rs[0] == re[0] && ls[0] == rs[0] {
        return (ls[1].max(le[1]).min(rs[1].max(re[1])) - ls[1].min(le[1]).max(rs[1].min(re[1])))
            .max(0.0);
    }
    if ls[1] == le[1] && rs[1] == re[1] && ls[1] == rs[1] {
        return (ls[0].max(le[0]).min(rs[0].max(re[0])) - ls[0].min(le[0]).max(rs[0].min(re[0])))
            .max(0.0);
    }
    0.0
}

fn point_blocked(p: Pt, obstacles: &[Rect]) -> bool {
    obstacles
        .iter()
        .any(|r| p[0] >= r.x && p[0] <= r.x + r.width && p[1] >= r.y && p[1] <= r.y + r.height)
}

fn orthogonal_touch_on_avoided_interior(s: Pt, e: Pt, as_: Pt, ae: Pt) -> bool {
    let cand_h = (s[1] - e[1]).abs() <= EPS;
    let cand_v = (s[0] - e[0]).abs() <= EPS;
    let av_h = (as_[1] - ae[1]).abs() <= EPS;
    let av_v = (as_[0] - ae[0]).abs() <= EPS;
    if cand_h && av_v {
        let (x, y) = (as_[0], s[1]);
        return x >= s[0].min(e[0]) - EPS
            && x <= s[0].max(e[0]) + EPS
            && y > as_[1].min(ae[1]) + EPS
            && y < as_[1].max(ae[1]) - EPS;
    }
    if cand_v && av_h {
        let (x, y) = (s[0], as_[1]);
        return y >= s[1].min(e[1]) - EPS
            && y <= s[1].max(e[1]) + EPS
            && x > as_[0].min(ae[0]) + EPS
            && x < as_[0].max(ae[0]) - EPS;
    }
    false
}

fn conflicts_with_avoided(
    s: Pt,
    e: Pt,
    avoided: &[Seg],
    min_overlap: f64,
    allow_crossings: bool,
) -> bool {
    avoided.iter().any(|a| {
        (!allow_crossings
            && (proper_segment_intersection(s, e, a.start, a.end).is_some()
                || orthogonal_touch_on_avoided_interior(s, e, a.start, a.end)))
            || collinear_overlap(s, e, a.start, a.end) >= min_overlap
    })
}

fn point_on_segment_interior(p: Pt, s: Pt, e: Pt) -> bool {
    let cross = (e[0] - s[0]) * (p[1] - s[1]) - (e[1] - s[1]) * (p[0] - s[0]);
    if cross.abs() > EPS {
        return false;
    }
    let dot = (p[0] - s[0]) * (p[0] - e[0]) + (p[1] - s[1]) * (p[1] - e[1]);
    dot < -EPS
}

/// The JS binary min-heap, verbatim (see the module notes on ties).
struct MinHeap {
    entries: Vec<(usize, f64)>,
}

impl MinHeap {
    fn push(&mut self, key: usize, distance: f64) {
        let entry = (key, distance);
        self.entries.push(entry);
        let mut index = self.entries.len() - 1;
        while index > 0 {
            let parent = (index - 1) / 2;
            if self.entries[parent].1 <= distance {
                break;
            }
            self.entries[index] = self.entries[parent];
            index = parent;
        }
        self.entries[index] = entry;
    }

    fn pop(&mut self) -> Option<(usize, f64)> {
        if self.entries.is_empty() {
            return None;
        }
        let first = self.entries[0];
        let last = self.entries.pop()?;
        if self.entries.is_empty() {
            return Some(first);
        }
        let len = self.entries.len();
        let mut index = 0;
        loop {
            let left = index * 2 + 1;
            let right = left + 1;
            if left >= len {
                break;
            }
            let child = if right < len && self.entries[right].1 < self.entries[left].1 {
                right
            } else {
                left
            };
            if self.entries[child].1 >= last.1 {
                break;
            }
            self.entries[index] = self.entries[child];
            index = child;
        }
        self.entries[index] = last;
        Some(first)
    }
}

/// A `Set` of numbers in insertion order.
fn push_unique(values: &mut Vec<f64>, x: f64) {
    if !values.contains(&x) {
        values.push(x);
    }
}

/// Sort (stable) and coalesce lines closer than `min` to their *sorted* predecessor, except `keep`.
fn coalesce(mut values: Vec<f64>, keep: [f64; 2], min: f64) -> Vec<f64> {
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    values
        .iter()
        .enumerate()
        .filter(|(i, v)| *i == 0 || keep.contains(v) || **v - values[i - 1] >= min)
        .map(|(_, v)| *v)
        .collect()
}

/// `shortestOrthogonalGridRoute`, with the counters it writes into `metrics`.
pub fn shortest_orthogonal_grid_route_with_metrics(q: &GridQuery) -> GridOutcome {
    let mut m = GridMetrics {
        status: GridStatus::Initializing,
        maximum_grid_nodes: q.maximum_grid_nodes,
        obstacle_count: 0,
        avoided_segment_count: 0,
        coordinate_count: 0,
        candidate_node_count: 0,
        usable_node_count: 0,
        graph_edge_count: 0,
        visited_node_count: 0,
    };
    let fail = |mut m: GridMetrics, status: GridStatus| {
        m.status = status;
        GridOutcome {
            route: None,
            metrics: m,
        }
    };
    let (Some(from_side), Some(to_side)) = (q.from_side, q.to_side) else {
        return fail(m, GridStatus::UnsupportedEndpointSide);
    };
    let stub_px = q.endpoint_stub_px.unwrap_or(q.clearance + 2.0);
    let start_stub = move_outward(q.start, from_side, stub_px);
    let end_stub = move_outward(q.end, to_side, stub_px);

    let expanded: Vec<Rect> = q
        .obstacles
        .iter()
        .filter(|r| r.is_finite())
        .map(|r| r.expanded(q.clearance))
        .collect();
    let avoided = q.avoided_segments;
    let borders = q.border_segments;
    m.obstacle_count = expanded.len();
    m.avoided_segment_count = avoided.len();
    if expanded.len() > q.maximum_obstacle_count {
        return fail(m, GridStatus::ObstacleBudgetExceeded);
    }

    let mut xs: Vec<f64> = Vec::new();
    let mut ys: Vec<f64> = Vec::new();
    push_unique(&mut xs, start_stub[0]);
    push_unique(&mut xs, end_stub[0]);
    push_unique(&mut ys, start_stub[1]);
    push_unique(&mut ys, end_stub[1]);
    for p in q.points {
        push_unique(&mut xs, p[0]);
    }
    for p in q.points {
        push_unique(&mut ys, p[1]);
    }
    for r in &expanded {
        push_unique(&mut xs, r.x - 1.0);
        push_unique(&mut xs, r.x + r.width + 1.0);
        push_unique(&mut ys, r.y - 1.0);
        push_unique(&mut ys, r.y + r.height + 1.0);
    }
    let sep = q.route_separation_px;
    for s in avoided {
        push_unique(&mut xs, s.start[0]);
        push_unique(&mut xs, s.end[0]);
        push_unique(&mut ys, s.start[1]);
        push_unique(&mut ys, s.end[1]);
        if (s.start[0] - s.end[0]).abs() <= EPS {
            push_unique(&mut xs, s.start[0] - sep);
            push_unique(&mut xs, s.start[0] + sep);
        }
        if (s.start[1] - s.end[1]).abs() <= EPS {
            push_unique(&mut ys, s.start[1] - sep);
            push_unique(&mut ys, s.start[1] + sep);
        }
    }
    for s in borders {
        if (s.start[0] - s.end[0]).abs() <= EPS {
            push_unique(&mut xs, s.start[0] - sep);
            push_unique(&mut xs, s.start[0] + sep);
        }
        if (s.start[1] - s.end[1]).abs() <= EPS {
            push_unique(&mut ys, s.start[1] - sep);
            push_unique(&mut ys, s.start[1] + sep);
        }
    }
    let ordered_x = coalesce(xs, [start_stub[0], end_stub[0]], q.minimum_segment_px);
    let ordered_y = coalesce(ys, [start_stub[1], end_stub[1]], q.minimum_segment_px);
    let (nx, ny) = (ordered_x.len(), ordered_y.len());
    m.coordinate_count = nx + ny;
    m.candidate_node_count = nx * ny;
    if q.maximum_grid_nodes
        .is_some_and(|max| m.candidate_node_count > max)
    {
        return fail(m, GridStatus::NodeBudgetExceeded);
    }

    // Node id = ix * ny + iy: JS `Map` insertion order (x-major).
    let pt = |id: usize| -> Pt { [ordered_x[id / ny], ordered_y[id % ny]] };
    let mut usable = vec![false; nx * ny];
    for ix in 0..nx {
        for iy in 0..ny {
            let p = [ordered_x[ix], ordered_y[iy]];
            if !point_blocked(p, &expanded)
                && (q.allow_avoided_crossings
                    || !avoided
                        .iter()
                        .any(|s| point_on_segment_interior(p, s.start, s.end)))
            {
                usable[ix * ny + iy] = true;
            }
        }
    }
    m.usable_node_count = usable.iter().filter(|u| **u).count();
    let find = |axis: &[f64], v: f64| axis.iter().position(|a| *a == v);
    let node_of = |p: Pt| -> Option<usize> {
        let ix = find(&ordered_x, p[0])?;
        let iy = find(&ordered_y, p[1])?;
        usable[ix * ny + iy].then_some(ix * ny + iy)
    };
    let (Some(source), Some(target)) = (node_of(start_stub), node_of(end_stub)) else {
        return fail(m, GridStatus::EndpointBlocked);
    };

    // Neighbour lists: horizontal lines (left then right), then vertical lines (up then down).
    let mut adjacency: Vec<Vec<(usize, f64, u8)>> = vec![Vec::new(); nx * ny];
    let mut graph_edge_count = 0usize;
    let mut connect_line = |line: &[usize], horizontal: bool| {
        for w in line.windows(2) {
            let (left, right) = (pt(w[0]), pt(w[1]));
            let seg = Seg::new(left, right);
            if expanded
                .iter()
                .any(|r| segment_intersects_rect(&seg, r, 0.0))
            {
                continue;
            }
            if conflicts_with_avoided(
                left,
                right,
                avoided,
                q.minimum_avoided_overlap_px,
                q.allow_avoided_crossings,
            ) {
                continue;
            }
            if borders
                .iter()
                .any(|b| collinear_overlap(left, right, b.start, b.end) > EPS)
            {
                continue;
            }
            let distance = (right[0] - left[0]).abs() + (right[1] - left[1]).abs();
            let (fwd, back) = if horizontal {
                (DIR_R, 1)
            } else {
                (DIR_D, DIR_U)
            };
            adjacency[w[0]].push((w[1], distance, fwd));
            adjacency[w[1]].push((w[0], distance, back));
            graph_edge_count += 1;
        }
    };
    for iy in 0..ny {
        let line: Vec<usize> = (0..nx)
            .map(|ix| ix * ny + iy)
            .filter(|id| usable[*id])
            .collect();
        connect_line(&line, true);
    }
    for ix in 0..nx {
        let line: Vec<usize> = (0..ny)
            .map(|iy| ix * ny + iy)
            .filter(|id| usable[*id])
            .collect();
        connect_line(&line, false);
    }
    m.graph_edge_count = graph_edge_count;

    let source_dir = direction_of(outward(from_side));
    let target_dir = opposite(direction_of(outward(to_side)));
    let bend = q.bend_penalty_px;
    let source_state = source * 4 + source_dir as usize;
    let mut distances = vec![f64::INFINITY; nx * ny * 4];
    let mut previous = vec![usize::MAX; nx * ny * 4];
    distances[source_state] = 0.0;
    let mut queue = MinHeap {
        entries: Vec::new(),
    };
    queue.push(source_state, 0.0);
    let mut visited = 0usize;
    let mut target_state: Option<(usize, f64)> = None;
    while let Some((current, current_distance)) = queue.pop() {
        if current_distance != distances[current] {
            continue;
        }
        visited += 1;
        let (node, dir) = (current / 4, (current % 4) as u8);
        if node == target {
            // Arriving on the wrong axis costs one final turn onto the end stub.
            let arrival = current_distance + if dir == target_dir { 0.0 } else { bend };
            if target_state.is_none_or(|t| arrival < t.1) {
                target_state = Some((current, arrival));
            }
            if dir == target_dir || bend == 0.0 {
                break;
            }
            continue;
        }
        if target_state.is_some_and(|t| current_distance >= t.1) {
            break;
        }
        for &(neighbor, weight, axis) in &adjacency[node] {
            if axis == opposite(dir) {
                continue;
            }
            let candidate = current_distance + weight + if axis == dir { 0.0 } else { bend };
            let ns = neighbor * 4 + axis as usize;
            if candidate >= distances[ns] {
                continue;
            }
            distances[ns] = candidate;
            previous[ns] = current;
            queue.push(ns, candidate);
        }
    }
    m.visited_node_count = visited;
    let Some((target_key, _)) = target_state else {
        return fail(m, GridStatus::NoRoute);
    };
    let mut reversed: Vec<Pt> = Vec::new();
    let mut key = target_key;
    loop {
        reversed.push(pt(key / 4));
        if key == source_state || previous[key] == usize::MAX {
            break;
        }
        key = previous[key];
    }
    if key / 4 != source {
        return fail(m, GridStatus::BrokenPredecessorChain);
    }
    reversed.reverse();
    let mut full = Vec::with_capacity(reversed.len() + 2);
    full.push(q.start);
    full.extend(reversed);
    full.push(q.end);
    let points = normalize(&full);
    m.status = GridStatus::Routed;
    let length = orthogonal_length(&points);
    GridOutcome {
        route: Some(GridRoute {
            points,
            length,
            obstacle_count: expanded.len(),
        }),
        metrics: m,
    }
}

// ---------------------------------------------------------------------------------------------
// The detour oracle: `cleanRouteDetourProblems`, minus the diagnostic recording (the
// `composition/excessive-route-detour` gate of P3.6 turns a finding into a diagnostic).
// ---------------------------------------------------------------------------------------------

/// The `DEFAULTS` of route-quality.mjs, overridable per call (`thresholds`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DetourThresholds {
    pub clearance: f64,
    pub minimum_detour_ratio: f64,
    pub minimum_excess_length_px: f64,
    pub minimum_empty_excursion_px: f64,
    pub maximum_obstacle_count: usize,
    pub shared_corridor_minimum_px: f64,
}

impl Default for DetourThresholds {
    fn default() -> Self {
        Self {
            clearance: 2.0,
            minimum_detour_ratio: 2.5,
            minimum_excess_length_px: 200.0,
            minimum_empty_excursion_px: 96.0,
            maximum_obstacle_count: 80,
            shared_corridor_minimum_px: 32.0,
        }
    }
}

/// A relationship as the oracle reads it, already resolved by the caller (`pathFor`, `fromSideFor`,
/// `toSideFor`).
#[derive(Clone, Debug)]
pub struct DetourRelation {
    pub id: Option<String>,
    pub from: String,
    pub to: String,
    /// `via` is a non-empty array: only authored corridors are judged.
    pub has_via: bool,
    /// The resolved route (`pathFor(relation).points`), un-normalised; empty when there is none.
    pub points: Vec<Pt>,
    /// `None` falls back to the side inferred from the route's end segments.
    pub from_side: Option<Side>,
    pub to_side: Option<Side>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds {
    pub left: f64,
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
    pub width: f64,
    pub height: f64,
}

/// How far the route reaches beyond the content bounds on each side (`outsideExcursion`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Excursion {
    pub left: f64,
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
    pub maximum: f64,
}

/// The control point farthest from every rect (`emptyControlPointClearance`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ControlClearance {
    pub maximum: f64,
    pub point: Pt,
}

/// One `composition/excessive-route-detour` finding with the evidence the diagnostic carries.
#[derive(Clone, Debug, PartialEq)]
pub struct DetourFinding {
    pub relation_index: usize,
    pub id: Option<String>,
    pub from: String,
    pub to: String,
    pub points: Vec<Pt>,
    pub actual_length: f64,
    pub shortest_points: Vec<Pt>,
    pub shortest_length: f64,
    pub detour_ratio: f64,
    pub excess_length: f64,
    pub route_bounds: Bounds,
    pub content_bounds: Option<Bounds>,
    pub excursion: Option<Excursion>,
    pub empty_clearance: Option<ControlClearance>,
    pub obstacle_count: usize,
    pub thresholds: DetourThresholds,
}

impl DetourFinding {
    /// The JS message, for `diagramType` and `relationCollection` (`connections`, `flows`, ...).
    pub fn message(&self, diagram_type: &str, collection: &str) -> String {
        let rounded = |v: f64| js_round(v * 100.0) / 100.0;
        let id = self
            .id
            .as_ref()
            .map(|id| format!(" id \"{id}\""))
            .unwrap_or_default();
        let beyond = self.excursion.map_or(0.0, |e| e.maximum);
        format!(
            "[composition/excessive-route-detour] {diagram_type} {collection}[{}]{id} \"{}\" -> \"{}\" travels {}px, {}x the {}px shortest obstacle-clearing orthogonal route, and reaches {}px beyond the content bounds \u{2014} remove the distant via corridor or move it close to the connected content.",
            self.relation_index,
            self.from,
            self.to,
            js_round(self.actual_length),
            rounded(self.detour_ratio),
            js_round(self.shortest_length),
            js_round(beyond),
        )
    }
}

fn usable_rects(rects: &[Rect]) -> Vec<Rect> {
    rects
        .iter()
        .copied()
        .filter(|r| r.is_finite() && r.width >= 0.0 && r.height >= 0.0)
        .collect()
}

/// `boundsForRects`.
pub fn bounds_for_rects(rects: &[Rect]) -> Option<Bounds> {
    let usable = usable_rects(rects);
    if usable.is_empty() {
        return None;
    }
    let left = usable.iter().map(|r| r.x).fold(f64::INFINITY, f64::min);
    let top = usable.iter().map(|r| r.y).fold(f64::INFINITY, f64::min);
    let right = usable
        .iter()
        .map(|r| r.x + r.width)
        .fold(f64::NEG_INFINITY, f64::max);
    let bottom = usable
        .iter()
        .map(|r| r.y + r.height)
        .fold(f64::NEG_INFINITY, f64::max);
    Some(Bounds {
        left,
        top,
        right,
        bottom,
        width: right - left,
        height: bottom - top,
    })
}

fn bounds_for_points(points: &[Pt]) -> Option<Bounds> {
    if points.is_empty() {
        return None;
    }
    let left = points.iter().map(|p| p[0]).fold(f64::INFINITY, f64::min);
    let top = points.iter().map(|p| p[1]).fold(f64::INFINITY, f64::min);
    let right = points
        .iter()
        .map(|p| p[0])
        .fold(f64::NEG_INFINITY, f64::max);
    let bottom = points
        .iter()
        .map(|p| p[1])
        .fold(f64::NEG_INFINITY, f64::max);
    Some(Bounds {
        left,
        top,
        right,
        bottom,
        width: right - left,
        height: bottom - top,
    })
}

fn outside_excursion(route: &Bounds, content: &Bounds) -> Excursion {
    let left = (content.left - route.left).max(0.0);
    let top = (content.top - route.top).max(0.0);
    let right = (route.right - content.right).max(0.0);
    let bottom = (route.bottom - content.bottom).max(0.0);
    Excursion {
        left,
        top,
        right,
        bottom,
        maximum: left.max(top).max(right).max(bottom),
    }
}

/// Note the JS metric: `dx + dy`, not the Euclidean distance.
fn point_distance_from_rect(p: Pt, r: &Rect) -> f64 {
    let dx = (r.x - p[0]).max(0.0).max(p[0] - (r.x + r.width));
    let dy = (r.y - p[1]).max(0.0).max(p[1] - (r.y + r.height));
    dx + dy
}

fn empty_control_point_clearance(points: &[Pt], content: &[Rect]) -> Option<ControlClearance> {
    let rects = usable_rects(content);
    if points.len() < 3 || rects.is_empty() {
        return None;
    }
    let controls = &points[1..points.len() - 1];
    let distances: Vec<f64> = controls
        .iter()
        .map(|p| {
            rects
                .iter()
                .map(|r| point_distance_from_rect(*p, r))
                .fold(f64::INFINITY, f64::min)
        })
        .collect();
    let maximum = distances.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    // `distances.indexOf(maximum)`: the first control point that reaches it.
    let index = distances.iter().position(|d| *d == maximum)?;
    Some(ControlClearance {
        maximum,
        point: controls[index],
    })
}

/// `inferredSide`: the side a route's end segment implies (exact axis test, as the JS).
pub fn inferred_side(points: &[Pt], source: bool) -> Option<Side> {
    if points.len() < 2 {
        return None;
    }
    let (a, b) = if source {
        (points[0], points[1])
    } else {
        (points[points.len() - 2], points[points.len() - 1])
    };
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    if source {
        if dx > 0.0 && dy == 0.0 {
            return Some(Side::Right);
        }
        if dx < 0.0 && dy == 0.0 {
            return Some(Side::Left);
        }
        if dy > 0.0 && dx == 0.0 {
            return Some(Side::Bottom);
        }
        if dy < 0.0 && dx == 0.0 {
            return Some(Side::Top);
        }
    } else {
        if dx > 0.0 && dy == 0.0 {
            return Some(Side::Left);
        }
        if dx < 0.0 && dy == 0.0 {
            return Some(Side::Right);
        }
        if dy > 0.0 && dx == 0.0 {
            return Some(Side::Top);
        }
        if dy < 0.0 && dx == 0.0 {
            return Some(Side::Bottom);
        }
    }
    None
}

fn segment_outside_content(s: Pt, e: Pt, content: &Option<Bounds>) -> bool {
    let Some(b) = content else { return false };
    let mid = [(s[0] + e[0]) / 2.0, (s[1] + e[1]) / 2.0];
    mid[0] < b.left || mid[0] > b.right || mid[1] < b.top || mid[1] > b.bottom
}

fn shares_outer_corridor(
    index: usize,
    relations: &[DetourRelation],
    points: &[Pt],
    content: &Option<Bounds>,
    minimum_overlap: f64,
) -> bool {
    let relation = &relations[index];
    for (other_index, other) in relations.iter().enumerate() {
        if other_index == index {
            continue;
        }
        let related = relation.from == other.from
            || relation.from == other.to
            || relation.to == other.from
            || relation.to == other.to;
        if !related {
            continue;
        }
        let other_points = normalize(&other.points);
        for l in points.windows(2) {
            if !segment_outside_content(l[0], l[1], content) {
                continue;
            }
            for r in other_points.windows(2) {
                if collinear_overlap(l[0], l[1], r[0], r[1]) >= minimum_overlap {
                    return true;
                }
            }
        }
    }
    false
}

/// `cleanRouteDetourProblems`: conspicuous authored `via` detours, in relation order.
///
/// `showcase` is `profile === 'showcase'` (anything else returns nothing). `content_rects` defaults
/// to `obstacles`. Differs from the JS only where it would throw: a finding with no content bounds
/// reports an excursion of 0 in its message instead.
pub fn clean_route_detour_problems(
    relations: &[DetourRelation],
    obstacles: &[Rect],
    content_rects: Option<&[Rect]>,
    endpoint_ids: &HashSet<String>,
    showcase: bool,
    thresholds: &DetourThresholds,
) -> Vec<DetourFinding> {
    if !showcase {
        return Vec::new();
    }
    let content_bounds = bounds_for_rects(content_rects.unwrap_or(obstacles));
    let mut problems = Vec::new();
    for (relation_index, relation) in relations.iter().enumerate() {
        if !endpoint_ids.contains(&relation.from)
            || !endpoint_ids.contains(&relation.to)
            || !relation.has_via
        {
            continue;
        }
        let points = normalize(&relation.points);
        if points.len() < 3 {
            continue;
        }
        let Some(actual_length) = orthogonal_length(&points) else {
            continue;
        };
        let start = points[0];
        let end = points[points.len() - 1];
        let manhattan = (end[0] - start[0]).abs() + (end[1] - start[1]).abs();
        if actual_length < manhattan * thresholds.minimum_detour_ratio
            || actual_length - manhattan < thresholds.minimum_excess_length_px
        {
            continue;
        }
        let Some(route_bounds) = bounds_for_points(&points) else {
            continue;
        };
        let excursion = content_bounds
            .as_ref()
            .map(|c| outside_excursion(&route_bounds, c));
        let empty_clearance = empty_control_point_clearance(&points, obstacles);
        let reach = excursion
            .map_or(0.0, |e| e.maximum)
            .max(empty_clearance.map_or(0.0, |c| c.maximum));
        if reach < thresholds.minimum_empty_excursion_px {
            continue;
        }
        if shares_outer_corridor(
            relation_index,
            relations,
            &points,
            &content_bounds,
            thresholds.shared_corridor_minimum_px,
        ) {
            continue;
        }
        let from_side = relation.from_side.or_else(|| inferred_side(&points, true));
        let to_side = relation.to_side.or_else(|| inferred_side(&points, false));
        let mut q = GridQuery::new(
            start,
            end,
            &points,
            obstacles,
            from_side,
            to_side,
            thresholds.clearance,
            thresholds.maximum_obstacle_count,
        );
        q.bend_penalty_px = 0.0;
        let Some(shortest) = shortest_orthogonal_grid_route(&q) else {
            continue;
        };
        let Some(shortest_length) = shortest.length.filter(|l| l.is_finite() && *l > 0.0) else {
            continue;
        };
        let detour_ratio = actual_length / shortest_length;
        let excess_length = actual_length - shortest_length;
        if detour_ratio < thresholds.minimum_detour_ratio
            || excess_length < thresholds.minimum_excess_length_px
        {
            continue;
        }
        problems.push(DetourFinding {
            relation_index,
            id: relation.id.clone(),
            from: relation.from.clone(),
            to: relation.to.clone(),
            points,
            actual_length,
            shortest_points: shortest.points,
            shortest_length,
            detour_ratio,
            excess_length,
            route_bounds,
            content_bounds,
            excursion,
            empty_clearance,
            obstacle_count: shortest.obstacle_count,
            thresholds: *thresholds,
        });
    }
    problems
}
