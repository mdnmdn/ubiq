//! The frame driver (P7.3, `04` §6.1): one [`MotionState`] per picture, the overlay it samples each
//! frame, and the one decision of whether another frame is wanted.
//!
//! The picture's own state (the focused node and its reach, the node under the pointer) stays where
//! `state.rs` keeps it. Each frame [`Motions::frame`] *follows* it: a focus that changed becomes a
//! focus dim (the single source of truth for opacity, P2.8's instant dim is gone), a hovered node
//! becomes the intent trace, and the zoom tier becomes the detail fade. Then it ticks, samples and
//! asks [`MotionState::is_active`]. Nothing here is GPUI but [`advance`], the glue `viewer.rs` calls
//! from render: it asks `Window::on_next_frame` for one frame, whose callback notifies, which
//! renders, which asks again. Idle asks for nothing.
//!
//! **Config.** Capable when the document says `meta.animation: "trace"` and it is the first scene
//! this picture has had (the ambient trace starts at most once; a later edit rebuilds the state
//! without replaying it). The Diagrams "Motion" switch off and the OS reduced-motion flag both set
//! `system_reduced` (static comets, instant fades), the same rule as the camera (`motion::config_for`).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use ubiq_archify::motion::consts::DIM_FOCUS;
use ubiq_archify::motion::lens::{self, LensFilter};
use ubiq_archify::motion::{
    Dim, DimKind, LensState, MotionConfig, MotionState, Overlay, Owner, Pinned, Route, RouteChoice,
    sample,
};
use ubiq_archify::scene::{Bounds, GroupKind, Scene, Tier};
use gpui::{Context, Window};
use crate::app::AppState;
use crate::state::viewport::Viewport;

use crate::state::archify::focus::{self, Reach};
use crate::state::archify::motion::{self, Framing};
use crate::ui::archify::paint::tier_of;
use crate::state::archify::{Interaction, ui};

/// A frame asked for and not delivered for this long (ms) is assumed lost (the window closed under
/// it) and may be asked for again.
const LOST_AFTER_MS: f64 = 250.0;

/// What the user has asked for, from the picture's [`Interaction`]: the focused node and how far
/// the light reaches, and the node under the pointer.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Wanted {
    pub focus: Option<(String, Reach)>,
    pub hover: Option<String>,
}

impl Wanted {
    /// A stale focus (the node an edit removed) and a hover on anything but a node want nothing.
    pub fn of(scene: &Scene, view: &Interaction) -> Self {
        Wanted {
            focus: view
                .focus
                .iter()
                .find(|node| scene.node(node).is_some())
                .map(|node| (node.clone(), view.reach)),
            hover: view
                .hover
                .and_then(|h| scene.group(h.group))
                .filter(|g| g.kind == GroupKind::Node)
                .and_then(|g| g.node_id.clone()),
        }
    }
}

/// Everything one frame reads besides the clock.
pub struct Input<'a> {
    pub key: &'a str,
    pub scene: &'a Arc<Scene>,
    /// `meta.animation == "trace"`.
    pub trace: bool,
    /// Motion switched off or the OS asks for less: static comets, instant fades.
    pub reduced: bool,
    /// The reading tier of the camera now; `None` until the panel is measured.
    pub tier: Option<Tier>,
    pub wanted: &'a Wanted,
}

/// A camera move an interaction wants: frame these nodes as `framing` says. Handed back by the
/// frame, because only the glue (`advance`) has the viewport and the camera.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CameraAsk {
    pub nodes: Vec<String>,
    pub framing: Framing,
}

/// What a frame hands back: the overlay to paint, whether to ask for the next frame, and a camera
/// move the interactions want (a relationship framed, a journey step followed).
pub struct Frame {
    pub overlay: Arc<Overlay>,
    pub request: bool,
    pub camera: Option<CameraAsk>,
}

/// Whether to ask for another frame: something is moving and no request is already on its way.
pub fn want_frame(active: bool, scheduled: Option<f64>, now: f64) -> bool {
    active && scheduled.is_none_or(|at| now - at >= LOST_AFTER_MS)
}

/// What has been handed to the state so far.
#[derive(Default)]
struct Applied {
    focus: Option<(String, Reach)>,
    hover: Option<String>,
}

struct DocMotion {
    /// The scene the state was built for; another `Arc` rebuilds it.
    scene: Arc<Scene>,
    state: MotionState,
    applied: Applied,
    /// When a frame was asked for, until it is delivered.
    scheduled: Option<f64>,
    /// A camera move waiting for the next frame to carry it.
    camera: Option<CameraAsk>,
    /// The route's cursor (`index`, length) the camera last followed.
    seen: Option<(Option<usize>, usize)>,
}

