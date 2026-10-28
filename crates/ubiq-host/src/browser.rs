//! The internal browser's host half: the history worker, and the table an agent's browser
//! command waits in.
//!
//! **History** is a worker thread on [`crate::watch`]'s shape: the coordinator hands it a [`Job`]
//! and returns, the worker reads and writes `local/browser-history.json`
//! ([`crate::store::browser_history`]) and answers on the mailbox the job names. It keeps each
//! project's list in memory once read, so a visit costs one write and no read. The worker ends
//! when the [`History`] handle is dropped.
//!
//! **Commands** are [`crate::ask::Asks`]'s pattern, live. The host holds no web view, so an
//! `ubiq-browser` tool call has to be carried out by a window: the MCP thread files a row here,
//! says [`Message::BrowserCommand`] through the host's own voice, and waits on the row's channel.
//! The coordinator — the only half that knows which window owns what — addresses the command,
//! notes the window in the row ([`Commands::sent_to`]), and releases it when that window's
//! [`Message::BrowserReply`] arrives ([`Commands::reply`]). Neither thread waits on the other.
//! Every wait is bounded ([`ubiq_proto::browser::BROWSER_TIMEOUT_SECS`]), and a window that goes
//! releases everything addressed to it ([`Commands::client_gone`]).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::time::Duration;

use chrono::{DateTime, Utc};
use ubiq_proto::browser::BrowserHistoryEntry;
use ubiq_proto::bus::{ClientId, Mailbox};
use ubiq_proto::ids::{BrowserReqId, ProjectId};
use ubiq_proto::messages::Message;

use crate::store::browser_history;
use crate::store::project_dir::ProjectDirs;

/// What the tool reads when no window could be asked, or the one asked went away.
pub const NO_WINDOW: &str = "no browser window";

// ── history ─────────────────────────────────────────────────────────────────────────────────

/// One thing the history worker is asked to do.
pub enum Job {
    /// Send the project's history to `asker`.
    Get {
        project_id: ProjectId,
        asker: Mailbox,
    },
    /// Record a visit, write the file, tell every window.
    Add {
        project_id: ProjectId,
        url: String,
        title: String,
        at: DateTime<Utc>,
    },
    /// Forget the history, write the file, tell every window.
    Clear { project_id: ProjectId },
}

/// The handle the coordinator holds. Dropping it ends the worker.
pub struct History {
    jobs: Sender<Job>,
}

impl History {
    /// Start the worker over the config root. `everyone` is where a change is announced.
    pub fn start(root: PathBuf, everyone: Mailbox) -> Self {
        let (jobs, inbox) = channel::<Job>();
        let spawned = std::thread::Builder::new()
            .name("ubiq-browser-history".to_string())
            .spawn(move || work(ProjectDirs::new(root), everyone, inbox));
        if let Err(error) = spawned {
            tracing::error!("the browser history worker could not start: {error}");
        }
        Self { jobs }
    }

    /// Hand a job over. A worker that never started drops it; the history is a convenience and
    /// its absence is a log line, not an error anybody has to read.
    pub fn submit(&self, job: Job) {
        let _ = self.jobs.send(job);
    }
}

/// One project's list as the worker keeps it. No path: the file is re-resolved on every write, so
/// a storage-mode switch (`D173`) moves the next write with the project rather than leaving it in
/// the old tree.
struct Kept {
    entries: Vec<BrowserHistoryEntry>,
    writable: bool,
}

/// One project's list, read from disk the first time it is asked about.
fn open<'a>(
    kept: &'a mut HashMap<ProjectId, Kept>,
    dirs: &ProjectDirs,
    project_id: ProjectId,
) -> &'a mut Kept {
    kept.entry(project_id).or_insert_with(|| {
        let loaded = browser_history::load(&browser_history::path(&dirs.data(project_id)));
        Kept {
            entries: loaded.entries,
            writable: loaded.writable,
        }
    })
}

fn work(dirs: ProjectDirs, everyone: Mailbox, inbox: Receiver<Job>) {
    let mut kept: HashMap<ProjectId, Kept> = HashMap::new();
    while let Ok(job) = inbox.recv() {
        match job {
            Job::Get { project_id, asker } => {
                let entries = open(&mut kept, &dirs, project_id).entries.clone();
                asker.send(Message::BrowserHistory {
                    project_id,
                    entries,
                });
            }
            Job::Add {
                project_id,
                url,
                title,
                at,
            } => {
                let list = open(&mut kept, &dirs, project_id);
                if !browser_history::record(&mut list.entries, &url, &title, at) {
                    continue;
                }
                persist(&dirs, project_id, list);
                everyone.send(Message::BrowserHistory {
                    project_id,
                    entries: list.entries.clone(),
                });
            }
            Job::Clear { project_id } => {
                let list = open(&mut kept, &dirs, project_id);
                list.entries.clear();
                persist(&dirs, project_id, list);
                everyone.send(Message::BrowserHistory {
                    project_id,
                    entries: Vec::new(),
                });
            }
        }
    }
}

fn persist(dirs: &ProjectDirs, project_id: ProjectId, list: &Kept) {
    if !list.writable {
        return;
    }
    let path = browser_history::path(&dirs.data(project_id));
    if let Err(error) = browser_history::save(&path, &list.entries) {
        tracing::warn!(path = %path.display(), %error, "browser history not written");
    }
}

