//! M10: the camera tween and frame-to-nodes, as pure functions (`V/viewer-camera.js:308-407`,
//! `00` §6.4). The UI applies the result; nothing here knows the viewport widget.
//!
//! The state is Archify's: `{scale, x, y}` with the content (the viewBox fitted and centred in the
//! viewport) drawn as `translate(x, y) scale(scale)`, all in viewport px. The desktop clamp
//! (`scale` in `[1, 3]`, content always covering the viewport) is [`CameraState::clamped`]; the
//! JS applies it on every frame, the caller decides whether the base viewport already does.

use serde::{Deserialize, Serialize};

use super::config::MotionConfig;
use super::timeline::{Easing, Ms, Timeline, lerp};
use crate::scene::Bounds;

pub const TWEEN_DEFAULT_MS: f64 = 420.0;
pub const TWEEN_MIN_MS: f64 = 180.0;
pub const TWEEN_MAX_MS: f64 = 520.0;
pub const PADDING: f64 = 48.0;
/// The bottom edge keeps at least this much room (`Math.max(padding, 72)`).
pub const BOTTOM_MIN: f64 = 72.0;
pub const FIT_FACTOR: f64 = 0.9;
pub const MAX_SCALE: f64 = 2.15;
pub const MAX_SCALE_NEIGHBOURS: f64 = 1.9;
/// A computed scale below this snaps to 1.
pub const SNAP_BELOW: f64 = 1.08;
pub const MIN_ZOOM: f64 = 1.0;
pub const MAX_ZOOM: f64 = 3.0;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct CameraState {
    pub scale: f64,
    pub x: f64,
    pub y: f64,
}

impl CameraState {
    pub const OVERVIEW: CameraState = CameraState { scale: 1.0, x: 0.0, y: 0.0 };

    /// `x` in `[w - w*scale, 0]`, `y` in `[h - h*scale, 0]`: the content always covers the viewport.
    pub fn clamped(self, viewport: [f64; 2]) -> CameraState {
        let [w, h] = viewport;
        CameraState {
            scale: self.scale,
            x: self.x.max(w - w * self.scale).min(0.0),
            y: self.y.max(h - h * self.scale).min(0.0),
        }
    }
}

/// The tween duration: the requested one clamped to 180-520 ms, 420 when absent or 0.
pub fn tween_duration_ms(requested: Option<f64>) -> f64 {
    match requested {
        Some(d) if d.is_finite() && d != 0.0 => d.clamp(TWEEN_MIN_MS, TWEEN_MAX_MS),
        _ => TWEEN_DEFAULT_MS,
    }
}

/// One camera move in flight: `lerp` from `from` to `to` with out-cubic.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct CameraTween {
    pub from: CameraState,
    pub to: CameraState,
    pub timeline: Timeline,
}

impl CameraTween {
    pub fn new(from: CameraState, to: CameraState, start_ms: Ms, duration_ms: Ms) -> Self {
        Self { from, to, timeline: Timeline::new(start_ms, duration_ms, Easing::OutCubic) }
    }

    /// The state at `t` (the end state once finished).
    pub fn at(&self, t: Ms) -> CameraState {
        let e = self.timeline.eased(t);
        CameraState {
            scale: lerp(self.from.scale, self.to.scale, e),
            x: lerp(self.from.x, self.to.x, e),
            y: lerp(self.from.y, self.to.y, e),
        }
    }

    pub fn is_done(&self, t: Ms) -> bool {
        self.timeline.is_done(t)
    }

    /// Manual input mid-flight: the *rendered* state to continue from, in manual mode. The
    /// tween is dropped by the caller.
    pub fn interrupt(&self, t: Ms) -> CameraState {
        self.at(t)
    }
}

/// How a requested move is carried out.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CameraMove {
    /// Reduced motion, a hidden window or an explicit `instant`: jump.
    Instant(CameraState),
    Tween(CameraTween),
}

/// Start a move from the *rendered* state `from` (so an interrupted tween continues from where it
/// was). Instant if asked, under reduced motion or hidden window ([`MotionConfig::instant`]).
pub fn start_move(
    config: &MotionConfig,
    from: CameraState,
    to: CameraState,
    now_ms: Ms,
    duration_ms: Option<f64>,
    instant: bool,
) -> CameraMove {
    if instant || config.instant() {
        CameraMove::Instant(to)
    } else {
        CameraMove::Tween(CameraTween::new(from, to, now_ms, tween_duration_ms(duration_ms)))
    }
}

/// Room kept free of overlay chrome (the focus chip, a route receipt), in viewport px.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Insets {
    pub left: f64,
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameOptions {
    pub padding: f64,
    /// `None`: 2.15, or 1.9 with neighbours. Journeys and stories pass 1.65.
    pub max_scale: Option<f64>,
    pub include_neighbours: bool,
    pub insets: Insets,
}

