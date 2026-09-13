//! Mutate a project's repository: stage, unstage, commit, fetch, pull, push.
//!
//! Runs on the git worker's thread, the same one that reads, so the per-project `Repository`
//! handle stays un-mutexed — the thread is the lock (`D122`). Status walks still leave the
//! index-stat cache alone; these paths write the index and the refs the user asked to write.

use std::path::{Component, Path};

use git2::{
    BranchType, ErrorCode, FetchOptions, PushOptions, RemoteCallbacks, Repository, RepositoryState,
    build::CheckoutBuilder,
};
use ubiq_proto::git::{GitError, GitWriteOp};

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
    }
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
    let mut callbacks = RemoteCallbacks::new();
    callbacks.credentials(credentials);
    let mut opts = PushOptions::new();
    opts.remote_callbacks(callbacks);
    remote
        .push(&[&refspec], Some(&mut opts))
        .map_err(map_error)?;
    Ok(())
}

fn fetch_remote(repo: &Repository, name: &str) -> Result<(), GitError> {
    let mut remote = repo.find_remote(name).map_err(map_error)?;
    let mut callbacks = RemoteCallbacks::new();
    callbacks.credentials(credentials);
    let mut opts = FetchOptions::new();
    opts.remote_callbacks(callbacks);
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

fn credentials(
    url: &str,
    username: Option<&str>,
    allowed: git2::CredentialType,
) -> Result<git2::Cred, git2::Error> {
    if allowed.contains(git2::CredentialType::SSH_KEY) {
        let user = username.unwrap_or("git");
        if let Ok(cred) = git2::Cred::ssh_key_from_agent(user) {
            return Ok(cred);
        }
    }
    if allowed.contains(git2::CredentialType::USER_PASS_PLAINTEXT)
        && let Ok(config) = git2::Config::open_default()
        && let Ok(cred) = git2::Cred::credential_helper(&config, url, username)
    {
        return Ok(cred);
    }
    git2::Cred::default()
}
