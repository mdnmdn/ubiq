//! The camera tween (P7.2, M10; `00` §6.4, `04` §4): a move of the picture's camera that glides
//! instead of jumping, and gives way to the first wheel, drag or key.
//!
//! The maths is the core's (`ubiq_archify::motion`: out-cubic, 420 ms clamped 180-520, `frame_to_nodes`
//! with padding 48, `0.9·fit`, max 2.15 and 1.9 on a narrow panel). This module is the half that
//! needs the viewport: it works in the *base viewport's* space (`offset + p·scale` from the panel's
//! top-left, `crate::state::viewport::Camera`), converts the core's Archify space to it, and applies a
//! sample through the base's public API. The base has no "set camera" (D14), so a sample is a zoom
//! about the panel's origin and a drag by the remainder.
//!
//! **Frames.** Nothing ticks while idle. [`drive`] is called from the viewer's render; only when a
//! move is in flight does it ask the window for one more frame (`Window::on_next_frame`), whose
//! callback applies the sample and notifies, which renders, which asks again. The last sample ends
//! the chain.
//!
//! **Interruption.** Every sample is read back after it is applied. If the viewport's camera is not
//! what was last applied, somebody else moved it (wheel, drag, pinch, Fit, a key): the move is
//! dropped and the viewport's own value, the one on screen, becomes the manual state.

use std::collections::HashMap;
use std::time::Instant;

use ubiq_archify::motion::{
    CameraMove, CameraState, FrameOptions, MotionConfig, frame_to_nodes as core_frame, start_move,
};
use ubiq_archify::scene::Bounds;
use gpui::{Context, Window, point, px};
use crate::app::AppState;
use crate::state::viewport::Viewport;

use crate::state::archify::ui;

/// Below this panel width the max scale is 1.9 rather than 2.15 (`04` §4: the mobile threshold).
pub const NARROW_PX: f64 = 720.0;
/// The max scale on a narrow panel.
pub const NARROW_MAX_SCALE: f64 = 1.9;

/// A move older than this without a sample is assumed lost (the window closed under it) and may be
/// scheduled again.
const LOST_AFTER_MS: f64 = 250.0;

// --------------------------------------------------------------------------------------------- //
// Space
// --------------------------------------------------------------------------------------------- //

/// The base viewport's camera as a [`CameraState`]: `scale` and the panel offset.
pub fn rendered(vp: &Viewport) -> CameraState {
    let c = vp.camera(vp.content, vp.panel_w, vp.panel_h);
    CameraState {
        scale: f64::from(c.scale),
        x: f64::from(c.offset_x),
        y: f64::from(c.offset_y),
    }
}

/// Archify's camera (`translate(x, y) scale(s)` over the viewBox fitted and centred in the panel
/// without a margin) as the base's: a viewBox point `p` lands at `x + s·(off + p·fit)`.
pub fn to_base(a: CameraState, view_box: [f64; 2], panel: [f64; 2]) -> CameraState {
    let fit = (panel[0] / view_box[0]).min(panel[1] / view_box[1]);
    let off = [
        (panel[0] - view_box[0] * fit) / 2.0,
        (panel[1] - view_box[1] * fit) / 2.0,
    ];
    CameraState {
        scale: a.scale * fit,
        x: a.x + a.scale * off[0],
        y: a.y + a.scale * off[1],
    }
}

/// What a camera move frames, and so how far it may zoom (`00` §6.4): a node (padding 48, up to
/// 2.15), a node with its neighbours (1.9), a whole route or relationship (2.15), or one step of a
/// route journey (padding 64, up to 1.65).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Framing {
    Node,
    Neighbours,
    Route,
    Step,
}

impl Framing {
    /// The core's options, the cap lowered to 1.9 on a narrow panel.
    pub fn options(self, panel_width: f64) -> FrameOptions {
        let base = match self {
            Framing::Node | Framing::Route => FrameOptions::default(),
            Framing::Neighbours => FrameOptions {
                include_neighbours: true,
                ..FrameOptions::default()
            },
            Framing::Step => FrameOptions {
                max_scale: Some(STEP_MAX_SCALE),
                padding: STEP_PADDING,
                ..FrameOptions::default()
            },
        };
        let narrow = (panel_width < NARROW_PX).then_some(NARROW_MAX_SCALE);
        FrameOptions {
            max_scale: match (base.max_scale, narrow) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, b) => a.or(b),
            },
            ..base
        }
    }
}

