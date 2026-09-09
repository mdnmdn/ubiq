//! The "Remote hosts" manager panel's state: which connections exist and what they are doing.
//!
//! Nothing here opens a socket. The live connections live on the [`Bus`](crate::app::Bus), the
//! saved entries in the settings record, the retry loop in `app::remote_connect` — this is only
//! what the panel draws: whether it is up, and the last thing each entry's test button said.

use std::collections::{HashMap, HashSet};

/// The manager panel, while it is up.
#[derive(Clone, Debug, Default)]
pub struct RemoteManagerState {
    /// Whether the panel is on screen. The connections it lists live on the bus and in the
    /// settings record — this is only the panel's own chrome and test outcomes.
    pub open: bool,
    /// The saved-host key (`host_secrets::key_for`) being renamed, while the rename prompt is up.
    pub renaming: Option<String>,
    /// One test outcome per saved-host key, from the last test run for it. Kept rather than
    /// cleared so a result outlives the row's re-render; a new test overwrites it.
    pub tests: HashMap<String, TestOutcome>,
    /// Keys with a test currently in flight. The row draws a spinner rather than a second
    /// button while one is running.
    pub testing: HashSet<String>,
}

/// What one test button run said.
#[derive(Clone, Debug)]
pub struct TestOutcome {
    /// Whether the host answered the `Stats` poll — connected *and* speaking Ubiq.
    pub ok: bool,
    /// Worded for the row to show directly.
    pub report: String,
}

impl RemoteManagerState {
    /// A test started: the old outcome stays visible until the new one lands, but the row spins.
    pub fn test_started(&mut self, key: String) {
        self.testing.insert(key);
    }

    /// A test landed: spin stops, outcome recorded.
    pub fn test_finished(&mut self, key: String, ok: bool, report: String) {
        self.testing.remove(&key);
        self.tests.insert(key, TestOutcome { ok, report });
    }
}
