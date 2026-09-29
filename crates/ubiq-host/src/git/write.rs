//! Mutate a project's repository: stage, unstage, commit, fetch, pull, push, branch, stash, undo.
//!
//! Runs on the git worker's thread, the same one that reads, so the per-project `Repository`
//! handle stays un-mutexed — the thread is the lock (`D122`). Status walks still leave the
//! index-stat cache alone; these paths write the index and the refs the user asked to write.

use std::path::{Component, Path, PathBuf};

use git2::{
    BranchType, ErrorCode, FetchOptions, IndexAddOption, PushOptions, RemoteCallbacks, Repository,
    RepositoryState, build::CheckoutBuilder,
};
use ubiq_proto::git::{GitError, GitResetMode, GitWriteOp};

use super::observe::map_error;

/// Open the repository at `root` and apply `op`. The worker uses [`apply`] on its cached handle;
/// tests use this so they do not have to name `git2`.
pub fn apply_at(root: &Path, managed: &[String], op: &GitWriteOp) -> Result<(), GitError> {
    let repo = super::observe::open(root)?
        .ok_or_else(|| GitError::Failed("the project is not a repository".into()))?;
    apply(&repo, root, managed, op)
}

/// Apply one write to the project's own repository, or to the nested repository a path belongs
/// to. A successful call has already persisted; the worker then re-observes.
pub fn apply(
    repo: &Repository,
    root: &Path,
    managed: &[String],
    op: &GitWriteOp,
) -> Result<(), GitError> {
    if repo.is_bare() {
        return Err(GitError::Failed(
            "a bare repository has no working tree to write".into(),
        ));
    }
    match op {
        GitWriteOp::Stage { rel_path } => {
            let target = resolve_target(repo, root, managed, rel_path)?;
            stage(target.repo(), target.rel())
        }
        GitWriteOp::Unstage { rel_path } => {
            let target = resolve_target(repo, root, managed, rel_path)?;
            unstage(target.repo(), target.rel())
        }
        GitWriteOp::Commit { message, amend } => {
            refuse_if_busy(repo)?;
            commit(repo, message, *amend)
        }
        GitWriteOp::FetchAll => fetch_all(repo),
        GitWriteOp::Pull => {
            refuse_if_busy(repo)?;
            pull(repo)
        }
        GitWriteOp::Push => {
            refuse_if_busy(repo)?;
            push(repo)
        }
        GitWriteOp::StageAll => {
            let scope = super::observe::scope(root, repo)?;
            let pathspec = if scope.is_empty() {
                "*"
            } else {
                scope.as_str()
            };
            stage_all(repo, pathspec)
        }
        GitWriteOp::UnstageAll => {
            let scope = super::observe::scope(root, repo)?;
            unstage_all(repo, &scope)
        }
        GitWriteOp::CreateBranch { name } => {
            refuse_if_busy(repo)?;
            create_branch(repo, name)
        }
        GitWriteOp::Stash => {
            refuse_if_busy(repo)?;
            stash(repo)
        }
        GitWriteOp::UndoCommit => {
            refuse_if_busy(repo)?;
            undo_commit(repo)
        }
        GitWriteOp::Checkout { rev, force } => {
            refuse_if_busy(repo)?;
            checkout(repo, rev, *force)
        }
        GitWriteOp::Merge { rev } => {
            refuse_if_busy(repo)?;
            merge(repo, rev)
        }
        GitWriteOp::DeleteRef { name, force } => delete_ref(repo, name, *force),
        GitWriteOp::Discard { rel_path } => {
            let target = resolve_target(repo, root, managed, rel_path)?;
            discard(target.repo(), target.rel())
        }
        GitWriteOp::RestoreFile { rel_path, rev } => {
            let target = resolve_target(repo, root, managed, rel_path)?;
            restore_file(target.repo(), target.rel(), rev)
        }
        GitWriteOp::CherryPick { sha } => {
            refuse_if_busy(repo)?;
            cherry_pick(repo, sha)
        }
        GitWriteOp::RevertCommit { sha } => {
            refuse_if_busy(repo)?;
            revert_commit(repo, sha)
        }
        GitWriteOp::Reset { sha, mode } => {
            refuse_if_busy(repo)?;
            reset(repo, sha, mode)
        }
    }
}

