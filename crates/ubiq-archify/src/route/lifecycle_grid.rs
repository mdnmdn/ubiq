//! The lifecycle v2 grid router (P4.2): `createLifecycleGridRouter` of
//! `renderers/lifecycle/grid-routing.mjs`, line for line. 00 section 5.6.5; `03` section 3.4.
//!
//! Plan kinds `horizontal`, `vertical`, `channel`, `corridor` and `loop`; ports per `(state,
//! side)`; the snap of near-aligned ends; one track per run in each row gap (interval colouring,
//! classes permuted exhaustively up to 7, else a pairwise hill-climb); up to 12 nudge rounds.
//!
//! # JS parity
//!
//! - Every JS `Map` is iterated in insertion order. `plans`, `sideEnds`, `corridorUse`,
//!   `runsByGap` and `pathCache` are `Vec`s here, appended in the same order (a later `set` on an
//!   existing key keeps the key's slot, as `Map.set` does).
//! - Every `Array.prototype.sort` is stable (`sort_by` is too) and the comparators are the JS
//!   ones, ties included.
//! - `permutations(items)` enumerates in lexicographic index order (the recursive
//!   `flatMap` of the source); `.slice(1)` skips the identity and a strictly lower score wins, so
//!   the first best order is kept.
//! - A transition is identified by its index in the planned list (`transitions.indexOf`).

use std::collections::HashMap;

use crate::geom::{Pt, Rect, Side};

const PORT_GUTTER: f64 = 16.0;
const PORT_SPACING: f64 = 30.0;
const SNAP_LIMIT: f64 = 16.0;
const TRACK_TOP_CLEARANCE: f64 = 18.0;
const TRACK_BOTTOM_CLEARANCE: f64 = 18.0;
const PREFERRED_TRACK_SPACING: f64 = 22.0;
const MAX_TRACK_SPACING: f64 = 24.0;
const CORRIDOR_SPACING: f64 = 10.0;
const EXHAUSTIVE_TRACK_LIMIT: usize = 7;

/// A measured state as the router sees it (one entry of the `states` Map, in Map order).
#[derive(Clone, Debug, PartialEq)]
pub struct GridState {
    pub id: String,
    pub rect: Rect,
    /// `state.cx` as measured (the column centre; `x + width / 2` may differ in the last ulp).
    pub cx: f64,
    /// `rowOf(state)`: the index of its lane among the rendered rows, if any.
    pub row: Option<usize>,
    /// `state.type === 'start'` (the initial marker blocks a left loop).
    pub start: bool,
}

impl GridState {
    fn cx(&self) -> f64 {
        self.cx
    }
    fn cy(&self) -> f64 {
        self.rect.cy()
    }
}

