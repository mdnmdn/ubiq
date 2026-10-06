//! Animation: the pure motion model and `sample(scene, state, t)` (P7.1). `00` §6, `04`.
//! No GPUI, no clock, no thread: every function is deterministic and golden-testable.
//!
//! # Shape
//!
//! | Piece | Role |
//! |---|---|
//! | [`MotionConfig`] | the governor's switches: capable (`meta.animation`), Live/Still, OS reduced motion, refcounted suspensions; `effective_paused` |
//! | [`Owners`], [`Owner`], [`Token`] | owner priority route > lens > relationship > intent > focus > legend; explicit `claim`/`release` with monotonic tokens |
//! | [`Timeline`], [`Easing`] | a single run (`start`, `delay`, `duration`, easing); out-cubic, ease, ease-in-out, beziers; keyframes |
//! | [`Steps`] and the A/W rules | the ambient `min(12, step) * 160 ms` delays, from id lists or from a scene |
//! | [`CometKind`], [`comet_frame`] | the 5 parameter sets; the arclength slice of a flattened polyline at `t` |
//! | [`Ambient`] | M1/M2 lifecycle (starts at most once, then settles) plus the edge dashes and node pulse |
//! | [`Dim`], [`LodFade`] | M12 dim/focus (180 ms) and M11 detail (160 ms) fades as group/detail multipliers |
//! | [`Dwell`] | tick-driven step timer with `elapsed` that survives pause |
//! | [`camera`] | M10: [`CameraTween`] (out-cubic 420 ms, clamped 180-520), [`frame_to_nodes`] |
//! | `interact`, [`lens`], [`path`] | M5-M8 on [`MotionState`]: `relate`, `set_lens`, `route_*` (probe, step, journey); the pure selection sets and BFS paths they use |
//! | [`MotionState`] | all the mutable data in one serde value; `tick`, `is_active` |
//! | [`sample`] | `sample(&Scene, &MotionState, t) -> `[`Overlay`], the one pure function the painter calls |
//!
//! # Contract for the UI (P7.3)
//!
//! Hold one [`MotionState`] per document. On every event call its method with the monotonic time
//! `now`; on every frame call [`MotionState::tick`], paint the static scene, then the
//! [`Overlay`] (see its z-order notes), and request another frame only while
//! [`MotionState::is_active`]. When idle the overlay is empty and nothing is requested. The
//! camera tween is separate: [`camera`] gives target states, the UI owns the in-flight
//! [`CameraTween`] and sampling it is `tween.at(now)`.

mod ambient;
pub mod camera;
mod comet;
mod config;
mod dim;
mod dwell;
mod interact;
pub mod lens;
pub mod path;
mod overlay;
mod owner;
mod sample;
mod state;
mod step;
mod timeline;

pub use ambient::{Ambient, Phase, Settle};
pub use camera::{
    CameraMove, CameraState, CameraTween, FrameOptions, Insets, frame_to_nodes, start_move, tween_duration_ms,
};
pub use comet::{CometFrame, CometKind, CometParams, Direction, comet_frame, slice_polyline, static_frame};
pub use config::{Mode, MotionConfig};
pub use dim::{Dim, DimKind, LodFade, tier_alpha};
pub use dwell::{Advance, Dwell, JOURNEY_DWELL_MS, story_dwell_ms};
pub use interact::{LENS_DELAY_MS, LensState, MAX_TICK_MS, Pinned, Route, RouteChoice, route_lit};
pub use lens::{LensFilter, LensOption, LensSelection};
pub use overlay::{DetailAlpha, GroupAlpha, Highlight, Overlay, OverlayStroke, StrokeKind};
pub use owner::{Claim, Owner, Owners, Token};
pub use sample::sample;
pub use state::{CometRun, INTENT_HOVER_DELAY_MS, Intent, MotionState};
pub use step::{Steps, architecture_steps, scene_steps, workflow_steps};
pub use timeline::{Easing, Ms, Timeline, cubic_bezier, keyframes, keyframes_eased};

/// The constants, by module, for callers that want the numbers (`ambient`, `comet`, `dim`,
/// `step`, `camera`).
pub mod consts {
    pub use super::ambient::{
        DASH_OFF, DASH_ON, DASH_OFFSET_START, EDGE_ALPHA_START, EDGE_MS, EDGE_SETTLE_AT, NODE_GLOW_PX, NODE_MS,
        NODE_STROKE_PEAK, NODE_STROKE_REST,
    };
    pub use super::camera::{
        BOTTOM_MIN, FIT_FACTOR, MAX_SCALE, MAX_SCALE_NEIGHBOURS, MAX_ZOOM, MIN_ZOOM, PADDING, SNAP_BELOW,
        TWEEN_DEFAULT_MS, TWEEN_MAX_MS, TWEEN_MIN_MS,
    };
    pub use super::comet::{LENS_MAX_EDGES, PROBE_PARK_ALPHA, STATIC_ALPHA};
    pub use super::dim::{
        DIM_FADE_MS, DIM_FOCUS, DIM_INTENT, DIM_LENS, DIM_LENS_PEER, DIM_RELATIONSHIP, DIM_ROUTE, DIM_ROUTE_FUTURE,
        DIM_ROUTE_PAST, LOD_FADE_MS,
    };
    pub use super::step::{MAX_STEP, STEP_DELAY_MS};
}
