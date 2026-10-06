//! `Timeline`, `Easing` and keyframe interpolation (`04` §5, `00` §6.3). Every Archify animation
//! runs once, so a timeline has no repeat. Time is milliseconds on one monotonic clock, as `f64`.

use serde::{Deserialize, Serialize};

/// Milliseconds on the host's monotonic clock (the zero is the caller's choice).
pub type Ms = f64;

/// The easings Archify uses. The CSS ones are the named cubic-beziers.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Easing {
    Linear,
    /// CSS `ease`, the 160/180 ms fades.
    Ease,
    /// CSS `ease-in-out`, the node pulse.
    EaseInOut,
    /// `1 - (1 - t)^3`, the camera tween.
    OutCubic,
    /// `cubic-bezier(x1, y1, x2, y2)`.
    Bezier(f64, f64, f64, f64),
}

impl Easing {
    /// `cubic-bezier(.22, 1, .36, 1)`: route probe, journey pulse.
    pub const EMPHASIZED: Easing = Easing::Bezier(0.22, 1.0, 0.36, 1.0);
    /// `cubic-bezier(.2, 0, .2, 1)`: the flow token.
    pub const SPLINE: Easing = Easing::Bezier(0.2, 0.0, 0.2, 1.0);

    /// Map linear progress `x` in `[0, 1]` to eased progress (clamped).
    pub fn apply(self, x: f64) -> f64 {
        let x = x.clamp(0.0, 1.0);
        match self {
            Easing::Linear => x,
            Easing::Ease => cubic_bezier(0.25, 0.1, 0.25, 1.0, x),
            Easing::EaseInOut => cubic_bezier(0.42, 0.0, 0.58, 1.0, x),
            Easing::OutCubic => 1.0 - (1.0 - x).powi(3),
            Easing::Bezier(a, b, c, d) => cubic_bezier(a, b, c, d, x),
        }
    }
}