// ── commands ────────────────────────────────────────────────────────────────────────────────

/// How a browser command ended: the window's JSON text, or a sentence for the agent.
pub type Outcome = Result<String, String>;

/// One command waiting on a window.
struct Row {
    /// The URL segment of the agent that called — an `AgentId` or a `PaneId` as a string, which
    /// is what the coordinator resolves the owning window from.
    caller: String,
    /// The window it was sent to, once the coordinator has addressed it. A reply from any other
    /// window is dropped.
    target: Option<ClientId>,
    reply: Sender<Outcome>,
}

/// Every browser command an agent is waiting on. Shared behind an `Arc` between the MCP listener
/// (which files and waits) and the coordinator (which addresses and releases).
#[derive(Default)]
pub struct Commands {
    pending: Mutex<HashMap<BrowserReqId, Row>>,
}

impl Commands {
    pub fn new() -> Self {
        Self::default()
    }

    /// File a command for the agent whose URL segment is `caller`, and hand back the id the
    /// message carries and the end the calling thread waits on.
    pub fn raise(&self, caller: &str) -> (BrowserReqId, Receiver<Outcome>) {
        let req_id = BrowserReqId::generate();
        let (reply, waiting) = channel();
        self.lock().insert(
            req_id,
            Row {
                caller: caller.to_string(),
                target: None,
                reply,
            },
        );
        (req_id, waiting)
    }

    /// Who filed this command, if it is still waiting.
    pub fn caller(&self, req_id: BrowserReqId) -> Option<String> {
        self.lock().get(&req_id).map(|row| row.caller.clone())
    }

    /// Note which window the command went to.
    pub fn sent_to(&self, req_id: BrowserReqId, client: ClientId) {
        if let Some(row) = self.lock().get_mut(&req_id) {
            row.target = Some(client);
        }
    }

    /// End a command with `outcome`. `false` when nothing was waiting — a second reply, or one
    /// after the wait gave up.
    pub fn finish(&self, req_id: BrowserReqId, outcome: Outcome) -> bool {
        let Some(row) = self.lock().remove(&req_id) else {
            return false;
        };
        let _ = row.reply.send(outcome);
        true
    }

    /// A window's reply. Only the window the command was sent to may end it; the first reply
    /// wins.
    pub fn reply(&self, client: ClientId, req_id: BrowserReqId, outcome: Outcome) -> bool {
        let mut pending = self.lock();
        match pending.get(&req_id) {
            Some(row) if row.target == Some(client) => {}
            Some(_) => {
                tracing::debug!(req = %req_id, "a browser reply from a window it was not sent to");
                return false;
            }
            None => return false,
        }
        let row = pending.remove(&req_id).expect("checked above");
        drop(pending);
        let _ = row.reply.send(outcome);
        true
    }

    /// Wait for the command's end, giving up after `bound`. Called on the MCP call's own thread.
    pub fn wait(
        &self,
        req_id: BrowserReqId,
        waiting: &Receiver<Outcome>,
        bound: Duration,
    ) -> Outcome {
        match waiting.recv_timeout(bound) {
            Ok(outcome) => outcome,
            Err(RecvTimeoutError::Timeout) => {
                // A reply that beat the give-up by a lock is still collected: `finish` removes
                // the row and then sends, the same race `Asks::give_up` closes.
                if self.lock().remove(&req_id).is_none()
                    && let Ok(outcome) = waiting.try_recv()
                {
                    return outcome;
                }
                Err(format!(
                    "the browser window did not answer within {}s",
                    bound.as_secs()
                ))
            }
            Err(RecvTimeoutError::Disconnected) => Err(NO_WINDOW.to_string()),
        }
    }

    /// A window has gone: everything sent to it ends as [`NO_WINDOW`].
    pub fn client_gone(&self, client: ClientId) {
        let mut pending = self.lock();
        let gone: Vec<BrowserReqId> = pending
            .iter()
            .filter(|(_, row)| row.target == Some(client))
            .map(|(req_id, _)| *req_id)
            .collect();
        for req_id in gone {
            if let Some(row) = pending.remove(&req_id) {
                let _ = row.reply.send(Err(NO_WINDOW.to_string()));
            }
        }
    }

    /// A poisoned lock is taken back: the table is a set of waiting calls, not an invariant.
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<BrowserReqId, Row>> {
        self.pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wait_nobody_answers_times_out_and_forgets_the_row() {
        let commands = Commands::new();
        let (req_id, waiting) = commands.raise("agent");
        let outcome = commands.wait(req_id, &waiting, Duration::from_millis(5));
        assert!(outcome.unwrap_err().contains("did not answer"));
        assert!(commands.caller(req_id).is_none());
    }

    #[test]
    fn a_finished_command_releases_the_wait_and_a_second_finish_is_ignored() {
        let commands = Commands::new();
        let (req_id, waiting) = commands.raise("agent");
        assert!(commands.finish(req_id, Ok("{}".to_string())));
        assert!(!commands.finish(req_id, Ok("again".to_string())));
        assert_eq!(
            commands.wait(req_id, &waiting, Duration::from_secs(1)),
            Ok("{}".to_string())
        );
    }
}