/// Resolve `rev` to a commit, however it is spelled — a branch, a tag, a short or full sha.
fn resolve_commit<'a>(repo: &'a Repository, rev: &str) -> Result<git2::Commit<'a>, GitError> {
    let rev = rev.trim();
    if rev.is_empty() {
        return Err(GitError::Failed("no rev was given".into()));
    }
    let object = repo
        .revparse_single(rev)
        .map_err(|_| GitError::Failed(format!("'{rev}' is not a valid rev")))?;
    object
        .peel_to_commit()
        .map_err(|_| GitError::Failed(format!("'{rev}' does not name a commit")))
}

/// Check out `rev`'s tree. A local branch of that name is checked out attached; anything else
/// leaves `HEAD` detached at the commit.
fn checkout(repo: &Repository, rev: &str, force: bool) -> Result<(), GitError> {
    let commit = resolve_commit(repo, rev)?;
    let mut opts = CheckoutBuilder::new();
    if force {
        opts.force();
    } else {
        opts.safe();
    }
    repo.checkout_tree(commit.as_object(), Some(&mut opts))
        .map_err(map_error)?;
    match repo.find_branch(rev.trim(), BranchType::Local) {
        Ok(branch) => {
            let refname = branch
                .get()
                .name()
                .map_err(|_| GitError::Failed("the branch name is not utf-8".into()))?
                .to_string();
            repo.set_head(&refname).map_err(map_error)?;
        }
        Err(_) => {
            repo.set_head_detached(commit.id()).map_err(map_error)?;
        }
    }
    Ok(())
}

/// Merge `rev` into the current branch: fast-forward when possible, else a real merge with the
/// working tree updated. Left with unmerged index entries and `GitOperation::Merge` showing on the
/// overview when there are conflicts to resolve — `Commit` finishes it.
fn merge(repo: &Repository, rev: &str) -> Result<(), GitError> {
    let commit = resolve_commit(repo, rev)?;
    let annotated = repo.find_annotated_commit(commit.id()).map_err(map_error)?;
    let (analysis, _) = repo.merge_analysis(&[&annotated]).map_err(map_error)?;
    if analysis.is_up_to_date() {
        return Err(GitError::Failed("already up to date".into()));
    }

    let head = repo.head().map_err(map_error)?;
    if !head.is_branch() {
        return Err(GitError::Failed(
            "detached HEAD cannot be merged onto".into(),
        ));
    }
    let refname = head
        .name()
        .map_err(|_| GitError::Failed("the branch name is not utf-8".into()))?
        .to_string();
    let head_commit = head.peel_to_commit().map_err(map_error)?;

    if analysis.is_fast_forward() {
        let mut opts = CheckoutBuilder::new();
        opts.safe();
        repo.checkout_tree(commit.as_object(), Some(&mut opts))
            .map_err(map_error)?;
        let mut reference = repo.find_reference(&refname).map_err(map_error)?;
        reference
            .set_target(commit.id(), "fast-forward merge")
            .map_err(map_error)?;
        repo.set_head(&refname).map_err(map_error)?;
        return Ok(());
    }

    repo.merge(&[&annotated], None, None).map_err(map_error)?;
    let mut index = repo.index().map_err(map_error)?;
    if index.has_conflicts() {
        return Err(GitError::Failed(
            "the merge has conflicts to resolve".into(),
        ));
    }
    let tree_id = index.write_tree().map_err(map_error)?;
    let tree = repo.find_tree(tree_id).map_err(map_error)?;
    let sig = repo
        .signature()
        .map_err(|_| GitError::Failed("user.name and user.email are not set".into()))?;
    let message = format!(
        "Merge {} into {}",
        rev.trim(),
        head.shorthand().unwrap_or("HEAD")
    );
    repo.commit(
        Some("HEAD"),
        &sig,
        &sig,
        &message,
        &tree,
        &[&head_commit, &commit],
    )
    .map_err(map_error)?;
    repo.cleanup_state().map_err(map_error)?;
    Ok(())
}

