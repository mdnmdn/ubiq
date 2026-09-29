//! A project's repository as the host observes it — and, when the Git screen asks, writes it.
//!
//! **Nothing here runs on the coordinator's thread.** A cold status on a large repository is
//! seconds, and seconds on the coordinator's thread is every pane's keystrokes stalled behind it.
//! The shape [`crate::files::Files`] proved is copied: a worker thread, a `Job` that carries the
//! root rather than a way to look one up, and a coordinator that looks the record up in memory,
//! submits, and answers nothing itself.
//!
//! Status walks still leave the index-stat cache alone (`D30`). Writes — stage, unstage, commit,
//! fetch, pull, push — run on this same thread, so the per-project handle stays un-mutexed: the
//! thread is the lock (`D122`).
//!
//! Two queues on the one thread: overviews ahead of working-tree walks, so the branch name is not
//! stuck behind badges. A second full refresh for a project still walking replaces the queued one
//! rather than lining up behind it. A write is never replaced; it runs, then re-observes.

pub mod graph;
pub mod history;
pub mod nested;
pub mod observe;
pub mod write;

pub use observe::{Observation, WorkingTree, observe};

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::thread;

use git2::Repository;
use ubiq_proto::bus::Mailbox;
use ubiq_proto::git::{GitError, GitPathChange, GitWriteOp};
use ubiq_proto::ids::ProjectId;
use ubiq_proto::messages::Message;

use self::observe::{canonical, observe_repo, open};

/// What one git-family request is, once the coordinator has resolved which project it is for.
pub enum Request {
    /// Refs and HEAD only. No tree walk.
    Overview,
    /// Overview plus the working-tree map.
    Full,
    /// Drop the cached repository. The project's folder moved, or the record is gone.
    Forget,
    /// Branches, tags and stashes. Cheap: refs only, no tree walk.
    Refs { with_tracking: bool },
    /// One page of history.
    Log {
        cursor: Option<String>,
        count: u32,
        rel_path: Option<String>,
        first_parent: bool,
        rev: Option<String>,
    },
    /// Mutate the repository, then re-observe it as a full refresh.
    Write { op: GitWriteOp },
    /// Paths that differ between two revs. `from` absent is the empty tree.
    Changed { from: Option<String>, to: String },
}

/// One request, addressed.
pub struct Job {
    pub project_id: ProjectId,
    /// The record's path, taken from memory on the coordinator's thread.
    pub root: PathBuf,
    /// The repositories inside the project it takes on, from the record beside `root`. Empty is
    /// the ordinary answer: a nested repository is found and named until the user ticks it.
    pub managed_repos: Vec<String>,
    /// The managed nested repository the request is against, as [`ubiq_proto::git::GitNested`]
    /// spells its `rel_path`; empty is the project's own. `Full` and `Forget` ignore it: the
    /// working-tree map is one for the whole project.
    pub repo: String,
    pub request: Request,
    pub reply_to: Mailbox,
}

/// The thread that answers the git family.
pub struct Git {
    jobs: flume::Sender<Job>,
}

impl Git {
    /// Start the worker. It ends when the coordinator that holds this drops it.
    pub fn start() -> Self {
        let (jobs, queue) = flume::unbounded::<Job>();
        thread::Builder::new()
            .name("ubiq-git".to_string())
            .spawn(move || run(queue))
            .expect("the git thread");
        Self { jobs }
    }

    /// Queue a request. Never blocks — the queue is unbounded, on the bus's own rule.
    pub fn submit(&self, job: Job) {
        if self.jobs.send(job).is_err() {
            tracing::error!("the git thread has gone; a request was dropped");
        }
    }
}

struct Cached {
    repo: Repository,
    root: PathBuf,
}

/// Commit-graph lane state for one project's most recent log walk, carried across pages.
///
/// Keyed by the cursor and filters the *next* page must arrive with; a fresh walk (`cursor:
/// None`), or a request that does not match, starts over with empty lanes rather than reusing
/// stale ones.
// ponytail: one cached walk per project, not one per cursor — a second concurrent walk on the
// same project just relays out from lane zero instead of growing this map.
struct LaneCache {
    rel_path: Option<String>,
    first_parent: bool,
    rev: Option<String>,
    expect_cursor: Option<String>,
    lanes: graph::Lanes,
}

/// A project and the repository in it: empty is the project's own, else a managed nested one.
type RepoKey = (ProjectId, String);

#[derive(Default)]
struct State {
    repos: HashMap<RepoKey, Cached>,
    generation: HashMap<ProjectId, u64>,
    lane_cache: HashMap<RepoKey, LaneCache>,
}

