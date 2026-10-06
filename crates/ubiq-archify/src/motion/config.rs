//! `MotionConfig`: the Motion Governor's switches (`V/motion-governor.js`, `00` §6.3).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The reader's switch (`localStorage 'archify-motion'`; here a Ubiq setting).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    #[default]
    Live,
    Still,
}

/// What may animate right now. Pure data; the UI keeps one and feeds it OS and window events.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MotionConfig {
    /// `meta.animation == "trace"`. Without it every method of the governor is a no-op (and no
    /// owner can be claimed), so the whole motion UI is hidden.
    pub capable: bool,
    pub mode: Mode,
    /// The OS reduced-motion preference: forces `Still`, and also makes fades and the camera
    /// instant.
    pub system_reduced: bool,
    /// Refcounted reasons motion is suspended; `visibility` is set while the window is hidden.
    #[serde(default)]
    pub suspensions: BTreeMap<String, u32>,
}

impl Default for MotionConfig {
    fn default() -> Self {
        Self { capable: true, mode: Mode::Live, system_reduced: false, suspensions: BTreeMap::new() }
    }
}

impl MotionConfig {
    /// The reason key the UI uses for a hidden or occluded window.
    pub const VISIBILITY: &'static str = "visibility";

    pub fn new(capable: bool) -> Self {
        Self { capable, ..Self::default() }
    }

    pub fn has_suspension(&self) -> bool {
        !self.suspensions.is_empty()
    }

    /// `readerPaused || systemPaused || any suspension`.
    pub fn effective_paused(&self) -> bool {
        self.mode == Mode::Still || self.system_reduced || self.has_suspension()
    }

    /// The window is hidden (the `visibility` suspension).
    pub fn hidden(&self) -> bool {
        self.suspensions.contains_key(Self::VISIBILITY)
    }

    /// Comets render as static lines instead of moving, journey and relationship overlays hide.
    pub fn comets_static(&self) -> bool {
        self.effective_paused()
    }

    /// Fades and the camera are instant: `prefers-reduced-motion`, or a hidden window. A reader's
    /// `Still` parks the signals but leaves transitions alone (`viewer-camera.js` `frameDesktop`).
    pub fn instant(&self) -> bool {
        self.system_reduced || self.hidden()
    }

    /// Hover delay before the intent trace shows: 90 ms, 0 under reduced motion.
    pub fn hover_delay_ms(&self) -> f64 {
        if self.system_reduced { 0.0 } else { crate::motion::INTENT_HOVER_DELAY_MS }
    }

    /// Take a suspension (refcounted per reason).
    pub fn suspend(&mut self, reason: &str) {
        *self.suspensions.entry(reason.to_string()).or_insert(0) += 1;
    }

    /// Give one back. `false` if the reason was not held.
    pub fn unsuspend(&mut self, reason: &str) -> bool {
        match self.suspensions.get_mut(reason) {
            Some(n) if *n > 1 => {
                *n -= 1;
                true
            }
            Some(_) => {
                self.suspensions.remove(reason);
                true
            }
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paused_is_reader_or_system_or_suspension() {
        let mut c = MotionConfig::default();
        assert!(!c.effective_paused());
        c.mode = Mode::Still;
        assert!(c.effective_paused() && !c.instant(), "Still parks signals, transitions stay");
        c.mode = Mode::Live;
        c.system_reduced = true;
        assert!(c.effective_paused() && c.instant());
        c.system_reduced = false;
        c.suspend("visibility");
        assert!(c.effective_paused() && c.hidden() && c.instant());
        assert!(c.unsuspend("visibility") && !c.effective_paused());
        assert!(!c.unsuspend("visibility"));
    }

    #[test]
    fn suspensions_are_refcounted() {
        let mut c = MotionConfig::default();
        c.suspend("export");
        c.suspend("export");
        assert!(c.unsuspend("export") && c.effective_paused());
        assert!(c.unsuspend("export") && !c.effective_paused());
    }

    #[test]
    fn hover_delay_is_zero_under_reduced_motion() {
        let mut c = MotionConfig::default();
        assert_eq!(c.hover_delay_ms(), 90.0);
        c.system_reduced = true;
        assert_eq!(c.hover_delay_ms(), 0.0);
    }
}