/// Delete a local branch or a tag, `name` spelled as the refs panel does. A branch that is not
/// merged into `HEAD` is refused unless `force`; the current branch is refused outright.
fn delete_ref(repo: &Repository, name: &str, force: bool) -> Result<(), GitError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(GitError::Failed("a ref needs a name".into()));
    }
    if let Ok(mut branch) = repo.find_branch(name, BranchType::Local) {
        if branch.is_head() {
            return Err(GitError::Failed(format!("'{name}' is the current branch")));
        }
        if !force {
            let tip = branch.get().peel_to_commit().map_err(map_error)?.id();
            let head = repo
                .head()
                .and_then(|head| head.peel_to_commit())
                .map_err(map_error)?
                .id();
            let merged = tip == head || repo.graph_descendant_of(head, tip).map_err(map_error)?;
            if !merged {
                return Err(GitError::Failed(format!(
                    "branch '{name}' is not fully merged"
                )));
            }
        }
        branch.delete().map_err(map_error)?;
        return Ok(());
    }
    match repo.tag_delete(name) {
        Ok(()) => Ok(()),
        Err(error) if error.code() == ErrorCode::NotFound => Err(GitError::Failed(format!(
            "'{name}' is not a local branch or a tag"
        ))),
        Err(error) => Err(map_error(error)),
    }
}

