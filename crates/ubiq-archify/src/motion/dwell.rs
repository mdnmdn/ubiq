//! `Dwell`: the tick-driven step timer of the route journey and guided views (`04` §3.2, §6.4).
//!
//! Archify used `setTimeout` and generation counters and had repeated bugs. Here the timer is
//! data: [`Dwell::advance`] adds the frame delta only while playing, so pause keeps `elapsed`,
//! resume continues with the remaining dwell, the last step completes without looping, and a
//! stale callback is dropped by comparing [`Dwell::generation`].

use serde::{Deserialize, Serialize};

/// Route journey: 1100 ms per step.
pub const JOURNEY_DWELL_MS: f64 = 1100.0;

/// Guided views: `max(1100, 3200 / steps)`.
pub fn story_dwell_ms(steps: usize) -> f64 {
    (3200.0 / steps.max(1) as f64).max(1100.0)
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Dwell {
    pub per_step_ms: f64,
    /// Number of steps (nodes of the route); the index runs `0..steps`.
    pub steps: usize,
    pub index: usize,
    /// Time spent on the current step, carried across pause and resume.
    pub elapsed_ms: f64,
    pub playing: bool,
    pub complete: bool,
    /// Bumped on restart and seek; a timer callback holding an older value is stale.
    pub generation: u32,
}

/// What one [`Dwell::advance`] did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Advance {
    /// How many steps were passed (almost always 0 or 1; more after a long frame gap).
    pub stepped: usize,
    /// The final step was reached and the dwell paused itself.
    pub completed: bool,
}

impl Dwell {
    pub fn new(per_step_ms: f64, steps: usize) -> Self {
        Self {
            per_step_ms,
            steps,
            index: 0,
            elapsed_ms: 0.0,
            playing: false,
            complete: steps < 2,
            generation: 0,
        }
    }

    pub fn journey(steps: usize) -> Self {
        Self::new(JOURNEY_DWELL_MS, steps)
    }

    pub fn story(steps: usize) -> Self {
        Self::new(story_dwell_ms(steps), steps)
    }

    /// Play from here; after completion, replay from step 0 (a new generation).
    pub fn play(&mut self) {
        if self.steps < 2 {
            return;
        }
        if self.complete {
            self.restart();
        }
        self.playing = true;
    }

    /// Pause, keeping `elapsed_ms` (`preserveElapsed`).
    pub fn pause(&mut self) {
        self.playing = false;
    }

    /// Back to step 0 with nothing elapsed.
    pub fn restart(&mut self) {
        self.index = 0;
        self.elapsed_ms = 0.0;
        self.complete = self.steps < 2;
        self.generation += 1;
    }

    /// Jump to a step (clamped), dropping the elapsed time.
    pub fn seek(&mut self, index: usize) {
        self.index = index.min(self.steps.saturating_sub(1));
        self.elapsed_ms = 0.0;
        self.complete = self.index + 1 >= self.steps;
        if self.complete {
            self.playing = false;
        }
        self.generation += 1;
    }

    /// Advance by a frame delta. Does nothing unless playing.
    pub fn advance(&mut self, dt_ms: f64) -> Advance {
        let mut out = Advance::default();
        if !self.playing || self.complete || dt_ms <= 0.0 {
            return out;
        }
        self.elapsed_ms += dt_ms;
        while self.elapsed_ms >= self.per_step_ms {
            self.elapsed_ms -= self.per_step_ms;
            self.index += 1;
            out.stepped += 1;
            if self.index + 1 >= self.steps {
                self.complete = true;
                self.playing = false;
                self.elapsed_ms = 0.0;
                out.completed = true;
                break;
            }
        }
        out
    }

    /// The dwell left on the current step: what resume waits for.
    pub fn remaining_ms(&self) -> f64 {
        (self.per_step_ms - self.elapsed_ms).max(0.0)
    }

    /// A callback armed at `generation` may still act.
    pub fn is_current(&self, generation: u32) -> bool {
        self.generation == generation
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn story_dwell_is_the_larger_of_1100_and_3200_over_n() {
        assert_eq!(story_dwell_ms(2), 1600.0);
        assert_eq!(story_dwell_ms(5), 1100.0);
        assert_eq!(story_dwell_ms(0), 3200.0);
    }

    #[test]
    fn it_steps_every_dwell_and_completes_on_the_last_without_looping() {
        let mut d = Dwell::journey(3);
        d.play();
        assert_eq!(d.advance(1099.0), Advance::default());
        assert_eq!(d.advance(1.0), Advance { stepped: 1, completed: false });
        assert_eq!(d.index, 1);
        assert_eq!(d.advance(1100.0), Advance { stepped: 1, completed: true });
        assert!(d.complete && !d.playing && d.index == 2);
        assert_eq!(d.advance(5000.0), Advance::default(), "no loop");
        assert_eq!(d.index, 2);
    }

    #[test]
    fn pause_keeps_elapsed_and_resume_uses_the_remainder() {
        let mut d = Dwell::journey(4);
        d.play();
        d.advance(400.0);
        d.pause();
        assert_eq!(d.advance(10_000.0), Advance::default(), "paused time does not count");
        assert_eq!((d.index, d.elapsed_ms, d.remaining_ms()), (0, 400.0, 700.0));
        d.play();
        assert_eq!(d.advance(700.0).stepped, 1);
    }

    #[test]
    fn a_long_frame_gap_steps_several_times_but_stops_at_the_end() {
        let mut d = Dwell::journey(3);
        d.play();
        assert_eq!(d.advance(10_000.0), Advance { stepped: 2, completed: true });
    }

    #[test]
    fn replay_restarts_with_a_new_generation_and_stale_callbacks_are_dropped() {
        let mut d = Dwell::journey(2);
        d.play();
        let armed = d.generation;
        d.advance(1100.0);
        assert!(d.complete);
        d.play();
        assert_eq!((d.index, d.playing), (0, true));
        assert!(!d.is_current(armed));
        d.seek(0);
        assert!(!d.is_current(armed + 1));
    }

    #[test]
    fn a_one_node_route_has_nothing_to_play() {
        let mut d = Dwell::journey(1);
        d.play();
        assert!(!d.playing && d.complete);
    }
}
