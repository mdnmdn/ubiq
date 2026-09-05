//! Refs and the commit log. Kept apart from `observe.rs` so that file does not grow a second
//! subject: one is the working tree's present, this is the repository's past.

use std::collections::HashMap;

use git2::{BranchType, DiffOptions, Oid, Sort};
use ubiq_proto::git::{GitCommit, GitError, GitRef, GitRefKind, GitWho};

use super::observe::{map_error, short_id, tracking};

/// A path with no history would walk to the root. The scan is bounded at this many commits and
/// the page simply comes back short; if that reads badly in a history view, the upgrade is a
/// commit-graph bloom filter, not a longer walk.
// ponytail: bounded scan instead of an index, upgrade to a commit-graph filter if it matters.
const PATH_SCAN_CEILING: usize = 5_000;

/// Every ref the sidebar draws. `with_tracking` costs one merge-base walk per local branch.
pub fn refs(repo: &git2::Repository, with_tracking: bool) -> Result<Vec<GitRef>, GitError> {
    let mut out = Vec::new();

    for branch in repo.branches(Some(BranchType::Local)).map_err(map_error)? {
        let (branch, _) = branch.map_err(map_error)?;
        let Some(name) = branch.name().map_err(map_error)? else {
            continue;
        };
        let Some(target) = branch.get().target() else {
            continue;
        };
        let (ahead, behind) = if with_tracking {
            let (_, ahead, behind) = tracking(repo, name);
            (ahead, behind)
        } else {
            (None, None)
        };
        out.push(GitRef {
            name: name.to_string(),
            kind: GitRefKind::Local,
            target: short_id(repo, target),
            current: branch.is_head(),
            ahead,
            behind,
        });
    }

    for branch in repo.branches(Some(BranchType::Remote)).map_err(map_error)? {
        let (branch, _) = branch.map_err(map_error)?;
        let Some(name) = branch.name().map_err(map_error)? else {
            continue;
        };
        let Some(target) = branch.get().target() else {
            continue;
        };
        out.push(GitRef {
            name: name.to_string(),
            kind: GitRefKind::Remote,
            target: short_id(repo, target),
            current: false,
            ahead: None,
            behind: None,
        });
    }

    let tag_names = repo.tag_names(None).map_err(map_error)?;
    for name in tag_names.iter().filter_map(|found| found.ok().flatten()) {
        let Ok(object) = repo.revparse_single(&format!("refs/tags/{name}")) else {
            continue;
        };
        let Ok(commit) = object.peel_to_commit() else {
            continue;
        };
        out.push(GitRef {
            name: name.to_string(),
            kind: GitRefKind::Tag,
            target: short_id(repo, commit.id()),
            current: false,
            ahead: None,
            behind: None,
        });
    }

    match repo.reflog("refs/stash") {
        Ok(reflog) => {
            for (i, entry) in reflog.iter().enumerate() {
                out.push(GitRef {
                    name: format!("stash@{{{i}}}"),
                    kind: GitRefKind::Stash,
                    target: short_id(repo, entry.id_new()),
                    current: false,
                    ahead: None,
                    behind: None,
                });
            }
        }
        Err(error) if error.code() == git2::ErrorCode::NotFound => {}
        Err(error) => return Err(map_error(error)),
    }

    Ok(out)
}

