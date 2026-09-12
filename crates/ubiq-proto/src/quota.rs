//! How much of an account's plan is left, as it crosses the bus.
//!
//! The mirror of `agent_manager::quota` — the library owns the provider facts (which endpoint,
//! which header, how a window is spelled) and the host maps its answer onto these types, exactly
//! as [`crate::conversation::RateLimitRecord`] mirrors the library's `RateLimitWindow`. The UI
//! crate does not depend on `agent-manager` and never will, so the wire carries its own vocabulary.
//!
//! Two properties decide the shape, and both are the opposite of [`crate::stats`]'s:
//!
//! - **It is an account fact, not a conversation one.** A window belongs to an identity. Two
//!   agents signed in as the same account read the same window, and an account with nothing
//!   running still has one — which is exactly when the question gets asked.
//! - **It is a list of gauges, not a struct of every provider's fields.** The providers do not
//!   agree on what a limit is, so [`QuotaReading`] is the extension point and a union struct that
//!   reads `None` on most of its fields is not.
//!
//! Nothing here is credential material. A snapshot carries percentages, a plan name and a
//! timestamp; the token that bought them never leaves the library.

use serde::{Deserialize, Serialize};

/// Whether a harness can be asked how much of the plan is left, and what asking costs.
///
/// Rides [`crate::messages::AgentTypeInfo`] so a surface can offer the question, disable it, or
/// say the provider states nothing — before anything is spawned.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuotaSource {
    /// The provider publishes no limit anybody can read. Not an Ubiq gap: Copilot, opencode and
    /// Gemini state a plan ceiling as prose and nothing queryable behind it.
    #[default]
    None,
    /// It arrives unasked while a turn runs, and there is no way to ask.
    Push,
    /// It can be asked with no process at all.
    Probe,
    /// It can be asked, but only down a bridge that is already running.
    Bridge,
}

impl QuotaSource {
    /// Whether this harness ever states a limit, by any route. What decides whether a surface
    /// draws the readout at all — a harness that says nothing says so once, in place, rather than
    /// leaving an empty panel to be interpreted.
    pub fn reports(self) -> bool {
        !matches!(self, QuotaSource::None)
    }

    /// Whether an answer can be had with nothing running. False for [`QuotaSource::Push`], which
    /// is why an account whose only source is a live turn reads as "not said" while idle.
    pub fn probeable(self) -> bool {
        matches!(self, QuotaSource::Probe)
    }
}

/// What one account has left, as the provider states it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct QuotaSnapshot {
    /// The account this is about.
    pub account: String,
    /// Which harness answered. One account can serve several, and each states its own limits.
    pub harness: String,
    /// The provider's own word for the plan — `"pro"`, `"max"`. `None` where it does not say,
    /// and never guessed.
    #[serde(default)]
    pub plan: Option<String>,
    /// One entry per window the provider states. **Empty is an answer**: the provider was asked
    /// and named no limit, which is not the same as not having been asked.
    pub gauges: Vec<QuotaGauge>,
    /// When it was read, unix seconds — so a surface says how old the reading is rather than
    /// implying it is current.
    pub as_of: i64,
}

impl QuotaSnapshot {
    /// The fullest gauge that states a percentage, or `None` when nothing stated one.
    ///
    /// What a single ring reports: the window closest to exhausted is the one that stops the next
    /// turn, so a strip with room for one glyph draws that one. `None` is drawn as no ring at
    /// all — the same rule the context ring follows, because a percentage of a window nobody
    /// named is a wrong ring, and a wrong ring is worse than none.
    pub fn worst_pct(&self) -> Option<u8> {
        self.gauges
            .iter()
            .filter_map(|gauge| gauge.reading.used_pct())
            .max()
    }

    /// The gauge [`Self::worst_pct`] reports, for the sentence that names it.
    pub fn worst(&self) -> Option<&QuotaGauge> {
        self.gauges
            .iter()
            .filter(|gauge| gauge.reading.used_pct().is_some())
            .max_by_key(|gauge| gauge.reading.used_pct())
    }
}

/// One window, count or balance the provider states.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct QuotaGauge {
    /// The provider's own name for it — `"5 hours"`, `"Week"`. Carried through rather than
    /// translated, so a reader comparing Ubiq with the provider's own dashboard finds the same
    /// word on both.
    pub label: String,
    /// How full it is.
    pub reading: QuotaReading,
    /// When it resets, unix seconds. `None` where the provider states no reset.
    #[serde(default)]
    pub resets_at: Option<i64>,
    /// One line under the gauge, verbatim from the provider where it gives one.
    #[serde(default)]
    pub detail: Option<String>,
}

