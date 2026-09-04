//! Content search across a project's files.
//!
//! One worker thread, one unbounded queue, and a parallel walk inside the worker that uses the
//! project's own ignore rules. The coordinator resolves which project a search is for and submits
//! a [`Job`]; the worker sends batches of [`Message::SearchMatches`] back through the [`Mailbox`],
//! and a final [`Message::SearchFinished`].
//!
//! A search is interruptible via an [`Arc<AtomicBool>`]; the walker checks it between files and the
//! sink checks it between matched lines. A new search for the same project supersedes the old by
//! setting its flag.

pub mod ceiling;
pub mod fallback;
pub mod walk;
pub mod worker;

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::thread;

use ubiq_proto::bus::Mailbox;
use ubiq_proto::ids::{ProjectId, SearchId};
use ubiq_proto::search::{Filter, Query};

/// The thread that answers the search family.
///
/// One thread, not a pool. The parallelism is inside the walk, not in the queue — so batches for
/// one search arrive in the walk's own order, and a second search that supersedes the first is
/// interrupted rather than queued behind it.
pub struct Search {
    jobs: flume::Sender<Job>,
}

impl Search {
    /// Start the worker. It ends when the coordinator that holds this drops it.
    pub fn start() -> Self {
        let (jobs, queue) = flume::unbounded::<Job>();
        thread::Builder::new()
            .name("ubiq-search".to_string())
            .spawn(move || {
                while let Ok(job) = queue.recv() {
                    worker::run(&job);
                }
            })
            .expect("the search thread");
        Self { jobs }
    }

    /// Queue a search. Never blocks — the queue is unbounded.
    pub fn submit(&self, job: Job) {
        if self.jobs.send(job).is_err() {
            tracing::error!("the search thread has gone; a request was dropped");
        }
    }
}

/// One search request, addressed.
pub struct Job {
    pub project_id: ProjectId,
    pub search_id: SearchId,
    pub root: std::path::PathBuf,
    pub query: Query,
    /// What the search looks at, beside the query: include globs and a starting subdirectory.
    pub filter: Filter,
    /// Application-wide and per-project excludes, already merged — the coordinator is the only
    /// place holding [`crate::settings::Settings`], so it is the only place that can merge them.
    pub excludes: Vec<String>,
    /// External tools tried, in order, when the built-in regex engine cannot compile the query.
    pub fallbacks: Vec<String>,
    pub cancel: Arc<AtomicBool>,
    pub reply_to: Mailbox,
}