/// A planned transition: its endpoints as indices into the state list (`states.get`).
#[derive(Clone, Debug, PartialEq)]
pub struct GridTransition {
    pub from: Option<usize>,
    pub to: Option<usize>,
    /// `transition.label || transition.note` (labelled edges never loop).
    pub labelled: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Horizontal,
    Vertical,
    Channel,
    Corridor,
    Loop,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Leg {
    Up,
    Down,
}

#[derive(Clone, Copy, Debug)]
struct Run {
    gap: usize,
    legs: [Leg; 2],
}

#[derive(Clone, Debug)]
struct Plan {
    transition: usize,
    kind: Kind,
    from: usize,
    to: usize,
    from_side: Side,
    to_side: Side,
    runs: Vec<Run>,
    corridor: f64,
    corridor_x: f64,
    loop_side: Side,
    loop_x: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum End {
    Source,
    Target,
}

#[derive(Clone, Copy, Debug)]
struct SideEnd {
    plan: usize,
    end: End,
    heading: f64,
    positive: bool,
}

struct SideEnds {
    state: usize,
    side: Side,
    ends: Vec<SideEnd>,
}

#[derive(Clone, Copy, Debug)]
struct GapRun {
    plan: usize,
    index: usize,
    x1: f64,
    x2: f64,
    legs: [Leg; 2],
}

fn opposite(side: Side) -> Side {
    match side {
        Side::Top => Side::Bottom,
        Side::Bottom => Side::Top,
        Side::Left => Side::Right,
        Side::Right => Side::Left,
    }
}

/// `permutations(items)` as index orders, in the source's (lexicographic) order.
fn permutations(n: usize) -> Vec<Vec<usize>> {
    fn go(items: &[usize]) -> Vec<Vec<usize>> {
        if items.len() <= 1 {
            return vec![items.to_vec()];
        }
        let mut out = Vec::new();
        for (index, item) in items.iter().enumerate() {
            let rest: Vec<usize> = items[..index].iter().chain(&items[index + 1..]).copied().collect();
            for tail in go(&rest) {
                let mut order = vec![*item];
                order.extend(tail);
                out.push(order);
            }
        }
        out
    }
    go(&(0..n).collect::<Vec<_>>())
}

/// The router: plans, ports, tracks and the cached routes.
#[derive(Clone, Debug)]
pub struct LifecycleGrid {
    plans: Vec<Plan>,
    /// Transition index to plan index.
    plan_of: Vec<Option<usize>>,
    /// `[source, target]` per plan.
    ports: Vec<[Pt; 2]>,
    track_counts: HashMap<usize, usize>,
    track_y: HashMap<(usize, usize, usize), f64>,
    /// `pathCache` for planned transitions, by plan index.
    paths: Vec<Vec<Pt>>,
    /// The degenerate stub of an unplanned transition.
    stubs: Vec<Option<Vec<Pt>>>,
}

impl LifecycleGrid {
    /// `createLifecycleGridRouter(states, transitions, { rowOf, columnXs })`.
    pub fn new(states: &[GridState], transitions: &[GridTransition], column_xs: &[f64]) -> Self {
        // byRow, in state order; rows ascending.
        let mut by_row: Vec<(usize, Vec<usize>)> = Vec::new();
        for (i, state) in states.iter().enumerate() {
            let Some(row) = state.row else { continue };
            match by_row.iter_mut().find(|(r, _)| *r == row) {
                Some((_, list)) => list.push(i),
                None => by_row.push((row, vec![i])),
            }
        }
        let mut rows: Vec<usize> = by_row.iter().map(|(r, _)| *r).collect();
        rows.sort_unstable();
        let members = |row: usize| -> &[usize] {
            by_row.iter().find(|(r, _)| *r == row).map(|(_, l)| l.as_slice()).unwrap_or(&[])
        };
        let row_top: HashMap<usize, f64> = rows
            .iter()
            .map(|&r| (r, members(r).iter().map(|&i| states[i].rect.y).fold(f64::INFINITY, f64::min)))
            .collect();
        let row_bottom: HashMap<usize, f64> = rows
            .iter()
            .map(|&r| (r, members(r).iter().map(|&i| states[i].rect.bottom()).fold(f64::NEG_INFINITY, f64::max)))
            .collect();
        let next_row = |row: usize| rows.iter().copied().find(|&c| c > row);
        let previous_row = |row: usize| rows.iter().rev().copied().find(|&c| c < row);

        // Gap g sits below row g; the gap under the last row borders the legend.
        let gap_band = |row: usize| -> (f64, f64) {
            let top = row_bottom[&row] + TRACK_TOP_CLEARANCE;
            let bottom = match next_row(row) {
                None => row_bottom[&row] + 40.0,
                Some(below) => row_top[&below] - TRACK_BOTTOM_CLEARANCE,
            };
            (top, top.max(bottom))
        };

        let blocks_vertical = |x: f64, from_row: usize, to_row: usize, exclude: &[usize]| -> bool {
            let (low, high) = if from_row < to_row { (from_row, to_row) } else { (to_row, from_row) };
            states.iter().enumerate().any(|(i, s)| {
                if exclude.contains(&i) {
                    return false;
                }
                s.row.is_some_and(|row| row > low && row < high)
                    && x >= s.rect.x - 6.0
                    && x <= s.rect.right() + 6.0
            })
        };

        let blocks_horizontal = |from: usize, to: usize| -> bool {
            let row = states[from].row.unwrap_or(0);
            let (left, right) = if states[from].cx() < states[to].cx() { (from, to) } else { (to, from) };
            let (l, r) = (&states[left].rect, &states[right].rect);
            members(row).iter().any(|&i| {
                let s = &states[i].rect;
                i != left
                    && i != right
                    && s.x < r.x
                    && s.right() > l.right()
                    && s.y < l.bottom().max(r.bottom())
                    && s.bottom() > l.y.min(r.y)
            })
        };

        // The empty corridors between neighbouring columns.
        let corridor_xs: Vec<f64> = column_xs.windows(2).map(|w| (w[1] + w[0]) / 2.0).collect();
        let min_of = |f: &dyn Fn(&GridState) -> f64| states.iter().map(f).fold(f64::INFINITY, f64::min);
        let max_of = |f: &dyn Fn(&GridState) -> f64| states.iter().map(f).fold(f64::NEG_INFINITY, f64::max);
        let leftmost_x = min_of(&|s| s.cx());
        let rightmost_x = max_of(&|s| s.cx());
        let grid_left = min_of(&|s| s.rect.x);
        let grid_right = max_of(&|s| s.rect.right());

        // The initial-state marker occupies a start state's left side.
        let loop_blocked = |from: usize, to: usize, side: Side| -> bool {
            if side != Side::Left {
                return false;
            }
            let (fr, tr) = (states[from].row.unwrap_or(0), states[to].row.unwrap_or(0));
            let (low, high) = (fr.min(tr), fr.max(tr));
            states.iter().any(|s| {
                s.start
                    && (s.cx() - states[from].cx()).abs() < 1.0
                    && s.row.is_some_and(|r| r >= low && r <= high)
            })
        };

        // Plan: sides, the gap each horizontal run uses, and where each end heads.
        let mut plans: Vec<Plan> = Vec::new();
        let mut plan_of = vec![None; transitions.len()];
        for (ti, t) in transitions.iter().enumerate() {
            let (Some(from), Some(to)) = (t.from, t.to) else { continue };
            if from == to {
                continue;
            }
            let (Some(from_row), Some(to_row)) = (states[from].row, states[to].row) else { continue };
            let exclude = [from, to];
            let mut plan = Plan {
                transition: ti,
                kind: Kind::Horizontal,
                from,
                to,
                from_side: Side::Right,
                to_side: Side::Left,
                runs: Vec::new(),
                corridor: 0.0,
                corridor_x: 0.0,
                loop_side: Side::Left,
                loop_x: 0.0,
            };
            let (fcx, tcx) = (states[from].cx(), states[to].cx());
            if from_row == to_row {
                if !blocks_horizontal(from, to) {
                    let from_side = if tcx > fcx { Side::Right } else { Side::Left };
                    plan.from_side = from_side;
                    plan.to_side = opposite(from_side);
                } else {
                    let gap_row = previous_row(from_row);
                    let side = if gap_row.is_some() { Side::Top } else { Side::Bottom };
                    plan.kind = Kind::Channel;
                    plan.from_side = side;
                    plan.to_side = side;
                    plan.runs = vec![match gap_row {
                        Some(g) => Run { gap: g, legs: [Leg::Down, Leg::Down] },
                        None => Run { gap: from_row, legs: [Leg::Up, Leg::Up] },
                    }];
                }
                plan_of[ti] = Some(plans.len());
                plans.push(plan);
                continue;
            }
            let down = to_row > from_row;
            plan.from_side = if down { Side::Bottom } else { Side::Top };
            plan.to_side = if down { Side::Top } else { Side::Bottom };
            let gap_near_target = if down { previous_row(to_row) } else { Some(to_row) };
            let gap_near_source = if down { Some(from_row) } else { previous_row(from_row) };
            let legs = if down { [Leg::Up, Leg::Down] } else { [Leg::Down, Leg::Up] };
            let run = |gap: Option<usize>| Run { gap: gap.unwrap_or(0), legs };
            let from_clear = !blocks_vertical(fcx, from_row, to_row, &exclude);
            if from_clear && (fcx - tcx).abs() < 1.0 {
                plan.kind = Kind::Vertical;
                plan.runs = vec![run(gap_near_target)];
            } else if from_clear {
                plan.kind = Kind::Channel;
                plan.runs = vec![run(gap_near_target)];
            } else if !blocks_vertical(tcx, from_row, to_row, &exclude) {
                plan.kind = Kind::Channel;
                plan.runs = vec![run(gap_near_source)];
            } else if !t.labelled
                && (fcx - tcx).abs() < 1.0
                && (fcx <= leftmost_x || fcx >= rightmost_x)
                && !loop_blocked(from, to, if fcx <= leftmost_x { Side::Left } else { Side::Right })
            {
                // A blocked edge column loops around the outside of the grid.
                let side = if fcx <= leftmost_x { Side::Left } else { Side::Right };
                plan.kind = Kind::Loop;
                plan.from_side = side;
                plan.to_side = side;
                plan.loop_side = side;
            } else {
                let middle = (fcx + tcx) / 2.0;
                let mut open: Vec<f64> =
                    corridor_xs.iter().copied().filter(|&x| !blocks_vertical(x, from_row, to_row, &[])).collect();
                open.sort_by(|a, b| (a - middle).abs().total_cmp(&(b - middle).abs()));
                match open.first() {
                    None => {
                        plan.kind = Kind::Channel;
                        plan.runs = vec![run(gap_near_target)];
                    }
                    Some(&corridor) => {
                        plan.kind = Kind::Corridor;
                        plan.corridor = corridor;
                        plan.runs = vec![run(gap_near_source), run(gap_near_target)];
                    }
                }
            }
            plan_of[ti] = Some(plans.len());
            plans.push(plan);
        }

        // Corridor offsets: routes sharing one corridor run side by side (Map keyed by x).
        let mut corridor_use: Vec<(f64, Vec<usize>)> = Vec::new();
        for (pi, plan) in plans.iter().enumerate() {
            if plan.kind != Kind::Corridor {
                continue;
            }
            match corridor_use.iter_mut().find(|(x, _)| *x == plan.corridor) {
                Some((_, list)) => list.push(pi),
                None => corridor_use.push((plan.corridor, vec![pi])),
            }
        }
        for (x, mut list) in corridor_use {
            list.sort_by(|&a, &b| {
                let (pa, pb) = (&plans[a], &plans[b]);
                let d = states[pa.from].cx() - states[pb.from].cx();
                let d = if d != 0.0 { d } else { states[pa.to].cx() - states[pb.to].cx() };
                d.partial_cmp(&0.0).unwrap_or(std::cmp::Ordering::Equal)
            });
            let n = list.len() as f64;
            for (index, pi) in list.into_iter().enumerate() {
                plans[pi].corridor_x = x + (index as f64 - (n - 1.0) / 2.0) * CORRIDOR_SPACING;
            }
        }
        // Outer loops nest: a longer span sits further out so loops never cross.
        let span = |p: &Plan| (states[p.from].row.unwrap_or(0) as f64 - states[p.to].row.unwrap_or(0) as f64).abs();
        for side in [Side::Left, Side::Right] {
            let mut loops: Vec<usize> =
                (0..plans.len()).filter(|&i| plans[i].kind == Kind::Loop && plans[i].loop_side == side).collect();
            loops.sort_by(|&a, &b| span(&plans[a]).total_cmp(&span(&plans[b])));
            for (index, pi) in loops.into_iter().enumerate() {
                let offset = index as f64 * CORRIDOR_SPACING;
                plans[pi].loop_x = if side == Side::Left { grid_left - 18.0 - offset } else { grid_right + 18.0 + offset };
            }
        }

        // Where each end heads after leaving its side, used to order the ports.
        let heading_for = |plan: &Plan, end: End| -> f64 {
            let (this, other) = if end == End::Source { (plan.from, plan.to) } else { (plan.to, plan.from) };
            match plan.kind {
                Kind::Horizontal | Kind::Loop => states[other].cy(),
                Kind::Corridor => plan.corridor_x,
                _ if other == this => states[this].cx(),
                _ => states[other].cx(),
            }
        };

        let mut side_ends: Vec<SideEnds> = Vec::new();
        for (pi, plan) in plans.iter().enumerate() {
            for end in [End::Source, End::Target] {
                let (state, side) = if end == End::Source { (plan.from, plan.from_side) } else { (plan.to, plan.to_side) };
                let slot = match side_ends.iter().position(|e| e.state == state && e.side == side) {
                    Some(slot) => slot,
                    None => {
                        side_ends.push(SideEnds { state, side, ends: Vec::new() });
                        side_ends.len() - 1
                    }
                };
                // Movers in the positive direction (right/down) take the first slot.
                let (fr, tr) = (states[plan.from].row, states[plan.to].row);
                let positive = if plan.kind == Kind::Horizontal {
                    states[plan.to].cx() > states[plan.from].cx()
                } else {
                    tr > fr || (tr == fr && states[plan.to].cx() > states[plan.from].cx())
                };
                side_ends[slot].ends.push(SideEnd { plan: pi, end, heading: heading_for(plan, end), positive });
            }
        }

        let mut ports: Vec<[Pt; 2]> = vec![[[0.0, 0.0]; 2]; plans.len()];
        for group in side_ends.iter_mut() {
            group.ends.sort_by(|a, b| {
                let d = a.heading - b.heading;
                let d = if d != 0.0 { d } else { f64::from(u8::from(b.positive)) - f64::from(u8::from(a.positive)) };
                d.partial_cmp(&0.0).unwrap_or(std::cmp::Ordering::Equal)
            });
            let (state, cx) = (&states[group.state].rect, states[group.state].cx);
            let horizontal_side = matches!(group.side, Side::Top | Side::Bottom);
            let length = if horizontal_side { state.width } else { state.height };
            let gutter = PORT_GUTTER.min(length / 4.0);
            let n = group.ends.len();
            let spacing = if n > 1 { PORT_SPACING.min((length - gutter * 2.0) / (n - 1) as f64) } else { 0.0 };
            for (index, entry) in group.ends.iter().enumerate() {
                let offset = (index as f64 - (n as f64 - 1.0) / 2.0) * spacing;
                let point = if horizontal_side {
                    [cx + offset, if group.side == Side::Top { state.y } else { state.bottom() }]
                } else {
                    [if group.side == Side::Left { state.x } else { state.right() }, state.cy() + offset]
                };
                ports[entry.plan][entry.end as usize] = point;
            }
        }

        let mut router = LifecycleGrid {
            plans,
            plan_of,
            ports,
            track_counts: HashMap::new(),
            track_y: HashMap::new(),
            paths: Vec::new(),
            stubs: Vec::new(),
        };
        router.snap(states, &side_ends);
        router.tracks(&gap_band);
        router.nudge(states, &side_ends);
        router.stubs = transitions
            .iter()
            .enumerate()
            .map(|(ti, t)| {
                if router.plan_of[ti].is_some() {
                    return None;
                }
                // Self transitions and unknown endpoints are rejected by validation; a degenerate
                // stub keeps that diagnostic reachable.
                let point = t.from.or(t.to).map_or([0.0, 0.0], |i| [states[i].rect.right(), states[i].cy()]);
                Some(vec![point, point])
            })
            .collect();
        router
    }