/// Restore `rel` to its `HEAD` content, index and worktree together. A path `HEAD` does not carry
/// — untracked, or the repository has no commit yet — is simply removed.
fn discard(repo: &Repository, rel: &str) -> Result<(), GitError> {
    let path = Path::new(rel);
    let head_commit = match repo.head() {
        Ok(head) => Some(head.peel_to_commit().map_err(map_error)?),
        Err(error)
            if error.code() == ErrorCode::UnbornBranch || error.code() == ErrorCode::NotFound =>
        {
            None
        }
        Err(error) => return Err(map_error(error)),
    };
    let in_head = match &head_commit {
        Some(commit) => commit.tree().map_err(map_error)?.get_path(path).is_ok(),
        None => false,
    };
    if let Some(commit) = &head_commit
        && in_head
    {
        let mut opts = CheckoutBuilder::new();
        opts.force();
        opts.path(path);
        repo.checkout_tree(commit.as_object(), Some(&mut opts))
            .map_err(map_error)?;
        repo.reset_default(Some(commit.as_object()), [path])
            .map_err(map_error)?;
        return Ok(());
    }
    let mut index = repo.index().map_err(map_error)?;
    let _ = index.remove_path(path);
    index.write().map_err(map_error)?;
    if let Some(workdir) = repo.workdir() {
        let full = workdir.join(path);
        // `symlink_metadata` does not follow a final symlink component, so a path that is itself
        // a symlink is unlinked as one — never recursed into, whatever it points at.
        match std::fs::symlink_metadata(&full) {
            Ok(meta) if meta.is_dir() => {
                std::fs::remove_dir_all(&full)
                    .map_err(|error| GitError::Failed(error.to_string()))?;
            }
            Ok(_) => {
                std::fs::remove_file(&full).map_err(|error| GitError::Failed(error.to_string()))?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(GitError::Failed(error.to_string())),
        }
    }
    Ok(())
}

/// Restore `rel` to its content at `rev`, writing the worktree only — the index is left as it is,
/// so the change reads as an ordinary uncommitted edit. Goes through `checkout_tree`, forced and
/// scoped to this one path, so the entry's filemode (a symlink, the executable bit) is honoured
/// the same way `discard` honours it, rather than hand-writing bytes with the wrong mode.
fn restore_file(repo: &Repository, rel: &str, rev: &str) -> Result<(), GitError> {
    let path = Path::new(rel);
    let commit = resolve_commit(repo, rev)?;
    let tree = commit.tree().map_err(map_error)?;
    let entry = tree
        .get_path(path)
        .map_err(|_| GitError::Failed(format!("'{rel}' does not exist at {}", rev.trim())))?;
    if entry.kind() != Some(git2::ObjectType::Blob) {
        return Err(GitError::Failed(format!(
            "'{rel}' at {} is not a file",
            rev.trim()
        )));
    }
    let mut opts = CheckoutBuilder::new();
    opts.force();
    opts.path(path);
    // `checkout_tree` updates the index to match the checked-out paths by default; turn that off
    // so this reads as an ordinary uncommitted worktree edit, same as `restore_file`'s contract.
    opts.update_index(false);
    repo.checkout_tree(commit.as_object(), Some(&mut opts))
        .map_err(map_error)?;
    Ok(())
}

/// Apply a commit's diff on top of `HEAD` as a new commit, keeping the original's author and
/// message. Left with unmerged index entries when there are conflicts to resolve.
fn cherry_pick(repo: &Repository, sha: &str) -> Result<(), GitError> {
    let commit = resolve_commit(repo, sha)?;
    repo.cherrypick(&commit, None).map_err(map_error)?;
    let mut index = repo.index().map_err(map_error)?;
    if index.has_conflicts() {
        return Err(GitError::Failed(
            "the cherry-pick has conflicts to resolve".into(),
        ));
    }
    let tree_id = index.write_tree().map_err(map_error)?;
    let tree = repo.find_tree(tree_id).map_err(map_error)?;
    let head_commit = repo
        .head()
        .and_then(|head| head.peel_to_commit())
        .map_err(map_error)?;
    let author = commit.author();
    let committer = repo
        .signature()
        .map_err(|_| GitError::Failed("user.name and user.email are not set".into()))?;
    let message = commit.message().unwrap_or_default();
    repo.commit(
        Some("HEAD"),
        &author,
        &committer,
        message,
        &tree,
        &[&head_commit],
    )
    .map_err(map_error)?;
    repo.cleanup_state().map_err(map_error)?;
    Ok(())
}

/// Create a commit that undoes `sha`'s changes. Left with unmerged index entries when there are
/// conflicts to resolve.
fn revert_commit(repo: &Repository, sha: &str) -> Result<(), GitError> {
    let commit = resolve_commit(repo, sha)?;
    repo.revert(&commit, None).map_err(map_error)?;
    let mut index = repo.index().map_err(map_error)?;
    if index.has_conflicts() {
        return Err(GitError::Failed(
            "the revert has conflicts to resolve".into(),
        ));
    }
    let tree_id = index.write_tree().map_err(map_error)?;
    let tree = repo.find_tree(tree_id).map_err(map_error)?;
    let head_commit = repo
        .head()
        .and_then(|head| head.peel_to_commit())
        .map_err(map_error)?;
    let sig = repo
        .signature()
        .map_err(|_| GitError::Failed("user.name and user.email are not set".into()))?;
    let summary = commit.summary().ok().flatten().unwrap_or_default();
    let message = format!(
        "Revert \"{summary}\"\n\nThis reverts commit {}.\n",
        commit.id()
    );
    repo.commit(Some("HEAD"), &sig, &sig, &message, &tree, &[&head_commit])
        .map_err(map_error)?;
    repo.cleanup_state().map_err(map_error)?;
    Ok(())
}

/// Move `HEAD` to `sha`. `mode` decides how far the index and the worktree follow it.
fn reset(repo: &Repository, sha: &str, mode: &GitResetMode) -> Result<(), GitError> {
    let commit = resolve_commit(repo, sha)?;
    let reset_type = match mode {
        GitResetMode::Soft => git2::ResetType::Soft,
        GitResetMode::Mixed => git2::ResetType::Mixed,
        GitResetMode::Hard => git2::ResetType::Hard,
    };
    repo.reset(commit.as_object(), reset_type, None)
        .map_err(map_error)?;
    Ok(())
}

/// A branch at `HEAD`, checked out. The name is validated the way `git branch` does, and an
/// existing branch is refused rather than moved.
fn create_branch(repo: &Repository, name: &str) -> Result<(), GitError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(GitError::Failed("a branch needs a name".into()));
    }
    if !git2::Branch::name_is_valid(name).map_err(map_error)? {
        return Err(GitError::Failed(format!(
            "'{name}' is not a valid branch name"
        )));
    }
    let head = repo
        .head()
        .and_then(|head| head.peel_to_commit())
        .map_err(|_| GitError::Failed("there is no commit to branch from".into()))?;
    let branch = repo
        .branch(name, &head, false)
        .map_err(|error| match error.code() {
            ErrorCode::Exists => GitError::Failed(format!("branch '{name}' already exists")),
            _ => map_error(error),
        })?;
    let refname = branch
        .get()
        .name()
        .map_err(|_| GitError::Failed("the branch name is not utf-8".into()))?
        .to_string();
    // The new branch points at the commit HEAD already names, so the tree is the one on disk.
    repo.set_head(&refname).map_err(map_error)?;
    Ok(())
}

