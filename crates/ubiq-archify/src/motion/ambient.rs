//! M1/M2: the ambient trace, only with `meta.animation: "trace"` (`04` §1.2, `00` §6.2-6.3).
//!
//! **Lifecycle** (`motion-governor.js:29-53`): it starts at most once, only if capable, not paused,
//! no owner and the window visible. Any of those failing settles it for good (`Suppressed`);
//! finishing settles it (`Complete`). A settled trace never restarts, and settled means the
//! picture equals the static scene.
//!
//! **Edge** (`archify-edge-flow`, 2.4 s linear): dashes `10 8` march from offset 54 to 0 over the
//! first 88 % while the edge fades 0.42 -> 1. The overlay carries the cut-out dashes at full alpha
//! and the edge group's multiplier carries the fade, so the gaps read dim (CSS leaves them empty;
//! a scene edge cannot be restyled, D-rule "never mutate authored shapes").
//! **Node** (`archify-node-pulse`, 3.6 s ease-in-out per keyframe interval): the stroke goes
//! 1.5 -> 2.4 with an 8 px glow at 18-36 % and rests from 72 %.
//! Both start at `min(12, step) * 160 ms`.

use serde::{Deserialize, Serialize};

use super::slice_polyline;
use super::step::Steps;
use super::timeline::{Easing, Ms, keyframes, keyframes_eased};
use crate::geom::{Pt, polyline_length};
use crate::scene::{GroupKind, Scene};

pub const EDGE_MS: f64 = 2400.0;
pub const NODE_MS: f64 = 3600.0;
/// `stroke-dasharray: 10 8`.
pub const DASH_ON: f64 = 10.0;
pub const DASH_OFF: f64 = 8.0;
/// `stroke-dashoffset` at 0 %.
pub const DASH_OFFSET_START: f64 = 54.0;
/// The offset reaches 0 and the opacity 1 here.
pub const EDGE_SETTLE_AT: f64 = 0.88;
pub const EDGE_ALPHA_START: f64 = 0.42;
/// Node pulse: rest and peak stroke widths and the glow radius.
pub const NODE_STROKE_REST: f64 = 1.5;
pub const NODE_STROKE_PEAK: f64 = 2.4;
pub const NODE_GLOW_PX: f64 = 8.0;

const NODE_PULSE: &[(f64, f64)] = &[(0.0, 0.0), (0.18, 1.0), (0.36, 1.0), (0.72, 0.0), (1.0, 0.0)];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Settle {
    Complete,
    Suppressed,
    /// Nothing to animate.
    Empty,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum Phase {
    #[default]
    Pending,
    Running { start_ms: Ms, end_ms: Ms },
    Settled(Settle),
}

/// The once-only lifecycle.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Ambient {
    pub phase: Phase,
}

impl Ambient {
    /// Start the single pass (`false` if it already started or settled).
    pub fn start(&mut self, start_ms: Ms, total_ms: Ms) -> bool {
        if self.phase != Phase::Pending {
            return false;
        }
        self.phase = if total_ms <= 0.0 {
            Phase::Settled(Settle::Empty)
        } else {
            Phase::Running { start_ms, end_ms: start_ms + total_ms }
        };
        true
    }

    /// Settle for good. No effect once settled.
    pub fn settle(&mut self, reason: Settle) -> bool {
        if matches!(self.phase, Phase::Settled(_)) {
            return false;
        }
        self.phase = Phase::Settled(reason);
        true
    }

    pub fn is_running(&self) -> bool {
        matches!(self.phase, Phase::Running { .. })
    }

    /// Needs frames at `t`.
    pub fn is_active(&self, t: Ms) -> bool {
        matches!(self.phase, Phase::Running { end_ms, .. } if t < end_ms)
    }

    /// `Complete` once the pass has run out (the `animationend` bookkeeping, `elapsed >= duration`).
    pub fn tick(&mut self, t: Ms) {
        if let Phase::Running { end_ms, .. } = self.phase
            && t >= end_ms
        {
            self.phase = Phase::Settled(Settle::Complete);
        }
    }

    /// Milliseconds since the pass started, while it is running.
    pub fn elapsed(&self, t: Ms) -> Option<Ms> {
        match self.phase {
            Phase::Running { start_ms, end_ms } if t < end_ms => Some((t - start_ms).max(0.0)),
            _ => None,
        }
    }
}

/// The whole pass: the last element's delay plus its duration (<= 12 x 160 + 3600 = 5.52 s). 0 for
/// a scene with no edge and no node.
pub fn total_ms(scene: &Scene, steps: &Steps) -> Ms {
    let (mut edge_ord, mut end) = (0usize, 0.0f64);
    for g in &scene.groups {
        match g.kind {
            GroupKind::Edge => {
                end = end.max(steps.edge_delay_ms(edge_ord) + EDGE_MS);
                edge_ord += 1;
            }
            GroupKind::Node => {
                if let Some(id) = &g.node_id {
                    end = end.max(steps.node_delay_ms(id) + NODE_MS);
                }
            }
            _ => {}
        }
    }
    end
}

