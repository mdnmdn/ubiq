//! Comets: one primitive, five parameter sets (`00` §6.2, `04` §1.2). A comet is a short dash,
//! a fraction of the edge's length, that runs source -> target once; direction only changes the
//! colour. The dash is cut out of the flattened polyline by arclength (no dash support needed).
//!
//! CSS runs the dash with `stroke-dasharray: d (1-d)` and `stroke-dashoffset: 0 -> -1` on a
//! `pathLength=1` clone, so the visible dash is `[p, p + d]` of the path, clipped to `[0, 1]`: it
//! enters at the source, leaves through the target, and the opacity envelope closes it.

use serde::{Deserialize, Serialize};

use super::timeline::{Easing, Timeline, keyframes_eased};
use crate::geom::{Pt, polyline_length};
use crate::tokens::{Kind, Token};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CometKind {
    /// M4: hover/focus preview of a node's incident edges.
    Intent,
    /// M5 (deferred).
    Relationship,
    /// M7 (deferred): parks at alpha .58.
    RouteProbe,
    /// M8 (deferred): the per-step pulse.
    RouteJourney,
    /// M6 (deferred): at most [`LENS_MAX_EDGES`] comets.
    Lens,
}

/// The lens animates at most this many edges (`semantic-lens.js:28`).
pub const LENS_MAX_EDGES: usize = 24;

/// Relative to the focused/selected node: `Out` leaves it, `In` arrives, `Loop` is a self-edge;
/// the lens also has `Within`. Forward/reverse are `Out`/`In`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    Out,
    In,
    Loop,
    Within,
}

/// One row of the parameter table.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CometParams {
    /// Screen px.
    pub width: f64,
    /// Dash as a fraction of the path.
    pub dash: f64,
    pub duration_ms: f64,
    pub easing: Easing,
    /// Where the dash window moves: `(offset, progress)`; `progress` 0 -> 1 is the dash offset.
    pub motion: &'static [(f64, f64)],
    /// `(offset, alpha)` opacity envelope.
    pub envelope: &'static [(f64, f64)],
    /// Glow radius in screen px.
    pub glow: f64,
    /// Static fallback (Still, reduced motion, hidden): full-length line `(width, alpha)`;
    /// `None` hides the overlay.
    pub static_line: Option<(f64, f64)>,
}

const LINEAR_RUN: &[(f64, f64)] = &[(0.0, 0.0), (1.0, 1.0)];

impl CometKind {
    pub fn params(self) -> CometParams {
        match self {
            CometKind::Intent => CometParams {
                width: 3.1,
                dash: 0.10,
                duration_ms: 1150.0,
                easing: Easing::Linear,
                motion: LINEAR_RUN,
                envelope: &[(0.0, 0.28), (0.12, 0.94), (0.78, 0.94), (1.0, 0.0)],
                glow: 4.0,
                static_line: Some((3.1, STATIC_ALPHA)),
            },
            CometKind::Relationship => CometParams {
                width: 3.35,
                dash: 0.085,
                duration_ms: 1200.0,
                easing: Easing::Linear,
                motion: LINEAR_RUN,
                envelope: &[(0.0, 0.18), (0.10, 0.98), (0.76, 0.98), (1.0, 0.0)],
                glow: 0.0,
                static_line: None,
            },
            CometKind::RouteProbe => CometParams {
                width: 3.25,
                dash: 0.085,
                duration_ms: 1100.0,
                easing: Easing::EMPHASIZED,
                motion: &[(0.0, 0.0), (0.78, 1.0), (1.0, 1.0)],
                envelope: &[(0.0, 0.96), (0.78, 0.96), (1.0, PROBE_PARK_ALPHA)],
                glow: 5.0,
                static_line: Some((3.25, STATIC_ALPHA)),
            },
            CometKind::RouteJourney => CometParams {
                width: 3.5,
                dash: 0.10,
                duration_ms: 780.0,
                easing: Easing::EMPHASIZED,
                motion: LINEAR_RUN,
                envelope: &[(0.0, 0.16), (0.14, 1.0), (0.76, 1.0), (1.0, 0.0)],
                glow: 0.0,
                static_line: None,
            },
            CometKind::Lens => CometParams {
                width: 3.05,
                dash: 0.075,
                duration_ms: 1350.0,
                easing: Easing::Linear,
                motion: LINEAR_RUN,
                envelope: &[(0.0, 0.2), (0.12, 0.9), (0.78, 0.9), (1.0, 0.0)],
                glow: 0.0,
                static_line: Some((2.2, 0.34)),
            },
        }
    }

