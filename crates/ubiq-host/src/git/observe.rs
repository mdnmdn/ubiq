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
    AHEAD_BEHIND_CAP, GitCounts, GitEntry, GitError, GitHead, GitMark, GitNested, GitOperation,
    GitPathChange, GitRemote, GitRollup, GitSubmodule, GitSubmoduleState, MAX_WORKING_TREE,
    RepoOverview,
};

use super::nested;

/// What one look at a project found.
pub struct Observation {
    pub overview: Option<RepoOverview>,
    pub tree: Option<WorkingTree>,
}

/// Paths that have something to say, and the directory rollups the explorer cannot derive itself.
///
/// One map for the whole project, however many repositories it holds: a nested repository's paths
/// are prefixed with its own root and merged in, and `repos` is where the boundary is drawn
/// (`D99`).
pub struct WorkingTree {
    pub entries: Vec<GitEntry>,
    pub rollups: Vec<GitRollup>,
    /// The repositories below the project, whose entries are already merged into `entries`.
    pub repos: Vec<GitNested>,
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
///
/// `managed` is the project's answer about the repositories *inside* it — the record's
/// `managed_repos`, as [`GitNested::rel_path`] spells them. Every repository found is named either
/// way; only one on that list is opened.
pub fn observe(
    root: &Path,
    generation: u64,
    full: bool,
    managed: &[String],
) -> Result<Observation, GitError> {
    let Some(repo) = open(root)? else {
        return Ok(Observation {
            overview: None,
            tree: nested_only(root, full, managed),
        });
    };
    observe_repo(root, &repo, generation, full, managed)
}

/// The working tree of a project that has no repository of its own but holds some.
///
/// A folder holding several independent clones is a real project: there is no overview to draw,
/// and the badges inside each clone are still the truth. `None` when nothing was found, which is
/// what an ordinary folder answers.
pub fn nested_only(root: &Path, full: bool, managed: &[String]) -> Option<WorkingTree> {
    if !full {
        return None;
    }
    let found = nested::discover(root);
    if found.roots.is_empty() {
        return None;
    }
    let mut entries = Vec::new();
    let repos = merge_nested(root, &found.roots, &[], managed, &mut entries);
    Some(finish(entries, repos, found.truncated))
}

pub(crate) fn observe_repo(
    root: &Path,
    repo: &Repository,
    generation: u64,
    full: bool,
    managed: &[String],
) -> Result<Observation, GitError> {
    let is_bare = repo.is_bare();
    let scoped_to = scope(root, repo)?;
    let (head, (upstream, ahead, behind)) = head_and_tracking(repo)?;
    let operation = operation(repo.state());

    let submodules = submodules(repo, &scoped_to);

    let (counts, tree) = if full && !is_bare {
        let (mut entries, mut truncated) = status_entries(repo, &scoped_to)?;
        let found = nested::discover(root);
        truncated |= found.truncated;
        drop_nested_roots(&mut entries, &found.roots);
        // The project's own counts are its own repository's: the outer repository's account of
        // itself, with the folders it does not own dropped and nothing nested merged in yet. The
        // status bar reads the branch it names, not the sum of every clone below it.
        let counts = counts_of(&entries);
        let pinned: Vec<&str> = submodules.iter().map(|sm| sm.rel_path.as_str()).collect();
        let repos = merge_nested(root, &found.roots, &pinned, managed, &mut entries);
        (Some(counts), Some(finish(entries, repos, truncated)))
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
        submodules,
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

/// Walk every repository in `roots` the project manages, prefix its paths with its own
/// project-relative root, and merge them into `entries`. Every root is listed either way.
///
/// Each repository is walked with an **empty** scope: the project's prefix is a fact about the
/// *outer* repository and means nothing inside this one. `drop_nested_roots` has already taken the
/// outer repository's own entry off these folders.
///
/// A root not in `managed` is named and nothing else: it is never opened, so it has no HEAD to
/// read, no counts and no entries. That is what the setting means — found, listed for the user to
/// tick, and unread until they do.
///
// ponytail: no handle is cached. `Repository::open` on an exact working-tree root takes no upward
// walk, and the worker's cache is keyed by `ProjectId` alone — a cache holding one handle per
// nested root is a bigger change than this needs. `G226` is the row if the repeat cost is ever
// measured to matter.
fn merge_nested(
    root: &Path,
    roots: &[String],
    pinned: &[&str],
    managed: &[String],
    entries: &mut Vec<GitEntry>,
) -> Vec<GitNested> {
    if roots.is_empty() {
        return Vec::new();
    }
    let mut repos = Vec::with_capacity(roots.len());
    for rel_root in roots {
        let submodule = pinned.contains(&rel_root.as_str());
        if !managed.contains(rel_root) {
            // Nothing is read from it: no `Repository::open`, no status walk, no entries and no
            // rollups. An unborn branch with no name is the wire's shape for "no commit to point
            // at", which is as much as can be said without opening anything. The pin is still the
            // outer repository's own account and holds either way.
            repos.push(GitNested {
                rel_path: rel_root.clone(),
                head: GitHead::Unborn(String::new()),
                submodule,
                counts: None,
                managed: false,
            });
            continue;
        }
        let Ok(repo) = Repository::open(root.join(rel_root)) else {
            // One broken clone must not blank the project's badges: it is reported with no counts
            // and contributes no entries, and every other repository is still walked.
            repos.push(GitNested {
                rel_path: rel_root.clone(),
                // A repository that will not open has no HEAD to name, and an unborn branch with
                // no name is the wire's shape for "no commit to point at".
                head: GitHead::Unborn(String::new()),
                submodule,
                counts: None,
                managed: true,
            });
            continue;
        };
        let head = match head_and_tracking(&repo) {
            Ok((head, _)) => head,
            Err(_) => GitHead::Unborn(String::new()),
        };
        let walked = if repo.is_bare() {
            None
        } else {
            status_entries(&repo, "").ok()
        };
        let Some((inner, _)) = walked else {
            repos.push(GitNested {
                rel_path: rel_root.clone(),
                head,
                submodule,
                counts: None,
                managed: true,
            });
            continue;
        };
        repos.push(GitNested {
            rel_path: rel_root.clone(),
            head,
            submodule,
            counts: Some(counts_of(&inner)),
            managed: true,
        });
        entries.extend(inner.into_iter().map(|mut entry| {
            entry.rel_path = format!("{rel_root}/{}", entry.rel_path);
            entry
        }));
    }
    repos
}

/// Take the outer repository's own account of a nested repository's folder out of `entries`.
///
/// An independent tree is one `Untracked` directory to the outer repository, and the interface
/// pushes an untracked directory's status onto every child — which is why every file in a nested
/// clone read as untracked. The nested repository's own entries and rollup supply that folder's
/// badge instead.
fn drop_nested_roots(entries: &mut Vec<GitEntry>, roots: &[String]) {
    if roots.is_empty() {
        return;
    }
    entries.retain(|entry| !roots.iter().any(|root| at_or_under(&entry.rel_path, root)));
}

/// Whether `path` is `root` itself or something below it.
fn at_or_under(path: &str, root: &str) -> bool {
    path == root
        || path
            .strip_prefix(root)
            .is_some_and(|rest| rest.starts_with('/'))
}

/// Cap the merged entry set, roll it up, and say honestly whether anything was cut.
///
/// The rollups are computed over the *merged* set, so a folder holding a nested repository gets
/// the badge that repository's own changes earn it.
fn finish(mut entries: Vec<GitEntry>, repos: Vec<GitNested>, mut truncated: bool) -> WorkingTree {
    if !repos.is_empty() {
        entries.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
    }
    if entries.len() > MAX_WORKING_TREE {
        entries.truncate(MAX_WORKING_TREE);
        truncated = true;
    }
    let rollups = rollups_of(&entries);
    WorkingTree {
        entries,
        rollups,
        repos,
        truncated,
    }
}

/// One repository's status walk, project-relative. Bounded by [`MAX_WORKING_TREE`] on its own, so
/// no single repository can fill the merged map on its way to being capped again.
fn status_entries(repo: &Repository, scoped_to: &str) -> Result<(Vec<GitEntry>, bool), GitError> {
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

    Ok((entries, truncated))
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
