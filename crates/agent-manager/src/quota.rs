//! How much of a subscription is left — the account's side of the ledger.
//!
//! [`crate::io::model::UsageUpdate`] and its friends say what a turn *spent*. This module says
//! what remains, which is the other direction and a fact about the **account** rather than about
//! any one conversation: two agents signed in as the same identity read the same window, and an
//! account with nothing running still has one.
//!
//! A snapshot is a **list of gauges**, not a struct of every provider's fields. The providers do
//! not agree on what a limit is — Claude has two rolling windows, Copilot counts premium requests
//! against a monthly ceiling, a credit-based endpoint has money left — and a union struct would
//! grow a field per provider and read `None` on almost all of them. [`QuotaReading`] is the
//! extension point: a provider that genuinely reads differently adds a variant, and every gauge
//! already drawn keeps drawing.
//!
//! **No credential material is read here at all.** A probe asks the *harness* — Claude Code's own
//! `/usage`, run headless — and the harness asks the provider with the login it already holds. No
//! token is opened, copied, returned or logged; a snapshot is percentages, a plan name and a
//! timestamp. That is stronger than the invariant [`crate::account`] states for the account index
//! itself, and it is the reason the ask is shaped this way.

use std::path::Path;

use anyhow::Result;
use serde::{Deserialize, Serialize};

/// Whether a harness can be asked how much of the plan is left, and what asking costs.
///
/// Declared on [`crate::harness::IoSupport`] so a caller can decide whether to offer the question
/// at all *before* it spawns anything — the same fact-before-process split
/// [`crate::harness::IoSupport::multi_turn`] makes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuotaSource {
    /// The provider states no limit anybody can read. Not a gap in this crate: Copilot, opencode
    /// and Gemini publish a plan ceiling as prose and nothing queryable behind it.
    #[default]
    None,
    /// It arrives unasked on the structured stream while a turn runs, and there is no way to ask.
    Push,
    /// It can be asked with no conversation running — a short-lived process of its own, or one
    /// request, and an answer.
    Probe,
    /// It can be asked, but only down a structured bridge that is already running.
    Bridge,
}

impl QuotaSource {
    /// Whether [`crate::harness::Harness::quota`] can answer with no conversation running.
    pub fn probeable(self) -> bool {
        matches!(self, QuotaSource::Probe)
    }

    /// Whether this harness ever states a limit, by any route.
    pub fn reports(self) -> bool {
        !matches!(self, QuotaSource::None)
    }
}

/// What one account has left, as the provider states it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QuotaSnapshot {
    /// The account id this is about — the key, because a window belongs to an identity.
    pub account: String,
    /// Which harness answered. A field rather than part of the key: one account can serve
    /// several harnesses, and each states its own limits.
    pub harness: String,
    /// What the provider calls the plan — `"pro"`, `"Copilot Business"`. `None` where it does not
    /// say, which is never drawn as a guess.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<String>,
    /// One entry per window the provider states. Empty is a legitimate answer: the account is
    /// known and the provider named no limit.
    pub gauges: Vec<QuotaGauge>,
    /// When this was read, unix seconds — so a reader can say how stale it is rather than
    /// implying "now".
    pub as_of: i64,
}

/// One window, count or balance the provider states.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QuotaGauge {
    /// The provider's own name for it — `"5 hours"`, `"Week"`, `"Premium requests"`. Carried
    /// through rather than translated, because the provider's word is what its own dashboard
    /// says and a reader comparing the two should find the same label.
    pub label: String,
    /// How full it is.
    pub reading: QuotaReading,
    /// When it resets, unix seconds. `None` where the provider states no reset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resets_at: Option<i64>,
    /// One line under the gauge, verbatim from the provider where it gives one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// The three shapes a limit actually comes in.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuotaReading {
    /// A fraction of a window the provider itself computed, 0 to 100. The provider did the
    /// division, so nothing here invents a denominator.
    Window {
        /// How full, as the provider rounded it.
        used_pct: u8,
    },
    /// A count against a stated ceiling. `limit` is `None` where the provider counts but names no
    /// ceiling — a reader draws the count and no bar, because a meter without a denominator is
    /// exactly the thing Ubiq's stats rule refuses.
    Count {
        /// How many have been used.
        used: u64,
        /// The ceiling, where the provider states one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        limit: Option<u64>,
    },
    /// Money left: minor units plus ISO 4217, matching [`crate::io::model::Cost`]'s currency
    /// convention.
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
    /// A `Credit` balance has no ceiling to divide by (nobody says what a "full" wallet is), and
    /// a `Count` with no `limit` has none either. Both answer `None` rather than a figure a
    /// reader would draw a bar from.
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
}

