//! The per-project index: what a project contains, kept so a question about it need not re-read
//! every byte.
//!
//! Only the full-text half exists here. It answers *which files could match a query*, and never
//! what matched — [`crate::search`] reads the candidates back and produces the hits, so the index
//! is a shortcut into the existing search rather than a second one beside it (`D75`).
//!
//! One thread, one unbounded queue, and one open index per project, on the shape
//! [`crate::search`] and [`crate::watch`] already proved. The thread owns every [`text::Text`],
//! so a writer is never shared and never locked; what leaves it is a [`text::Reader`], which
//! carries no writer and cannot block the thread that is indexing.
//!
//! Whether a project has an index at all is the user's choice, per project, defaulting from an
//! application-wide setting. Nothing here decides that: the coordinator resolves the level and
//! either sends a [`Job::Build`] or does not.

pub mod ceiling;
pub mod text;

use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;

use ubiq_proto::ids::ProjectId;

/// The handle the coordinator holds. Dropping it ends the thread.
pub struct Index {
    jobs: flume::Sender<Job>,
    /// Read handles, published by the worker as each project's index opens.
    ///
    /// A map beside the queue rather than a reply on it: a search needs the reader *now*, on the
    /// coordinator's own thread, and asking the index thread for it would put a request behind
    /// whatever build is in flight — which is exactly the wait this design exists to avoid.
    readers: Arc<Mutex<HashMap<ProjectId, text::Reader>>>,
}

/// What the index thread is asked to do.
pub enum Job {
    /// Build a project's index from nothing, or open the one already on disk.
    Build {
        project_id: ProjectId,
        /// The project's folder — what is indexed.
        root: PathBuf,
        /// The host-owned directory the index lives in, outside the project's folder.
        home: PathBuf,
        /// Application-wide and per-project excludes, already merged by the coordinator.
        excludes: Vec<String>,
    },
    /// Files changed under a project the watcher is watching.
    Changed {
        project_id: ProjectId,
        root: PathBuf,
        /// Project-relative paths. A path that no longer exists is a delete.
        paths: Vec<String>,
        /// The watcher dropped the names — what changed is unknown, so the index is rebuilt.
        truncated: bool,
    },
    /// Forget this project's index and delete it from disk. What a level of `none` means, and
    /// what closing a project does.
    Drop(ProjectId),
}

impl Index {
    /// Start the worker. It ends when the coordinator that holds this drops it.
    pub fn start() -> Self {
        let (jobs, queue) = flume::unbounded::<Job>();
        let readers: Arc<Mutex<HashMap<ProjectId, text::Reader>>> = Arc::default();
        let published = readers.clone();

        thread::Builder::new()
            .name("ubiq-index".to_string())
            .spawn(move || worker(queue, published))
            .expect("the index thread");

        Self { jobs, readers }
    }

    /// Queue a job. Never blocks — the queue is unbounded.
    pub fn submit(&self, job: Job) {
        if self.jobs.send(job).is_err() {
            tracing::error!("the index thread has gone; a request was dropped");
        }
    }

    /// A sender the watcher can keep, so a change reaches the index without the coordinator
    /// relaying it.
    ///
    /// The watch thread pushes `ProjectFilesChanged` straight to its client and the coordinator
    /// never sees it, so there is nothing for it to forward. One extra send beside that push is
    /// cheaper than a second watch, and the watcher's own 150ms debounce has already coalesced
    /// the burst.
    pub fn sender(&self) -> flume::Sender<Job> {
        self.jobs.clone()
    }

    /// A read handle on one project's index, if it has one.
    ///
    /// `None` means no index — never an error, and never a reason to refuse a search. The caller
    /// puts it on the [`crate::search::Job`] and the worker walks when it is absent.
    pub fn reader(&self, project_id: ProjectId) -> Option<text::Reader> {
        self.readers.lock().ok()?.get(&project_id).cloned()
    }
}