impl DocMotion {
    fn new(scene: Arc<Scene>, config: MotionConfig, now: f64) -> Self {
        let state = MotionState::new(config, &scene, now);
        DocMotion {
            scene,
            state,
            applied: Applied::default(),
            scheduled: None,
            camera: None,
            seen: None,
        }
    }

    /// The route moved (begun, a destination, a step by hand or by the journey): the camera follows
    /// it, the whole route in the overview and a step's neighbourhood while it plays.
    fn camera_check(&mut self) {
        let now_seen = self.state.route().map(|r| (r.index, r.nodes.len()));
        if now_seen == self.seen {
            return;
        }
        self.seen = now_seen;
        if let Some(r) = self.state.route() {
            let framing = if r.index.is_some() {
                Framing::Step
            } else {
                Framing::Route
            };
            self.camera = Some(CameraAsk {
                nodes: r.window().to_vec(),
                framing,
            });
        }
    }

    /// Bring the state to what the picture wants. `fresh` is a state just rebuilt: the focus it
    /// inherits is in place at once, not faded in again.
    fn follow(&mut self, input: &Input<'_>, fresh: bool, now: f64) {
        if self.state.config().system_reduced != input.reduced {
            self.state.set_system_reduced(input.reduced, now);
        }
        if let Some(tier) = input.tier {
            self.state.set_tier(tier, now);
        }
        if self.applied.focus != input.wanted.focus {
            self.set_focus(input.wanted.focus.clone(), fresh, now);
            // A hover held back by the old focus (or released by the new) is looked at again.
            self.applied.hover = None;
        }
        if self.applied.hover != input.wanted.hover {
            self.applied.hover.clone_from(&input.wanted.hover);
            match &input.wanted.hover {
                Some(node) => {
                    self.state.hover_intent(&self.scene, node, now);
                }
                None => self.state.clear_intent(now),
            }
        }
    }

    /// Focus a node: the trace gives way (the core blocks it under focus), the focus owner is
    /// raised and everything outside the lit set fades to [`DIM_FOCUS`]. `None` fades it back.
    fn set_focus(&mut self, focus: Option<(String, Reach)>, fresh: bool, now: f64) {
        self.applied.focus.clone_from(&focus);
        let lit = focus
            .as_ref()
            .and_then(|(node, reach)| focus::lit(&self.scene, node, *reach));
        let Some(lit) = lit else {
            self.state.release_dim(DimKind::Focus, now);
            self.state.set_interaction(Owner::Focus, false, now);
            return;
        };
        self.state.clear_intent(now);
        self.state.set_interaction(Owner::Focus, true, now);
        let instant = fresh || self.state.config().instant();
        let groups = lit.groups.iter().map(|g| (*g, 1.0)).collect();
        self.state.dim(
            Dim::new(DimKind::Focus, DIM_FOCUS, groups, now, instant),
            now,
        );
    }

    fn frame(&mut self, now: f64) -> Frame {
        self.state.tick(now);
        self.camera_check();
        let overlay = sample(&self.scene, &self.state, now);
        let request = want_frame(self.state.is_active(now), self.scheduled, now);
        if request {
            self.scheduled = Some(now);
        }
        Frame {
            overlay: Arc::new(overlay),
            request,
            camera: self.camera.take(),
        }
    }
}

/// The motion states of every picture, keyed like its viewport. Lives in the `Ui` global.
#[derive(Default)]
pub struct Motions {
    docs: HashMap<String, DocMotion>,
    /// Pictures that have had their ambient pass (or were not allowed one): it does not replay.
    played: HashSet<String>,
}

impl Motions {
    /// One frame of picture `input.key` at `now` (ms): follow the picture, tick, sample. A new
    /// scene (the first, or a recompile) rebuilds the state.
    pub fn frame(&mut self, input: &Input<'_>, now: f64) -> Frame {
        let fresh = self
            .docs
            .get(input.key)
            .is_none_or(|d| !Arc::ptr_eq(&d.scene, input.scene));
        if fresh {
            let first = self.played.insert(input.key.to_string());
            let config = MotionConfig {
                capable: input.trace && first,
                system_reduced: input.reduced,
                ..MotionConfig::default()
            };
            self.docs.insert(
                input.key.to_string(),
                DocMotion::new(input.scene.clone(), config, now),
            );
        }
        let doc = self.docs.get_mut(input.key).expect("inserted above");
        doc.follow(input, fresh, now);
        doc.frame(now)
    }

    /// "Replay trace": forget that `key` had its ambient pass, so the next frame builds its state
    /// afresh and runs it again (when the document asks for the trace at all).
    pub fn replay(&mut self, key: &str) {
        self.played.remove(key);
        self.docs.remove(key);
    }

    /// The frame asked for `key` arrived: it may ask for the next.
    pub fn arrived(&mut self, key: &str) {
        if let Some(d) = self.docs.get_mut(key) {
            d.scheduled = None;
        }
    }