impl QuotaSnapshot {
    /// The fullest gauge that states a percentage — what a single glyph with room for one number
    /// should report, because the window closest to exhausted is the one that stops the next turn.
    pub fn worst_pct(&self) -> Option<u8> {
        self.gauges
            .iter()
            .filter_map(|gauge| gauge.reading.used_pct())
            .max()
    }
}

/// Ask Claude what is left for `account`, by running Claude Code's own `/usage` against the
/// login it keeps in `home` — a profile's shared config home, or its own default when `None`
/// (`D194`).
///
/// **The harness is the source, not an endpoint.** Claude Code holds the login and asks the
/// provider itself; what comes back is the `usage_report` it attaches to the answer, which is
/// the same structured fact its own screen draws. Nothing here opens a credential file, reads a
/// Keychain item or names a URL, so a change to how Claude Code stores or renews a login is not
/// a change here.
///
/// A failure is a sentence the user reads and never a number: the run is best-effort by
/// construction, and a window that states no percentage produces no gauge.
pub fn claude(account: &str, harness: &str, home: Option<&Path>) -> Result<QuotaSnapshot> {
    let report = crate::harness::claude::usage_via_jsonl(home)?;
    Ok(QuotaSnapshot {
        account: account.to_string(),
        harness: harness.to_string(),
        // `/usage` names no plan — it says "your subscription" and nothing more. `None` is what
        // a surface draws as nothing, which is the honest answer rather than a guessed tier.
        plan: None,
        gauges: claude_gauges(&report),
        as_of: now(),
    })
}

/// Read whatever windows the report states.
///
/// `usage_report.rate_limits.limits` is a **list**, not a fixed set of fields, so a window Claude
/// adds later draws itself with no change here: an unknown `kind` keeps its own name. It is read
/// defensively all the same — a limit carrying no `percent` produces no gauge rather than a zero,
/// the same rule [`QuotaReading::Count`] states about a missing denominator.
fn claude_gauges(report: &serde_json::Value) -> Vec<QuotaGauge> {
    let limits = report
        .pointer("/rate_limits/limits")
        .and_then(serde_json::Value::as_array);
    let mut gauges: Vec<QuotaGauge> = limits
        .into_iter()
        .flatten()
        .filter_map(claude_gauge)
        .collect();
    gauges.extend(claude_extra_usage(
        report.pointer("/rate_limits/extra_usage"),
    ));
    gauges
}

/// One entry of `rate_limits.limits`.
fn claude_gauge(limit: &serde_json::Value) -> Option<QuotaGauge> {
    let used_pct = limit
        .get("percent")
        .and_then(serde_json::Value::as_f64)
        .map(|pct| pct.round().clamp(0.0, 100.0) as u8)?;
    // The provider's own word, as `/usage` prints it: "Current session", "Current week (all
    // models)", "Current week (Fable)" — shortened to what fits beside a bar, and a scoped
    // window carries the model it is scoped to because that is the only thing telling two
    // weekly gauges apart.
    let label = match limit.get("kind").and_then(serde_json::Value::as_str)? {
        "session" => "Session".to_string(),
        "weekly_all" => "Week".to_string(),
        "weekly_scoped" => match limit
            .pointer("/scope/model/display_name")
            .and_then(serde_json::Value::as_str)
        {
            Some(model) => format!("Week ({model})"),
            None => "Week (scoped)".to_string(),
        },
        other => other.replace('_', " "),
    };
    Some(QuotaGauge {
        label,
        reading: QuotaReading::Window { used_pct },
        resets_at: limit
            .get("resets_at")
            .and_then(serde_json::Value::as_str)
            .and_then(unix_seconds),
        detail: None,
    })
}

