//! The deferred interaction motions on [`MotionState`] (M5-M8, `00` §6.2): the relationship
//! between two picked nodes, the semantic lens, and the route probe and journey. Each one raises
//! its owner flag, holds one dim and plays comets through the shared primitive, so priority is the
//! governor's (`Owner`: route > lens > relationship > intent > focus).
//!
//! - **Relationship** (M5): comets along the path A to B, the rest at .16. Blocked while a lens or
//!   a route is up (they own the budget); it clears the intent trace.
//! - **Lens** (M6): a kind or tag lit with its relationships, the rest at .11 (peers .62), comets on
//!   the matched edges (none past 24: quiet density). Choosing one ends a route and a relationship,
//!   as the viewer's `prepareForLens` does.
//! - **Route** (M7, M8): a path from a start node. [`MotionState::route_target`] resolves the
//!   shortest directed path and plays the probe (hop `i` after `i * 160 ms`, parked at .58);
//!   [`MotionState::route_step`] walks one outgoing edge at a time; [`MotionState::route_play`]
//!   is the journey: a tick-driven [`Dwell`] of 1100 ms per node, a 780 ms pulse on the edge just
//!   crossed, past/current/future at .62/1/.34, an explicit `Route` claim while it plays. Starting
//!   a route ends a lens and a relationship (`begin` of `route-probe.js`).
//!
//! Nothing here reads a clock: the caller passes `now`, and [`MotionState::tick`] advances the
//! journey with the frame delta (capped, so a gap while hidden cannot skip steps).

use serde::{Deserialize, Serialize};

use super::comet::{CometKind, Direction};
use super::dim::{DIM_LENS, DIM_RELATIONSHIP, DIM_ROUTE, DIM_ROUTE_FUTURE, DIM_ROUTE_PAST, Dim, DimKind};
use super::dwell::Dwell;
use super::lens::{self, LensFilter};
use super::owner::{Owner, Token};
use super::path::{self, labels_of};
use super::state::MotionState;
use super::step::STEP_DELAY_MS;
use super::timeline::Ms;
use crate::scene::{GroupId, Scene};

/// The lens comets start `i * 80 ms` apart (`--lens-flow-delay: step * 0.08s`).
pub const LENS_DELAY_MS: f64 = 80.0;

/// The longest frame gap the journey's dwell counts: a longer one (the window was hidden) must not
/// skip steps.
pub const MAX_TICK_MS: f64 = 250.0;

/// A relationship between two picked nodes, shown.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Pinned {
    pub from: String,
    pub to: String,
    /// The nodes of the path, in order (two for a direct relationship).
    pub nodes: Vec<String>,
    pub node_groups: Vec<GroupId>,
    pub edges: Vec<GroupId>,
    pub direct: bool,
}

/// The lens that is on.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LensState {
    pub filters: Vec<LensFilter>,
    /// The selected nodes (they glow).
    pub selected: Vec<GroupId>,
    /// Matched edges, including the ones too many to animate.
    pub edges: usize,
    pub quiet: bool,
}

/// A route: a walk from `nodes[0]` with a cursor, the probe's overview and the journey.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Route {
    pub nodes: Vec<String>,
    pub node_groups: Vec<GroupId>,
    /// `edges[i]` runs `nodes[i]` to `nodes[i + 1]`.
    pub edges: Vec<GroupId>,
    /// The labels of each edge, so a step restyles without the scene.
    pub edge_labels: Vec<Vec<GroupId>>,
    /// The node the journey is on; `None` while the whole route is shown (the probe's overview).
    pub index: Option<usize>,
    pub dwell: Dwell,
    /// The explicit `Route` claim, held while the journey plays.
    pub token: Option<Token>,
}

impl Route {
    pub fn playing(&self) -> bool {
        self.dwell.playing
    }

    /// The nodes the camera frames at the current step: the one before, the current and the next.
    pub fn window(&self) -> &[String] {
        match self.index {
            None => &self.nodes,
            Some(i) => &self.nodes[i.saturating_sub(1)..(i + 2).min(self.nodes.len())],
        }
    }
}

/// What choosing a route destination did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RouteChoice {
    /// The path is shown and the probe runs.
    Found,
    /// The destination is the start.
    Same,
    /// The start does not reach it along the edges' direction.
    Unreachable,
    /// No route is begun, or the destination is not a node.
    Nothing,
}

