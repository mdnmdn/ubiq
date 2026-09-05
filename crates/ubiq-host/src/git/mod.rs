//! A project's repository as the host observes it: the overview the status bar reads, and the
//! working-tree map the explorer's badges read.
//!
//! **Nothing here runs on the coordinator's thread.** A cold status on a large repository is
//! seconds, and seconds on the coordinator's thread is every pane's keystrokes stalled behind it.
//! The shape [`crate::files::Files`] proved is copied: a worker thread, a `Job` that carries the
//! root rather than a way to look one up, and a coordinator that looks the record up in memory,
//! submits, and answers nothing itself.
//!
//! **Ubiq never writes into a repository.** Status walks with the index-stat refresh turned off.
//! The git directory is inside the project's folder; `D30` covers it.
//!
//! Two queues on the one thread: overviews ahead of working-tree walks, so the branch name is not
//! stuck behind badges. A second full refresh for a project still walking replaces the queued one
//! rather than lining up behind it.

pub mod graph;
pub mod history;
pub mod observe;

pub use observe::{Observation, WorkingTree, observe};

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::thread;

use git2::Repository;
use ubiq_proto::bus::Mailbox;
use ubiq_proto::git::GitError;
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
    },
}

/// One request, addressed.
pub struct Job {
    pub project_id: ProjectId,
    /// The record's path, taken from memory on the coordinator's thread.
    pub root: PathBuf,
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
    expect_cursor: Option<String>,
    lanes: graph::Lanes,
}

#[derive(Default)]
struct State {
    repos: HashMap<ProjectId, Cached>,
    generation: HashMap<ProjectId, u64>,
    lane_cache: HashMap<ProjectId, LaneCache>,
}

/// The lane table to hand `history::log` for this request: the cached one if this request
/// continues the walk it belongs to, otherwise a fresh, empty table.
fn lanes_for(
    state: &mut State,
    project_id: ProjectId,
    cursor: &Option<String>,
    rel_path: &Option<String>,
    first_parent: bool,
) -> graph::Lanes {
    // A fresh walk always starts over, cache hit or not.
    if cursor.is_none() {
        return Vec::new();
    }
    match state.lane_cache.get(&project_id) {
        Some(cached)
            if &cached.expect_cursor == cursor
                && &cached.rel_path == rel_path
                && cached.first_parent == first_parent =>
        {
            state.lane_cache.remove(&project_id).unwrap().lanes
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
        Request::Overview | Request::Forget | Request::Refs { .. } | Request::Log { .. } => {
            cheap.push_back(job)
        }
        Request::Full => {
            let project_id = job.project_id;
            if fulls.insert(project_id, job).is_none() {
                full_order.push_back(project_id);
            }
        }
    }
}

fn answer(state: &mut State, job: Job) {
    match job.request {
        Request::Forget => {
            state.repos.remove(&job.project_id);
            state.generation.remove(&job.project_id);
            state.lane_cache.remove(&job.project_id);
        }
        Request::Overview => {
            let generation = state.generation.get(&job.project_id).copied().unwrap_or(0);
            let message = match observation(state, &job, generation, false) {
                Ok(found) => Message::GitOverview {
                    project_id: job.project_id,
                    overview: found.overview,
                },
                Err(error) => git_error(job.project_id, error),
            };
            job.reply_to.send(message);
        }
        Request::Full => {
            let generation = {
                let held = state.generation.entry(job.project_id).or_insert(0);
                *held = held.saturating_add(1);
                *held
            };
            match observation(state, &job, generation, true) {
                Ok(found) => {
                    job.reply_to.send(Message::GitOverview {
                        project_id: job.project_id,
                        overview: found.overview,
                    });
                    if let Some(tree) = found.tree {
                        job.reply_to.send(Message::GitWorkingTree {
                            project_id: job.project_id,
                            generation,
                            entries: tree.entries,
                            rollups: tree.rollups,
                            truncated: tree.truncated,
                        });
                    }
                }
                Err(error) => {
                    job.reply_to.send(git_error(job.project_id, error));
                }
            }
        }
        Request::Refs { with_tracking } => {
            let message = match ensure_repo(state, job.project_id, &job.root) {
                Ok(false) => Message::GitRefs {
                    project_id: job.project_id,
                    refs: Vec::new(),
                },
                Ok(true) => {
                    let repo = &state
                        .repos
                        .get(&job.project_id)
                        .expect("just inserted or confirmed")
                        .repo;
                    match history::refs(repo, with_tracking) {
                        Ok(refs) => Message::GitRefs {
                            project_id: job.project_id,
                            refs,
                        },
                        Err(error) => git_error(job.project_id, error),
                    }
                }
                Err(error) => git_error(job.project_id, error),
            };
            job.reply_to.send(message);
        }
        Request::Log {
            cursor,
            count,
            rel_path,
            first_parent,
        } => {
            let message = match ensure_repo(state, job.project_id, &job.root) {
                Ok(false) => Message::GitLogPage {
                    project_id: job.project_id,
                    cursor,
                    commits: Vec::new(),
                    next_cursor: None,
                },
                Ok(true) => {
                    let mut lanes =
                        lanes_for(state, job.project_id, &cursor, &rel_path, first_parent);
                    let cached = state
                        .repos
                        .get(&job.project_id)
                        .expect("just inserted or confirmed");
                    let scoped_to = match observe::scope(&job.root, &cached.repo) {
                        Ok(scoped_to) => scoped_to,
                        Err(error) => {
                            job.reply_to.send(git_error(job.project_id, error));
                            return;
                        }
                    };
                    match history::log(
                        &cached.repo,
                        &scoped_to,
                        cursor.as_deref(),
                        count,
                        rel_path.as_deref(),
                        first_parent,
                        &mut lanes,
                    ) {
                        Ok((commits, next_cursor)) => {
                            state.lane_cache.insert(
                                job.project_id,
                                LaneCache {
                                    rel_path: rel_path.clone(),
                                    first_parent,
                                    expect_cursor: next_cursor.clone(),
                                    lanes,
                                },
                            );
                            Message::GitLogPage {
                                project_id: job.project_id,
                                cursor,
                                commits,
                                next_cursor,
                            }
                        }
                        Err(error) => git_error(job.project_id, error),
                    }
                }
                Err(error) => git_error(job.project_id, error),
            };
            job.reply_to.send(message);
        }
    }
}

fn observation(
    state: &mut State,
    job: &Job,
    generation: u64,
    full: bool,
) -> Result<Observation, GitError> {
    if !ensure_repo(state, job.project_id, &job.root)? {
        return Ok(Observation {
            overview: None,
            tree: None,
        });
    }
    let cached = state
        .repos
        .get(&job.project_id)
        .expect("just inserted or confirmed");
    observe_repo(&job.root, &cached.repo, generation, full)
}

fn ensure_repo(
    state: &mut State,
    project_id: ProjectId,
    root: &std::path::Path,
) -> Result<bool, GitError> {
    let root = canonical(root);
    if state
        .repos
        .get(&project_id)
        .is_some_and(|held| held.root == root)
    {
        return Ok(true);
    }
    state.repos.remove(&project_id);
    match open(&root)? {
        None => Ok(false),
        Some(repo) => {
            state.repos.insert(project_id, Cached { repo, root });
            Ok(true)
        }
    }
}

/// One project's failure, addressed so the interface can clear badges rather than freeze them.
pub fn git_error(project_id: ProjectId, error: GitError) -> Message {
    if matches!(error, GitError::Failed(_)) {
        tracing::warn!("git {project_id}: {error}");
    }
    Message::GitError { project_id, error }
}