/// The extra-usage wallet, where the account has one switched on.
///
/// Only `utilization` is read: the report also carries `used_credits` and `monthly_limit`, but
/// both read `null` on every account observed so far, and a gauge built from an unobserved shape
/// is a guess. An account with extra usage off states `is_enabled: false` and produces nothing.
fn claude_extra_usage(extra: Option<&serde_json::Value>) -> Option<QuotaGauge> {
    let extra = extra?;
    if extra.get("is_enabled").and_then(serde_json::Value::as_bool) != Some(true) {
        return None;
    }
    let used_pct = extra
        .get("utilization")
        .and_then(serde_json::Value::as_f64)
        .map(|pct| if pct <= 1.0 { pct * 100.0 } else { pct })
        .map(|pct| pct.round().clamp(0.0, 100.0) as u8)?;
    Some(QuotaGauge {
        label: "Extra usage".to_string(),
        reading: QuotaReading::Window { used_pct },
        resets_at: None,
        detail: None,
    })
}

/// An RFC-3339 timestamp — `2026-09-28T16:00:00.327867+00:00`, `…Z` — as unix seconds.
///
/// Hand-rolled rather than a date crate: this is the only timestamp `agent-manager` parses, and
/// the crate stays free of `chrono`/`time` on purpose (`cli::session` says the same). Anything
/// that does not parse answers `None`, which draws as a gauge with no reset rather than a wrong
/// one.
fn unix_seconds(stamp: &str) -> Option<i64> {
    let (date, rest) = stamp.split_once('T')?;
    let mut date = date.split('-');
    let year: i64 = date.next()?.parse().ok()?;
    let month: i64 = date.next()?.parse().ok()?;
    let day: i64 = date.next()?.parse().ok()?;

    // Everything after the `T` is `HH:MM:SS[.frac]<zone>`, and the zone is where the first `+`,
    // `-` or `Z` appears — none of which can occur in the time itself.
    let zone_at = rest.find(['+', '-', 'Z'])?;
    let time_end = rest[..zone_at].find('.').unwrap_or(zone_at);
    let mut time = rest[..time_end].split(':');
    let hour: i64 = time.next()?.parse().ok()?;
    let minute: i64 = time.next()?.parse().ok()?;
    let second: i64 = time.next().unwrap_or("0").parse().ok()?;

    let zone = &rest[zone_at..];
    let offset = if zone.starts_with('Z') {
        0
    } else {
        let sign = if zone.starts_with('-') { -1 } else { 1 };
        let mut zone = zone[1..].split(':');
        let zone_hours: i64 = zone.next()?.parse().ok()?;
        let zone_minutes: i64 = zone.next().unwrap_or("0").parse().ok()?;
        sign * (zone_hours * 3600 + zone_minutes * 60)
    };

    Some(days_from_civil(year, month, day) * 86_400 + hour * 3600 + minute * 60 + second - offset)
}

/// Days from 1970-01-01 to a proleptic-Gregorian date — Howard Hinnant's `days_from_civil`.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let day_of_year = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// Now, in unix seconds.
pub(crate) fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or_default()
}

