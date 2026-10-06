//! `MotionState`: every bit of mutable motion data, in one serde value that [`sample`] reads.
//!
//! The UI keeps one per document and does three things with it: feed it events (a hover, a
//! claim, a pause, a tier change), call [`MotionState::tick`] once per frame with the monotonic
//! time, and ask [`MotionState::is_active`] whether to request another frame. Painting is
//! `sample(scene, &state, t)`, a pure function of the state and `t`.
//!
//! [`sample`]: super::sample

use serde::{Deserialize, Serialize};

use super::ambient::{self, Ambient, Settle};
use super::comet::{CometKind, Direction, LENS_MAX_EDGES};
use super::config::{MotionConfig, Mode};
use super::dim::{Dim, DimKind, LodFade, tier_alpha};
use super::interact::{LensState, Pinned, Route};
use super::owner::{Claim, Owner, Owners, Token};
use super::step::{Steps, scene_steps};
use super::timeline::{Ms, Timeline};
use crate::scene::{GroupId, Scene, Tier};

/// The pointer must rest on a node this long before the intent trace shows (0 under reduced
/// motion): `intent-trace.js:166`.
pub const INTENT_HOVER_DELAY_MS: f64 = 90.0;

/// One comet on one edge group.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct CometRun {
    pub kind: CometKind,
    pub group: GroupId,
    pub direction: Direction,
    /// `start_ms` is when the run exists (nothing is drawn before it); `delay_ms` is the hop or
    /// lens delay after which it moves. Before the delay it waits at its first keyframe.
    pub timeline: Timeline,
}

/// A node whose incident edges are being previewed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Intent {
    pub node: String,
    pub node_group: GroupId,
    /// When it shows: request time plus the hover delay.
    pub shown_at: Ms,
    /// The `Intent` owner flag is up (set by [`MotionState::tick`] at `shown_at`).
    pub promoted: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MotionState {
    pub(super) config: MotionConfig,
    pub(super) owners: Owners,
    pub(super) ambient: Ambient,
    pub(super) steps: Steps,
    pub(super) ambient_total_ms: Ms,
    pub(super) intent: Option<Intent>,
    pub(super) runs: Vec<CometRun>,
    pub(super) dims: Vec<Dim>,
    pub(super) tier: Option<Tier>,
    pub(super) lod: Option<LodFade>,
    /// M5-M8 (`interact.rs`).
    pub(super) relationship: Option<Pinned>,
    pub(super) lens: Option<LensState>,
    pub(super) route: Option<Route>,
    /// The time of the last [`MotionState::tick`], for the journey's dwell.
    pub(super) last_tick: Option<Ms>,
}

impl MotionState {
    /// A fresh state for `scene` at time `now`. The ambient trace starts here if the config
    /// allows it (capable, live, visible, no owner) and never starts again.
    pub fn new(config: MotionConfig, scene: &Scene, now: Ms) -> Self {
        let steps = scene_steps(scene);
        let mut s = Self {
            config,
            owners: Owners::default(),
            ambient: Ambient::default(),
            ambient_total_ms: ambient::total_ms(scene, &steps),
            steps,
            intent: None,
            runs: Vec::new(),
            dims: Vec::new(),
            tier: None,
            lod: None,
            relationship: None,
            lens: None,
            route: None,
            last_tick: None,
        };
        s.refresh(now);
        s
    }

    /// Replace the ambient steps (rule W for workflows). Only matters before the pass starts;
    /// it recomputes the pass length from `scene`.
    pub fn set_steps(&mut self, steps: Steps, scene: &Scene) {
        self.ambient_total_ms = ambient::total_ms(scene, &steps);
        self.steps = steps;
    }

    pub fn config(&self) -> &MotionConfig {
        &self.config
    }

    pub fn owner(&self) -> Option<Owner> {
        self.owners.owner()
    }

    pub fn ambient(&self) -> &Ambient {
        &self.ambient
    }

    pub fn intent(&self) -> Option<&Intent> {
        self.intent.as_ref()
    }

    pub fn runs(&self) -> &[CometRun] {
        &self.runs
    }