/// The lane table to hand `history::log` for this request: the cached one if this request
/// continues the walk it belongs to, otherwise a fresh, empty table.
fn lanes_for(
    state: &mut State,
    key: &RepoKey,
    cursor: &Option<String>,
    rel_path: &Option<String>,
    first_parent: bool,
    rev: &Option<String>,
) -> graph::Lanes {
    // A fresh walk always starts over, cache hit or not.
    if cursor.is_none() {
        return Vec::new();
    }
    match state.lane_cache.get(key) {
        Some(cached)
            if &cached.expect_cursor == cursor
                && &cached.rel_path == rel_path
                && cached.first_parent == first_parent
                && &cached.rev == rev =>
        {
            state.lane_cache.remove(key).unwrap().lanes
        }
        _ => Vec::new(),
    }
}

fn run(queue: flume::Receiver<Job>) {
    let mut cheap = VecDeque::new();
    let mut full_order = VecDeque::new();
    let mut fulls: HashMap<ProjectId, Job> = HashMap::new();
    let mut state = State::default();

    loop {
        if cheap.is_empty() && fulls.is_empty() {
            match queue.recv() {
                Ok(job) => enqueue(&mut cheap, &mut full_order, &mut fulls, job),
                Err(_) => break,
            }
        }
        while let Ok(job) = queue.try_recv() {
            enqueue(&mut cheap, &mut full_order, &mut fulls, job);
        }

        if let Some(job) = cheap.pop_front() {
            answer(&mut state, job);
            continue;
        }

        if let Some(project_id) = full_order.pop_front()
            && let Some(job) = fulls.remove(&project_id)
        {
            answer(&mut state, job);
        }
    }
}

fn enqueue(
    cheap: &mut VecDeque<Job>,
    full_order: &mut VecDeque<ProjectId>,
    fulls: &mut HashMap<ProjectId, Job>,
    job: Job,
) {
    match job.request {
        Request::Overview
        | Request::Forget
        | Request::Refs { .. }
        | Request::Log { .. }
        | Request::Write { .. }
        | Request::Changed { .. } => cheap.push_back(job),
        Request::Full => {
            let project_id = job.project_id;
            if fulls.insert(project_id, job).is_none() {
                full_order.push_back(project_id);
            }
        }
    }
}