    /// `sideRange(state, side)`.
    fn side_range(state: &Rect, side: Side) -> (f64, f64) {
        if matches!(side, Side::Top | Side::Bottom) {
            (state.x + PORT_GUTTER / 2.0, state.right() - PORT_GUTTER / 2.0)
        } else {
            (state.y + PORT_GUTTER / 2.0, state.bottom() - PORT_GUTTER / 2.0)
        }
    }

    /// `portsOnSide(state, side, except)`: the current ports of the other ends on that side.
    fn ports_on_side(&self, side_ends: &[SideEnds], state: usize, side: Side, except: usize) -> Vec<Pt> {
        side_ends
            .iter()
            .find(|g| g.state == state && g.side == side)
            .map(|g| {
                g.ends
                    .iter()
                    .filter(|e| e.plan != except)
                    .map(|e| self.ports[e.plan][e.end as usize])
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Straight connections whose spread ports landed a few px apart: move one end onto the other's
    /// line when that side still has room there.
    fn snap(&mut self, states: &[GridState], side_ends: &[SideEnds]) {
        for pi in 0..self.plans.len() {
            let plan = self.plans[pi].clone();
            let axis = usize::from(plan.kind == Kind::Horizontal);
            let record = self.ports[pi];
            let delta = (record[0][axis] - record[1][axis]).abs();
            if delta < 0.5
                || delta >= SNAP_LIMIT
                || !matches!(plan.kind, Kind::Horizontal | Kind::Vertical | Kind::Channel)
            {
                continue;
            }
            for (end, fixed) in [(End::Target, End::Source), (End::Source, End::Target)] {
                let (state, side) = if end == End::Source { (plan.from, plan.from_side) } else { (plan.to, plan.to_side) };
                let value = self.ports[pi][fixed as usize][axis];
                let (low, high) = Self::side_range(&states[state].rect, side);
                let crowded = self
                    .ports_on_side(side_ends, state, side, pi)
                    .iter()
                    .any(|p| (p[axis] - value).abs() < 10.0);
                if value >= low && value <= high && !crowded {
                    self.ports[pi][end as usize][axis] = value;
                    break;
                }
            }
        }
    }

    /// Horizontal runs per gap, then tracks ordered to minimise crossings.
    fn tracks(&mut self, gap_band: &dyn Fn(usize) -> (f64, f64)) {
        let mut runs_by_gap: Vec<(usize, Vec<GapRun>)> = Vec::new();
        for (pi, plan) in self.plans.iter().enumerate() {
            if matches!(plan.kind, Kind::Horizontal | Kind::Loop) {
                continue;
            }
            let [source, target] = self.ports[pi];
            for (index, run) in plan.runs.iter().enumerate() {
                let x1 = if index == 0 { source[0] } else { plan.corridor_x };
                let x2 = if plan.kind == Kind::Corridor && index == 0 { plan.corridor_x } else { target[0] };
                if plan.kind != Kind::Corridor && (x1 - x2).abs() < 0.5 {
                    continue;
                }
                let entry = GapRun { plan: pi, index, x1, x2, legs: run.legs };
                match runs_by_gap.iter_mut().find(|(g, _)| *g == run.gap) {
                    Some((_, list)) => list.push(entry),
                    None => runs_by_gap.push((run.gap, vec![entry])),
                }
            }
        }

        for (gap, runs) in &runs_by_gap {
            let (top, bottom) = gap_band(*gap);
            // Greedy interval colouring; with at most 7 runs every run gets its own class.
            let mut sorted: Vec<usize> = (0..runs.len()).collect();
            sorted.sort_by(|&a, &b| runs[a].x1.min(runs[a].x2).total_cmp(&runs[b].x1.min(runs[b].x2)));
            let mut classes: Vec<Vec<usize>> = Vec::new();
            for r in sorted {
                let run = &runs[r];
                let low = run.x1.min(run.x2) - 8.0;
                let high = run.x1.max(run.x2) + 8.0;
                let found = classes.iter().position(|members| {
                    members.iter().all(|&o| {
                        let other = &runs[o];
                        high < other.x1.min(other.x2) - 8.0 || low > other.x1.max(other.x2) + 8.0
                    })
                });
                match found {
                    Some(c) if runs.len() > EXHAUSTIVE_TRACK_LIMIT => classes[c].push(r),
                    _ => classes.push(vec![r]),
                }
            }
            let count = classes.len();
            self.track_counts.insert(*gap, count);
            let spacing = if count > 1 { MAX_TRACK_SPACING.min((bottom - top) / (count - 1) as f64) } else { 0.0 };
            let center = (top + bottom) / 2.0;
            let ys: Vec<f64> =
                (0..count).map(|index| center + (index as f64 - (count as f64 - 1.0) / 2.0) * spacing).collect();
            // `order[k]` is the class on track k.
            let crossings = |order: &[usize]| -> u64 {
                let mut y = vec![0.0; runs.len()];
                for (index, &class) in order.iter().enumerate() {
                    for &r in &classes[class] {
                        y[r] = ys[index];
                    }
                }
                let mut total = 0;
                for (ai, a) in runs.iter().enumerate() {
                    for (bi, b) in runs.iter().enumerate() {
                        if ai == bi || a.plan == b.plan {
                            continue;
                        }
                        let (low, high) = (a.x1.min(a.x2), a.x1.max(a.x2));
                        for (x, leg) in [(b.x1, b.legs[0]), (b.x2, b.legs[1])] {
                            // An up leg and a down leg on one x overlap when the down leg starts
                            // above where the up leg ends: that merges two routes.
                            for (ax, a_leg) in [(a.x1, a.legs[0]), (a.x2, a.legs[1])] {
                                if (ax - x).abs() < 1.0 && a_leg == Leg::Up && leg == Leg::Down && y[bi] < y[ai] {
                                    total += 100;
                                }
                            }
                            if x <= low + 0.5 || x >= high - 0.5 {
                                continue;
                            }
                            if (leg == Leg::Up && y[ai] < y[bi]) || (leg == Leg::Down && y[ai] > y[bi]) {
                                total += 1;
                            }
                        }
                    }
                }
                total
            };
            let mut best: Vec<usize> = (0..count).collect();
            let mut best_score = crossings(&best);
            if count <= EXHAUSTIVE_TRACK_LIMIT {
                for order in permutations(count).into_iter().skip(1) {
                    let score = crossings(&order);
                    if score < best_score {
                        best = order;
                        best_score = score;
                    }
                }
            } else {
                // Too many tracks to enumerate: swap pairs while that still helps.
                let mut improved = true;
                while improved {
                    improved = false;
                    'scan: for i in 0..count {
                        for j in i + 1..count {
                            let mut order = best.clone();
                            order.swap(i, j);
                            let score = crossings(&order);
                            if score < best_score {
                                best = order;
                                best_score = score;
                                improved = true;
                                break 'scan;
                            }
                        }
                    }
                }
            }
            for (index, &class) in best.iter().enumerate() {
                for &r in &classes[class] {
                    let run = &runs[r];
                    self.track_y.insert((*gap, run.index, self.plans[run.plan].transition), ys[index]);
                }
            }
        }
    }

    /// `pointsFor(transition)` for a planned transition.
    fn points_for(&self, pi: usize) -> Vec<Pt> {
        let plan = &self.plans[pi];
        let [source, target] = self.ports[pi];
        match plan.kind {
            Kind::Horizontal => {
                if (source[1] - target[1]).abs() < 0.5 {
                    return vec![source, target];
                }
                let x = (source[0] + target[0]) / 2.0;
                vec![source, [x, source[1]], [x, target[1]], target]
            }
            Kind::Loop => vec![source, [plan.loop_x, source[1]], [plan.loop_x, target[1]], target],
            _ => {
                let track_for = |index: usize| {
                    self.track_y.get(&(plan.runs[index].gap, index, plan.transition)).copied().unwrap_or(f64::NAN)
                };
                if plan.kind == Kind::Corridor {
                    let (y1, y2) = (track_for(0), track_for(1));
                    return vec![
                        source,
                        [source[0], y1],
                        [plan.corridor_x, y1],
                        [plan.corridor_x, y2],
                        [target[0], y2],
                        target,
                    ];
                }
                if (source[0] - target[0]).abs() < 0.5 {
                    return vec![source, target];
                }
                let y = track_for(0);
                vec![source, [source[0], y], [target[0], y], target]
            }
        }
    }

    /// Ports of two routes can still line up across a gap so their vertical runs share one line:
    /// nudge a turning route's end sideways on its side (at most 12 rounds).
    fn nudge(&mut self, states: &[GridState], side_ends: &[SideEnds]) {
        self.paths = (0..self.plans.len()).map(|pi| self.points_for(pi)).collect();
        struct Vertical {
            x: f64,
            low: f64,
            high: f64,
            index: usize,
            last: bool,
        }
        let verticals = |points: &[Pt]| -> Vec<Vertical> {
            points
                .windows(2)
                .enumerate()
                .filter(|(_, w)| (w[0][0] - w[1][0]).abs() < 0.5 && (w[0][1] - w[1][1]).abs() >= 0.5)
                .map(|(index, w)| Vertical {
                    x: w[0][0],
                    low: w[0][1].min(w[1][1]),
                    high: w[0][1].max(w[1][1]),
                    index,
                    last: index == points.len() - 2,
                })
                .collect()
        };
        for _round in 0..12 {
            let mut conflict: Option<[(usize, usize, bool); 2]> = None;
            'find: for i in 0..self.paths.len() {
                for j in i + 1..self.paths.len() {
                    let right_all = verticals(&self.paths[j]);
                    for left in verticals(&self.paths[i]) {
                        if let Some(right) = right_all.iter().find(|o| {
                            (o.x - left.x).abs() < 1.0 && o.high.min(left.high) - o.low.max(left.low) > 0.5
                        }) {
                            conflict = Some([(i, left.index, left.last), (j, right.index, right.last)]);
                            break 'find;
                        }
                    }
                }
            }
            let Some(conflict) = conflict else { break };
            let mut moved = false;
            for (pi, index, last) in conflict {
                let plan = self.plans[pi].clone();
                let end = if index == 0 {
                    End::Source
                } else if last {
                    End::Target
                } else {
                    continue;
                };
                if self.paths[pi].len() < 4 || !matches!(plan.kind, Kind::Channel | Kind::Corridor) {
                    continue;
                }
                let (state, side) = if end == End::Source { (plan.from, plan.from_side) } else { (plan.to, plan.to_side) };
                let (low, high) = Self::side_range(&states[state].rect, side);
                let current = self.ports[pi][end as usize][0];
                let others = self.ports_on_side(side_ends, state, side, pi);
                let x = [12.0, -12.0, 20.0, -20.0].iter().map(|d| current + d).find(|&c| {
                    c >= low && c <= high && !others.iter().any(|p| (p[0] - c).abs() < 10.0)
                });
                let Some(x) = x else { continue };
                self.ports[pi][end as usize][0] = x;
                self.paths[pi] = self.points_for(pi);
                moved = true;
                break;
            }
            if !moved {
                break;
            }
        }
    }

    /// `gapHeight(row)`: the height the gap below `row` needs for its tracks at a readable spacing.
    pub fn gap_height(&self, row: usize) -> f64 {
        let count = self.track_counts.get(&row).copied().unwrap_or(0);
        TRACK_TOP_CLEARANCE + TRACK_BOTTOM_CLEARANCE + count.saturating_sub(1) as f64 * PREFERRED_TRACK_SPACING
    }

    /// `connectionSides(transition)`; an unplanned transition reports `right`/`right`.
    pub fn connection_sides(&self, transition: usize) -> (Side, Side) {
        match self.plan_of.get(transition).copied().flatten() {
            Some(pi) => (self.plans[pi].from_side, self.plans[pi].to_side),
            None => (Side::Right, Side::Right),
        }
    }

    /// `pathFor(transition)`: the un-rounded route.
    pub fn path_for(&self, transition: usize) -> Vec<Pt> {
        match self.plan_of.get(transition).copied().flatten() {
            Some(pi) => self.paths[pi].clone(),
            None => self.stubs.get(transition).cloned().flatten().unwrap_or_else(|| vec![[0.0, 0.0]; 2]),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permutations_follow_the_recursive_js_order() {
        let p = permutations(3);
        assert_eq!(p, vec![vec![0, 1, 2], vec![0, 2, 1], vec![1, 0, 2], vec![1, 2, 0], vec![2, 0, 1], vec![2, 1, 0]]);
        assert_eq!(permutations(7).len(), 5040);
    }

    fn state(id: &str, x: f64, y: f64, row: usize) -> GridState {
        GridState { id: id.into(), rect: Rect::new(x, y, 140.0, 64.0), cx: x + 70.0, row: Some(row), start: false }
    }

    #[test]
    fn neighbours_go_straight_and_rows_turn_once_in_the_gap() {
        let states = [state("a", 60.0, 56.0, 0), state("b", 264.0, 56.0, 0), state("c", 264.0, 240.0, 1)];
        let t = |from, to| GridTransition { from: Some(from), to: Some(to), labelled: false };
        let grid = LifecycleGrid::new(&states, &[t(0, 1), t(0, 2), t(1, 2)], &[130.0, 334.0, 538.0, 742.0, 946.0]);
        assert_eq!(grid.path_for(0), vec![[200.0, 88.0], [264.0, 88.0]]);
        assert_eq!(grid.connection_sides(1), (Side::Bottom, Side::Top));
        let turned = grid.path_for(1);
        assert_eq!(turned.len(), 4);
        assert_eq!(turned[1][1], turned[2][1]);
        assert_eq!(grid.gap_height(0), 36.0 + 22.0 * (grid.track_counts[&0] as f64 - 1.0));
    }
}