    fn doc(&mut self, key: &str) -> Result<&mut DocMotion, String> {
        self.docs
            .get_mut(key)
            .ok_or_else(|| "Nothing is drawn yet.".to_string())
    }

    /// Ask the camera to frame `nodes` at the next frame (the finder's pick).
    pub fn reveal(&mut self, key: &str, nodes: Vec<String>, framing: Framing) {
        if let Some(d) = self.docs.get_mut(key) {
            d.camera = Some(CameraAsk { nodes, framing });
        }
    }

    /// Show how `a` and `b` relate: comets along the path, the camera framing it. The `Err` is
    /// what to tell the user.
    pub fn relate(&mut self, key: &str, a: &str, b: &str, now: f64) -> Result<(), String> {
        let d = self.doc(key)?;
        if d.state.relationship_blocked() {
            return Err("A lens or a route is on: press Esc first.".into());
        }
        if !d.state.relate(&d.scene, a, b, now) {
            return Err(format!(
                "{} and {} are not connected.",
                label(&d.scene, a),
                label(&d.scene, b)
            ));
        }
        let nodes = d
            .state
            .relationship()
            .map(|r| r.nodes.clone())
            .unwrap_or_default();
        d.camera = Some(CameraAsk {
            nodes,
            framing: Framing::Route,
        });
        Ok(())
    }

    /// The lens control: none, then each kind and tag of the diagram in turn, then none again.
    pub fn cycle_lens(&mut self, key: &str, now: f64) -> Result<(), String> {
        let d = self.doc(key)?;
        let options = lens::options(&d.scene);
        let current = d.state.lens().and_then(|l| l.filters.first().cloned());
        match lens::cycle(&options, current.as_ref()) {
            Some(next) => {
                d.state.set_lens(&d.scene, &[next], now);
            }
            None if options.is_empty() => {
                return Err("This diagram has no node kinds to filter by.".into());
            }
            None => d.state.clear_lens(now),
        }
        Ok(())
    }

    /// One key of the route: begin at a node, step, step back, choose a destination, play or pause
    /// the journey. The `Err` is what to tell the user.
    pub fn route(&mut self, key: &str, act: RouteKey, now: f64) -> Result<(), String> {
        let d = self.doc(key)?;
        match act {
            RouteKey::Begin(node) => {
                if !d.state.route_begin(&d.scene, &node, now) {
                    return Err("Focus a node first, then press r.".into());
                }
            }
            RouteKey::Step => {
                if d.state.route().is_none() {
                    return Err("Focus a node and press r to begin a route.".into());
                }
                if d.state.route_step(&d.scene, now).is_none() {
                    return Err("End of the line: no outgoing edge to a new node.".into());
                }
            }
            RouteKey::Back => {
                d.state.route_back(now);
            }
            RouteKey::Target(node) => match d.state.route_target(&d.scene, &node, now) {
                RouteChoice::Found | RouteChoice::Nothing => {}
                RouteChoice::Same => return Err("That is where the route starts.".into()),
                RouteChoice::Unreachable => {
                    let start = d.state.route().and_then(|r| r.nodes.first().cloned());
                    return Err(format!(
                        "No route: {} is not reachable from {} along the edges.",
                        label(&d.scene, &node),
                        start.map_or_else(String::new, |s| label(&d.scene, &s)),
                    ));
                }
            },
            RouteKey::Play => {
                if d.state.route().is_some_and(|r| r.playing()) {
                    d.state.route_pause(now);
                } else if !d.state.route_play(now) {
                    return Err(
                        "Nothing to play: give the route a destination or step on, and motion must be on."
                            .into(),
                    );
                }
            }
        }
        Ok(())
    }

    /// Escape: a relationship, a lens or a route ends. Whether one did.
    pub fn end_interaction(&mut self, key: &str, now: f64) -> bool {
        self.docs
            .get_mut(key)
            .is_some_and(|d| d.state.end_interaction(now))
    }

    pub fn route_active(&self, key: &str) -> bool {
        self.docs
            .get(key)
            .is_some_and(|d| d.state.route().is_some())
    }

    /// What the picture's chip says about the interaction in progress.
    pub fn hud(&self, key: &str) -> Option<Hud> {
        let d = self.docs.get(key)?;
        let s = &d.state;
        if let Some(r) = s.route() {
            Some(Hud {
                text: route_text(&d.scene, r),
                route: true,
                playing: r.playing(),
            })
        } else if let Some(l) = s.lens() {
            Some(Hud {
                text: lens_text(l),
                route: false,
                playing: false,
            })
        } else {
            s.relationship().map(|p| Hud {
                text: relationship_text(&d.scene, p),
                route: false,
                playing: false,
            })
        }
    }

    /// The filter the lens is on (for its control's label).
    pub fn lens_label(&self, key: &str) -> Option<String> {
        let l = self.docs.get(key)?.state.lens()?;
        l.filters.first().map(LensFilter::label)
    }
}