/// Stash the uncommitted changes, untracked files with them. libgit2 wants `&mut Repository`, which
/// the worker's cached handle is not lent as, so this opens its own on the same git directory.
fn stash(repo: &Repository) -> Result<(), GitError> {
    let sig = repo
        .signature()
        .map_err(|_| GitError::Failed("user.name and user.email are not set".into()))?;
    let mut owned = Repository::open(repo.path()).map_err(map_error)?;
    owned
        .stash_save2(&sig, None, Some(git2::StashFlags::INCLUDE_UNTRACKED))
        .map_err(|error| match error.code() {
            ErrorCode::NotFound => GitError::Failed("nothing to stash".into()),
            _ => map_error(error),
        })?;
    Ok(())
}

/// Soft reset to the first parent: the commit goes, its changes stay in the index.
fn undo_commit(repo: &Repository) -> Result<(), GitError> {
    let head = repo
        .head()
        .and_then(|head| head.peel_to_commit())
        .map_err(|_| GitError::Failed("there is no commit to undo".into()))?;
    let parent = head
        .parent(0)
        .map_err(|_| GitError::Failed("the first commit cannot be undone".into()))?;
    repo.reset(parent.as_object(), git2::ResetType::Soft, None)
        .map_err(map_error)?;
    Ok(())
}

enum Target<'a> {
    Cached(&'a Repository, String),
    Nested(Repository, String),
}

impl Target<'_> {
    fn repo(&self) -> &Repository {
        match self {
            Target::Cached(repo, _) => repo,
            Target::Nested(repo, _) => repo,
        }
    }

    fn rel(&self) -> &str {
        match self {
            Target::Cached(_, rel) | Target::Nested(_, rel) => rel,
        }
    }
}

fn resolve_target<'a>(
    repo: &'a Repository,
    root: &Path,
    managed: &[String],
    rel_path: &str,
) -> Result<Target<'a>, GitError> {
    let rel_path = check_rel(rel_path)?;
    if let Some(prefix) = longest_managed(managed, rel_path) {
        let remainder = rel_path[prefix.len()..].trim_start_matches('/');
        if remainder.is_empty() {
            return Err(GitError::Failed("a repository root is not a file".into()));
        }
        let nested = Repository::open(root.join(prefix)).map_err(map_error)?;
        return Ok(Target::Nested(nested, remainder.to_string()));
    }
    let scope = super::observe::scope(root, repo)?;
    let repo_rel = if scope.is_empty() {
        rel_path.to_string()
    } else {
        format!("{scope}/{rel_path}")
    };
    Ok(Target::Cached(repo, repo_rel))
}

fn longest_managed<'a>(managed: &'a [String], rel_path: &str) -> Option<&'a str> {
    managed
        .iter()
        .filter(|nested| rel_path == nested.as_str() || rel_path.starts_with(&format!("{nested}/")))
        .max_by_key(|nested| nested.len())
        .map(String::as_str)
}

fn check_rel(rel_path: &str) -> Result<&str, GitError> {
    let rel_path = rel_path.trim();
    if rel_path.is_empty() {
        return Err(GitError::Failed("a write names no path".into()));
    }
    if rel_path.contains('\0') {
        return Err(GitError::Failed("a path holds a NUL byte".into()));
    }
    for component in Path::new(rel_path).components() {
        match component {
            Component::Normal(_) | Component::CurDir => {}
            _ => {
                return Err(GitError::Failed("a path is not project-relative".into()));
            }
        }
    }
    Ok(rel_path)
}

fn refuse_if_busy(repo: &Repository) -> Result<(), GitError> {
    if repo.state() != RepositoryState::Clean {
        return Err(GitError::Failed(
            "the repository has an operation in progress".into(),
        ));
    }
    Ok(())
}

