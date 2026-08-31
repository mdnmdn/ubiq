//! The log console's own state: which subsystem it is showing, the level it cuts off at, and
//! whether it follows the tail.
//!
//! The records are not here. They belong to the process-wide sink in [`crate::log`], and this is
//! only one window's view onto it — which is why nothing in this file is a fixture.

use crate::log::{Filter, LogLevel, Subsystem};

/// What the selector calls every subsystem at once.
const ALL: &str = "All";

pub struct LogState {
    /// `None` is every subsystem, which the selector shows as `All`.
    pub subsystem: Option<Subsystem>,
    /// The floor: a record quieter than this is not drawn.
    pub min_level: LogLevel,
    /// Whether an arriving record scrolls the list to the tail.
    pub follow: bool,
}

impl Default for LogState {
    fn default() -> Self {
        Self {
            subsystem: None,
            min_level: LogLevel::Debug,
            follow: true,
        }
    }
}

impl LogState {
    pub fn filter(&self) -> Filter {
        Filter {
            subsystem: self.subsystem,
            min_level: self.min_level,
        }
    }

    /// The subsystem selector's rows: `All`, then every subsystem in its own order.
    pub fn subsystem_items() -> Vec<&'static str> {
        let mut items = vec![ALL];
        items.extend(Subsystem::ALL.iter().map(|subsystem| subsystem.label()));
        items
    }

    /// Which row the selector marks, on the same indexing as [`Self::subsystem_items`].
    pub fn subsystem_index(&self) -> usize {
        self.subsystem
            .and_then(|chosen| Subsystem::ALL.iter().position(|s| *s == chosen))
            .map_or(0, |index| index + 1)
    }

    pub fn subsystem_label(&self) -> &'static str {
        self.subsystem.map_or(ALL, |subsystem| subsystem.label())
    }

    /// Take the selector's answer. Row zero is `All`; anything the list does not have is ignored.
    pub fn pick_subsystem(&mut self, index: usize) {
        self.subsystem = index
            .checked_sub(1)
            .and_then(|index| Subsystem::ALL.get(index).copied());
    }

    pub fn level_items() -> Vec<&'static str> {
        LogLevel::ALL.iter().map(|level| level.label()).collect()
    }

    pub fn level_index(&self) -> usize {
        LogLevel::ALL
            .iter()
            .position(|level| *level == self.min_level)
            .unwrap_or(0)
    }

    pub fn pick_level(&mut self, index: usize) {
        if let Some(level) = LogLevel::ALL.get(index) {
            self.min_level = *level;
        }
    }
}