/// The max scale of a journey step, and the room kept around it (`route-probe.js:535`).
pub const STEP_MAX_SCALE: f64 = 1.65;
pub const STEP_PADDING: f64 = 64.0;

/// The base-space camera that frames `boxes` (viewBox units) as `framing` says. `None` for no
/// boxes or an unmeasured panel.
pub fn frame(
    panel: [f32; 2],
    view_box: [f32; 2],
    boxes: &[Bounds],
    framing: Framing,
) -> Option<CameraState> {
    let panel = panel.map(f64::from);
    let view_box = view_box.map(f64::from);
    let a = core_frame(panel, view_box, boxes, &framing.options(panel[0]))?;
    Some(to_base(a, view_box, panel))
}

/// The base-space camera that frames `boxes` (viewBox units): padding 48, `0.9·fit`, scale in
/// `[1, 2.15]` times the fit (1.9 on a narrow panel). `None` for no boxes or an unmeasured panel.
pub fn frame_to_nodes(panel: [f32; 2], view_box: [f32; 2], boxes: &[Bounds]) -> Option<CameraState> {
    frame(panel, view_box, boxes, Framing::Node)
}

/// How to reach `target` from `cur` with the base's API: a zoom by `factor` about the panel's
/// origin (an offset `o` becomes `o·factor`), then a drag by `delta`. `factor` is `None` when the
/// scale already matches.
pub fn plan(cur: CameraState, target: CameraState) -> (Option<f32>, [f32; 2]) {
    let factor = target.scale / cur.scale;
    let zooms = (factor - 1.0).abs() > 1e-6;
    let f = if zooms { factor } else { 1.0 };
    (
        zooms.then_some(factor as f32),
        [(target.x - cur.x * f) as f32, (target.y - cur.y * f) as f32],
    )
}

/// Whether `now` is not where the last sample left the camera: a manual move.
pub fn moved(last: CameraState, now: CameraState) -> bool {
    (last.scale - now.scale).abs() > 1e-3 * last.scale.abs().max(1.0)
        || (last.x - now.x).abs() > 0.5
        || (last.y - now.y).abs() > 0.5
}

/// The switches the camera honours: the Diagrams "Motion" setting off, or the OS reduced-motion
/// preference, makes a move instant.
pub fn config_for(motion_on: bool, os_reduced: bool) -> MotionConfig {
    MotionConfig {
        system_reduced: !motion_on || os_reduced,
        ..MotionConfig::default()
    }
}

// --------------------------------------------------------------------------------------------- //
// The moves in flight
// --------------------------------------------------------------------------------------------- //

/// What a frame should do for one picture.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Step {
    /// No move in flight.
    Idle,
    /// Show this sample and ask for another frame.
    Apply(CameraState),
    /// Show the end state; the move is over.
    Done(CameraState),
    /// Somebody else moved the camera: it is theirs now, at the rendered state given.
    Interrupted(CameraState),
}

#[derive(Clone, Copy, Debug)]
struct Active {
    mv: CameraMove,
    /// What the camera was left at by the last sample (or the start).
    last: CameraState,
    /// When a frame was asked for, until it is delivered.
    scheduled: Option<f64>,
}

/// The camera moves in flight, per picture key. Lives in the `Ui` global.
pub struct Cameras {
    epoch: Instant,
    active: HashMap<String, Active>,
}

impl Default for Cameras {
    fn default() -> Self {
        Self {
            epoch: Instant::now(),
            active: HashMap::new(),
        }
    }
}

impl Cameras {
    pub fn now_ms(&self) -> f64 {
        self.epoch.elapsed().as_secs_f64() * 1000.0
    }

    pub fn is_active(&self, key: &str) -> bool {
        self.active.contains_key(key)
    }

    /// Start a move of `key` from the rendered `from` to `to` at `now_ms`. A move already in
    /// flight is replaced, and the new one continues from the rendered state, not from its old
    /// start. Nothing starts when the camera is already there.
    pub fn begin(
        &mut self,
        key: &str,
        config: &MotionConfig,
        from: CameraState,
        to: CameraState,
        now_ms: f64,
    ) {
        if !moved(from, to) {
            self.active.remove(key);
            return;
        }
        let mv = start_move(config, from, to, now_ms, None, false);
        self.active.insert(
            key.to_string(),
            Active {
                mv,
                last: from,
                scheduled: None,
            },
        );
    }

