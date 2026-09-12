//! What changed on disk in an open project, said without being asked.
//!
//! One recursive `notify` watcher per open project plus one debounce thread, the same shape
//! `crate::search` uses: a [`Job`] carries the root, the project's excludes and the [`Mailbox`] to
//! answer on, and the worker ends when the handle the coordinator holds is dropped.
//!
//! Two scopes out of one recursive watch, filtered rather than watched selectively — a selective
//! watch would have to be re-registered every time a directory appears:
//!
//! - anything under a `.git/` — the project's own, a nested clone's, or a submodule's git
//!   directory — never reaches `changed`; if it is `HEAD`, `MERGE_HEAD`, `index` or under
//!   `refs/`, it sets `repository` on the next flush instead
//! - everything else is dropped if the project's ignore rules exclude it
//!
//! Events are coalesced by path over a 150ms quiet window, and the batch is bounded like a search
//! batch: at 64 paths it flushes with `truncated`, and the reader re-lists the subtree instead of
//! patching the named paths.
//!
//! The quiet window is a floor and not the only trigger: a project under a build writes into
//! `target/` without a 150ms gap for as long as the build runs, and every one of those events is
//! dropped here rather than reported — so a window measured from the last event alone would hold a
//! real change back until the noise stopped. A change waits [`LATEST`] at the very most.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use notify::{RecursiveMode, Watcher as _};
use ubiq_proto::bus::Mailbox;
use ubiq_proto::ids::ProjectId;
use ubiq_proto::messages::Message;

/// How long a project has to be quiet before its changes are sent. An editor saving a directory
/// emits a burst of creates and renames; this collapses it to one change per path.
const QUIET: Duration = Duration::from_millis(150);

/// Paths per message, before the batch is called a burst instead of a list. Search's own bound.
const BOUND: usize = 64;

/// The longest a change waits, however busy the project stays. `QUIET` still decides when nothing
/// is happening; this is what a stream of events with no gap in it cannot outlast.
const LATEST: Duration = Duration::from_millis(1_200);

/// One project's watch, addressed.
pub struct Job {
    pub project_id: ProjectId,
    /// The project root, canonical — `notify` reports canonical paths, and `changed` is relative
    /// to this.
    pub root: PathBuf,
    /// Application-wide and per-project excludes, already merged by the coordinator, as the
    /// search job's are.
    pub excludes: Vec<String>,
    /// Where the same batch also goes, when the project keeps an index.
    ///
    /// A second destination rather than a relay through the coordinator: the push below goes
    /// straight from this thread to its client, so the coordinator never sees a change and has
    /// nothing to forward. The 150ms debounce has already coalesced the burst, so this is one
    /// extra send per flush and never one per event.
    #[cfg(feature = "index")]
    pub index: Option<flume::Sender<crate::index::Job>>,
    pub reply_to: Mailbox,
}

/// A live watch. Dropping it stops the watcher and ends the debounce thread.
pub struct Watcher {
    /// Held only to be dropped: dropping it closes the event channel, which is what ends the
    /// thread below.
    _watcher: notify::RecommendedWatcher,
}

/// Start watching `job.root`. Fails only if the platform watcher refuses the root.
pub fn start(mut job: Job) -> notify::Result<Watcher> {
    // `notify` reports canonical paths, and `classify` strips this root off them: a root reached
    // through a symlink — `/tmp`, `/var` and a home under one on macOS — would strip nothing and
    // every event would be dropped. Canonicalise once, here, rather than trust the catalogue's
    // spelling.
    if let Ok(canonical) = job.root.canonicalize() {
        job.root = canonical;
    }
    let (events, queue) = flume::unbounded::<notify::Event>();
    let mut watcher = notify::recommended_watcher(move |result: notify::Result<notify::Event>| {
        if let Ok(event) = result {
            let _ = events.send(event);
        }
    })?;
    watcher.watch(&job.root, RecursiveMode::Recursive)?;

    thread::Builder::new()
        .name(format!("ubiq-watch-{}", job.project_id))
        .spawn(move || debounce(job, queue))
        .expect("the watch thread");

    Ok(Watcher { _watcher: watcher })
}

/// Accumulate until the project goes quiet, then send. Ends when the watcher is dropped.
fn debounce(job: Job, queue: flume::Receiver<notify::Event>) {
    let ignore = ignore(&job.root, &job.excludes);
    let mut changed: HashSet<String> = HashSet::new();
    let mut repository = false;
    // When the oldest change still held was noticed. `None` means there is nothing to send.
    let mut since: Option<Instant> = None;

    loop {
        // Never wait past what the oldest held change is owed, so a project that keeps emitting
        // events cannot keep the batch from going out.
        let wait = match since {
            Some(first) => QUIET.min(LATEST.saturating_sub(first.elapsed())),
            None => QUIET,
        };
        match queue.recv_timeout(wait) {
            Ok(event) => {
                for path in &event.paths {
                    match classify(&job.root, path, ignore.as_ref()) {
                        Some(Change::Repository) => repository = true,
                        Some(Change::File(rel)) => {
                            changed.insert(rel);
                        }
                        None => continue,
                    }
                    since = since.or_else(|| Some(Instant::now()));
                }
                // A burst larger than the window can carry: say so and drop the names.
                if changed.len() >= BOUND {
                    changed.clear();
                    since = None;
                    if !flush(&job, Vec::new(), true, std::mem::take(&mut repository)) {
                        return;
                    }
                    continue;
                }
                // The project is not going quiet, and the oldest change has waited long enough.
                if since.is_some_and(|first| first.elapsed() >= LATEST) {
                    let names: Vec<String> = changed.drain().collect();
                    since = None;
                    if !flush(&job, names, false, std::mem::take(&mut repository)) {
                        return;
                    }
                }
            }
            Err(flume::RecvTimeoutError::Timeout) => {
                if changed.is_empty() && !repository {
                    since = None;
                    continue;
                }
                let names: Vec<String> = changed.drain().collect();
                since = None;
                if !flush(&job, names, false, std::mem::take(&mut repository)) {
                    return;
                }
            }
            Err(flume::RecvTimeoutError::Disconnected) => return,
        }
    }
}