fn stage(repo: &Repository, rel: &str) -> Result<(), GitError> {
    let path = Path::new(rel);
    let mut index = repo.index().map_err(map_error)?;
    let on_disk = repo
        .workdir()
        .map(|workdir| workdir.join(path).exists())
        .unwrap_or(false);
    if on_disk {
        index.add_path(path).map_err(map_error)?;
    } else {
        index.remove_path(path).map_err(map_error)?;
    }
    index.write().map_err(map_error)?;
    Ok(())
}

fn stage_all(repo: &Repository, pathspec: &str) -> Result<(), GitError> {
    let mut index = repo.index().map_err(map_error)?;
    index
        .add_all([pathspec], IndexAddOption::DEFAULT, None)
        .map_err(map_error)?;
    index.update_all([pathspec], None).map_err(map_error)?;
    index.write().map_err(map_error)?;
    Ok(())
}

/// `scope` is a project-relative directory (no wildcard, as [`super::observe::scope`] returns
/// it), or empty for the whole repository.
///
/// `reset_default` diffs the commit tree against the index through libgit2's pathspec-*prefix*
/// machinery, which treats a spec with no wildcard character as a literal string used to bound the
/// tree iterator — `"."` for the whole repo, or a bare directory name like `"pkg"` for a scoped
/// project, both match nothing there (`git_pathspec_prefix` needs an exact path match, not a
/// directory prefix) and `reset_default` silently deltas zero entries. A glob (`"*"` whole-repo,
/// `"{scope}/*"` scoped) is not a literal spec, so the prefix bound falls back to everything before
/// the wildcard and the real match happens in `fnmatch`, which does treat it as a directory.
fn unstage_all(repo: &Repository, scope: &str) -> Result<(), GitError> {
    let reset_pathspec = if scope.is_empty() {
        "*".to_string()
    } else {
        format!("{scope}/*")
    };
    match repo.head() {
        Ok(head) => {
            let commit = head.peel_to_commit().map_err(map_error)?;
            repo.reset_default(Some(commit.as_object()), [reset_pathspec.as_str()])
                .map_err(map_error)?;
        }
        Err(error)
            if error.code() == ErrorCode::UnbornBranch || error.code() == ErrorCode::NotFound =>
        {
            let mut index = repo.index().map_err(map_error)?;
            if scope.is_empty() {
                index.clear().map_err(map_error)?;
            } else {
                let prefix = scope.trim_end_matches('/');
                let victims: Vec<PathBuf> = index
                    .iter()
                    .filter_map(|entry| {
                        let path = std::str::from_utf8(&entry.path).ok()?;
                        if path == prefix || path.starts_with(&format!("{prefix}/")) {
                            Some(PathBuf::from(path))
                        } else {
                            None
                        }
                    })
                    .collect();
                for path in victims {
                    let _ = index.remove_path(&path);
                }
            }
            index.write().map_err(map_error)?;
        }
        Err(error) => return Err(map_error(error)),
    }
    Ok(())
}

fn unstage(repo: &Repository, rel: &str) -> Result<(), GitError> {
    let path = Path::new(rel);
    match repo.head() {
        Ok(head) => {
            let commit = head.peel_to_commit().map_err(map_error)?;
            repo.reset_default(Some(commit.as_object()), [path])
                .map_err(map_error)?;
        }
        Err(error)
            if error.code() == ErrorCode::UnbornBranch || error.code() == ErrorCode::NotFound =>
        {
            let mut index = repo.index().map_err(map_error)?;
            let _ = index.remove_path(path);
            index.write().map_err(map_error)?;
        }
        Err(error) => return Err(map_error(error)),
    }
    Ok(())
}