    /// Whether the caller should ask the window for a frame for `key`: a move is in flight and no
    /// frame is already on its way. Marks one as asked.
    pub fn claim_frame(&mut self, key: &str, now_ms: f64) -> bool {
        let Some(a) = self.active.get_mut(key) else {
            return false;
        };
        if a.scheduled.is_some_and(|at| now_ms - at < LOST_AFTER_MS) {
            return false;
        }
        a.scheduled = Some(now_ms);
        true
    }

    /// The frame arrived: `current` is the camera as rendered now.
    pub fn step(&mut self, key: &str, current: CameraState, now_ms: f64) -> Step {
        let Some(a) = self.active.get_mut(key) else {
            return Step::Idle;
        };
        a.scheduled = None;
        if moved(a.last, current) {
            self.active.remove(key);
            return Step::Interrupted(current);
        }
        match a.mv {
            CameraMove::Instant(to) => {
                self.active.remove(key);
                Step::Done(to)
            }
            CameraMove::Tween(t) if t.is_done(now_ms) => {
                self.active.remove(key);
                Step::Done(t.to)
            }
            CameraMove::Tween(t) => Step::Apply(t.at(now_ms)),
        }
    }

    /// Record what the camera reads after a sample was applied.
    pub fn applied(&mut self, key: &str, camera: CameraState) {
        if let Some(a) = self.active.get_mut(key) {
            a.last = camera;
        }
    }
}

// --------------------------------------------------------------------------------------------- //
// The driver
// --------------------------------------------------------------------------------------------- //

/// Move picture `key`'s camera to `target` (base space), gliding unless motion is off. Call from
/// render, once the panel is measured; [`drive`] then keeps the frames coming.
pub fn glide_to(app: &AppState, key: &str, target: CameraState, cx: &mut Context<AppState>) {
    let vp = app.viewport(key);
    if !vp.measured() {
        return;
    }
    let config = config_for(ui(cx).settings.motion, cx.reduce_motion());
    let cameras = &mut ui(cx).cameras;
    let now = cameras.now_ms();
    cameras.begin(key, &config, rendered(&vp), target, now);
}

/// Ask for the next frame if `key` has a move in flight. Call from render; idle costs nothing.
pub fn drive(key: &str, window: &mut Window, cx: &mut Context<AppState>) {
    let cameras = &mut ui(cx).cameras;
    let now = cameras.now_ms();
    if !cameras.claim_frame(key, now) {
        return;
    }
    let entity = cx.entity();
    let key = key.to_string();
    window.on_next_frame(move |_, cx| {
        entity.update(cx, |app, cx| tick(app, &key, cx));
    });
}

/// The frame arrived: sample, apply, and notify so the next render asks again.
fn tick(app: &mut AppState, key: &str, cx: &mut Context<AppState>) {
    let current = rendered(&app.viewport(key));
    let cameras = &mut ui(cx).cameras;
    let now = cameras.now_ms();
    match cameras.step(key, current, now) {
        Step::Idle | Step::Interrupted(_) => {}
        Step::Apply(sample) => {
            apply(app, key, sample, cx);
            let after = rendered(&app.viewport(key));
            ui(cx).cameras.applied(key, after);
            cx.notify();
        }
        Step::Done(end) => {
            apply(app, key, end, cx);
            cx.notify();
        }
    }
}

