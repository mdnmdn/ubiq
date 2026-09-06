//! What this Ubiq is doing right now, and what its agents have spent.
//!
//! Two unequal halves, and the inequality is the point. [`HostStats`]'s first five fields are
//! readings the coordinator takes when it is asked — memory, uptime, and three counts it already
//! holds. The two [`UsageRow`] vectors are the usage meter, written by the conversation pumps as
//! their harnesses report spend. **A harness that reports none contributes no rows at all** —
//! Codex, Copilot and opencode say nothing about what they spend today — so an empty vector means
//! "nobody said", never "zero". The screen says so rather than inventing a number.
//!
//! Every field is a sample rather than an event, which is why the interface asks on a timer
//! instead of being told — see `_docs/features/stats.md`.

use serde::{Deserialize, Serialize};

/// One reading of the host, taken when a window asked for it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct HostStats {
    /// The host process's resident set, in bytes. `None` where the platform would not say, which
    /// the screen reports as unavailable rather than as zero.
    pub rss_bytes: Option<u64>,
    /// How long the coordinator has been running. Not the process: the few milliseconds before the
    /// coordinator's thread exists are not observable in a figure rendered as minutes.
    pub uptime_secs: u64,
    /// Projects with at least one pane running in them.
    pub open_projects: usize,
    /// Harness conversations alive now — one entry is one harness on one pump thread.
    pub agents_live: usize,
    /// Every conversation started since this run began, including those that have since ended.
    /// A count rather than a length, because the ones it counts are gone.
    pub agents_this_run: usize,
    /// This run, minute by minute.
    pub this_run: Vec<UsageRow>,
    /// The durable aggregate, hour by hour, across every run.
    pub history: Vec<UsageRow>,
}

/// One bucket of usage: what was spent, by whom, on what.
///
/// The same record the host writes and the interface draws — an insert is a row with a bucket, a
/// read is the row back. The six dimensions are strings rather than options because that is how
/// they are keyed on disk: an empty string is "not said", and `NULL` would break the accumulating
/// upsert that makes a bucket a running total.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct UsageRow {
    /// Unix seconds, floored to the bucket — the hour for [`HostStats::history`], the minute for
    /// [`HostStats::this_run`]. UTC; naming the hour in the reader's zone is the interface's job.
    pub bucket: i64,
    /// The project's ULID, or empty when the work belonged to no project.
    pub project: String,
    /// The agent type: `claude`, `codex`, and the rest.
    pub harness: String,
    /// Empty when the harness ran as its own default identity.
    pub account: String,
    /// Empty when the harness did not say which model answered.
    pub model: String,
    /// The subagent type that spent it, empty when the conversation itself did. A sixth dimension
    /// rather than a flag: a turn's spend splits between the conversation and the agents it
    /// spawned, and the two must sum to what the harness billed.
    pub subagent: String,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub tokens_think: u64,
    /// Everything the harness counted that is none of the three above — cache reads and creations,
    /// most usefully.
    pub tokens_other: u64,
    pub msgs_in: u64,
    pub msgs_out: u64,
    pub tool_calls: u64,
}

impl UsageRow {
    /// Every token in the bucket, however it was spent. The one derived figure the screen shows,
    /// here rather than in the interface so the host's rows and the screen's total cannot drift.
    pub fn tokens_total(&self) -> u64 {
        self.tokens_in
            .saturating_add(self.tokens_out)
            .saturating_add(self.tokens_think)
            .saturating_add(self.tokens_other)
    }
}