/// A key of the route interaction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RouteKey {
    Begin(String),
    Step,
    Back,
    Target(String),
    Play,
}

/// The chip over the picture while a relationship, a lens or a route is up.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hud {
    pub text: String,
    /// A route: its step and play controls show.
    pub route: bool,
    pub playing: bool,
}

/// A node's label (its id when the scene has none, or no such node).
fn label(scene: &Scene, node: &str) -> String {
    scene
        .node(node)
        .and_then(|g| scene.group(g))
        .map(|g| g.label.clone())
        .filter(|l| !l.is_empty())
        .unwrap_or_else(|| node.to_string())
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

pub fn relationship_text(scene: &Scene, p: &Pinned) -> String {
    let how = if p.direct {
        plural(p.edges.len(), "direct edge", "direct edges")
    } else {
        plural(p.edges.len(), "hop", "hops")
    };
    format!(
        "Relationship \u{b7} {} \u{2192} {} \u{b7} {how}",
        label(scene, &p.from),
        label(scene, &p.to)
    )
}

pub fn lens_text(l: &LensState) -> String {
    let filters: Vec<String> = l.filters.iter().map(LensFilter::label).collect();
    let mut text = format!(
        "Lens \u{b7} {} \u{b7} {} \u{b7} {}",
        filters.join(" + "),
        plural(l.selected.len(), "node", "nodes"),
        plural(l.edges, "relationship", "relationships")
    );
    if l.quiet {
        text.push_str(" \u{b7} too many to animate");
    }
    text
}

pub fn route_text(scene: &Scene, r: &Route) -> String {
    let first = r.nodes.first().map_or_else(String::new, |n| label(scene, n));
    if r.nodes.len() < 2 {
        return format!("Route from {first} \u{b7} n steps on, click a node for a destination");
    }
    let last = r.nodes.last().map_or_else(String::new, |n| label(scene, n));
    let at = match r.index {
        Some(i) => format!(
            "{}/{}{}",
            i + 1,
            r.nodes.len(),
            if r.playing() { " \u{b7} playing" } else { "" }
        ),
        None => plural(r.nodes.len(), "node", "nodes"),
    };
    format!("Route \u{b7} {first} \u{2192} {last} \u{b7} {at}")
}

// --------------------------------------------------------------------------------------------- //
// The glue
// --------------------------------------------------------------------------------------------- //

/// The overlay for picture `key` this frame, asking the window for the next while something moves.
/// Call from render, before the picture is built.
pub fn advance(
    app: &AppState,
    key: &str,
    scene: &Arc<Scene>,
    trace: bool,
    view: &Interaction,
    window: &mut Window,
    cx: &mut Context<AppState>,
) -> Arc<Overlay> {
    let vp = app.viewport(key);
    let input = Input {
        key,
        scene,
        trace,
        reduced: !ui(cx).settings.motion || cx.reduce_motion(),
        tier: tier_now(&vp),
        wanted: &Wanted::of(scene, view),
    };
    let now = ui(cx).cameras.now_ms();
    let frame = ui(cx).motions.frame(&input, now);
    if let Some(ask) = &frame.camera {
        let boxes = ask_boxes(scene, ask);
        if let Some(target) = motion::frame(
            [vp.panel_w, vp.panel_h],
            [vp.content.width, vp.content.height],
            &boxes,
            ask.framing,
        ) {
            motion::glide_to(app, key, target, cx);
        }
    }
    if frame.request {
        let entity = cx.entity();
        let key = key.to_string();
        window.on_next_frame(move |_, cx| {
            entity.update(cx, |_, cx| {
                ui(cx).motions.arrived(&key);
                cx.notify();
            });
        });
    }
    frame.overlay
}

/// Tell the user why something did not happen: a notification.
pub fn tell(app: &mut AppState, text: String) {
    app.raise_notification(crate::ui::archify::agent::notification(crate::ui::archify::agent::Notice { text }));
}

fn finish(this: &mut AppState, result: Result<(), String>, cx: &mut Context<AppState>) {
    if let Err(text) = result {
        tell(this, text);
    }
    cx.notify();
}

fn now_ms(cx: &mut Context<AppState>) -> f64 {
    ui(cx).cameras.now_ms()
}

/// `r`: begin a route at the focused node. With nothing focused while a route is on, it ends it.
pub fn begin_route(this: &mut AppState, key: &str, cx: &mut Context<AppState>) {
    let now = now_ms(cx);
    let focus = ui(cx).peek_view(key).focus;
    let Some(node) = focus else {
        if ui(cx).motions.route_active(key) {
            ui(cx).motions.end_interaction(key, now);
            cx.notify();
        } else {
            tell(this, "Focus a node first (click it), then press r.".into());
        }
        return;
    };
    // The route takes over from the focus, as the viewer's `begin` clears it.
    ui(cx).view(key).focus = None;
    let result = ui(cx).motions.route(key, RouteKey::Begin(node), now);
    finish(this, result, cx);
}

/// Step, step back, destination, play or pause on the route of picture `key`.
pub fn route_key(this: &mut AppState, key: &str, act: RouteKey, cx: &mut Context<AppState>) {
    // The route's keys do nothing, and say nothing, while there is no route.
    if !ui(cx).motions.route_active(key) && !matches!(act, RouteKey::Begin(_)) {
        return;
    }
    let now = now_ms(cx);
    let result = ui(cx).motions.route(key, act, now);
    finish(this, result, cx);
}

/// `l`: the next lens (none, each kind and tag, none). The lens takes over from the focus.
pub fn cycle_lens(this: &mut AppState, key: &str, cx: &mut Context<AppState>) {
    let now = now_ms(cx);
    ui(cx).view(key).focus = None;
    let result = ui(cx).motions.cycle_lens(key, now);
    finish(this, result, cx);
}

/// Shift-click `to` while `from` is focused: show how they relate and frame them.
pub fn relate(this: &mut AppState, key: &str, from: &str, to: &str, cx: &mut Context<AppState>) {
    let now = now_ms(cx);
    let result = ui(cx).motions.relate(key, from, to, now);
    if result.is_ok() {
        ui(cx).view(key).focus = None;
    }
    finish(this, result, cx);
}

/// Escape: a relationship, a lens or a route ends first. Whether one did (then Escape is spent).
pub fn escape(key: &str, cx: &mut Context<AppState>) -> bool {
    let now = now_ms(cx);
    let ended = ui(cx).motions.end_interaction(key, now);
    if ended {
        cx.notify();
    }
    ended
}

/// A plain click ends a lens or a relationship that was on before it focuses (or clears).
pub fn click_ends_interaction(key: &str, cx: &mut Context<AppState>) {
    let now = now_ms(cx);
    ui(cx).motions.end_interaction(key, now);
}

/// The boxes a camera ask frames: its nodes' bounds, and with `Neighbours` the nodes beside them.
pub fn ask_boxes(scene: &Scene, ask: &CameraAsk) -> Vec<Bounds> {
    let mut ids: Vec<String> = ask.nodes.clone();
    if ask.framing == Framing::Neighbours {
        for node in &ask.nodes {
            for (_, other) in scene.edges_of(node) {
                if !ids.iter().any(|i| i == other) {
                    ids.push(other.to_string());
                }
            }
        }
    }
    ids.iter()
        .filter_map(|n| scene.node(n))
        .filter_map(|g| scene.group(g))
        .map(|g| g.bounds)
        .collect()
}

/// The reading tier the viewport shows now, once it is measured.
fn tier_now(vp: &Viewport) -> Option<Tier> {
    vp.measured().then(|| {
        let camera = vp.camera(vp.content, vp.panel_w, vp.panel_h);
        tier_of(
            camera.scale,
            Viewport::fit_scale(vp.content, vp.panel_w, vp.panel_h),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::archify::Hover;
    use ubiq_archify::motion::consts::{DIM_FADE_MS, DIM_INTENT};
    use ubiq_archify::scene::{
        Bounds, EdgeRef, Group, GroupId, Layer, PolylineShape, SceneBuilder, Shape, Stroke,
    };
    use ubiq_archify::tokens::{Kind, Token};

    /// a -> b, and c apart. Node groups 0..3, edge group 3.
    fn scene() -> Arc<Scene> {
        let mut b = SceneBuilder::new(200.0, 100.0);
        for (i, id) in ["a", "b", "c"].iter().enumerate() {
            b.group(Group::node(
                id,
                Kind::Backend,
                id,
                Bounds::new(i as f64 * 60.0, 0.0, 40.0, 20.0),
            ));
        }
        let edge = EdgeRef {
            key: 1,
            from: "a".into(),
            to: "b".into(),
            id: None,
        };
        let route = [[40.0, 10.0], [60.0, 10.0]];
        let e = b.group(Group::edge(edge, "", &route, 2.0));
        let line = PolylineShape {
            points: route.to_vec(),
            radius: 0.0,
            stroke: Stroke::solid(Token::Arrow, 1.4),
            marker: None,
            halo: false,
        };
        b.push_in(e, Layer::Edges, Shape::Polyline(line));
        Arc::new(b.build())
    }

    fn input<'a>(
        scene: &'a Arc<Scene>,
        wanted: &'a Wanted,
        trace: bool,
        reduced: bool,
    ) -> Input<'a> {
        Input {
            key: "k",
            scene,
            trace,
            reduced,
            tier: Some(Tier::Read),
            wanted,
        }
    }

    fn focus_on(node: &str) -> Wanted {
        Wanted {
            focus: Some((node.into(), Reach::Near)),
            hover: None,
        }
    }

    fn hover_on(node: &str) -> Wanted {
        Wanted {
            focus: None,
            hover: Some(node.into()),
        }
    }

    fn alpha(f: &Frame, group: u32) -> f64 {
        f.overlay.alpha_of(GroupId(group))
    }

    #[test]
    fn a_frame_is_asked_for_only_while_something_moves_and_none_is_on_its_way() {
        assert!(want_frame(true, None, 100.0));
        assert!(!want_frame(false, None, 100.0), "idle asks for nothing");
        assert!(
            !want_frame(true, Some(90.0), 100.0),
            "one is already coming"
        );
        assert!(
            want_frame(true, Some(90.0), 90.0 + LOST_AFTER_MS),
            "a lost one is asked again"
        );
        assert!(!want_frame(false, Some(0.0), 1e6));
    }

    #[test]
    fn an_untouched_picture_is_idle_and_paints_the_static_scene() {
        let (s, w) = (scene(), Wanted::default());
        let mut m = Motions::default();
        let f = m.frame(&input(&s, &w, false, false), 0.0);
        assert!(f.overlay.is_identity() && !f.request);
        let f = m.frame(&input(&s, &w, false, false), 5000.0);
        assert!(f.overlay.is_identity() && !f.request);
    }

    #[test]
    fn the_ambient_trace_runs_on_first_load_then_settles_and_does_not_replay() {
        let (s, w) = (scene(), Wanted::default());
        let mut m = Motions::default();
        let f = m.frame(&input(&s, &w, true, false), 0.0);
        assert!(f.request, "the pass is running");
        m.arrived("k");
        let f = m.frame(&input(&s, &w, true, false), 30_000.0);
        assert!(f.overlay.is_identity() && !f.request, "settled for good");
        // A recompile hands over a new scene: the state is rebuilt, the pass is not replayed.
        let again = Arc::new((*s).clone());
        let f = m.frame(&input(&again, &w, true, false), 31_000.0);
        assert!(f.overlay.is_identity() && !f.request);
    }

    #[test]
    fn replay_runs_the_ambient_pass_again() {
        let (s, w) = (scene(), Wanted::default());
        let mut m = Motions::default();
        m.frame(&input(&s, &w, true, false), 0.0);
        m.arrived("k");
        assert!(!m.frame(&input(&s, &w, true, false), 30_000.0).request);
        m.replay("k");
        let f = m.frame(&input(&s, &w, true, false), 31_000.0);
        assert!(f.request, "the pass is running again");
    }

    #[test]
    fn a_document_without_the_trace_flag_has_no_ambient_pass() {
        let (s, w) = (scene(), Wanted::default());
        let f = Motions::default().frame(&input(&s, &w, false, false), 0.0);
        assert!(!f.request);
    }

    #[test]
    fn a_hover_shows_the_trace_after_the_delay_and_is_idle_once_the_comets_end() {
        let s = scene();
        let (rest, over) = (Wanted::default(), hover_on("a"));
        let mut m = Motions::default();
        m.frame(&input(&s, &rest, false, false), 0.0);
        let f = m.frame(&input(&s, &over, false, false), 100.0);
        assert!(f.request, "the 90 ms delay is pending");
        assert!(f.overlay.strokes.is_empty());
        m.arrived("k");
        let f = m.frame(&input(&s, &over, false, false), 400.0);
        assert!(
            !f.overlay.strokes.is_empty(),
            "a comet on the incident edge"
        );
        assert!(!f.overlay.highlights.is_empty(), "the node glows");
        assert!(alpha(&f, 2) < 1.0, "c is dimmed by the intent");
        m.arrived("k");
        let f = m.frame(&input(&s, &over, false, false), 4000.0);
        assert!(!f.request, "the comets ended and the dim is held: idle");
        assert!((alpha(&f, 2) - DIM_INTENT).abs() < 1e-9);
        // The pointer leaves: the dim fades back and then nothing is left.
        let f = m.frame(&input(&s, &rest, false, false), 4100.0);
        assert!(f.request && f.overlay.strokes.is_empty());
        let f = m.frame(&input(&s, &rest, false, false), 4100.0 + DIM_FADE_MS + 1.0);
        assert!(f.overlay.is_identity() && !f.request);
    }

    #[test]
    fn a_focus_dims_through_the_fade_and_releases_through_it() {
        let s = scene();
        let (rest, on) = (Wanted::default(), focus_on("a"));
        let mut m = Motions::default();
        m.frame(&input(&s, &rest, false, false), 0.0);
        let f = m.frame(&input(&s, &on, false, false), 1000.0);
        assert!(f.request, "the fade runs");
        assert!(
            (alpha(&f, 2) - 1.0).abs() < 1e-9,
            "it starts from the full picture"
        );
        m.arrived("k");
        let f = m.frame(&input(&s, &on, false, false), 1000.0 + DIM_FADE_MS + 1.0);
        assert!(
            (alpha(&f, 2) - DIM_FOCUS).abs() < 1e-9,
            "c is outside the lit set"
        );
        assert_eq!(alpha(&f, 0), 1.0);
        assert_eq!(alpha(&f, 3), 1.0, "the focused node's edge stays lit");
        assert!(!f.request, "a held focus is idle");
        let f = m.frame(&input(&s, &rest, false, false), 3000.0);
        assert!(f.request);
        let f = m.frame(&input(&s, &rest, false, false), 3000.0 + DIM_FADE_MS + 1.0);
        assert!(f.overlay.is_identity());
    }

    #[test]
    fn focus_blocks_the_trace_and_reduced_motion_dims_at_once() {
        let s = scene();
        let both = Wanted {
            focus: Some(("a".into(), Reach::Near)),
            hover: Some("b".into()),
        };
        let mut m = Motions::default();
        let f = m.frame(&input(&s, &both, false, true), 0.0);
        assert!(!f.request, "an instant dim has nothing to animate");
        assert!((alpha(&f, 2) - DIM_FOCUS).abs() < 1e-9);
        assert!(f.overlay.strokes.is_empty(), "no trace under focus");
        assert!((f.overlay.alpha_of(GroupId(0)) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn a_recompile_keeps_the_focus_without_fading_it_in_again() {
        let (s, on) = (scene(), focus_on("a"));
        let mut m = Motions::default();
        m.frame(&input(&s, &on, false, false), 0.0);
        let again = Arc::new((*s).clone());
        let f = m.frame(&input(&again, &on, false, false), 5000.0);
        assert!((alpha(&f, 2) - DIM_FOCUS).abs() < 1e-9);
        assert!(!f.request);
    }

    #[test]
    fn a_tier_change_fades_the_detail_and_then_goes_quiet() {
        let (s, w) = (scene(), Wanted::default());
        let mut m = Motions::default();
        m.frame(&input(&s, &w, false, false), 0.0);
        let mut zoomed = input(&s, &w, false, false);
        zoomed.tier = Some(Tier::Full);
        let f = m.frame(&zoomed, 1000.0);
        assert!(f.request && f.overlay.detail_alpha.is_some());
        let f = m.frame(&zoomed, 1000.0 + 200.0);
        assert!(f.overlay.detail_alpha.is_none() && !f.request);
    }

    #[test]
    fn wanted_reads_the_node_under_the_pointer_and_drops_a_stale_focus() {
        let s = scene();
        let mut v = Interaction {
            hover: Some(Hover {
                group: GroupId(1),
                x: 0.0,
                y: 0.0,
            }),
            focus: Some("gone".into()),
            ..Interaction::default()
        };
        assert_eq!(
            Wanted::of(&s, &v),
            Wanted {
                focus: None,
                hover: Some("b".into())
            }
        );
        v.hover = Some(Hover {
            group: GroupId(3),
            x: 0.0,
            y: 0.0,
        });
        v.focus = Some("c".into());
        v.reach = Reach::Up;
        assert_eq!(
            Wanted::of(&s, &v),
            Wanted {
                focus: Some(("c".into(), Reach::Up)),
                hover: None
            },
            "an edge under the pointer is a tooltip, not a trace"
        );
    }

    /// A motion state on `scene()` with its first frame drawn (the interactions need one).
    fn started() -> (Motions, Arc<Scene>, Wanted) {
        let (s, w) = (scene(), Wanted::default());
        let mut m = Motions::default();
        m.frame(&input(&s, &w, false, false), 0.0);
        (m, s, w)
    }

    #[test]
    fn a_relationship_frames_its_path_once_and_the_chip_says_so() {
        let (mut m, s, w) = started();
        assert!(m.relate("k", "a", "b", 10.0).is_ok());
        let f = m.frame(&input(&s, &w, false, false), 20.0);
        let ask = f.camera.expect("the path is framed");
        assert_eq!(ask.nodes, ["a", "b"]);
        assert_eq!(ask.framing, Framing::Route);
        assert!(f.request, "the comets run");
        assert!(m.frame(&input(&s, &w, false, false), 30.0).camera.is_none(), "asked once");
        let hud = m.hud("k").unwrap();
        assert_eq!(hud.text, "Relationship \u{b7} a \u{2192} b \u{b7} 1 direct edge");
        assert!(!hud.route);
        // Esc ends it; a second Esc has nothing to end.
        assert!(m.end_interaction("k", 40.0) && !m.end_interaction("k", 41.0));
        assert!(m.hud("k").is_none());
    }

    #[test]
    fn an_unconnected_pair_is_refused_with_words_and_changes_nothing() {
        let (mut m, ..) = started();
        let err = m.relate("k", "a", "c", 10.0).unwrap_err();
        assert_eq!(err, "a and c are not connected.");
        assert!(m.hud("k").is_none());
        assert!(m.relate("nope", "a", "b", 0.0).is_err(), "no picture, no relationship");
    }

    #[test]
    fn the_lens_pill_cycles_through_the_kinds_and_back_to_off() {
        let (mut m, s, w) = started();
        assert_eq!(m.lens_label("k"), None);
        m.cycle_lens("k", 10.0).unwrap();
        assert_eq!(m.lens_label("k").as_deref(), Some("backend"));
        assert_eq!(
            m.hud("k").unwrap().text,
            "Lens \u{b7} backend \u{b7} 3 nodes \u{b7} 1 relationship"
        );
        m.cycle_lens("k", 20.0).unwrap();
        assert_eq!(m.lens_label("k"), None, "one kind only: the next is off");
        let f = m.frame(&input(&s, &w, false, false), 5000.0);
        assert!(f.overlay.is_identity() && !f.request);
    }

    #[test]
    fn the_route_keys_begin_step_and_follow_with_the_camera() {
        let (mut m, s, w) = started();
        assert!(!m.route_active("k"));
        m.route("k", RouteKey::Begin("a".into()), 10.0).unwrap();
        let f = m.frame(&input(&s, &w, false, false), 20.0);
        let ask = f.camera.unwrap();
        assert_eq!((ask.nodes, ask.framing), (vec!["a".to_string()], Framing::Step));
        assert_eq!(m.hud("k").unwrap().text, "Route from a \u{b7} n steps on, click a node for a destination");

        m.route("k", RouteKey::Step, 30.0).unwrap();
        let f = m.frame(&input(&s, &w, false, false), 40.0);
        let ask = f.camera.unwrap();
        assert_eq!(ask.nodes, ["a", "b"], "the node before and the one reached");
        assert_eq!(m.hud("k").unwrap().text, "Route \u{b7} a \u{2192} b \u{b7} 2/2");
        assert!(m.hud("k").unwrap().route);

        let end = m.route("k", RouteKey::Step, 50.0).unwrap_err();
        assert!(end.starts_with("End of the line"), "{end}");
        assert_eq!(
            m.route("k", RouteKey::Target("c".into()), 60.0).unwrap_err(),
            "No route: c is not reachable from a along the edges."
        );
        assert_eq!(
            m.route("k", RouteKey::Target("a".into()), 60.0).unwrap_err(),
            "That is where the route starts."
        );
        assert!(m.end_interaction("k", 70.0) && !m.route_active("k"));
    }

    #[test]
    fn a_destination_frames_the_whole_route_and_play_needs_motion_on() {
        let (mut m, s, w) = started();
        m.route("k", RouteKey::Begin("a".into()), 10.0).unwrap();
        m.route("k", RouteKey::Target("b".into()), 20.0).unwrap();
        let ask = m.frame(&input(&s, &w, false, false), 30.0).camera.unwrap();
        assert_eq!((ask.nodes.len(), ask.framing), (2, Framing::Route));
        // Reduced motion: a journey is refused, with a reason.
        let mut reduced = input(&s, &w, false, true);
        reduced.tier = None;
        m.frame(&reduced, 40.0);
        let err = m.route("k", RouteKey::Play, 50.0).unwrap_err();
        assert!(err.starts_with("Nothing to play"), "{err}");
        // Live: it plays and the chip says so; pressing again pauses.
        m.frame(&input(&s, &w, false, false), 60.0);
        m.route("k", RouteKey::Play, 70.0).unwrap();
        assert!(m.hud("k").unwrap().playing);
        m.route("k", RouteKey::Play, 80.0).unwrap();
        assert!(!m.hud("k").unwrap().playing);
    }

    #[test]
    fn a_camera_ask_with_neighbours_frames_the_node_and_the_nodes_beside_it() {
        let s = scene();
        let ask = CameraAsk {
            nodes: vec!["a".into()],
            framing: Framing::Neighbours,
        };
        let boxes = ask_boxes(&s, &ask);
        assert_eq!(boxes.len(), 2, "a and b, not the unconnected c");
        let alone = CameraAsk {
            framing: Framing::Node,
            ..ask
        };
        assert_eq!(ask_boxes(&s, &alone).len(), 1);
    }

    #[test]
    fn a_finder_reveal_asks_for_a_frame_and_a_state_rebuild_forgets_the_interactions() {
        let (mut m, s, w) = started();
        m.reveal("k", vec!["b".into()], Framing::Neighbours);
        assert!(m.frame(&input(&s, &w, false, false), 10.0).camera.is_some());
        m.relate("k", "a", "b", 20.0).unwrap();
        // An edit rebuilds the state: the relationship is gone with it.
        let again = Arc::new((*s).clone());
        m.frame(&input(&again, &w, false, false), 30.0);
        assert!(m.hud("k").is_none());
    }
}