/// Send one batch. `false` means the window has gone and there is nothing left to tell.
fn flush(job: &Job, changed: Vec<String>, truncated: bool, repository: bool) -> bool {
    // The index is told before the interface, and its failure is not the interface's problem: an
    // index thread that has gone means searches walk, which is what they did before one existed.
    // Only a file change matters — repository plumbing moving changes no file's content.
    notify_index(job, &changed, truncated);

    job.reply_to.send(Message::ProjectFilesChanged {
        project_id: job.project_id,
        changed,
        truncated,
        repository,
    })
}

#[cfg(feature = "index")]
fn notify_index(job: &Job, changed: &[String], truncated: bool) {
    if let Some(index) = &job.index
        && (!changed.is_empty() || truncated)
    {
        let _ = index.send(crate::index::Job::Changed {
            project_id: job.project_id,
            root: job.root.clone(),
            paths: changed.to_vec(),
            truncated,
        });
    }
}

/// Without `index` there is no index thread to tell.
#[cfg(not(feature = "index"))]
fn notify_index(_job: &Job, _changed: &[String], _truncated: bool) {}

enum Change {
    /// A project file, as a project-relative forward-slashed path.
    File(String),
    /// Repository plumbing moved.
    Repository,
}

/// What one event path means, or nothing if it is neither.
fn classify(
    root: &Path,
    path: &Path,
    ignore: Option<&ignore::gitignore::Gitignore>,
) -> Option<Change> {
    let rel = path.strip_prefix(root).ok()?;
    let rel: String = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/");
    if rel.is_empty() {
        return None;
    }

    // A `.git` **anywhere** under the project, not only at its root: a nested clone's plumbing
    // (`sub/.git/HEAD`) and a submodule's git directory (`.git/modules/sub/HEAD`) are both
    // repository moves, and neither is ever a project file. The repository *above* the project
    // keeps its `.git` outside the watched root, so a change there is still undetected (`G125`).
    if let Some(inner) = under_git_dir(&rel) {
        return plumbing(inner).then_some(Change::Repository);
    }

    if let Some(ignore) = ignore
        && ignore
            .matched_path_or_any_parents(path, path.is_dir())
            .is_ignore()
    {
        return None;
    }
    Some(Change::File(rel))
}

/// What is left of `rel` after the first `.git` component, or `None` if there is not one.
///
/// The `.git` directory itself answers `Some("")`, which is not a plumbing move.
fn under_git_dir(rel: &str) -> Option<&str> {
    let mut rest = rel;
    loop {
        let (head, tail) = match rest.split_once('/') {
            Some((head, tail)) => (head, tail),
            None => (rest, ""),
        };
        if head == ".git" {
            return Some(tail);
        }
        if tail.is_empty() {
            return None;
        }
        rest = tail;
    }
}

/// Whether a path inside a git directory is one the refresh cares about.
///
/// `HEAD`, `MERGE_HEAD`, `index` and anything under a `refs/` — at the top of the git directory,
/// or under `modules/<name>/` where a submodule's own HEAD actually lands. Everything else in
/// there (`objects/`, `index.lock`, `logs/`) says nothing the interface would redraw for.
fn plumbing(inner: &str) -> bool {
    if inner.is_empty() {
        return false;
    }
    let tail = inner.rsplit('/').next().unwrap_or(inner);
    matches!(tail, "HEAD" | "MERGE_HEAD" | "index") || inner.split('/').any(|part| part == "refs")
}

/// The project's ignore rules, as one matcher.
///
// ponytail: the root `.gitignore` plus the merged excludes, and nothing else — no nested
// `.gitignore`, no `.ignore`, no global excludesfile, and hidden files are not ignored the way
// `search::walk`'s `WalkBuilder` defaults ignore them. A change under an ignored nested directory
// therefore still reaches the interface, which re-lists and finds nothing new. Upgrade path: the
// `ignore` crate's `WalkBuilder` cannot answer per-path questions, so this becomes a stack of
// `Gitignore` matchers built per directory, or `ignore`'s own `dir` internals if they are ever
// exposed.
fn ignore(root: &Path, excludes: &[String]) -> Option<ignore::gitignore::Gitignore> {
    let mut builder = ignore::gitignore::GitignoreBuilder::new(root);
    builder.add(root.join(".gitignore"));
    for exclude in excludes {
        if let Err(error) = builder.add_line(None, exclude) {
            tracing::warn!(%exclude, %error, "an exclude the watcher could not compile");
        }
    }
    match builder.build() {
        Ok(ignore) => Some(ignore),
        Err(error) => {
            tracing::warn!(%error, "no ignore rules for this watch; every change will be reported");
            None
        }
    }
}
