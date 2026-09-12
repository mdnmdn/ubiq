//! How much of an account's plan is left: the cache, and the thread that asks.
//!
//! **A cache, not a record.** [`crate::store::usage`] writes spend to a database because nobody
//! can ask a harness what it spent last Tuesday — a report unfiled is a fact lost (`D78`). What
//! is *left* is the opposite: it is re-derivable by asking the provider again, so it is held in
//! memory, keyed by account and harness, and never written down. A restart re-probes, and the
//! worst a lost cache costs is one HTTPS call.
//!
//! Two things fill it. A [`Job`] asks the provider with nothing running, on this module's own
//! thread because the ask is a blocking network call. A running Claude conversation pushes a
//! reading unasked, mid-turn, which is why an account with a live agent reads fresher than one
//! without. Both file the same fact the same way: [`ubiq_proto::bus::Voice`] says
//! [`Message::QuotaChanged`] into the host's own inbox, and the coordinator — the only thread
//! that owns [`Quotas`] — decides whether anything changed and tells the windows.
//!
//! Nothing here is credential material. A snapshot is percentages, a plan name and a timestamp.

use std::collections::HashMap;
use std::path::PathBuf;
use std::thread;

use ubiq_proto::bus::{Mailbox, Voice};
use ubiq_proto::messages::Message;
use ubiq_proto::quota::QuotaSnapshot;

/// What each account has left, as the host last heard it.
///
/// Keyed by account *and* harness: one identity can serve several harnesses and each states its
/// own limits. Held by the coordinator alone, so no lock is needed to read one.
#[derive(Default)]
pub struct Quotas {
    held: HashMap<(String, String), QuotaSnapshot>,
}

impl Quotas {
    /// An empty cache — which is what every start is.
    pub fn new() -> Self {
        Self::default()
    }

    /// What this account last read under this harness, if anything has been read at all.
    pub fn get(&self, account: &str, harness: &str) -> Option<&QuotaSnapshot> {
        self.held.get(&(account.to_string(), harness.to_string()))
    }

    /// File a snapshot, and say whether it told the host anything new.
    ///
    /// **A reading that differs only in `as_of` is not a change.** Asking again always moves the
    /// timestamp, so comparing whole snapshots would broadcast on every probe and redraw every
    /// window for nothing. What a surface draws is the gauges and the plan, so that is what
    /// decides — the newer timestamp is still stored, because how old the reading is is the one
    /// thing a stale-looking gauge is judged by.
    pub fn put(&mut self, snapshot: QuotaSnapshot) -> bool {
        let key = (snapshot.account.clone(), snapshot.harness.clone());
        let changed = match self.held.get(&key) {
            Some(held) => held.plan != snapshot.plan || held.gauges != snapshot.gauges,
            None => true,
        };
        self.held.insert(key, snapshot);
        changed
    }
}

/// One probe: whose plan, under which harness, and who is waiting for the answer.
pub struct Job {
    /// The account id, the key a window belongs to.
    pub account: String,
    /// The agent type that answers. One account can serve several.
    pub harness: String,
    /// The window that asked. It gets [`Message::QuotaRead`] whether the ask worked or not.
    pub reply_to: Mailbox,
    /// The way back into the host's own inbox, so a reading that worked reaches the cache every
    /// window reads from rather than only the window that asked.
    pub voice: Voice,
}

/// The thread that asks a provider what is left.
///
/// **One thread, not a pool**, for the reason [`crate::files`] gives: a first-in-first-out queue
/// means two asks about one account answer in the order they were made. It exists at all because
/// the ask is a blocking HTTPS call, and anything blocking on the coordinator's thread stalls
/// every pane's keystrokes.
///
/// No ticker. Nothing here wakes up on its own: a reading is asked for, or it is pushed by a
/// running conversation.
pub struct Quota {
    jobs: flume::Sender<Job>,
}