/// Put the base camera at `target` through its public API: zoom about the panel's origin, then
/// drag by what is left.
fn apply(app: &mut AppState, key: &str, target: CameraState, cx: &mut Context<AppState>) {
    let vp = app.viewport(key);
    let origin = point(px(vp.origin_x), px(vp.origin_y));
    let (factor, [dx, dy]) = plan(rendered(&vp), target);
    if let Some(factor) = factor {
        app.zoom_viewport(key, factor, origin, cx);
    }
    if dx.abs() > 1e-3 || dy.abs() > 1e-3 {
        app.start_viewport_drag(key, origin, cx);
        app.drag_viewport(key, point(origin.x + px(dx), origin.y + px(dy)), cx);
        app.end_viewport_drag(cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ubiq_archify::motion::{CameraTween, consts};
    use crate::state::viewport::Content;

    const A: CameraState = CameraState { scale: 1.0, x: 0.0, y: 0.0 };
    const B: CameraState = CameraState { scale: 2.0, x: -100.0, y: -40.0 };

    fn live() -> MotionConfig {
        config_for(true, false)
    }

    fn tween(cams: &mut Cameras, from: CameraState, to: CameraState, at: f64) {
        cams.begin("k", &live(), from, to, at);
    }

    #[test]
    fn the_tween_is_out_cubic_over_420_ms() {
        let t = CameraTween::new(A, B, 1000.0, consts::TWEEN_DEFAULT_MS);
        assert_eq!(t.at(1000.0), A);
        assert_eq!(t.at(1420.0), B);
        // Out-cubic at the half: 1 - 0.5^3 = 0.875 of the way.
        let mid = t.at(1210.0);
        assert!((mid.scale - 1.875).abs() < 1e-9, "{mid:?}");
        assert!((mid.x - -87.5).abs() < 1e-9 && (mid.y - -35.0).abs() < 1e-9);
    }

    #[test]
    fn a_move_runs_then_ends_on_the_target() {
        let mut cams = Cameras::default();
        tween(&mut cams, A, B, 0.0);
        assert!(cams.is_active("k"));
        let Step::Apply(s) = cams.step("k", A, 100.0) else { panic!("mid-flight") };
        assert!(s.scale > 1.0 && s.scale < 2.0);
        cams.applied("k", s);
        assert_eq!(cams.step("k", s, 420.0), Step::Done(B));
        assert!(!cams.is_active("k"));
        assert_eq!(cams.step("k", B, 500.0), Step::Idle);
    }

    #[test]
    fn a_manual_move_interrupts_and_keeps_the_rendered_state() {
        let mut cams = Cameras::default();
        tween(&mut cams, A, B, 0.0);
        let Step::Apply(s) = cams.step("k", A, 100.0) else { panic!() };
        cams.applied("k", s);
        // The wheel zoomed between frames: the camera is not where the sample left it.
        let manual = CameraState { scale: s.scale * 1.2, ..s };
        assert_eq!(cams.step("k", manual, 120.0), Step::Interrupted(manual));
        assert!(!cams.is_active("k"), "the move is dropped, not resumed");
        assert_eq!(cams.step("k", manual, 140.0), Step::Idle);
    }

    #[test]
    fn a_new_move_continues_from_the_rendered_state() {
        let mut cams = Cameras::default();
        tween(&mut cams, A, B, 0.0);
        let rendered_now = CameraTween::new(A, B, 0.0, 420.0).interrupt(100.0);
        // Replace mid-flight, starting where the camera is.
        tween(&mut cams, rendered_now, A, 100.0);
        let Step::Apply(s) = cams.step("k", rendered_now, 101.0) else { panic!() };
        assert!((s.scale - rendered_now.scale).abs() < 0.05, "no jump back to the old start");
    }

    #[test]
    fn the_duration_is_clamped_and_a_move_to_here_is_nothing() {
        use ubiq_archify::motion::tween_duration_ms;
        assert_eq!(tween_duration_ms(Some(10.0)), consts::TWEEN_MIN_MS);
        assert_eq!(tween_duration_ms(Some(9000.0)), consts::TWEEN_MAX_MS);
        assert_eq!(tween_duration_ms(None), 420.0);
        let mut cams = Cameras::default();
        tween(&mut cams, A, A, 0.0);
        assert!(!cams.is_active("k"));
    }

    #[test]
    fn reduced_motion_or_motion_off_jumps() {
        for cfg in [config_for(false, false), config_for(true, true)] {
            let mut cams = Cameras::default();
            cams.begin("k", &cfg, A, B, 0.0);
            assert_eq!(cams.step("k", A, 1.0), Step::Done(B), "instant: one frame, the end state");
        }
    }

    #[test]
    fn one_frame_is_asked_for_at_a_time() {
        let mut cams = Cameras::default();
        assert!(!cams.claim_frame("k", 0.0), "idle asks for nothing");
        tween(&mut cams, A, B, 0.0);
        assert!(cams.claim_frame("k", 1.0));
        assert!(!cams.claim_frame("k", 2.0), "already on its way");
        assert!(cams.claim_frame("k", 1.0 + LOST_AFTER_MS + 1.0), "a lost frame is asked again");
        let _ = cams.step("k", A, 3.0);
        assert!(cams.claim_frame("k", 4.0), "delivered, so the next may be asked");
    }

    #[test]
    fn frame_to_nodes_centres_the_box_with_the_documented_limits() {
        // A 1000x500 viewBox in a 1000x500 panel (fit 1); a 100x50 node at (400, 200).
        let node = Bounds::new(400.0, 200.0, 100.0, 50.0);
        let c = frame_to_nodes([1000.0, 500.0], [1000.0, 500.0], &[node]).unwrap();
        assert_eq!(c.scale, 2.15, "capped, not 0.9*fit");
        // Its centre (450, 225) lands on the free rect's centre (500, 214): bottom keeps 72.
        assert!((c.x + 450.0 * c.scale - 500.0).abs() < 1e-9);
        assert!((c.y + 225.0 * c.scale - (48.0 + (500.0 - 72.0)) / 2.0).abs() < 1e-9);
        // Narrow panel: 1.9.
        let n = frame_to_nodes([600.0, 500.0], [600.0, 500.0], &[node]).unwrap();
        assert_eq!(n.scale, 1.9);
        // The whole diagram frames at the overview.
        let all = Bounds::new(0.0, 0.0, 1000.0, 500.0);
        assert_eq!(frame_to_nodes([1000.0, 500.0], [1000.0, 500.0], &[all]).unwrap().scale, 1.0);
        assert!(frame_to_nodes([1000.0, 500.0], [1000.0, 500.0], &[]).is_none());
    }

    #[test]
    fn base_space_matches_where_archify_puts_a_point() {
        // viewBox 400x200 in a 800x800 panel: fit 2, centred vertically (offset 200).
        let a = CameraState { scale: 1.5, x: -30.0, y: 12.0 };
        let b = to_base(a, [400.0, 200.0], [800.0, 800.0]);
        let p = [100.0, 50.0];
        let archify = [a.x + a.scale * (0.0 + p[0] * 2.0), a.y + a.scale * (200.0 + p[1] * 2.0)];
        let base = [b.x + p[0] * b.scale, b.y + p[1] * b.scale];
        assert!((archify[0] - base[0]).abs() < 1e-9 && (archify[1] - base[1]).abs() < 1e-9);
    }

    #[test]
    fn the_plan_reaches_the_target_through_the_base_api() {
        let content = Content::from_size(400.0, 200.0);
        let mut vp = Viewport::default();
        vp.set_panel(800.0, 600.0, 10.0, 20.0);
        vp.set_content(content);
        let target = CameraState { scale: 3.0, x: -250.0, y: -90.0 };

        let (factor, [dx, dy]) = plan(rendered(&vp), target);
        let origin = (vp.origin_x, vp.origin_y);
        vp.zoom_at(factor.unwrap(), origin.0, origin.1);
        vp.pan_by(dx, dy);
        let got = rendered(&vp);
        assert!((got.scale - 3.0).abs() < 1e-3, "{got:?}");
        assert!((got.x - -250.0).abs() < 0.01 && (got.y - -90.0).abs() < 0.01, "{got:?}");

        // A pure pan needs no zoom.
        let (factor, _) = plan(got, CameraState { x: 0.0, ..got });
        assert!(factor.is_none());
    }

    #[test]
    fn each_framing_has_its_own_cap_and_a_narrow_panel_lowers_it() {
        let cap = |f: Framing, w: f64| {
            let o = f.options(w);
            (o.max_scale, o.padding, o.include_neighbours)
        };
        assert_eq!(cap(Framing::Node, 1000.0), (None, 48.0, false), "the core's 2.15");
        assert_eq!(cap(Framing::Neighbours, 1000.0), (None, 48.0, true), "the core's 1.9");
        assert_eq!(cap(Framing::Route, 1000.0), (None, 48.0, false));
        assert_eq!(cap(Framing::Step, 1000.0), (Some(1.65), 64.0, false));
        assert_eq!(cap(Framing::Node, 600.0).0, Some(1.9));
        assert_eq!(cap(Framing::Step, 600.0).0, Some(1.65), "a journey step is already below 1.9");
    }

    #[test]
    fn a_step_never_zooms_past_165_and_a_node_goes_to_215() {
        let boxes = [Bounds::new(450.0, 300.0, 40.0, 20.0)];
        let (panel, vb) = ([1000.0, 700.0], [1000.0, 700.0]);
        let scale = |f| frame(panel, vb, &boxes, f).unwrap().scale;
        assert!((scale(Framing::Node) - 2.15).abs() < 1e-9);
        assert!((scale(Framing::Step) - 1.65).abs() < 1e-9);
        assert_eq!(frame_to_nodes(panel, vb, &boxes), frame(panel, vb, &boxes, Framing::Node));
        assert_eq!(frame(panel, vb, &[], Framing::Route), None);
    }
}