/// The three shapes a limit actually comes in.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuotaReading {
    /// A fraction the provider itself computed, 0 to 100.
    Window {
        /// How full, as the provider rounded it.
        used_pct: u8,
    },
    /// A count against a ceiling. `limit` is `None` where the provider counts and names no
    /// ceiling — drawn as the count with no bar, because a meter needs a denominator somebody
    /// stated.
    Count {
        /// How many have been used.
        used: u64,
        /// The ceiling, where the provider states one.
        #[serde(default)]
        limit: Option<u64>,
    },
    /// Money left: minor units plus ISO 4217.
    Credit {
        /// What is left, in minor units.
        remaining: i64,
        /// ISO 4217.
        currency: String,
    },
}

impl QuotaReading {
    /// How full this reads, 0 to 100 — or `None` where nothing stated a denominator.
    ///
    /// A `Credit` balance has no ceiling to divide by, and a `Count` without a `limit` has none
    /// either. Both answer `None` rather than a figure a bar would be drawn from.
    pub fn used_pct(&self) -> Option<u8> {
        match self {
            QuotaReading::Window { used_pct } => Some((*used_pct).min(100)),
            QuotaReading::Count {
                used,
                limit: Some(limit),
            } if *limit > 0 => {
                Some((((*used as f64 / *limit as f64) * 100.0).round() as u64).min(100) as u8)
            }
            _ => None,
        }
    }

    /// The reading as the user reads it: `"7%"`, `"150 / 300"`, `"42 left"`, `"$5.00 left"`.
    pub fn say(&self) -> String {
        match self {
            QuotaReading::Window { used_pct } => format!("{used_pct}%"),
            QuotaReading::Count {
                used,
                limit: Some(limit),
            } => format!("{used} / {limit}"),
            QuotaReading::Count { used, limit: None } => format!("{used} used"),
            QuotaReading::Credit {
                remaining,
                currency,
            } => format!("{:.2} {currency} left", *remaining as f64 / 100.0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(label: &str, used_pct: u8) -> QuotaGauge {
        QuotaGauge {
            label: label.to_string(),
            reading: QuotaReading::Window { used_pct },
            resets_at: None,
            detail: None,
        }
    }

    fn snapshot(gauges: Vec<QuotaGauge>) -> QuotaSnapshot {
        QuotaSnapshot {
            account: "work".to_string(),
            harness: "claude-code".to_string(),
            plan: None,
            gauges,
            as_of: 0,
        }
    }

    #[test]
    fn one_ring_reports_the_window_closest_to_exhausted() {
        let snapshot = snapshot(vec![window("5 hours", 7), window("Week", 88)]);
        assert_eq!(snapshot.worst_pct(), Some(88));
        assert_eq!(snapshot.worst().map(|g| g.label.as_str()), Some("Week"));
    }

    #[test]
    fn a_provider_that_named_no_limit_draws_no_ring() {
        assert_eq!(
            snapshot(Vec::new()).worst_pct(),
            None,
            "an em dash, never a zero"
        );
    }

    #[test]
    fn a_reading_with_no_denominator_states_no_percentage() {
        let balance = QuotaReading::Credit {
            remaining: 500,
            currency: "USD".to_string(),
        };
        assert_eq!(balance.used_pct(), None);
        assert_eq!(balance.say(), "5.00 USD left");

        let uncapped = QuotaReading::Count {
            used: 42,
            limit: None,
        };
        assert_eq!(uncapped.used_pct(), None);
        assert_eq!(uncapped.say(), "42 used");
        assert_eq!(
            snapshot(vec![QuotaGauge {
                label: "Premium requests".to_string(),
                reading: uncapped,
                resets_at: None,
                detail: None,
            }])
            .worst_pct(),
            None,
            "a count nobody capped moves no ring"
        );
    }

    #[test]
    fn a_count_against_a_stated_ceiling_is_a_percentage() {
        let counted = QuotaReading::Count {
            used: 150,
            limit: Some(300),
        };
        assert_eq!(counted.used_pct(), Some(50));
        assert_eq!(counted.say(), "150 / 300");
    }

    #[test]
    fn only_a_harness_with_a_source_offers_the_readout() {
        assert!(!QuotaSource::None.reports());
        assert!(QuotaSource::Push.reports());
        assert!(!QuotaSource::Push.probeable(), "a push needs a live turn");
        assert!(QuotaSource::Probe.probeable());
    }
}
