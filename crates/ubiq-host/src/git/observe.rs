//! Read a repository without writing it.
//!
//! Discovery walks upward from the project's folder, the same walk a diff already takes. Status
//! never refreshes the index: [`git2::StatusOptions::update_index`] stays false, which is `D30`
//! applied to the git directory.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use git2::{
    BranchType, ErrorClass, ErrorCode, Oid, Repository, RepositoryState, Status, StatusOptions,
    SubmoduleIgnore, SubmoduleStatus,
};
use ubiq_proto::files::LIST_HIDE;
use ubiq_proto::git::{
    AHEAD_BEHIND_CAP, GitCounts, GitEntry, GitError, GitHead, GitMark, GitOperation, GitPathChange,
    GitRemote, GitRollup, GitSubmodule, GitSubmoduleState, MAX_WORKING_TREE, RepoOverview,
};

/// What one look at a project found.
pub struct Observation {
    pub overview: Option<RepoOverview>,
    pub tree: Option<WorkingTree>,
}

/// Paths that have something to say, and the directory rollups the explorer cannot derive itself.
pub struct WorkingTree {
    pub entries: Vec<GitEntry>,
    pub rollups: Vec<GitRollup>,
    pub truncated: bool,
}

/// Open the repository that contains `root`, or `None` when there is not one.
pub fn open(root: &Path) -> Result<Option<Repository>, GitError> {
    match Repository::discover(root) {
        Ok(repo) => Ok(Some(repo)),
        Err(error) if error.code() == ErrorCode::NotFound => Ok(None),
        Err(error) => Err(map_error(error)),
    }
}

/// Observe the repository at `root`. `full` walks the working tree; otherwise only refs are read.
pub fn observe(root: &Path, generation: u64, full: bool) -> Result<Observation, GitError> {
    let Some(repo) = open(root)? else {
        return Ok(Observation {
            overview: None,
            tree: None,
        });
    };
    observe_repo(root, &repo, generation, full)
}

pub(crate) fn observe_repo(
    root: &Path,
    repo: &Repository,
    generation: u64,
    full: bool,
) -> Result<Observation, GitError> {
    let is_bare = repo.is_bare();
    let scoped_to = scope(root, repo)?;
    let (head, (upstream, ahead, behind)) = head_and_tracking(repo)?;
    let operation = operation(repo.state());

    let (counts, tree) = if full && !is_bare {
        let tree = working_tree(repo, &scoped_to)?;
        let counts = counts_of(&tree.entries);
        (Some(counts), Some(tree))
    } else {
        (None, None)
    };

    let overview = RepoOverview {
        scoped_to: scoped_to.clone(),
        head,
        upstream,
        ahead,
        behind,
        operation,
        counts,
        is_bare,
        generation,
        remotes: remotes(repo),
        submodules: submodules(repo, &scoped_to),
    };

    Ok(Observation {
        overview: Some(overview),
        tree,
    })
}

/// The project's prefix inside the repository's working tree. Empty when they are the same folder.
pub(crate) fn scope(root: &Path, repo: &Repository) -> Result<String, GitError> {
    let Some(workdir) = repo.workdir() else {
        return Ok(String::new());
    };
    let workdir = fs::canonicalize(workdir).map_err(io_error)?;
    let project = fs::canonicalize(root).map_err(io_error)?;
    if project == workdir {
        return Ok(String::new());
    }
    match project.strip_prefix(&workdir) {
        Ok(prefix) => Ok(to_rel(prefix)),
        Err(_) => Err(GitError::Failed(
            "the project is not inside the repository's working tree".to_string(),
        )),
    }
}