/// One project's open index, and what it still owes a commit.
struct Open {
    text: text::Text,
    root: PathBuf,
    excludes: Vec<String>,
    /// Paths written since the last commit. Committing is the expensive part of an incremental
    /// update, and an editor saving a file produces a burst rather than one event.
    pending: usize,
}

fn worker(queue: flume::Receiver<Job>, readers: Arc<Mutex<HashMap<ProjectId, text::Reader>>>) {
    let mut open: HashMap<ProjectId, Open> = HashMap::new();

    loop {
        match queue.recv_timeout(ceiling::COMMIT_QUIET) {
            Ok(job) => {
                handle(job, &mut open, &readers);
                // A burst that never goes quiet must still land, so a big change is not held back
                // by the user continuing to work.
                for entry in open.values_mut() {
                    if entry.pending >= ceiling::COMMIT_PENDING {
                        commit(entry);
                    }
                }
            }
            // The queue went quiet: land whatever is outstanding.
            Err(flume::RecvTimeoutError::Timeout) => {
                for entry in open.values_mut() {
                    if entry.pending > 0 {
                        commit(entry);
                    }
                }
            }
            Err(flume::RecvTimeoutError::Disconnected) => break,
        }
    }
}

fn handle(
    job: Job,
    open: &mut HashMap<ProjectId, Open>,
    readers: &Arc<Mutex<HashMap<ProjectId, text::Reader>>>,
) {
    match job {
        Job::Build {
            project_id,
            root,
            home,
            excludes,
        } => {
            if let Entry::Occupied(existing) = open.entry(project_id) {
                // Already open. A second Build is a level change or a reopen, not a reason to
                // throw away an index that is already answering.
                let _ = existing;
                return;
            }
            match build(project_id, &root, &home, &excludes) {
                Ok(text) => {
                    if let Ok(mut readers) = readers.lock() {
                        readers.insert(project_id, text.reader());
                    }
                    open.insert(
                        project_id,
                        Open {
                            text,
                            root,
                            excludes,
                            pending: 0,
                        },
                    );
                }
                Err(error) => {
                    // No index for this project, so every search walks. That is the behaviour
                    // before any index existed, which is why this is a warning and not an error
                    // anybody is shown.
                    tracing::warn!(project = %project_id, %error, "the index could not be built; searches will walk");
                }
            }
        }

        Job::Changed {
            project_id,
            root,
            paths,
            truncated,
        } => {
            let Some(entry) = open.get_mut(&project_id) else {
                return;
            };
            if truncated {
                // The watcher dropped the names, so what changed is unknown. Rebuilding is the
                // only honest answer, and searches walk while it runs — the same fallback the
                // explorer takes on the same signal.
                tracing::debug!(project = %project_id, "a truncated burst; rebuilding the index");
                entry.text.set_ready(false);
                if entry.text.clear().is_ok() {
                    let root = entry.root.clone();
                    let excludes = entry.excludes.clone();
                    fill(&mut entry.text, &root, &excludes);
                    entry.text.set_ready(true);
                }
                entry.pending = 0;
                return;
            }
            for rel in paths {
                let abs = root.join(&rel);
                entry.text.put(&rel, read_indexable(&abs).as_deref());
                entry.pending += 1;
            }
        }

        Job::Drop(project_id) => {
            if let Ok(mut readers) = readers.lock() {
                readers.remove(&project_id);
            }
            open.remove(&project_id);
            // The directory goes with it. It is derived data, and a project nobody has open
            // should not be paying disk for an index nobody is asking.
        }
    }
}

fn commit(entry: &mut Open) {
    if let Err(error) = entry.text.commit() {
        // The batch is lost, so what is on disk no longer matches what the index believes. A
        // stale index cannot invent a hit — every candidate is re-read — so the cost is a missed
        // file until it is touched again, not a wrong answer.
        tracing::warn!(%error, "an index commit failed; its batch was dropped");
    }
    entry.pending = 0;
}