fn commit(repo: &Repository, message: &str, amend: bool) -> Result<(), GitError> {
    let message = message.trim();
    if message.is_empty() {
        return Err(GitError::Failed("a commit needs a message".into()));
    }
    let sig = repo
        .signature()
        .map_err(|_| GitError::Failed("user.name and user.email are not set".into()))?;
    let mut index = repo.index().map_err(map_error)?;
    let tree_id = index.write_tree().map_err(map_error)?;
    let tree = repo.find_tree(tree_id).map_err(map_error)?;

    let head = match repo.head() {
        Ok(head) => Some(head.peel_to_commit().map_err(map_error)?),
        Err(error)
            if error.code() == ErrorCode::UnbornBranch || error.code() == ErrorCode::NotFound =>
        {
            None
        }
        Err(error) => return Err(map_error(error)),
    };

    if amend {
        let Some(current) = head else {
            return Err(GitError::Failed("nothing to amend".into()));
        };
        let parents: Vec<_> = current.parents().collect();
        let parent_refs: Vec<&git2::Commit> = parents.iter().collect();
        repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &parent_refs)
            .map_err(map_error)?;
        return Ok(());
    }

    if let Some(current) = &head
        && current.tree_id() == tree_id
    {
        return Err(GitError::Failed("nothing to commit".into()));
    }

    let parents: Vec<git2::Commit<'_>> = head.into_iter().collect();
    let parent_refs: Vec<&git2::Commit> = parents.iter().collect();
    repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &parent_refs)
        .map_err(map_error)?;
    Ok(())
}

fn fetch_all(repo: &Repository) -> Result<(), GitError> {
    let names = remote_names(repo)?;
    if names.is_empty() {
        return Err(GitError::Failed("no remotes to fetch".into()));
    }
    for name in &names {
        fetch_remote(repo, name)?;
    }
    Ok(())
}

fn pull(repo: &Repository) -> Result<(), GitError> {
    let head = repo.head().map_err(map_error)?;
    if !head.is_branch() {
        return Err(GitError::Failed("detached HEAD cannot be pulled".into()));
    }
    let branch_name = head.shorthand().unwrap_or("HEAD").to_string();
    let refname = format!("refs/heads/{branch_name}");
    let remote_name = repo
        .branch_upstream_remote(&refname)
        .map_err(|_| GitError::Failed("no upstream to pull from".into()))?;
    let remote_name = remote_name
        .as_str()
        .map_err(|_| GitError::Failed("upstream remote is not utf-8".into()))?
        .to_string();

    fetch_remote(repo, &remote_name)?;

    let branch = repo
        .find_branch(&branch_name, BranchType::Local)
        .map_err(map_error)?;
    let upstream = branch
        .upstream()
        .map_err(|_| GitError::Failed("no upstream to pull from".into()))?;
    let upstream_id = upstream.get().peel_to_commit().map_err(map_error)?.id();
    let annotated = repo.find_annotated_commit(upstream_id).map_err(map_error)?;
    let (analysis, _) = repo.merge_analysis(&[&annotated]).map_err(map_error)?;

    if analysis.is_up_to_date() {
        return Ok(());
    }
    if !analysis.is_fast_forward() {
        return Err(GitError::Failed("not a fast-forward".into()));
    }

    // Checkout the incoming tree while HEAD still names the old commit, so a safe
    // checkout sees a clean worktree. Moving HEAD first makes those same files look
    // locally modified and the checkout refuses them.
    let commit = repo.find_commit(upstream_id).map_err(map_error)?;
    {
        let mut opts = CheckoutBuilder::new();
        opts.safe();
        repo.checkout_tree(commit.as_object(), Some(&mut opts))
            .map_err(map_error)?;
    }
    let mut reference = repo.find_reference(&refname).map_err(map_error)?;
    reference
        .set_target(upstream_id, "fast-forward")
        .map_err(map_error)?;
    repo.set_head(&refname).map_err(map_error)?;
    Ok(())
}