/// The repository's named URLs. `origin` is the default when it exists, else the first named.
fn remotes(repo: &Repository) -> Vec<GitRemote> {
    let Ok(names) = repo.remotes() else {
        return Vec::new();
    };
    let mut remotes: Vec<GitRemote> = names
        .iter()
        .filter_map(|found| found.ok().flatten())
        .filter_map(|name| {
            let url = repo.find_remote(name).ok()?.url().ok()?.to_string();
            Some(GitRemote {
                name: name.to_string(),
                url,
                is_default: false,
            })
        })
        .collect();
    let default_index = remotes
        .iter()
        .position(|remote| remote.name == "origin")
        .or(if remotes.is_empty() { None } else { Some(0) });
    if let Some(index) = default_index {
        remotes[index].is_default = true;
    }
    remotes
}

/// The submodules whose path falls inside the project's scope.
fn submodules(repo: &Repository, scoped_to: &str) -> Vec<GitSubmodule> {
    let Ok(subs) = repo.submodules() else {
        return Vec::new();
    };
    subs.iter()
        .filter_map(|sm| {
            let name = sm.name().ok()?.to_string();
            let url = sm.url().ok().flatten()?.to_string();
            let rel_path = project_rel(&to_rel(sm.path()), scoped_to)?;
            let state = submodule_state(repo, &name);
            Some(GitSubmodule {
                name,
                rel_path,
                url,
                state,
            })
        })
        .collect()
}

fn submodule_state(repo: &Repository, name: &str) -> GitSubmoduleState {
    let Ok(status) = repo.submodule_status(name, SubmoduleIgnore::None) else {
        return GitSubmoduleState::Clean;
    };
    if status.intersects(SubmoduleStatus::WD_UNINITIALIZED) {
        GitSubmoduleState::Uninitialised
    } else if status.intersects(
        SubmoduleStatus::WD_INDEX_MODIFIED
            | SubmoduleStatus::WD_WD_MODIFIED
            | SubmoduleStatus::WD_UNTRACKED
            | SubmoduleStatus::WD_MODIFIED,
    ) {
        GitSubmoduleState::Dirty
    } else {
        GitSubmoduleState::Clean
    }
}

type Tracking = (Option<String>, Option<u32>, Option<u32>);

fn head_and_tracking(repo: &Repository) -> Result<(GitHead, Tracking), GitError> {
    match repo.head() {
        Ok(head) => {
            if head.is_branch() {
                let name = head.shorthand().unwrap_or("HEAD").to_string();
                Ok((GitHead::Branch(name.clone()), tracking(repo, &name)))
            } else {
                let short_id = head
                    .target()
                    .map(|oid| short_id(repo, oid))
                    .unwrap_or_else(|| "HEAD".to_string());
                Ok((GitHead::Detached { short_id }, (None, None, None)))
            }
        }
        Err(error) if error.code() == ErrorCode::UnbornBranch => {
            Ok((GitHead::Unborn(unborn_name(repo)), (None, None, None)))
        }
        Err(error) => Err(map_error(error)),
    }
}

pub(crate) fn tracking(
    repo: &Repository,
    name: &str,
) -> (Option<String>, Option<u32>, Option<u32>) {
    let Ok(branch) = repo.find_branch(name, BranchType::Local) else {
        return (None, None, None);
    };
    let Ok(upstream) = branch.upstream() else {
        return (None, None, None);
    };
    let upstream_name = upstream.name().ok().flatten().map(ToOwned::to_owned);
    let Some(local) = branch.get().target() else {
        return (upstream_name, None, None);
    };
    let Some(remote) = upstream.get().target() else {
        return (upstream_name, None, None);
    };
    match repo.graph_ahead_behind(local, remote) {
        Ok((ahead, behind)) => (
            upstream_name,
            Some((ahead as u32).min(AHEAD_BEHIND_CAP)),
            Some((behind as u32).min(AHEAD_BEHIND_CAP)),
        ),
        Err(_) => (upstream_name, None, None),
    }
}

fn unborn_name(repo: &Repository) -> String {
    let Ok(head) = repo.find_reference("HEAD") else {
        return "HEAD".to_string();
    };
    let Ok(Some(target)) = head.symbolic_target() else {
        return "HEAD".to_string();
    };
    target
        .strip_prefix("refs/heads/")
        .unwrap_or(target)
        .to_string()
}