/// Open a project's index and fill it, answering a handle ready to be consulted.
fn build(
    project_id: ProjectId,
    root: &std::path::Path,
    home: &std::path::Path,
    excludes: &[String],
) -> Result<text::Text, String> {
    let started = std::time::Instant::now();
    std::fs::create_dir_all(home).map_err(|error| error.to_string())?;
    let mut text = text::Text::open(home).map_err(|error| error.to_string())?;

    // Emptied first, even when a stamped index was already on disk. The walk writes over every
    // file it finds, but nothing visits a file that was *deleted* while the project was closed,
    // so its document would survive as a candidate that no longer exists — a wasted read on every
    // query naming it, for as long as the project lives.
    //
    // Nothing is lost by this today: there is no per-file reuse, so a build re-reads and
    // re-tokenises the whole tree either way and the kept directory saved only its own creation.
    //
    // ponytail: whole rebuild per open. Key each document by modification time and length — the
    // walk already yields both — and reuse the ones that match, deleting only what the walk did
    // not see. Worth it when a cold start on a large project is slow enough to notice.
    if let Err(error) = text.clear() {
        tracing::warn!(%error, "the stale index could not be emptied; rebuilding cold");
    }
    fill(&mut text, root, excludes);
    text.set_ready(true);

    tracing::info!(
        project = %project_id,
        files = text.len(),
        elapsed_ms = started.elapsed().as_millis(),
        "the index is built"
    );
    Ok(text)
}

/// Walk the project and write every file the rules allow.
///
/// The walk is [`crate::search::walk::builder`] — the same one content search runs, so the index
/// and the search can never disagree about what a project's files are.
fn fill(text: &mut text::Text, root: &std::path::Path, excludes: &[String]) {
    let builder = match crate::search::walk::builder(root, root, &[], excludes) {
        Ok(builder) => builder,
        Err(error) => {
            tracing::warn!(%error, "the index walk could not start");
            return;
        }
    };

    // Read in parallel, write on this thread: the reads are what the time goes on, and the writer
    // is not shared. A bounded channel would deadlock against a walker that cannot yield, so the
    // walk collects and this drains it.
    let (found, ready) = flume::unbounded::<(String, String)>();
    let root = root.to_path_buf();
    builder.build_parallel().run(|| {
        let found = found.clone();
        let root = root.clone();
        Box::new(move |entry| {
            let Ok(entry) = entry else {
                return ignore::WalkState::Continue;
            };
            if !entry.file_type().is_some_and(|kind| kind.is_file()) {
                return ignore::WalkState::Continue;
            }
            let len = entry.metadata().map(|meta| meta.len()).unwrap_or(u64::MAX);
            if let Some(body) = read_indexable_with_len(entry.path(), len) {
                let rel = entry
                    .path()
                    .strip_prefix(&root)
                    .unwrap_or(entry.path())
                    .to_string_lossy()
                    .into_owned();
                let _ = found.send((rel, body));
            }
            ignore::WalkState::Continue
        })
    });
    drop(found);

    for (written, (rel, body)) in ready.iter().enumerate() {
        if written >= ceiling::FILES {
            tracing::warn!(
                ceiling = ceiling::FILES,
                "the file ceiling was hit; the rest of the project is not indexed"
            );
            break;
        }
        text.put(&rel, Some(&body));
    }

    if let Err(error) = text.commit() {
        tracing::warn!(%error, "the index could not be committed after a build");
    }
}

/// A file's text, if it is one worth indexing. `None` covers gone, unreadable, too big and binary
/// — all of which mean the same thing here: the index has nothing to say about it.
fn read_indexable(path: &std::path::Path) -> Option<String> {
    let len = std::fs::metadata(path).ok()?.len();
    read_indexable_with_len(path, len)
}

fn read_indexable_with_len(path: &std::path::Path, len: u64) -> Option<String> {
    if !text::indexable(path, len) {
        return None;
    }
    std::fs::read_to_string(path).ok()
}