fn push(repo: &Repository) -> Result<(), GitError> {
    let head = repo.head().map_err(map_error)?;
    if !head.is_branch() {
        return Err(GitError::Failed("detached HEAD cannot be pushed".into()));
    }
    let branch_name = head.shorthand().unwrap_or("HEAD").to_string();
    let local_ref = format!("refs/heads/{branch_name}");
    let (remote_name, dest) = match repo.branch_upstream_remote(&local_ref) {
        Ok(remote) => {
            let remote_name = remote
                .as_str()
                .map_err(|_| GitError::Failed("upstream remote is not utf-8".into()))?
                .to_string();
            let dest = match repo.branch_upstream_name(&local_ref) {
                Ok(name) => name
                    .as_str()
                    .ok()
                    .and_then(|upstream| {
                        upstream
                            .strip_prefix(&format!("refs/remotes/{remote_name}/"))
                            .map(|short| format!("refs/heads/{short}"))
                    })
                    .unwrap_or_else(|| local_ref.clone()),
                Err(_) => local_ref.clone(),
            };
            (remote_name, dest)
        }
        Err(_) => {
            let names = remote_names(repo)?;
            let remote_name = default_remote(&names)
                .ok_or_else(|| GitError::Failed("no remote to push to".into()))?
                .to_string();
            (remote_name, local_ref.clone())
        }
    };

    let mut remote = repo.find_remote(&remote_name).map_err(map_error)?;
    let refspec = format!("{local_ref}:{dest}");
    let mut opts = PushOptions::new();
    opts.remote_callbacks(remote_auth());
    remote
        .push(&[&refspec], Some(&mut opts))
        .map_err(map_error)?;
    Ok(())
}

fn fetch_remote(repo: &Repository, name: &str) -> Result<(), GitError> {
    let mut remote = repo.find_remote(name).map_err(map_error)?;
    let mut opts = FetchOptions::new();
    opts.remote_callbacks(remote_auth());
    remote
        .fetch(&[] as &[&str], Some(&mut opts), None)
        .map_err(map_error)?;
    Ok(())
}

fn remote_names(repo: &Repository) -> Result<Vec<String>, GitError> {
    let names = repo.remotes().map_err(map_error)?;
    Ok(names
        .iter()
        .filter_map(|found| found.ok().flatten().map(str::to_string))
        .collect())
}

fn default_remote(names: &[String]) -> Option<&str> {
    names
        .iter()
        .find(|name| *name == "origin")
        .or_else(|| names.first())
        .map(String::as_str)
}

fn remote_auth() -> RemoteCallbacks<'static> {
    let mut auth = Auth::default();
    let mut callbacks = RemoteCallbacks::new();
    callbacks.credentials(move |url, username, allowed| auth.cred(url, username, allowed));
    callbacks
}

#[derive(Default)]
struct Auth {
    tried_agent: bool,
    tried_key: usize,
}

const SSH_KEYS: &[&str] = &["id_ed25519", "id_rsa", "id_ecdsa"];

impl Auth {
    fn cred(
        &mut self,
        url: &str,
        username: Option<&str>,
        allowed: git2::CredentialType,
    ) -> Result<git2::Cred, git2::Error> {
        let user = username.unwrap_or("git");

        // libgit2 asks for a username before any key when the URL did not carry one.
        if allowed.contains(git2::CredentialType::USERNAME) {
            return git2::Cred::username(user);
        }

        if allowed.contains(git2::CredentialType::SSH_KEY) {
            if !self.tried_agent {
                self.tried_agent = true;
                if let Ok(cred) = git2::Cred::ssh_key_from_agent(user) {
                    return Ok(cred);
                }
            }
            if let Some(home) = home_dir() {
                let ssh = home.join(".ssh");
                while self.tried_key < SSH_KEYS.len() {
                    let name = SSH_KEYS[self.tried_key];
                    self.tried_key += 1;
                    let private = ssh.join(name);
                    if !private.is_file() {
                        continue;
                    }
                    if let Ok(cred) = git2::Cred::ssh_key(user, None, &private, None) {
                        return Ok(cred);
                    }
                }
            }
        }

        if allowed.contains(git2::CredentialType::USER_PASS_PLAINTEXT)
            && let Ok(config) = git2::Config::open_default()
            && let Ok(cred) = git2::Cred::credential_helper(&config, url, username)
        {
            return Ok(cred);
        }

        if allowed.contains(git2::CredentialType::DEFAULT) {
            return git2::Cred::default();
        }

        Err(git2::Error::from_str("authentication failed"))
    }
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    #[test]
    fn libgit2_links_ssh() {
        assert!(
            git2::Version::get().ssh(),
            "git@host:path remotes need libgit2's ssh transport"
        );
    }
}