/// The CSS timing function: solve `bx(s) = x` by bisection (monotone for `x1, x2` in `[0, 1]`),
/// return `by(s)`. Deterministic, 40 halvings.
pub fn cubic_bezier(x1: f64, y1: f64, x2: f64, y2: f64, x: f64) -> f64 {
    // Exact ends, so a finished fade equals the static scene bit for bit.
    if x <= 0.0 {
        return 0.0;
    }
    if x >= 1.0 {
        return 1.0;
    }
    let at = |a: f64, b: f64, s: f64| {
        let u = 1.0 - s;
        3.0 * u * u * s * a + 3.0 * u * s * s * b + s * s * s
    };
    let (mut lo, mut hi) = (0.0f64, 1.0f64);
    for _ in 0..40 {
        let mid = (lo + hi) / 2.0;
        if at(x1, x2, mid) < x {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    at(y1, y2, (lo + hi) / 2.0)
}

/// A single run: `start`, an optional `delay`, a `duration` and an easing.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Timeline {
    pub start_ms: Ms,
    pub delay_ms: Ms,
    pub duration_ms: Ms,
    pub easing: Easing,
}

impl Timeline {
    pub fn new(start_ms: Ms, duration_ms: Ms, easing: Easing) -> Self {
        Self { start_ms, delay_ms: 0.0, duration_ms, easing }
    }

    pub fn delayed(mut self, delay_ms: Ms) -> Self {
        self.delay_ms = delay_ms;
        self
    }

    /// When the run begins moving.
    pub fn begin_ms(&self) -> Ms {
        self.start_ms + self.delay_ms
    }

    /// When the run is finished.
    pub fn end_ms(&self) -> Ms {
        self.begin_ms() + self.duration_ms
    }

    /// Linear progress in `[0, 1]`: 0 before the delay (CSS fill `backwards`), 1 after the end.
    pub fn progress(&self, t: Ms) -> f64 {
        if self.duration_ms <= 0.0 {
            return if t >= self.begin_ms() { 1.0 } else { 0.0 };
        }
        ((t - self.begin_ms()) / self.duration_ms).clamp(0.0, 1.0)
    }

    pub fn eased(&self, t: Ms) -> f64 {
        self.easing.apply(self.progress(t))
    }

    /// Before the delay has elapsed.
    pub fn is_pending(&self, t: Ms) -> bool {
        t < self.begin_ms()
    }

    /// `elapsed >= duration`: nothing left to draw, and a frame is not needed for it.
    pub fn is_done(&self, t: Ms) -> bool {
        t >= self.end_ms()
    }

    /// Needs frames: not yet finished (a pending delay counts, the next frame is the start).
    pub fn is_active(&self, t: Ms) -> bool {
        !self.is_done(t)
    }
}

/// Piecewise-linear keyframes `(offset, value)` with offsets in `[0, 1]`, ascending. Before the
/// first and after the last, the nearest value holds.
pub fn keyframes(frames: &[(f64, f64)], p: f64) -> f64 {
    keyframes_eased(frames, p, Easing::Linear)
}

/// As [`keyframes`], with `easing` applied inside each interval (CSS applies `animation-timing-
/// function` per keyframe interval, not over the whole run).
pub fn keyframes_eased(frames: &[(f64, f64)], p: f64, easing: Easing) -> f64 {
    let (Some(first), Some(last)) = (frames.first(), frames.last()) else {
        return 0.0;
    };
    if p <= first.0 {
        return first.1;
    }
    if p >= last.0 {
        return last.1;
    }
    for w in frames.windows(2) {
        let ((p0, v0), (p1, v1)) = (w[0], w[1]);
        if p >= p0 && p <= p1 {
            if p1 <= p0 {
                return v1;
            }
            let k = easing.apply((p - p0) / (p1 - p0));
            return v0 + (v1 - v0) * k;
        }
    }
    last.1
}

/// `a + (b - a) * k`.
pub fn lerp(a: f64, b: f64, k: f64) -> f64 {
    a + (b - a) * k
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn out_cubic_is_the_camera_curve() {
        assert_eq!(Easing::OutCubic.apply(0.0), 0.0);
        assert_eq!(Easing::OutCubic.apply(1.0), 1.0);
        assert!((Easing::OutCubic.apply(0.5) - 0.875).abs() < 1e-12);
    }

    #[test]
    fn beziers_hit_their_endpoints_and_are_monotone() {
        for e in [Easing::Ease, Easing::EaseInOut, Easing::EMPHASIZED, Easing::SPLINE] {
            assert!(e.apply(0.0).abs() < 1e-9 && (e.apply(1.0) - 1.0).abs() < 1e-9, "{e:?}");
            let mut prev = 0.0;
            for i in 1..=50 {
                let v = e.apply(i as f64 / 50.0);
                assert!(v >= prev - 1e-9, "{e:?} not monotone at {i}");
                prev = v;
            }
        }
        // ease-in-out is symmetric about the midpoint.
        assert!((Easing::EaseInOut.apply(0.5) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn timeline_is_single_run_and_clamped() {
        let tl = Timeline::new(100.0, 1000.0, Easing::Linear).delayed(160.0);
        assert!(tl.is_pending(259.0) && tl.progress(259.0) == 0.0);
        assert!((tl.progress(760.0) - 0.5).abs() < 1e-12);
        assert!(tl.is_active(1259.0) && tl.is_done(1260.0));
        assert_eq!(tl.progress(9999.0), 1.0);
    }

    #[test]
    fn keyframes_interpolate_and_hold() {
        let f = [(0.0, 0.28), (0.12, 0.94), (0.78, 0.94), (1.0, 0.0)];
        assert_eq!(keyframes(&f, 0.0), 0.28);
        assert!((keyframes(&f, 0.06) - 0.61).abs() < 1e-12);
        assert_eq!(keyframes(&f, 0.5), 0.94);
        assert_eq!(keyframes(&f, 1.0), 0.0);
        assert_eq!(keyframes(&f, 2.0), 0.0);
    }
}