    /// `render()` of the governor: settle the ambient trace if anything forbids it, else start it.
    pub(super) fn refresh(&mut self, now: Ms) {
        if !self.config.capable {
            return;
        }
        if self.config.effective_paused() || self.owners.owner().is_some() {
            self.ambient.settle(Settle::Suppressed);
        } else {
            self.ambient.start(now, self.ambient_total_ms);
        }
    }

    // ---- the governor's switches ----

    pub fn set_mode(&mut self, mode: Mode, now: Ms) {
        self.config.mode = mode;
        self.refresh(now);
    }

    pub fn set_system_reduced(&mut self, reduced: bool, now: Ms) {
        self.config.system_reduced = reduced;
        self.refresh(now);
    }

    pub fn suspend(&mut self, reason: &str, now: Ms) {
        self.config.suspend(reason);
        self.refresh(now);
    }

    pub fn unsuspend(&mut self, reason: &str, now: Ms) -> bool {
        let held = self.config.unsuspend(reason);
        self.refresh(now);
        held
    }

    /// Raise or drop an interaction flag (route, lens, relationship, focus, legend; the intent
    /// flag is managed by the intent trace itself).
    pub fn set_interaction(&mut self, owner: Owner, on: bool, now: Ms) {
        self.owners.set_active(owner, on);
        self.refresh(now);
    }

    /// Explicit claim (a journey, a story). Token 0 when the config is not capable.
    pub fn claim(&mut self, owner: Owner, now: Ms) -> Claim {
        if !self.config.capable {
            return Claim { token: Token(0), preempted: None };
        }
        let c = self.owners.claim(owner);
        self.refresh(now);
        c
    }

    pub fn release(&mut self, token: Token, now: Ms) -> bool {
        let released = self.config.capable && self.owners.release(token);
        self.refresh(now);
        released
    }

    // ---- M4: the intent trace ----

    /// The intent trace is blocked while a lens, relationship preview, route or focus is active
    /// or an explicit claim holds the budget.
    pub fn intent_blocked(&self) -> bool {
        use Owner::*;
        self.owners.explicit().is_some() || [Lens, Relationship, Route, Focus].into_iter().any(|o| self.owners.is_active(o))
    }

    /// Pointer rests on `node`: show after the hover delay (90 ms, 0 reduced). `false` (and the
    /// trace is cleared) when blocked or the node is unknown.
    pub fn hover_intent(&mut self, scene: &Scene, node: &str, now: Ms) -> bool {
        self.show_intent(scene, node, now, self.config.hover_delay_ms())
    }

    /// Keyboard focus on `node`: show at once.
    pub fn focus_intent(&mut self, scene: &Scene, node: &str, now: Ms) -> bool {
        self.show_intent(scene, node, now, 0.0)
    }

    fn show_intent(&mut self, scene: &Scene, node: &str, now: Ms, delay: Ms) -> bool {
        if self.intent_blocked() {
            self.clear_intent(now);
            return false;
        }
        if self.intent.as_ref().is_some_and(|i| i.node == node) {
            return true;
        }
        let Some(group) = scene.node(node) else {
            self.clear_intent(now);
            return false;
        };
        self.clear_intent(now);
        let shown_at = now + delay;
        for (edge, _) in scene.edges_of(node) {
            let Some(e) = scene.group(edge).and_then(|g| g.edge.as_ref()) else { continue };
            let direction = match (e.from == node, e.to == node) {
                (true, true) => Direction::Loop,
                (true, false) => Direction::Out,
                _ => Direction::In,
            };
            let params = CometKind::Intent.params();
            self.runs.push(CometRun {
                kind: CometKind::Intent,
                group: edge,
                direction,
                timeline: Timeline::new(shown_at, params.duration_ms, params.easing),
            });
        }
        if let Some(dim) = Dim::neighbourhood(DimKind::Intent, scene, node, shown_at, self.config.instant()) {
            self.dims.push(dim);
        }
        self.intent = Some(Intent { node: node.to_string(), node_group: group, shown_at, promoted: false });
        true
    }