impl Quota {
    /// Start the worker over Ubiq's config root. It ends when the coordinator that holds this
    /// drops it.
    ///
    /// It takes a root of its own rather than a handle the coordinator shares, because everything
    /// a probe needs is a path: the account store is a path wrapper, and building one per job is
    /// what keeps a login captured elsewhere visible without a restart.
    pub fn start(root: PathBuf) -> Self {
        let (jobs, queue) = flume::unbounded::<Job>();
        thread::Builder::new()
            .name("ubiq-quota".to_string())
            .spawn(move || {
                while let Ok(job) = queue.recv() {
                    answer(&root, job);
                }
            })
            .expect("the quota thread");
        Self { jobs }
    }

    /// Queue an ask. Never blocks — the queue is unbounded, on the bus's own rule.
    pub fn submit(&self, job: Job) {
        if self.jobs.send(job).is_err() {
            tracing::error!("the quota thread has gone; an ask was dropped");
        }
    }
}

/// Ask once, tell the window that asked, and file what came back.
fn answer(root: &std::path::Path, job: Job) {
    let (snapshot, error) = match crate::agent::quota_of(root, &job.account, &job.harness) {
        Ok(snapshot) => (Some(snapshot), None),
        // The sentence the user reads. `anyhow`'s chain names what was being done and what
        // failed; nothing token-shaped ever reaches it.
        Err(error) => (None, Some(format!("{error:#}"))),
    };

    // Filed before the answer goes out, so a window that redraws on `QuotaChanged` and one that
    // redraws on its own `QuotaRead` never disagree about which reading is newer.
    if let Some(snapshot) = &snapshot {
        job.voice.say(Message::QuotaChanged {
            account: job.account.clone(),
            harness: job.harness.clone(),
            snapshot: snapshot.clone(),
        });
    }

    // A window that has gone is not an error: nothing is left to draw the answer.
    job.reply_to.send(Message::QuotaRead {
        account: job.account,
        harness: job.harness,
        snapshot,
        error,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use ubiq_proto::quota::{QuotaGauge, QuotaReading};

    fn snapshot(used_pct: u8, as_of: i64) -> QuotaSnapshot {
        QuotaSnapshot {
            account: "work".to_string(),
            harness: "claude-code".to_string(),
            plan: Some("max".to_string()),
            gauges: vec![QuotaGauge {
                label: "5 hours".to_string(),
                reading: QuotaReading::Window { used_pct },
                resets_at: Some(1_788_474_600),
                detail: None,
            }],
            as_of,
        }
    }

    #[test]
    fn the_first_reading_of_an_account_is_a_change() {
        let mut quotas = Quotas::new();
        assert!(quotas.put(snapshot(7, 100)));
        assert_eq!(
            quotas.get("work", "claude-code").map(|s| s.as_of),
            Some(100)
        );
    }

    #[test]
    fn asking_again_and_hearing_the_same_thing_is_not_a_change() {
        let mut quotas = Quotas::new();
        quotas.put(snapshot(7, 100));
        assert!(
            !quotas.put(snapshot(7, 200)),
            "a newer timestamp alone tells a window nothing"
        );
        assert_eq!(
            quotas.get("work", "claude-code").map(|s| s.as_of),
            Some(200),
            "the newer reading is still what is held"
        );
        assert!(quotas.put(snapshot(9, 300)), "a moved gauge is a change");
    }

    #[test]
    fn one_account_holds_a_reading_per_harness() {
        let mut quotas = Quotas::new();
        quotas.put(snapshot(7, 100));
        let mut other = snapshot(50, 100);
        other.harness = "codex".to_string();
        assert!(quotas.put(other));
        assert_eq!(
            quotas
                .get("work", "claude-code")
                .and_then(|s| s.worst_pct()),
            Some(7)
        );
        assert_eq!(
            quotas.get("work", "codex").and_then(|s| s.worst_pct()),
            Some(50)
        );
    }
}