    /// Whether the run ends parked as a full-length line instead of vanishing.
    pub fn parks(self) -> bool {
        self == CometKind::RouteProbe
    }

    /// The colour token for a direction.
    pub fn token(self, dir: Direction) -> Token {
        match self {
            CometKind::Relationship => Token::ArrowEmphasis,
            CometKind::RouteProbe | CometKind::RouteJourney => Token::KindStroke(Kind::Backend),
            CometKind::Intent => Token::KindStroke(match dir {
                Direction::Out | Direction::Within => Kind::Frontend,
                Direction::In => Kind::Database,
                Direction::Loop => Kind::Security,
            }),
            CometKind::Lens => Token::KindStroke(match dir {
                Direction::Out | Direction::Loop => Kind::Frontend,
                Direction::In => Kind::Database,
                Direction::Within => Kind::Messagebus,
            }),
        }
    }
}

/// The static fallback alpha of intent and probe comets (`viewer.css:706-714`).
pub const STATIC_ALPHA: f64 = 0.72;

/// Where the route probe parks (`archify-route-probe-flow` 100%).
pub const PROBE_PARK_ALPHA: f64 = 0.58;

/// A comet frame: the visible sub-polyline and its alpha.
#[derive(Clone, Debug, PartialEq)]
pub struct CometFrame {
    pub points: Vec<Pt>,
    pub alpha: f64,
    pub parked: bool,
}

/// The visible slice of one comet run at `t`, or `None` when nothing is drawn (before nothing,
/// after the end unless it parks, or the dash is past the target).
///
/// Before the delay the comet sits at its first keyframe (CSS fill `both`).
pub fn comet_frame(kind: CometKind, flat: &[Pt], tl: &Timeline, t: f64) -> Option<CometFrame> {
    let params = kind.params();
    let total = polyline_length(flat);
    if flat.len() < 2 || total.is_nan() || total <= 0.0 {
        return None;
    }
    if tl.is_done(t) {
        return kind.parks().then(|| CometFrame {
            points: flat.to_vec(),
            alpha: PROBE_PARK_ALPHA,
            parked: true,
        });
    }
    let p = tl.progress(t);
    let moved = keyframes_eased(params.motion, p, params.easing);
    let alpha = keyframes_eased(params.envelope, p, params.easing);
    let points = slice_polyline(flat, moved * total, (moved + params.dash) * total);
    (points.len() >= 2 && alpha > 0.0).then_some(CometFrame { points, alpha, parked: false })
}

/// The static fallback: the whole edge as a line, or `None` for overlays that hide.
pub fn static_frame(kind: CometKind, flat: &[Pt]) -> Option<(Vec<Pt>, f64, f64)> {
    let (width, alpha) = kind.params().static_line?;
    (flat.len() >= 2).then(|| (flat.to_vec(), width, alpha))
}