/// The lit set of a route: every node and edge at 1 in the overview, else past/current/future at
/// .62/1/.34 (an edge is as far along as the node it arrives at).
pub fn route_lit(route: &Route) -> Vec<(GroupId, f64)> {
    let state = |step: usize| match route.index {
        None => 1.0,
        Some(i) if step < i => DIM_ROUTE_PAST,
        Some(i) if step == i => 1.0,
        Some(_) => DIM_ROUTE_FUTURE,
    };
    let mut lit: Vec<(GroupId, f64)> =
        route.node_groups.iter().enumerate().map(|(k, g)| (*g, state(k))).collect();
    for (k, edge) in route.edges.iter().enumerate() {
        lit.push((*edge, state(k + 1)));
        lit.extend(route.edge_labels[k].iter().map(|l| (*l, state(k + 1))));
    }
    lit
}

impl MotionState {
    pub fn relationship(&self) -> Option<&Pinned> {
        self.relationship.as_ref()
    }

    pub fn lens(&self) -> Option<&LensState> {
        self.lens.as_ref()
    }

    pub fn route(&self) -> Option<&Route> {
        self.route.as_ref()
    }

    /// Anything of the three is up.
    pub fn interaction_active(&self) -> bool {
        self.relationship.is_some() || self.lens.is_some() || self.route.is_some()
    }

    /// End a relationship, a lens and a route: what Escape does first. Whether anything ended.
    pub fn end_interaction(&mut self, now: Ms) -> bool {
        let any = self.interaction_active();
        self.clear_relationship(now);
        self.clear_lens(now);
        self.end_route(now);
        any
    }

    // ---- M5: the relationship ----

    /// A route or a lens owns the budget: a relationship is not shown over it.
    pub fn relationship_blocked(&self) -> bool {
        self.owners.is_active(Owner::Route) || self.owners.is_active(Owner::Lens)
    }

    /// Show how `a` and `b` relate: comets along the path (the edges between them, else the
    /// shortest path), everything else dimmed. `false`, and nothing changes, when blocked, when
    /// either is not a node, or when they are not connected.
    pub fn relate(&mut self, scene: &Scene, a: &str, b: &str, now: Ms) -> bool {
        if self.relationship_blocked() || scene.node(a).is_none() || scene.node(b).is_none() {
            return false;
        }
        let Some(rel) = path::relationship(scene, a, b) else { return false };
        self.clear_relationship(now);
        self.clear_intent(now);
        let node_groups: Vec<GroupId> = rel.path.nodes.iter().filter_map(|n| scene.node(n)).collect();
        let edges = rel.path.edges.clone();
        let mut lit: Vec<(GroupId, f64)> = node_groups.iter().map(|g| (*g, 1.0)).collect();
        lit.extend(edges.iter().map(|g| (*g, 1.0)));
        lit.extend(labels_of(scene, &edges).into_iter().map(|g| (g, 1.0)));
        let instant = self.config.instant();
        self.dim(Dim::new(DimKind::Relationship, DIM_RELATIONSHIP, lit, now, instant), now);
        self.owners.set_active(Owner::Relationship, true);
        self.refresh(now);
        let runs: Vec<(GroupId, Direction)> = edges.iter().map(|e| (*e, Direction::Out)).collect();
        // Parallel edges of one relationship pulse together; a path hops.
        let hop = if rel.direct { 0.0 } else { STEP_DELAY_MS };
        self.play_comets(CometKind::Relationship, &runs, now, hop);
        self.relationship =
            Some(Pinned { from: a.to_string(), to: b.to_string(), nodes: rel.path.nodes, node_groups, edges, direct: rel.direct });
        true
    }

    pub fn clear_relationship(&mut self, now: Ms) {
        if self.relationship.take().is_none() {
            return;
        }
        self.clear_comets(CometKind::Relationship);
        self.release_dim(DimKind::Relationship, now);
        self.owners.set_active(Owner::Relationship, false);
        self.refresh(now);
    }

    // ---- M6: the lens ----

    /// Light what `filters` select. An empty selection clears the lens and answers `false`.
    pub fn set_lens(&mut self, scene: &Scene, filters: &[LensFilter], now: Ms) -> bool {
        let sel = lens::select(scene, filters);
        if sel.is_empty() {
            self.clear_lens(now);
            return false;
        }
        self.end_route(now);
        self.clear_relationship(now);
        self.clear_intent(now);
        self.clear_comets(CometKind::Lens);
        let instant = self.config.instant();
        self.dim(Dim::new(DimKind::Lens, DIM_LENS, sel.lit.clone(), now, instant), now);
        self.owners.set_active(Owner::Lens, true);
        self.refresh(now);
        self.play_comets(CometKind::Lens, sel.flow(), now, LENS_DELAY_MS);
        self.lens =
            Some(LensState { filters: sel.filters.clone(), selected: sel.nodes.clone(), edges: sel.edges.len(), quiet: sel.quiet });
        true
    }

