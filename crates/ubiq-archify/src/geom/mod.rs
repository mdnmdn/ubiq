//! Rects, segments, predicates, anchors, ports (P2.2). Port of `renderers/shared/geometry.mjs`,
//! the primitives only: the `clean*` gates of that file belong to `gates/`, `labelPoint` to
//! `labels.rs`, and `automaticPortRhythmBridge` to the architecture router (it calls the gates).
//!
//! - [`rect`]: [`Rect`], [`Seg`], `rects_overlap` (ε 1e-4, negative-gap convention), segment/rect
//!   predicates, crossings, collinear overlap, frame borders.
//! - [`route`]: `normalize`, `join_route_points`, `rounded_path` and its flattening, arclength sampling.
//! - [`ports`]: [`Side`], `anchor`, both side-inference rules, `automatic_port_spread`.
//!
//! No rounding in this module follows `Math.round`; a caller that needs it uses
//! `crate::diag::js_round`.

pub mod ports;
pub mod rect;
pub mod route;

pub use ports::*;
pub use rect::*;
pub use route::*;