/// Edge at linear progress `p`: `(dash offset, edge alpha)`.
pub fn edge_at(p: f64) -> (f64, f64) {
    let offset = keyframes(&[(0.0, DASH_OFFSET_START), (EDGE_SETTLE_AT, 0.0), (1.0, 0.0)], p);
    let alpha = keyframes(&[(0.0, EDGE_ALPHA_START), (EDGE_SETTLE_AT, 1.0), (1.0, 1.0)], p);
    (offset, alpha)
}

/// Node pulse strength in `[0, 1]` at linear progress `p`; the stroke and glow scale with it.
pub fn node_pulse(p: f64) -> f64 {
    keyframes_eased(NODE_PULSE, p, Easing::EaseInOut)
}

/// The `10 8` dashes of a path at `offset`, cut out by arclength: dash `k` covers
/// `[18k - offset, 18k - offset + 10]`.
pub fn dashes(flat: &[Pt], offset: f64) -> Vec<Vec<Pt>> {
    let total = polyline_length(flat);
    if flat.len() < 2 || total.is_nan() || total <= 0.0 {
        return Vec::new();
    }
    let period = DASH_ON + DASH_OFF;
    let mut out = Vec::new();
    let mut k = ((offset - DASH_ON) / period).floor() as i64;
    loop {
        let a = k as f64 * period - offset;
        if a >= total {
            break;
        }
        let piece = slice_polyline(flat, a, a + DASH_ON);
        if piece.len() >= 2 {
            out.push(piece);
        }
        k += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_starts_at_most_once_and_settles_for_good() {
        let mut a = Ambient::default();
        assert!(a.start(100.0, 5000.0));
        assert!(!a.start(900.0, 5000.0), "never a second pass");
        assert!(a.is_active(5099.0) && !a.is_active(5100.0));
        a.tick(5100.0);
        assert_eq!(a.phase, Phase::Settled(Settle::Complete));
        assert!(!a.start(6000.0, 5000.0), "settled never restarts");
        assert!(!a.settle(Settle::Suppressed), "the first settle reason sticks");
        assert_eq!(a.phase, Phase::Settled(Settle::Complete));
    }

    #[test]
    fn suppression_before_the_start_still_counts_as_the_one_pass() {
        let mut a = Ambient::default();
        assert!(a.settle(Settle::Suppressed));
        assert!(!a.start(0.0, 5000.0));
        assert!(!a.is_running());
        let mut e = Ambient::default();
        e.start(0.0, 0.0);
        assert_eq!(e.phase, Phase::Settled(Settle::Empty));
    }

    #[test]
    fn edge_keyframes_are_54_to_0_and_042_to_1_over_88_percent() {
        assert_eq!(edge_at(0.0), (54.0, 0.42));
        let (o, a) = edge_at(0.44);
        assert!((o - 27.0).abs() < 1e-9 && (a - 0.71).abs() < 1e-9);
        assert_eq!(edge_at(0.88), (0.0, 1.0));
        assert_eq!(edge_at(1.0), (0.0, 1.0));
    }

    #[test]
    fn node_pulse_peaks_between_18_and_36_percent_and_rests() {
        assert_eq!(node_pulse(0.0), 0.0);
        assert_eq!(node_pulse(0.18), 1.0);
        assert_eq!(node_pulse(0.30), 1.0);
        assert_eq!(node_pulse(0.36), 1.0);
        assert!(node_pulse(0.54) > 0.0 && node_pulse(0.54) < 1.0);
        assert!((node_pulse(0.54) - 0.5).abs() < 1e-6, "ease-in-out is symmetric");
        assert_eq!(node_pulse(0.72), 0.0);
        assert_eq!(node_pulse(1.0), 0.0);
    }

    #[test]
    fn dashes_follow_the_pattern_and_march_forward() {
        let line = [[0.0, 0.0], [60.0, 0.0]];
        // offset 0: dashes [0,10] [18,28] [36,46] [54,60 clipped]
        let d = dashes(&line, 0.0);
        let spans: Vec<(f64, f64)> = d.iter().map(|p| (p[0][0], p.last().unwrap()[0])).collect();
        assert_eq!(spans, vec![(0.0, 10.0), (18.0, 28.0), (36.0, 46.0), (54.0, 60.0)]);
        // offset 54 (start): pattern shifted back by 54 = 3 periods, same layout; offset 9 moves them 9 back.
        let d = dashes(&line, 9.0);
        let first = (d[0][0][0], d[0].last().unwrap()[0]);
        assert_eq!(first, (0.0, 1.0), "the tail of the dash that started 9 before the path");
        assert_eq!(dashes(&line, 54.0), dashes(&line, 0.0), "54 = 3 periods of 18");
        assert!(dashes(&[[0.0, 0.0]], 0.0).is_empty());
    }

    #[test]
    fn the_whole_pass_is_bounded_by_the_capture_budget() {
        let mut s = Steps::default();
        s.nodes.insert("a".into(), 400);
        let mut scene = Scene::default();
        scene.groups.push(crate::scene::Group::node(
            "a",
            crate::tokens::Kind::Backend,
            "A",
            crate::scene::Bounds::new(0.0, 0.0, 1.0, 1.0),
        ));
        assert_eq!(total_ms(&scene, &s), 12.0 * 160.0 + NODE_MS);
        assert!(total_ms(&scene, &s) <= 5520.0);
        assert_eq!(total_ms(&Scene::default(), &s), 0.0);
    }
}