    pub fn clear_lens(&mut self, now: Ms) {
        if self.lens.take().is_none() {
            return;
        }
        self.clear_comets(CometKind::Lens);
        self.release_dim(DimKind::Lens, now);
        self.owners.set_active(Owner::Lens, false);
        self.refresh(now);
    }

    // ---- M7, M8: the route ----

    /// Begin a route at `node`: it alone is lit, ready to step or to be given a destination.
    pub fn route_begin(&mut self, scene: &Scene, node: &str, now: Ms) -> bool {
        let Some(group) = scene.node(node) else { return false };
        self.end_route(now);
        self.clear_relationship(now);
        self.clear_lens(now);
        self.clear_intent(now);
        self.route = Some(Route {
            nodes: vec![node.to_string()],
            node_groups: vec![group],
            edges: Vec::new(),
            edge_labels: Vec::new(),
            index: Some(0),
            dwell: Dwell::journey(1),
            token: None,
        });
        self.last_tick = Some(now);
        self.owners.set_active(Owner::Route, true);
        self.refresh(now);
        self.restyle_route(now);
        true
    }

    /// End the route: the claim is given back, the comets go at once, the dim fades out.
    pub fn end_route(&mut self, now: Ms) -> bool {
        let Some(route) = self.route.take() else { return false };
        if let Some(token) = route.token {
            self.release(token, now);
        }
        self.clear_comets(CometKind::RouteProbe);
        self.clear_comets(CometKind::RouteJourney);
        self.release_dim(DimKind::Route, now);
        self.owners.set_active(Owner::Route, false);
        self.refresh(now);
        true
    }

    /// Give the route the destination `to`: the shortest directed path from its start is shown and
    /// the probe runs along it, hop by hop, then parks.
    pub fn route_target(&mut self, scene: &Scene, to: &str, now: Ms) -> RouteChoice {
        let Some(start) = self.route.as_ref().and_then(|r| r.nodes.first().cloned()) else {
            return RouteChoice::Nothing;
        };
        if scene.node(to).is_none() {
            return RouteChoice::Nothing;
        }
        if to == start {
            return RouteChoice::Same;
        }
        let Some(found) = path::shortest_path(scene, &start, to) else { return RouteChoice::Unreachable };
        self.pause_journey(now);
        self.clear_comets(CometKind::RouteProbe);
        self.clear_comets(CometKind::RouteJourney);
        let node_groups: Vec<GroupId> = found.nodes.iter().filter_map(|n| scene.node(n)).collect();
        let edge_labels = found.edges.iter().map(|e| labels_of(scene, &[*e])).collect();
        let runs: Vec<(GroupId, Direction)> = found.edges.iter().map(|e| (*e, Direction::Out)).collect();
        if let Some(route) = self.route.as_mut() {
            route.dwell = Dwell::journey(found.nodes.len());
            route.nodes = found.nodes;
            route.node_groups = node_groups;
            route.edges = found.edges;
            route.edge_labels = edge_labels;
            route.index = None;
        }
        self.play_comets(CometKind::RouteProbe, &runs, now, STEP_DELAY_MS);
        self.restyle_route(now);
        RouteChoice::Found
    }

    /// Move the cursor to the next node, which is the route's own next node, or, at its end, the
    /// first outgoing edge to a node the route has not visited (the route grows by it). From the
    /// overview it enters the journey at the start. The node reached, `None` at a dead end.
    pub fn route_step(&mut self, scene: &Scene, now: Ms) -> Option<String> {
        let route = self.route.as_ref()?;
        let n = route.nodes.len();
        let Some(i) = route.index else {
            self.goto(0, false, now);
            return self.route.as_ref().map(|r| r.nodes[0].clone());
        };
        if i + 1 < n {
            self.goto(i + 1, true, now);
            return self.route.as_ref().map(|r| r.nodes[i + 1].clone());
        }
        let (edge, next) = path::outgoing(scene, &route.nodes[i]).into_iter().find(|(_, to)| !route.nodes.contains(to))?;
        let group = scene.node(&next)?;
        let labels = labels_of(scene, &[edge]);
        let route = self.route.as_mut()?;
        route.nodes.push(next.clone());
        route.node_groups.push(group);
        route.edges.push(edge);
        route.edge_labels.push(labels);
        self.goto(n, true, now);
        Some(next)
    }

