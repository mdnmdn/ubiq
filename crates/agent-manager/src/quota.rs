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
//! **No credential material leaves this module.** A probe reads the token, spends it on one
//! request and returns percentages; the token is never logged, never returned and never put in a
//! snapshot. That is the same invariant [`crate::account`] states for the account index itself.

use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};
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
    /// It can be asked with no process at all — a stored credential and one request.
    Probe,
    /// It can be asked, but only down a structured bridge that is already running.
    Bridge,
}

impl QuotaSource {
    /// Whether [`crate::harness::Harness::quota`] can answer with no process running.
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

/// Where Claude states what is left. Unofficial: it is the endpoint Claude Code's own client
/// calls, it is not in the published API, and it rate-limits. Everything that reads it treats a
/// failure as "not read" rather than as a number.
const CLAUDE_USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";

/// The beta header the endpoint requires, as Claude Code sends it.
const CLAUDE_OAUTH_BETA: &str = "oauth-2025-04-20";

/// Where a captured Claude login keeps its credentials, relative to the account home — the same
/// path [`crate::harness::SeedFile::credential`] seeds from in `harness::claude`.
const CLAUDE_CREDENTIALS_REL: &str = ".claude/.credentials.json";

/// Ask Claude what is left for `account`.
///
/// `login` is the account's captured-login source ([`crate::account::AccountStore::login_source`]),
/// which is how the credential is reached without this module knowing whether the store keeps a
/// home directory or the bytes themselves. Where there is none, the macOS Keychain entry the
/// import path already reads stands in.
///
/// The access token is read here, spent on one request and dropped. It is never returned, never
/// logged and never reaches a [`QuotaSnapshot`].
pub fn claude(
    account: &str,
    harness: &str,
    login: Option<&crate::Source>,
) -> Result<QuotaSnapshot> {
    let creds = claude_credentials(login)?;
    let value: serde_json::Value =
        serde_json::from_slice(&creds).context("parsing Claude credentials JSON")?;
    let oauth = value.get("claudeAiOauth").unwrap_or(&value);
    let token = oauth
        .get("accessToken")
        .and_then(|t| t.as_str())
        .filter(|t| !t.is_empty())
        .context("the Claude login holds no access token, so its limits cannot be read")?;
    let plan = oauth
        .get("subscriptionType")
        .and_then(|s| s.as_str())
        .map(str::to_string);

    let body: serde_json::Value = ureq::get(CLAUDE_USAGE_URL)
        .timeout(Duration::from_secs(10))
        .set("authorization", &format!("Bearer {token}"))
        .set("anthropic-beta", CLAUDE_OAUTH_BETA)
        .call()
        .map_err(claude_refusal)?
        .into_json()
        .context("the usage endpoint answered something that is not JSON")?;

    Ok(QuotaSnapshot {
        account: account.to_string(),
        harness: harness.to_string(),
        plan: plan.or_else(|| {
            body.get("subscription_type")
                .and_then(|s| s.as_str())
                .map(str::to_string)
        }),
        gauges: claude_gauges(&body),
        as_of: now(),
    })
}

/// Read the credential bytes for a captured Claude login.
fn claude_credentials(login: Option<&crate::Source>) -> Result<Vec<u8>> {
    if let Some(source) = login
        && let Some(bytes) = source.read(Path::new(CLAUDE_CREDENTIALS_REL))?
    {
        return Ok(bytes);
    }
    // No captured home: the secure-store path records the account without materializing files
    // (`account::record_default_claude_from_keychain`), and the Keychain entry is the login.
    crate::account::read_claude_keychain_credentials()
        .context("no captured Claude login to read the usage limits with")
}

/// Turn a transport or status failure into a sentence a user reads, never a code.
///
/// A 429 from an endpoint that is unofficial in the first place is the expected failure, and it
/// says so in its own words rather than as "error 429".
fn claude_refusal(error: ureq::Error) -> anyhow::Error {
    match error {
        ureq::Error::Status(429, _) => {
            anyhow::anyhow!("Claude is rate-limiting the usage endpoint; try again in a minute")
        }
        ureq::Error::Status(401 | 403, _) => anyhow::anyhow!(
            "Claude refused the stored login when asked for usage limits; signing in again fixes it"
        ),
        ureq::Error::Status(code, _) => {
            anyhow::anyhow!("Claude's usage endpoint answered {code}")
        }
        ureq::Error::Transport(transport) => {
            anyhow::anyhow!("Claude's usage endpoint could not be reached: {transport}")
        }
    }
}

/// Read whatever windows the answer states.
///
/// The endpoint is unofficial, so its shape is read defensively: a window that is absent, or
/// carries no utilization, produces no gauge rather than a zero. Both spellings seen in the wild
/// are accepted for each field, and a fraction is rounded the same way
/// [`crate::io::model::RateLimitWindow`] rounds one.
fn claude_gauges(body: &serde_json::Value) -> Vec<QuotaGauge> {
    [
        ("five_hour", "5 hours"),
        ("seven_day", "Week"),
        ("seven_day_opus", "Week (Opus)"),
    ]
    .into_iter()
    .filter_map(|(key, label)| {
        let window = body.get(key)?;
        let used_pct = window
            .get("utilization")
            .or_else(|| window.get("used_pct"))
            .and_then(serde_json::Value::as_f64)
            .map(|v| if v <= 1.0 { v * 100.0 } else { v })
            .map(|v| v.round().clamp(0.0, 100.0) as u8)?;
        Some(QuotaGauge {
            label: label.to_string(),
            reading: QuotaReading::Window { used_pct },
            resets_at: window
                .get("resets_at")
                .and_then(serde_json::Value::as_i64)
                .filter(|at| *at > 0),
            detail: None,
        })
    })
    .collect()
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

    #[test]
    fn a_fraction_and_a_percentage_both_read_as_a_percentage() {
        let body = serde_json::json!({
            "five_hour": { "utilization": 0.07, "resets_at": 1_788_474_600i64 },
            "seven_day": { "utilization": 21.0, "resets_at": 1_788_796_800i64 },
        });
        let gauges = claude_gauges(&body);
        assert_eq!(gauges.len(), 2);
        assert_eq!(gauges[0].reading, QuotaReading::Window { used_pct: 7 });
        assert_eq!(gauges[0].resets_at, Some(1_788_474_600));
        assert_eq!(gauges[1].reading, QuotaReading::Window { used_pct: 21 });
    }

    #[test]
    fn a_window_that_states_nothing_produces_no_gauge() {
        let body = serde_json::json!({ "five_hour": { "resets_at": 1i64 }, "seven_day": {} });
        assert!(claude_gauges(&body).is_empty(), "no figure, no gauge");
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