pub(crate) fn short_id(repo: &Repository, oid: Oid) -> String {
    repo.find_object(oid, None)
        .ok()
        .and_then(|object| object.short_id().ok())
        .and_then(|buf| String::from_utf8(buf.to_vec()).ok())
        .filter(|id| !id.is_empty())
        .unwrap_or_else(|| oid.to_string().chars().take(7).collect())
}

fn operation(state: RepositoryState) -> Option<GitOperation> {
    match state {
        RepositoryState::Clean => None,
        RepositoryState::Merge => Some(GitOperation::Merge),
        RepositoryState::Revert | RepositoryState::RevertSequence => Some(GitOperation::Revert),
        RepositoryState::CherryPick | RepositoryState::CherryPickSequence => {
            Some(GitOperation::CherryPick)
        }
        RepositoryState::Bisect => Some(GitOperation::Bisect),
        RepositoryState::Rebase | RepositoryState::RebaseMerge => Some(GitOperation::Rebase),
        RepositoryState::RebaseInteractive => Some(GitOperation::RebaseInteractive),
        RepositoryState::ApplyMailbox | RepositoryState::ApplyMailboxOrRebase => {
            Some(GitOperation::ApplyMailbox)
        }
    }
}

fn working_tree(repo: &Repository, scoped_to: &str) -> Result<WorkingTree, GitError> {
    let mut opts = StatusOptions::new();
    opts.include_untracked(true)
        .recurse_untracked_dirs(false)
        .exclude_submodules(true)
        .update_index(false)
        // An ignored tree is unbounded — `target/`, `node_modules/`, `.venv/`. libgit2 yields it
        // as one entry when it is told not to recurse, which is what keeps the map proportional
        // to the change set. `project_rel` already strips the trailing slash.
        .include_ignored(true)
        .recurse_ignored_dirs(false);
    if !scoped_to.is_empty() {
        opts.pathspec(scoped_to);
    }

    let statuses = repo.statuses(Some(&mut opts)).map_err(map_error)?;
    let mut entries = Vec::new();
    let mut truncated = false;

    for found in statuses.iter() {
        let Ok(git_path) = found.path() else {
            continue;
        };
        let Some(rel_path) = project_rel(git_path, scoped_to) else {
            continue;
        };
        if hidden(&rel_path) {
            continue;
        }
        if entries.len() >= MAX_WORKING_TREE {
            truncated = true;
            break;
        }

        let status = found.status();
        let index = index_change(status, &found);
        let worktree = worktree_change(status, &found);
        let conflicted = status.is_conflicted();
        let ignored = status.is_ignored();
        if index.is_none() && worktree.is_none() && !conflicted && !ignored {
            continue;
        }
        entries.push(GitEntry {
            rel_path,
            index,
            worktree,
            conflicted,
            ignored,
        });
    }

    let rollups = rollups_of(&entries);
    Ok(WorkingTree {
        entries,
        rollups,
        truncated,
    })
}

fn index_change(status: Status, found: &git2::StatusEntry<'_>) -> Option<GitPathChange> {
    if status.is_index_new() {
        Some(GitPathChange::Added)
    } else if status.is_index_deleted() {
        Some(GitPathChange::Deleted)
    } else if status.is_index_renamed() {
        Some(GitPathChange::Renamed {
            from: rename_from(found.head_to_index()).unwrap_or_default(),
        })
    } else if status.is_index_typechange() {
        Some(GitPathChange::TypeChange)
    } else if status.is_index_modified() {
        Some(GitPathChange::Modified)
    } else {
        None
    }
}