/// The error every harness that states no limit answers with, so the sentence is written once.
pub(crate) fn unreported(harness: &str) -> anyhow::Error {
    anyhow::anyhow!("harness '{harness}' does not report usage limits")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real `/usage` answer, captured off the stream-json bridge — the recording `G248` asked
    /// for, so a schema change is found by CI rather than by a user seeing no gauges.
    const RECORDED: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/claude-usage-report.json"
    ));

    #[test]
    fn the_recorded_usage_report_parses_into_the_windows_it_states() {
        let report: serde_json::Value = serde_json::from_str(RECORDED).unwrap();
        let gauges = claude_gauges(&report);
        let read: Vec<_> = gauges
            .iter()
            .map(|gauge| (gauge.label.as_str(), gauge.reading.used_pct()))
            .collect();
        assert_eq!(
            read,
            vec![
                ("Session", Some(9)),
                ("Week", Some(88)),
                ("Week (Fable)", Some(0)),
            ],
            "a scoped weekly window is named by the model it is scoped to"
        );
        // `2026-09-28T10:50:00.327851+00:00`.
        assert_eq!(gauges[0].resets_at, Some(1_790_592_600));
        assert_eq!(
            gauges.len(),
            3,
            "extra usage is off in the recording, so it draws nothing"
        );
    }

    #[test]
    fn a_limit_that_states_nothing_produces_no_gauge() {
        let report = serde_json::json!({
            "rate_limits": { "limits": [
                { "kind": "session", "resets_at": "2026-09-28T10:50:00+00:00" },
                { "kind": "weekly_all", "percent": 21 },
            ]},
        });
        let gauges = claude_gauges(&report);
        assert_eq!(gauges.len(), 1, "no figure, no gauge");
        assert_eq!(gauges[0].label, "Week");
        assert_eq!(gauges[0].resets_at, None);
    }

    #[test]
    fn a_window_claude_adds_later_keeps_its_own_name() {
        let report = serde_json::json!({
            "rate_limits": { "limits": [{ "kind": "monthly_all", "percent": 4 }] },
        });
        let gauges = claude_gauges(&report);
        assert_eq!(gauges[0].label, "monthly all");
        assert_eq!(gauges[0].reading, QuotaReading::Window { used_pct: 4 });
    }

    #[test]
    fn extra_usage_draws_only_when_it_is_switched_on_and_states_a_figure() {
        let off = serde_json::json!({
            "rate_limits": { "extra_usage": { "is_enabled": false, "utilization": 0.5 } },
        });
        assert!(claude_gauges(&off).is_empty());
        let on = serde_json::json!({
            "rate_limits": { "extra_usage": { "is_enabled": true, "utilization": 0.25 } },
        });
        assert_eq!(
            claude_gauges(&on)[0].reading,
            QuotaReading::Window { used_pct: 25 }
        );
    }

    #[test]
    fn a_timestamp_reads_with_its_fraction_and_its_zone() {
        assert_eq!(
            unix_seconds("2026-09-28T16:00:00.327867+00:00"),
            Some(1_790_611_200)
        );
        assert_eq!(unix_seconds("2026-09-28T16:00:00Z"), Some(1_790_611_200));
        assert_eq!(
            unix_seconds("2026-09-28T18:00:00+02:00"),
            Some(1_790_611_200),
            "an offset is subtracted, not ignored"
        );
        assert_eq!(
            unix_seconds("2026-09-28T14:00:00-02:00"),
            Some(1_790_611_200)
        );
        assert_eq!(unix_seconds("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(unix_seconds("whenever"), None, "no guess from a bad stamp");
    }

    #[test]
    fn a_count_reads_as_a_percentage_only_when_a_ceiling_was_stated() {
        assert_eq!(
            QuotaReading::Count {
                used: 150,
                limit: Some(300)
            }
            .used_pct(),
            Some(50)
        );
        assert_eq!(
            QuotaReading::Count {
                used: 150,
                limit: None
            }
            .used_pct(),
            None,
            "a meter needs a denominator somebody stated"
        );
        assert_eq!(
            QuotaReading::Credit {
                remaining: 500,
                currency: "USD".to_string()
            }
            .used_pct(),
            None,
            "nobody says what a full wallet is"
        );
    }

    #[test]
    fn the_worst_gauge_is_the_one_a_single_glyph_reports() {
        let snapshot = QuotaSnapshot {
            account: "work".to_string(),
            harness: "claude-code".to_string(),
            plan: Some("pro".to_string()),
            gauges: vec![
                QuotaGauge {
                    label: "5 hours".to_string(),
                    reading: QuotaReading::Window { used_pct: 7 },
                    resets_at: None,
                    detail: None,
                },
                QuotaGauge {
                    label: "Week".to_string(),
                    reading: QuotaReading::Window { used_pct: 88 },
                    resets_at: None,
                    detail: None,
                },
            ],
            as_of: 0,
        };
        assert_eq!(snapshot.worst_pct(), Some(88));
    }

    #[test]
    fn an_account_with_no_stated_limit_has_no_worst_gauge() {
        let snapshot = QuotaSnapshot {
            account: "work".to_string(),
            harness: "copilot".to_string(),
            plan: None,
            gauges: Vec::new(),
            as_of: 0,
        };
        assert_eq!(snapshot.worst_pct(), None, "an em dash, never a zero");
    }
}