    /// Back one node. `false` at the start or from the overview.
    pub fn route_back(&mut self, now: Ms) -> bool {
        match self.route.as_ref().and_then(|r| r.index) {
            Some(i) if i > 0 => {
                self.goto(i - 1, true, now);
                true
            }
            _ => false,
        }
    }

    /// Play the journey from the cursor (from the start when it is finished or in the overview).
    /// Needs two nodes and a governor that is not paused (Still, reduced motion, hidden).
    pub fn route_play(&mut self, now: Ms) -> bool {
        let n = self.route.as_ref().map_or(0, |r| r.nodes.len());
        if n < 2 || self.config.effective_paused() {
            return false;
        }
        if self.route.as_ref().is_some_and(Route::playing) {
            return true;
        }
        let token = self.claim(Owner::Route, now).token;
        self.clear_comets(CometKind::RouteProbe);
        let Some(route) = self.route.as_mut() else { return false };
        route.token = (token.0 != 0).then_some(token);
        let at = match route.index {
            Some(i) if i + 1 < n => i,
            _ => 0,
        };
        // Resuming where the journey was paused keeps the elapsed dwell; anything else starts fresh.
        let resume = route.index == Some(at) && route.dwell.index == at && !route.dwell.complete && route.dwell.steps == n;
        if !resume {
            route.dwell = Dwell::journey(n);
            route.dwell.seek(at);
        }
        route.dwell.play();
        self.last_tick = Some(now);
        self.show_step(at, false, now);
        true
    }

    /// Stop the journey where it is (the elapsed dwell is kept) and give the claim back.
    pub fn route_pause(&mut self, now: Ms) {
        self.pause_journey(now);
    }

    fn pause_journey(&mut self, now: Ms) {
        let Some(route) = self.route.as_mut() else { return };
        route.dwell.pause();
        if let Some(token) = route.token.take() {
            self.release(token, now);
        }
    }

    /// A manual move of the cursor: a journey that was playing stops, the dwell starts afresh on
    /// `index`.
    fn goto(&mut self, index: usize, pulse: bool, now: Ms) {
        self.pause_journey(now);
        if let Some(route) = self.route.as_mut() {
            route.dwell = Dwell::journey(route.nodes.len());
            route.dwell.seek(index);
        }
        self.show_step(index, pulse, now);
    }

    /// The cursor is on `index`: pulse the edge just crossed, restyle past/current/future. The
    /// dwell is the caller's.
    fn show_step(&mut self, index: usize, pulse: bool, now: Ms) {
        let Some(route) = self.route.as_mut() else { return };
        let index = index.min(route.nodes.len().saturating_sub(1));
        route.index = Some(index);
        let edge = (pulse && index > 0).then(|| route.edges[index - 1]);
        self.clear_comets(CometKind::RouteJourney);
        if let Some(edge) = edge {
            self.play_comets(CometKind::RouteJourney, &[(edge, Direction::Out)], now, 0.0);
        }
        self.restyle_route(now);
    }

    fn restyle_route(&mut self, now: Ms) {
        let Some(route) = self.route.as_ref() else { return };
        let lit = route_lit(route);
        let instant = self.config.instant();
        self.dim(Dim::new(DimKind::Route, DIM_ROUTE, lit, now, instant), now);
    }

    /// The journey's frame: advance the dwell by the delta since the last tick. A paused governor
    /// pauses it (it does not resume by itself).
    pub(super) fn tick_route(&mut self, now: Ms) {
        let dt = self.last_tick.map_or(0.0, |last| (now - last).clamp(0.0, MAX_TICK_MS));
        self.last_tick = Some(now);
        if !self.route.as_ref().is_some_and(Route::playing) {
            return;
        }
        if self.config.effective_paused() {
            self.pause_journey(now);
            return;
        }
        let Some(route) = self.route.as_mut() else { return };
        let advance = route.dwell.advance(dt);
        let index = route.dwell.index;
        if advance.stepped > 0 {
            self.show_step(index, true, now);
        }
        if advance.completed {
            // The last dwell ran out: the journey is over, the route stays shown.
            self.pause_journey(now);
        }
    }
}