impl Default for FrameOptions {
    fn default() -> Self {
        Self { padding: PADDING, max_scale: None, include_neighbours: false, insets: Insets::default() }
    }
}

/// The camera that frames `boxes` (node bounds in view-box units): target scale
/// `0.9 * min(availW / bw, availH / bh)` clamped to `[1, max]` (below 1.08 it snaps to 1, rounded
/// to 2 dp), the union box centred in the free rect. `None` for no boxes or no free room.
pub fn frame_to_nodes(
    viewport: [f64; 2],
    view_box: [f64; 2],
    boxes: &[Bounds],
    opts: &FrameOptions,
) -> Option<CameraState> {
    let [w, h] = viewport;
    let [vb_w, vb_h] = view_box;
    if boxes.is_empty() || vb_w <= 0.0 || vb_h <= 0.0 || w <= 0.0 || h <= 0.0 {
        return None;
    }
    let fit = (w / vb_w).min(h / vb_h);
    let (off_x, off_y) = ((w - vb_w * fit) / 2.0, (h - vb_h * fit) / 2.0);
    let (mut min_x, mut min_y) = (f64::MAX, f64::MAX);
    let (mut max_x, mut max_y) = (f64::MIN, f64::MIN);
    for b in boxes {
        min_x = min_x.min(b.x);
        min_y = min_y.min(b.y);
        max_x = max_x.max(b.x + b.width);
        max_y = max_y.max(b.y + b.height);
    }
    let (bx, by) = (off_x + min_x * fit, off_y + min_y * fit);
    let (bw, bh) = (((max_x - min_x) * fit).max(1.0), ((max_y - min_y) * fit).max(1.0));

    let pad = if opts.padding > 0.0 { opts.padding } else { PADDING };
    let ins = opts.insets;
    let left = pad.max(ins.left);
    let right = (w - pad).min(w - ins.right);
    let top = pad.max(ins.top);
    let bottom = (h - pad.max(BOTTOM_MIN)).min(h - ins.bottom);
    if right <= left || bottom <= top {
        return None;
    }
    let max_scale = opts
        .max_scale
        .unwrap_or(if opts.include_neighbours { MAX_SCALE_NEIGHBOURS } else { MAX_SCALE });
    let mut scale = ((right - left) / bw).min((bottom - top) / bh) * FIT_FACTOR;
    scale = scale.clamp(MIN_ZOOM, max_scale);
    if scale < SNAP_BELOW {
        scale = 1.0;
    }
    let scale = (scale * 100.0).round() / 100.0;
    Some(CameraState {
        scale,
        x: (left + right) / 2.0 - (bx + bw / 2.0) * scale,
        y: (top + bottom) / 2.0 - (by + bh / 2.0) * scale,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duration_is_clamped_to_180_520_with_420_default() {
        assert_eq!(tween_duration_ms(None), 420.0);
        assert_eq!(tween_duration_ms(Some(0.0)), 420.0);
        assert_eq!(tween_duration_ms(Some(50.0)), 180.0);
        assert_eq!(tween_duration_ms(Some(320.0)), 320.0);
        assert_eq!(tween_duration_ms(Some(9000.0)), 520.0);
        assert_eq!(tween_duration_ms(Some(f64::NAN)), 420.0);
    }

    #[test]
    fn the_tween_is_out_cubic_and_lands_exactly() {
        let a = CameraState::OVERVIEW;
        let b = CameraState { scale: 2.0, x: -100.0, y: -40.0 };
        let tw = CameraTween::new(a, b, 1000.0, 400.0);
        assert_eq!(tw.at(1000.0), a);
        let mid = tw.at(1200.0);
        assert!((mid.scale - (1.0 + 0.875)).abs() < 1e-12, "out-cubic at .5 is .875");
        assert!((mid.x - (-87.5)).abs() < 1e-9);
        assert_eq!(tw.at(1400.0), b);
        assert_eq!(tw.at(99_999.0), b);
        assert!(!tw.is_done(1399.0) && tw.is_done(1400.0));
    }

    #[test]
    fn interrupting_continues_from_the_rendered_state() {
        let a = CameraState::OVERVIEW;
        let b = CameraState { scale: 2.0, x: -100.0, y: 0.0 };
        let tw = CameraTween::new(a, b, 0.0, 420.0);
        let live = tw.interrupt(100.0);
        assert_eq!(live, tw.at(100.0));
        let CameraMove::Tween(next) =
            start_move(&MotionConfig::default(), live, a, 100.0, Some(300.0), false)
        else {
            panic!("a live camera tweens")
        };
        assert_eq!(next.at(100.0), live, "no jump when replaced");
        assert_eq!(next.timeline.duration_ms, 300.0);
    }

    #[test]
    fn reduced_motion_hidden_or_asked_is_instant() {
        let to = CameraState { scale: 1.5, x: -1.0, y: -2.0 };
        let live = MotionConfig::default();
        let mv = |c: &MotionConfig, inst| start_move(c, CameraState::OVERVIEW, to, 0.0, None, inst);
        assert!(matches!(mv(&live, false), CameraMove::Tween(_)));
        assert_eq!(mv(&live, true), CameraMove::Instant(to));
        let reduced = MotionConfig { system_reduced: true, ..MotionConfig::default() };
        assert_eq!(mv(&reduced, false), CameraMove::Instant(to));
        let mut hidden = MotionConfig::default();
        hidden.suspend(MotionConfig::VISIBILITY);
        assert_eq!(mv(&hidden, false), CameraMove::Instant(to));
        let still = MotionConfig { mode: crate::motion::Mode::Still, ..MotionConfig::default() };
        assert!(matches!(mv(&still, false), CameraMove::Tween(_)), "Still parks signals, not the camera");
    }

    #[test]
    fn the_clamp_keeps_the_content_covering_the_viewport() {
        let c = CameraState { scale: 2.0, x: 50.0, y: -9999.0 }.clamped([800.0, 600.0]);
        assert_eq!((c.x, c.y), (0.0, -600.0));
        let o = CameraState { scale: 1.0, x: -30.0, y: 12.0 }.clamped([800.0, 600.0]);
        assert_eq!((o.x, o.y), (0.0, 0.0));
    }

    #[test]
    fn frame_to_nodes_centres_a_small_box_and_uses_09_of_the_fit() {
        // viewport 1000x700, view box 1000x700 (fit 1): a 100x50 box at (450, 300).
        let b = [Bounds::new(450.0, 300.0, 100.0, 50.0)];
        let cam = frame_to_nodes([1000.0, 700.0], [1000.0, 700.0], &b, &FrameOptions::default()).unwrap();
        // free rect: x 48..952 (904), y 48..(700-72=628) (580); min(9.04, 11.6) * .9 = 8.136, capped at 2.15.
        assert_eq!(cam.scale, 2.15);
        let (cx, cy) = (500.0 * 2.15, 325.0 * 2.15);
        assert!((cam.x - (500.0 - cx)).abs() < 1e-9 && (cam.y - (338.0 - cy)).abs() < 1e-9);
    }

    #[test]
    fn frame_to_nodes_fits_a_big_box_and_snaps_small_gains_to_one() {
        let vp = [1000.0, 700.0];
        // A box 600 wide: (904 / 600) * .9 = 1.356 -> 1.36.
        let b = [Bounds::new(100.0, 100.0, 600.0, 100.0)];
        let cam = frame_to_nodes(vp, vp, &b, &FrameOptions::default()).unwrap();
        assert_eq!(cam.scale, 1.36);
        // Nearly the whole view: below 1.08 snaps to 1.
        let big = [Bounds::new(0.0, 0.0, 950.0, 600.0)];
        assert_eq!(frame_to_nodes(vp, vp, &big, &FrameOptions::default()).unwrap().scale, 1.0);
    }

    #[test]
    fn frame_to_nodes_caps_follow_the_options_and_none_when_there_is_no_room() {
        let vp = [1000.0, 700.0];
        let b = [Bounds::new(450.0, 300.0, 10.0, 10.0)];
        let scale = |o: FrameOptions| frame_to_nodes(vp, vp, &b, &o).unwrap().scale;
        assert_eq!(scale(FrameOptions { include_neighbours: true, ..Default::default() }), 1.9);
        assert_eq!(scale(FrameOptions { max_scale: Some(1.65), padding: 64.0, ..Default::default() }), 1.65);
        assert!(frame_to_nodes(vp, vp, &[], &FrameOptions::default()).is_none());
        let walled = FrameOptions { insets: Insets { left: 990.0, ..Insets::default() }, ..Default::default() };
        assert!(frame_to_nodes(vp, vp, &b, &walled).is_none());
    }

    #[test]
    fn frame_to_nodes_accounts_for_a_letterboxed_view_box() {
        // view box 100x100 in a 1000x500 viewport: fit 5, offset x 250.
        let b = [Bounds::new(40.0, 40.0, 20.0, 20.0)];
        let cam = frame_to_nodes([1000.0, 500.0], [100.0, 100.0], &b, &FrameOptions::default()).unwrap();
        // The box centre (50, 50) maps to (250 + 250, 250) in the viewport; scale s keeps it centred
        // in the free rect, whose centre is (500, (48 + 428) / 2 = 238).
        assert!((cam.x + 500.0 * cam.scale - 500.0).abs() < 1e-9);
        assert!((cam.y + 250.0 * cam.scale - 238.0).abs() < 1e-9);
    }
}