fn answer(state: &mut State, mut job: Job) {
    // Taken out so the rest of the job stays whole to borrow while the request's fields move.
    let request = std::mem::replace(&mut job.request, Request::Forget);
    let project_id = job.project_id;
    let repo = job.repo.as_str();
    match request {
        Request::Forget => {
            state.repos.retain(|(held, _), _| *held != project_id);
            state.generation.remove(&project_id);
            state.lane_cache.retain(|(held, _), _| *held != project_id);
        }
        Request::Overview => {
            let generation = state.generation.get(&project_id).copied().unwrap_or(0);
            let message = match observation(state, &job, repo, generation, false) {
                Ok(found) => Message::GitOverview {
                    project_id,
                    repo: repo.to_string(),
                    overview: found.overview,
                },
                Err(error) => git_error(project_id, repo, error),
            };
            job.reply_to.send(message);
        }
        Request::Full => send_full(state, &job),
        Request::Refs { with_tracking } => {
            let message = match ensure_repo(state, &job, repo) {
                Ok(None) => Message::GitRefs {
                    project_id,
                    repo: repo.to_string(),
                    refs: Vec::new(),
                },
                Ok(Some(_)) => {
                    let held = &state.repos[&(project_id, repo.to_string())].repo;
                    match history::refs(held, with_tracking) {
                        Ok(refs) => Message::GitRefs {
                            project_id,
                            repo: repo.to_string(),
                            refs,
                        },
                        Err(error) => git_error(project_id, repo, error),
                    }
                }
                Err(error) => git_error(project_id, repo, error),
            };
            job.reply_to.send(message);
        }
        Request::Log {
            cursor,
            count,
            rel_path,
            first_parent,
            rev,
        } => {
            let message = match ensure_repo(state, &job, repo) {
                Ok(None) => Message::GitLogPage {
                    project_id,
                    repo: repo.to_string(),
                    cursor,
                    commits: Vec::new(),
                    next_cursor: None,
                },
                Ok(Some(root)) => {
                    let rel_path = match inner_path(repo, rel_path) {
                        Ok(rel_path) => rel_path,
                        Err(error) => {
                            job.reply_to.send(git_error(project_id, repo, error));
                            return;
                        }
                    };
                    let key = (project_id, repo.to_string());
                    let mut lanes = lanes_for(state, &key, &cursor, &rel_path, first_parent, &rev);
                    let cached = &state.repos[&key];
                    let scoped_to = match observe::scope(&root, &cached.repo) {
                        Ok(scoped_to) => scoped_to,
                        Err(error) => {
                            job.reply_to.send(git_error(project_id, repo, error));
                            return;
                        }
                    };
                    let from = match (
                        cursor.as_deref(),
                        rev.as_deref().filter(|name| !name.is_empty()),
                    ) {
                        (Some(id), _) => history::LogFrom::Cursor(id),
                        (None, Some(rev)) => history::LogFrom::Rev(rev),
                        (None, None) => history::LogFrom::Head,
                    };
                    match history::log(
                        &cached.repo,
                        &scoped_to,
                        from,
                        count,
                        rel_path.as_deref(),
                        first_parent,
                        &mut lanes,
                    ) {
                        Ok((commits, next_cursor)) => {
                            state.lane_cache.insert(
                                key,
                                LaneCache {
                                    rel_path,
                                    first_parent,
                                    rev: rev.clone(),
                                    expect_cursor: next_cursor.clone(),
                                    lanes,
                                },
                            );
                            Message::GitLogPage {
                                project_id,
                                repo: repo.to_string(),
                                cursor,
                                commits,
                                next_cursor,
                            }
                        }
                        Err(error) => git_error(project_id, repo, error),
                    }
                }
                Err(error) => git_error(project_id, repo, error),
            };
            job.reply_to.send(message);
        }
        Request::Write { ref op } => {
            let root = match ensure_repo(state, &job, repo) {
                Ok(Some(root)) => root,
                Ok(None) => {
                    job.reply_to.send(git_error(
                        project_id,
                        repo,
                        GitError::Failed("the project is not a repository".into()),
                    ));
                    return;
                }
                Err(error) => {
                    job.reply_to.send(git_error(project_id, repo, error));
                    return;
                }
            };
            let written = inner_op(repo, op).and_then(|op| {
                let held = &state.repos[&(project_id, repo.to_string())].repo;
                // A nested repository has nothing nested to route into: its paths are already
                // its own once `inner_op` has taken the prefix off.
                let managed: &[String] = if repo.is_empty() {
                    &job.managed_repos
                } else {
                    &[]
                };
                write::apply(held, &root, managed, &op)
            });
            if let Err(error) = written {
                job.reply_to.send(git_error(project_id, repo, error));
                return;
            }
            // A nested repository's own head moved, and the project's merged map — which the
            // explorer's badges draw from — is read again as after any write.
            if !repo.is_empty() {
                let generation = state.generation.get(&project_id).copied().unwrap_or(0);
                match observation(state, &job, repo, generation, false) {
                    Ok(found) => {
                        job.reply_to.send(Message::GitOverview {
                            project_id,
                            repo: repo.to_string(),
                            overview: found.overview,
                        });
                    }
                    Err(error) => {
                        job.reply_to.send(git_error(project_id, repo, error));
                    }
                }
            }
            send_full(state, &job);
        }
        Request::Changed { from, to } => {
            let message = match ensure_repo(state, &job, repo) {
                Ok(None) => git_error(
                    project_id,
                    repo,
                    GitError::Failed("the project is not a repository".into()),
                ),
                Err(error) => git_error(project_id, repo, error),
                Ok(Some(root)) => {
                    let held = &state.repos[&(project_id, repo.to_string())].repo;
                    match history::changed(held, &root, from.as_deref(), &to) {
                        Ok(mut files) => {
                            if !repo.is_empty() {
                                for file in &mut files {
                                    file.rel_path = format!("{repo}/{}", file.rel_path);
                                    if let GitPathChange::Renamed { from } = &mut file.change {
                                        *from = format!("{repo}/{from}");
                                    }
                                }
                            }
                            Message::GitChanged {
                                project_id,
                                repo: repo.to_string(),
                                from,
                                to,
                                files,
                            }
                        }
                        Err(error) => git_error(project_id, repo, error),
                    }
                }
            };
            job.reply_to.send(message);
        }
    }
}

/// Re-observe the project's own repository with its working tree, and answer both halves.
fn send_full(state: &mut State, job: &Job) {
    let project_id = job.project_id;
    let generation = {
        let held = state.generation.entry(project_id).or_insert(0);
        *held = held.saturating_add(1);
        *held
    };
    match observation(state, job, "", generation, true) {
        Ok(found) => {
            job.reply_to.send(Message::GitOverview {
                project_id,
                repo: String::new(),
                overview: found.overview,
            });
            if let Some(tree) = found.tree {
                job.reply_to.send(Message::GitWorkingTree {
                    project_id,
                    generation,
                    entries: tree.entries,
                    rollups: tree.rollups,
                    repos: tree.repos,
                    truncated: tree.truncated,
                });
            }
        }
        Err(error) => {
            job.reply_to.send(git_error(project_id, "", error));
        }
    }
}