    /// Pointer left, blur, pointer down elsewhere, window blur: the comets go at once, the dim
    /// fades back over 180 ms.
    pub fn clear_intent(&mut self, now: Ms) {
        let Some(i) = self.intent.take() else { return };
        self.runs.retain(|r| r.kind != CometKind::Intent);
        for d in self.dims.iter_mut().filter(|d| d.kind == DimKind::Intent) {
            d.release(now);
        }
        if i.promoted {
            self.owners.set_active(Owner::Intent, false);
        }
    }

    // ---- other comets (deferred motions M5-M8 share the primitive) ----

    /// Start comets on edge groups, hop `i` delayed by `i * hop_delay_ms` (the route probe uses
    /// 160; the lens, which animates at most [`LENS_MAX_EDGES`], its own delay).
    pub fn play_comets(&mut self, kind: CometKind, edges: &[(GroupId, Direction)], now: Ms, hop_delay_ms: f64) {
        let params = kind.params();
        let take = if kind == CometKind::Lens { LENS_MAX_EDGES } else { edges.len() };
        for (i, (group, direction)) in edges.iter().take(take).enumerate() {
            self.runs.push(CometRun {
                kind,
                group: *group,
                direction: *direction,
                timeline: Timeline::new(now, params.duration_ms, params.easing).delayed(hop_delay_ms * i as f64),
            });
        }
    }

    /// Drop every comet of a kind (the owner left; a new owner cancels the previous one).
    pub fn clear_comets(&mut self, kind: CometKind) {
        self.runs.retain(|r| r.kind != kind);
    }

    // ---- fades ----

    /// Add a dim (focus, lens, route; the intent dim is managed by the trace). A dim of the same
    /// kind that is still held is released first.
    pub fn dim(&mut self, dim: Dim, now: Ms) {
        for d in self.dims.iter_mut().filter(|d| d.kind == dim.kind && d.released.is_none()) {
            d.release(now);
        }
        self.dims.push(dim);
    }

    pub fn release_dim(&mut self, kind: DimKind, now: Ms) {
        for d in self.dims.iter_mut().filter(|d| d.kind == kind) {
            d.release(now);
        }
    }

    /// The reading-depth tier changed (zoom crossed a threshold): cross-fade context and fine
    /// texts over 160 ms. The first call only records the tier.
    pub fn set_tier(&mut self, tier: Tier, now: Ms) {
        match self.tier {
            Some(prev) if prev == tier => {}
            Some(prev) => {
                let current = self.lod.map_or_else(|| tier_alpha(prev), |f| f.at(now));
                self.lod = Some(LodFade::new(current, tier, now, self.config.instant()));
                self.tier = Some(tier);
            }
            None => self.tier = Some(tier),
        }
    }

    // ---- the frame loop ----

    /// Once per frame, before `sample`: promote a shown intent trace to owner, settle a finished
    /// ambient pass, drop faded-out dims and finished fades.
    pub fn tick(&mut self, now: Ms) {
        self.tick_route(now);
        if let Some(i) = self.intent.as_mut()
            && !i.promoted
            && now >= i.shown_at
        {
            i.promoted = true;
            self.owners.set_active(Owner::Intent, true);
            self.refresh(now);
        }
        self.ambient.tick(now);
        self.dims.retain(|d| !d.is_gone(now));
        if self.lod.is_some_and(|f| !f.is_active(now)) {
            self.lod = None;
        }
    }

    /// Does anything need another frame at `t`? Idle costs nothing: a settled scene, a held
    /// preview, a finished fade all answer `false`. A hidden window never does.
    pub fn is_active(&self, t: Ms) -> bool {
        if self.config.hidden() {
            return false;
        }
        let ambient = self.config.capable && self.ambient.is_active(t);
        let pending_intent = self.intent.as_ref().is_some_and(|i| t < i.shown_at);
        let comets = !self.config.comets_static() && self.runs.iter().any(|r| t >= r.timeline.start_ms && r.timeline.is_active(t));
        let fading = self.dims.iter().any(|d| d.is_active(t)) || self.lod.is_some_and(|f| f.is_active(t));
        let journey = self.route.as_ref().is_some_and(Route::playing);
        ambient || pending_intent || comets || fading || journey
    }
}