fn worktree_change(status: Status, found: &git2::StatusEntry<'_>) -> Option<GitPathChange> {
    if status.is_wt_new() {
        Some(GitPathChange::Untracked)
    } else if status.is_wt_deleted() {
        Some(GitPathChange::Deleted)
    } else if status.is_wt_renamed() {
        Some(GitPathChange::Renamed {
            from: rename_from(found.index_to_workdir()).unwrap_or_default(),
        })
    } else if status.is_wt_typechange() {
        Some(GitPathChange::TypeChange)
    } else if status.is_wt_modified() {
        Some(GitPathChange::Modified)
    } else {
        None
    }
}

fn rename_from(delta: Option<git2::DiffDelta<'_>>) -> Option<String> {
    delta.and_then(|delta| delta.old_file().path().map(to_rel))
}

pub(crate) fn project_rel(git_path: &str, scoped_to: &str) -> Option<String> {
    // libgit2 names an untracked directory with a trailing slash. The file family never does,
    // and a row whose path is `fresh` would miss a mark named `fresh/`.
    let git_path = git_path.replace('\\', "/");
    let git_path = git_path.trim_end_matches('/');
    if git_path.is_empty() {
        return None;
    }
    if scoped_to.is_empty() {
        return Some(git_path.to_string());
    }
    let prefix = scoped_to.trim_end_matches('/');
    if git_path == prefix {
        return None;
    }
    git_path
        .strip_prefix(prefix)
        .and_then(|rest| rest.strip_prefix('/'))
        .map(ToOwned::to_owned)
}

fn hidden(rel_path: &str) -> bool {
    rel_path
        .rsplit('/')
        .next()
        .is_some_and(|name| LIST_HIDE.contains(&name))
}

fn rollups_of(entries: &[GitEntry]) -> Vec<GitRollup> {
    let mut best: HashMap<String, GitMark> = HashMap::new();
    for entry in entries {
        let Some(mark) = entry.mark() else {
            continue;
        };
        let mut rest = entry.rel_path.as_str();
        while let Some((parent, _)) = rest.rsplit_once('/') {
            if parent.is_empty() {
                break;
            }
            best.entry(parent.to_string())
                .and_modify(|held| {
                    if mark.rank() > held.rank() {
                        *held = mark;
                    }
                })
                .or_insert(mark);
            rest = parent;
        }
    }
    let mut rollups: Vec<GitRollup> = best
        .into_iter()
        .map(|(rel_path, mark)| GitRollup { rel_path, mark })
        .collect();
    rollups.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
    rollups
}

fn counts_of(entries: &[GitEntry]) -> GitCounts {
    let mut counts = GitCounts {
        staged: 0,
        modified: 0,
        untracked: 0,
        conflicted: 0,
    };
    for entry in entries {
        if entry.conflicted {
            counts.conflicted += 1;
        }
        match &entry.worktree {
            Some(GitPathChange::Untracked) => counts.untracked += 1,
            Some(_) => counts.modified += 1,
            None => {}
        }
        if entry.index.is_some() {
            counts.staged += 1;
        }
    }
    counts
}

fn to_rel(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn io_error(error: std::io::Error) -> GitError {
    match error.kind() {
        std::io::ErrorKind::NotFound => GitError::NotFound,
        std::io::ErrorKind::PermissionDenied => GitError::Denied,
        _ => GitError::Failed(error.to_string()),
    }
}

pub(crate) fn map_error(error: git2::Error) -> GitError {
    if error.code() == ErrorCode::NotFound {
        return GitError::NotFound;
    }
    if error.class() == ErrorClass::Os || error.class() == ErrorClass::Filesystem {
        return GitError::Denied;
    }
    let message = error.message();
    if message.contains("corrupt") {
        return GitError::Corrupt;
    }
    if error.class() == ErrorClass::Object || error.class() == ErrorClass::Odb {
        return GitError::Corrupt;
    }
    GitError::Failed(message.to_string())
}

/// Canonical form of a project root, for the worker's cache key.
pub fn canonical(root: &Path) -> PathBuf {
    fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf())
}