/// `path` as the nested repository `repo` spells it: the `repo/` prefix taken off. The project's
/// own repository (`repo` empty) keeps every path as it is.
fn inner_rel(repo: &str, path: &str) -> Result<String, GitError> {
    if repo.is_empty() {
        return Ok(path.to_string());
    }
    path.strip_prefix(repo)
        .and_then(|rest| rest.strip_prefix('/'))
        .map(str::to_string)
        .ok_or_else(|| GitError::Failed("the path is outside the repository".into()))
}

/// [`inner_rel`] for an optional path, where the repository's own root is no path at all.
fn inner_path(repo: &str, path: Option<String>) -> Result<Option<String>, GitError> {
    match path {
        Some(path) if path != repo => inner_rel(repo, &path).map(Some),
        _ => Ok(None),
    }
}

/// A write with its paths made the nested repository's own.
fn inner_op(repo: &str, op: &GitWriteOp) -> Result<GitWriteOp, GitError> {
    Ok(match op {
        GitWriteOp::Stage { rel_path } if !repo.is_empty() => GitWriteOp::Stage {
            rel_path: inner_rel(repo, rel_path)?,
        },
        GitWriteOp::Unstage { rel_path } if !repo.is_empty() => GitWriteOp::Unstage {
            rel_path: inner_rel(repo, rel_path)?,
        },
        GitWriteOp::Discard { rel_path } if !repo.is_empty() => GitWriteOp::Discard {
            rel_path: inner_rel(repo, rel_path)?,
        },
        GitWriteOp::RestoreFile { rel_path, rev } if !repo.is_empty() => GitWriteOp::RestoreFile {
            rel_path: inner_rel(repo, rel_path)?,
            rev: rev.clone(),
        },
        other => other.clone(),
    })
}

fn observation(
    state: &mut State,
    job: &Job,
    repo: &str,
    generation: u64,
    full: bool,
) -> Result<Observation, GitError> {
    let Some(root) = ensure_repo(state, job, repo)? else {
        // No repository of the project's own is an ordinary answer, and it does not mean there is
        // nothing to draw: a folder holding several independent clones still has badges inside
        // each of them. The downward walk runs either way.
        return Ok(Observation {
            overview: None,
            tree: observe::nested_only(&job.root, full, &job.managed_repos),
        });
    };
    let cached = &state.repos[&(job.project_id, repo.to_string())];
    // A nested repository is observed as itself: no project prefix, and what is below it is its
    // own business.
    let managed: &[String] = if repo.is_empty() {
        &job.managed_repos
    } else {
        &[]
    };
    observe_repo(&root, &cached.repo, generation, full, managed)
}

/// Make sure the repository the request is against is held, and say where its working tree is.
///
/// `repo` empty is the project's own, found by walking upward; `None` means there is not one.
/// Otherwise it must be a repository the project manages — anything else is
/// [`GitError::NotFound`] — and it is opened exactly, never discovered, so a folder that stopped
/// being a repository does not silently answer with the one above it.
fn ensure_repo(state: &mut State, job: &Job, repo: &str) -> Result<Option<PathBuf>, GitError> {
    let root = if repo.is_empty() {
        job.root.clone()
    } else if job.managed_repos.iter().any(|managed| managed == repo) {
        job.root.join(repo)
    } else {
        return Err(GitError::NotFound);
    };
    let key = (job.project_id, repo.to_string());
    let canon = canonical(&root);
    if state.repos.get(&key).is_some_and(|held| held.root == canon) {
        return Ok(Some(root));
    }
    state.repos.remove(&key);
    let found = if repo.is_empty() {
        open(&canon)?
    } else {
        Some(Repository::open(&canon).map_err(observe::map_error)?)
    };
    Ok(found.map(|found| {
        state.repos.insert(
            key,
            Cached {
                repo: found,
                root: canon,
            },
        );
        root
    }))
}

/// One project's failure, addressed so the interface can clear badges rather than freeze them.
pub fn git_error(project_id: ProjectId, repo: &str, error: GitError) -> Message {
    if matches!(error, GitError::Failed(_)) {
        tracing::warn!("git {project_id}: {error}");
    }
    Message::GitError {
        project_id,
        repo: repo.to_string(),
        error,
    }
}