/// One page of history, newest first.
///
/// Returns the page and the id of the commit after it, which is the next cursor.
pub fn log(
    repo: &git2::Repository,
    scoped_to: &str,
    cursor: Option<&str>,
    count: u32,
    rel_path: Option<&str>,
    first_parent: bool,
) -> Result<(Vec<GitCommit>, Option<String>), GitError> {
    let mut walk = repo.revwalk().map_err(map_error)?;
    walk.set_sorting(Sort::TIME | Sort::TOPOLOGICAL)
        .map_err(map_error)?;

    match cursor {
        Some(id) => {
            let oid = Oid::from_str(id).map_err(map_error)?;
            walk.push(oid).map_err(map_error)?;
        }
        None => match walk.push_head() {
            Ok(()) => {}
            // An unborn HEAD (a fresh `git init`, no commit yet) fails here rather than on
            // `head()` — libgit2 reports it as a missing reference, not a distinct code.
            Err(error)
                if error.code() == git2::ErrorCode::UnbornBranch
                    || error.class() == git2::ErrorClass::Reference =>
            {
                return Ok((Vec::new(), None));
            }
            Err(error) => return Err(map_error(error)),
        },
    }

    if first_parent {
        walk.simplify_first_parent().map_err(map_error)?;
    }

    let decorations = decorations_of(repo)?;
    let full_path = rel_path.map(|rel| join_scope(scoped_to, rel));
    let page_size = count.min(ubiq_proto::git::MAX_LOG_PAGE) as usize;

    let mut commits = Vec::with_capacity(page_size);
    let mut next_cursor = None;
    let mut scanned = 0usize;

    for found in walk {
        let oid = found.map_err(map_error)?;
        if commits.len() >= page_size {
            next_cursor = Some(oid.to_string());
            break;
        }

        let commit = repo.find_commit(oid).map_err(map_error)?;

        if let Some(path) = &full_path {
            scanned += 1;
            if scanned > PATH_SCAN_CEILING {
                break;
            }
            if !touches_path(repo, &commit, path)? {
                continue;
            }
        }

        commits.push(to_git_commit(repo, &commit, &decorations));
    }

    Ok((commits, next_cursor))
}

fn join_scope(scoped_to: &str, rel_path: &str) -> String {
    if scoped_to.is_empty() {
        rel_path.to_string()
    } else if rel_path.is_empty() {
        scoped_to.to_string()
    } else {
        format!("{}/{}", scoped_to.trim_end_matches('/'), rel_path)
    }
}

fn touches_path(
    repo: &git2::Repository,
    commit: &git2::Commit<'_>,
    path: &str,
) -> Result<bool, GitError> {
    let tree = commit.tree().map_err(map_error)?;
    let parent_tree = match commit.parent(0) {
        Ok(parent) => Some(parent.tree().map_err(map_error)?),
        Err(_) => None,
    };
    let mut opts = DiffOptions::new();
    opts.pathspec(path);
    let diff = repo
        .diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), Some(&mut opts))
        .map_err(map_error)?;
    Ok(diff.deltas().len() > 0)
}

/// Ref names by the commit they point at, built once per page. `HEAD` and the stash are not
/// decorations any screen draws.
fn decorations_of(repo: &git2::Repository) -> Result<HashMap<Oid, Vec<String>>, GitError> {
    let mut map: HashMap<Oid, Vec<String>> = HashMap::new();
    for found in repo.references().map_err(map_error)? {
        let r = found.map_err(map_error)?;
        let name = r.name().ok();
        if name == Some("HEAD") || name.is_some_and(|n| n.starts_with("refs/stash")) {
            continue;
        }
        let Ok(commit) = r.peel_to_commit() else {
            continue;
        };
        let Some(shorthand) = r.shorthand().ok() else {
            continue;
        };
        map.entry(commit.id())
            .or_default()
            .push(shorthand.to_string());
    }
    Ok(map)
}

fn to_git_commit(
    repo: &git2::Repository,
    commit: &git2::Commit<'_>,
    decorations: &HashMap<Oid, Vec<String>>,
) -> GitCommit {
    let mine = repo
        .signature()
        .ok()
        .and_then(|sig| sig.email().ok().map(str::to_ascii_lowercase))
        .is_some_and(|configured| {
            commit
                .author()
                .email()
                .map(|email| email.to_ascii_lowercase() == configured)
                .unwrap_or(false)
        });
    GitCommit {
        id: commit.id().to_string(),
        short_id: short_id(repo, commit.id()),
        summary: commit
            .summary()
            .ok()
            .flatten()
            .unwrap_or_default()
            .to_string(),
        author: who_of(&commit.author()),
        committer: who_of(&commit.committer()),
        parents: commit.parent_count() as u32,
        refs: decorations.get(&commit.id()).cloned().unwrap_or_default(),
        mine,
    }
}

fn who_of(sig: &git2::Signature<'_>) -> GitWho {
    GitWho {
        name: sig.name().unwrap_or_default().to_string(),
        email: sig.email().unwrap_or_default().to_string(),
        time: sig.when().seconds(),
        offset: sig.when().offset_minutes(),
    }
}