/// The sub-polyline between arclengths `from` and `to` (clamped to the path), interpolating the
/// two ends. Fewer than two points when the interval is empty.
pub fn slice_polyline(points: &[Pt], from: f64, to: f64) -> Vec<Pt> {
    let total = polyline_length(points);
    let (from, to) = (from.max(0.0), to.min(total));
    if points.len() < 2 || to - from <= 1e-9 {
        return Vec::new();
    }
    let mut out: Vec<Pt> = Vec::new();
    let mut walked = 0.0;
    for w in points.windows(2) {
        let (a, b) = (w[0], w[1]);
        let len = (b[0] - a[0]).hypot(b[1] - a[1]);
        let (s0, s1) = (walked, walked + len);
        walked = s1;
        if len <= 0.0 || s1 <= from || s0 >= to {
            continue;
        }
        let at = |s: f64| {
            let k = ((s - s0) / len).clamp(0.0, 1.0);
            [a[0] + (b[0] - a[0]) * k, a[1] + (b[1] - a[1]) * k]
        };
        if out.is_empty() {
            out.push(at(from.max(s0)));
        }
        out.push(at(to.min(s1)));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line() -> Vec<Pt> {
        vec![[0.0, 0.0], [100.0, 0.0]]
    }

    #[test]
    fn the_five_parameter_sets_match_the_catalogue() {
        let rows = [
            (CometKind::Intent, 3.1, 0.10, 1150.0),
            (CometKind::Relationship, 3.35, 0.085, 1200.0),
            (CometKind::RouteProbe, 3.25, 0.085, 1100.0),
            (CometKind::RouteJourney, 3.5, 0.10, 780.0),
            (CometKind::Lens, 3.05, 0.075, 1350.0),
        ];
        for (k, w, d, ms) in rows {
            let p = k.params();
            assert_eq!((p.width, p.dash, p.duration_ms), (w, d, ms), "{k:?}");
        }
    }

    #[test]
    fn direction_changes_only_the_colour() {
        let k = |d| CometKind::Intent.token(d);
        assert_eq!(k(Direction::Out), Token::KindStroke(Kind::Frontend));
        assert_eq!(k(Direction::In), Token::KindStroke(Kind::Database));
        assert_eq!(k(Direction::Loop), Token::KindStroke(Kind::Security));
        assert_eq!(CometKind::Lens.token(Direction::Within), Token::KindStroke(Kind::Messagebus));
        assert_eq!(CometKind::RouteProbe.token(Direction::In), Token::KindStroke(Kind::Backend));
    }

    #[test]
    fn slice_cuts_by_arclength_across_corners() {
        let l = vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]];
        assert_eq!(slice_polyline(&l, 5.0, 15.0), vec![[5.0, 0.0], [10.0, 0.0], [10.0, 5.0]]);
        assert_eq!(slice_polyline(&l, 12.0, 99.0), vec![[10.0, 2.0], [10.0, 10.0]]);
        assert!(slice_polyline(&l, 20.0, 30.0).is_empty());
        assert!(slice_polyline(&l, 5.0, 5.0).is_empty());
    }

    #[test]
    fn intent_comet_enters_at_the_source_and_leaves_through_the_target() {
        let tl = Timeline::new(0.0, 1150.0, Easing::Linear);
        let f0 = comet_frame(CometKind::Intent, &line(), &tl, 0.0).unwrap();
        assert_eq!(f0.points, vec![[0.0, 0.0], [10.0, 0.0]]);
        assert!((f0.alpha - 0.28).abs() < 1e-12);
        let mid = comet_frame(CometKind::Intent, &line(), &tl, 575.0).unwrap();
        assert_eq!(mid.points, vec![[50.0, 0.0], [60.0, 0.0]]);
        assert!((mid.alpha - 0.94).abs() < 1e-12);
        assert!(comet_frame(CometKind::Intent, &line(), &tl, 1150.0).is_none(), "one shot, then gone");
    }

    #[test]
    fn the_head_never_runs_backwards() {
        let tl = Timeline::new(0.0, 1150.0, Easing::Linear);
        let mut prev = -1.0;
        for i in 0..=100 {
            if let Some(f) = comet_frame(CometKind::Intent, &line(), &tl, 11.5 * i as f64) {
                let head = f.points.last().unwrap()[0];
                assert!(head >= prev);
                prev = head;
            }
        }
    }

    #[test]
    fn route_probe_parks_at_alpha_058() {
        let tl = Timeline::new(0.0, 1100.0, Easing::EMPHASIZED).delayed(160.0);
        let before = comet_frame(CometKind::RouteProbe, &line(), &tl, 0.0).unwrap();
        assert!((before.alpha - 0.96).abs() < 1e-12, "fill both: waits at the first keyframe");
        let parked = comet_frame(CometKind::RouteProbe, &line(), &tl, 5000.0).unwrap();
        assert!(parked.parked && parked.points.len() == 2 && parked.alpha == PROBE_PARK_ALPHA);
        assert!(comet_frame(CometKind::Intent, &line(), &Timeline::new(0.0, 1.0, Easing::Linear), 9.0).is_none());
    }

    #[test]
    fn static_fallbacks_follow_the_still_rules() {
        let (pts, w, a) = static_frame(CometKind::Intent, &line()).unwrap();
        assert_eq!((pts.len(), w, a), (2, 3.1, 0.72));
        let (_, w, a) = static_frame(CometKind::Lens, &line()).unwrap();
        assert_eq!((w, a), (2.2, 0.34));
        assert!(static_frame(CometKind::RouteJourney, &line()).is_none());
        assert!(static_frame(CometKind::Relationship, &line()).is_none());
    }

    #[test]
    fn degenerate_paths_draw_nothing() {
        let tl = Timeline::new(0.0, 1150.0, Easing::Linear);
        assert!(comet_frame(CometKind::Intent, &[[1.0, 1.0]], &tl, 10.0).is_none());
        assert!(comet_frame(CometKind::Intent, &[[1.0, 1.0], [1.0, 1.0]], &tl, 10.0).is_none());
    }
}
