//! The brand mark's spin: what starts one, and the wait that ends it.
//!
//! The mark is a watermark, and it rests still. Two things move it, and neither is a control: a
//! triple click on the cube in its middle, and the window's own idle — every few minutes, the mark
//! turns over once on its own. Nothing depends on either, and nothing reports either; what is here
//! is the state a spin needs, and the curve it follows is `ui/mark.rs`'s.

use std::hash::BuildHasher as _;
use std::time::{Duration, Instant};

use gpui::{Context, WeakEntity};

use super::AppState;
use crate::ui::mark::SPIN;

/// The shortest and longest the idle watch waits between spins.
const IDLE_MIN: Duration = Duration::from_secs(7 * 60);
const IDLE_MAX: Duration = Duration::from_secs(15 * 60);

impl AppState {
    /// Turn the mark over once, from wherever the current spin has got to.
    ///
    /// Asking again mid-spin restarts it rather than being dropped: the number the animated
    /// element is keyed on goes up, so GPUI mounts a new element and plays the curve from the top.
    pub fn spin_mark(&mut self, cx: &mut Context<Self>) {
        let mark = &mut self.workbench.mark;
        mark.spin += 1;
        mark.until = Some(Instant::now() + SPIN);
        let already = std::mem::replace(&mut mark.spinning, true);
        cx.notify();
        // One wait per run of spins, not one per spin. The wait re-reads the deadline, so a spin
        // asked for while it is sleeping extends the run instead of starting a second waiter.
        if already {
            return;
        }
        cx.spawn(async move |this: WeakEntity<Self>, cx| {
            loop {
                cx.background_executor().timer(SPIN).await;
                let settled = this.update(cx, |this, cx| {
                    let mark = &mut this.workbench.mark;
                    if mark.until.is_some_and(|until| Instant::now() < until) {
                        return false;
                    }
                    // Back to the still ring. The animated one has reached the end of its curve
                    // and is sitting on the same two full turns, so nothing moves as it goes.
                    mark.spinning = false;
                    mark.until = None;
                    cx.notify();
                    true
                });
                if !matches!(settled, Ok(false)) {
                    break;
                }
            }
        })
        .detach();
    }

    /// Spin the mark every seven to fifteen minutes, for as long as the window is open.
    ///
    /// **It does not ask whether the mark is on screen.** A spin on a window showing something
    /// else costs one repaint and mounts nothing — only a page drawing the mark has a ring to
    /// animate — and the alternative is a second copy of every screen's idea of what "empty"
    /// means, kept in step with the five that draw the mark. So the watch is blind and cheap, and
    /// whoever is looking at an empty page when it fires is the one who sees it.
    pub(super) fn watch_mark_idle(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this: WeakEntity<Self>, cx| {
            loop {
                cx.background_executor().timer(idle_wait()).await;
                if this.update(cx, |this, cx| this.spin_mark(cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
    }
}

/// How long to wait before the next idle spin: somewhere between seven and fifteen minutes.
///
/// No random-number generator is pulled in for this. The standard library seeds every
/// `RandomState` differently, which is more than enough spread for a wait nothing depends on.
fn idle_wait() -> Duration {
    let roll = std::collections::hash_map::RandomState::new().hash_one(0u8);
    IDLE_MIN + (IDLE_MAX - IDLE_MIN).mul_f64(roll as f64 / u64::MAX as f64)
}
