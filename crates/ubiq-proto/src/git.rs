//! What version control is on the wire: a project's repository as the host has observed it.
//!
//! A repository is a fact about a project, discovered by the host. The interface holds no
//! repository identity of its own, and no absolute path crosses — a repository root above the
//! project, or a prefix inside one, is a relative string. That is the file family's rule extended
//! to the family that would most naturally break it.
//!
//! **Not a repository is an ordinary answer**, not a failure: [`crate::messages::Message::GitOverview`]
//! carries `overview: None`, and the interface draws no branch and no badges. [`GitError`] is for
//! a repository that exists and could not be read.
//!
//! Nothing here touches disk. The `git2` types stay in the host and are converted at the worker's
//! edge.

use serde::{Deserialize, Serialize};

/// How far ahead or behind is walked. Past it the number is this, and the interface draws `99+`.
pub const AHEAD_BEHIND_CAP: u32 = 99;

/// How many changed paths one working-tree map may carry. Past it the map is `truncated`.
pub const MAX_WORKING_TREE: usize = 2_000;

/// What `HEAD` is pointing at.
///
/// A fresh `git init` has a branch name and no commit. Drawing that as detached would be wrong.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum GitHead {
    Branch(String),
    Detached { short_id: String },
    Unborn(String),
}

/// An in-progress operation on the repository, when there is one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum GitOperation {
    Merge,
    Rebase,
    RebaseInteractive,
    CherryPick,
    Revert,
    Bisect,
    ApplyMailbox,
}

/// Working-tree totals for the project's scope. Absent on the overview until a walk has run, and
/// absent on a bare or unborn repository rather than zero.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitCounts {
    pub staged: u32,
    pub modified: u32,
    pub untracked: u32,
    pub conflicted: u32,
}

/// What the status bar and the titlebar read. Everything here comes from refs and a handful of
/// files in the git directory, except `counts`, which arrive with the working-tree walk.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoOverview {
    /// The project's prefix inside the repository. Empty when the project *is* the root.
    pub scoped_to: String,
    pub head: GitHead,
    /// The remote-tracking ref the branch is configured against. Absent when there is none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upstream: Option<String>,
    /// Commits ahead of `upstream`. Absent, not zero, when there is no upstream. Capped at
    /// [`AHEAD_BEHIND_CAP`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ahead: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub behind: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation: Option<GitOperation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub counts: Option<GitCounts>,
    pub is_bare: bool,
    /// Bumped when a refresh starts. The interface discards a reply older than what it holds.
    pub generation: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub remotes: Vec<GitRemote>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub submodules: Vec<GitSubmodule>,
}

/// A named URL on the repository. **Not a submodule**: a submodule is a different repository,
/// pinned at a commit, with remotes of its own. The overview carries both lists and flattens
/// neither into the other.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitRemote {
    pub name: String,
    pub url: String,
    /// The one a fetch would use when none is named.
    pub is_default: bool,
}

/// How much of a submodule is actually here.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum GitSubmoduleState {
    /// Named in `.gitmodules`, never checked out. Not walked into.
    Uninitialised,
    Clean,
    Dirty,
}

/// A repository below the project, pinned by the outer one. Listed, never merged: it contributes
/// nothing to the outer project's counts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitSubmodule {
    pub name: String,
    /// Project-relative, forward-slashed. A submodule outside the project's scope is omitted.
    pub rel_path: String,
    pub url: String,
    pub state: GitSubmoduleState,
}

/// How one side of a path differs: the index against HEAD, or the worktree against the index.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum GitPathChange {
    Added,
    Modified,
    Deleted,
    Renamed {
        from: String,
    },
    TypeChange,
    /// Only valid on the worktree side.
    Untracked,
}

/// One path that has something to say. A row not in the map is clean, once a map has arrived.
///
/// The pair is the model. The interface's single [`GitMark`] is a projection of it, stated once so
/// two windows cannot disagree.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitEntry {
    /// Project-relative, forward-slashed, on the same discipline as the file family.
    pub rel_path: String,
    /// How the index differs from HEAD.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index: Option<GitPathChange>,
    /// How the worktree differs from the index.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree: Option<GitPathChange>,
    /// The path has unmerged stages. Overrides both sides for display.
    pub conflicted: bool,
    /// Set on collapsed ignored directory entries, when the host emits them.
    pub ignored: bool,
}

impl GitEntry {
    /// The single status the explorer draws. Stated here so the host's directory rollup and the
    /// interface's row cannot disagree.
    ///
    /// Conflicted, else worktree untracked, else a worktree change, else an index change, else
    /// ignored. A file both staged and modified draws as modified: the unstaged edit is the newer
    /// fact.
    pub fn mark(&self) -> Option<GitMark> {
        if self.conflicted {
            return Some(GitMark::Conflict);
        }
        match &self.worktree {
            Some(GitPathChange::Untracked) => return Some(GitMark::Untracked),
            Some(_) => return Some(GitMark::Modified),
            None => {}
        }
        if self.index.is_some() {
            return Some(GitMark::Staged);
        }
        if self.ignored {
            return Some(GitMark::Ignored);
        }
        None
    }
}

/// The explorer's one badge, a projection of [`GitEntry`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum GitMark {
    Modified,
    Untracked,
    Conflict,
    Staged,
    Ignored,
}

impl GitMark {
    /// Severity for a directory rollup. Conflict is worst; ignored is least.
    pub fn rank(self) -> u8 {
        match self {
            GitMark::Ignored => 1,
            GitMark::Staged => 2,
            GitMark::Untracked => 3,
            GitMark::Modified => 4,
            GitMark::Conflict => 5,
        }
    }
}

/// A folder's badge, rolled up by the host from the changed paths under it. The explorer expands
/// one level at a time and cannot derive this from children it has not asked for.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitRollup {
    pub rel_path: String,
    pub mark: GitMark,
}

/// How many commits one page may carry. A request for more is clamped, not refused.
pub const MAX_LOG_PAGE: u32 = 200;

/// Who and when. The committer is carried beside the author because they differ after a rebase,
/// and the difference is the point.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitWho {
    pub name: String,
    pub email: String,
    /// Seconds since the epoch.
    pub time: i64,
    /// Minutes east of UTC. The interface prints the commit's own clock, not the reader's.
    pub offset: i32,
}

/// One commit in a page of history.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitCommit {
    pub id: String,
    /// The abbreviation the repository's own config would use.
    pub short_id: String,
    /// The first line of the message. The body is fetched per commit, never per page.
    pub summary: String,
    pub author: GitWho,
    pub committer: GitWho,
    /// Parent commit ids, full form. Empty only for a root commit; two or more is a merge.
    /// Identity rather than a count, because a lane algorithm matches a child to the lane its
    /// parent occupies, and a count cannot say which lane that is.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parents: Vec<String>,
    /// Which lane this commit draws in. Computed host-side, on the same reasoning as the
    /// working-tree rollups: two windows must not lay out the same history differently.
    pub lane: usize,
    /// Lanes this commit merges from, for the merge lines drawn behind it. Empty for a
    /// single-parent commit.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub merges: Vec<usize>,
    /// Branch and tag names pointing at this commit, for the decorations.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub refs: Vec<String>,
    /// The author is the repository's configured identity. The host answers it because it is the
    /// host that can read `user.email`; the interface reads no config.
    pub mine: bool,
}

/// Which sidebar section a ref belongs in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum GitRefKind {
    Local,
    Remote,
    Tag,
    Stash,
}

/// One row of the ref list. Submodules are not here — they are on [`RepoOverview`], because a
/// submodule is a repository and not a ref.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitRef {
    /// Shorthand: `main`, `origin/main`, `v0.3.0`, `stash@{0}`.
    pub name: String,
    pub kind: GitRefKind,
    /// The commit it points at, abbreviated.
    pub target: String,
    /// This is what HEAD points at. At most one row answers true.
    pub current: bool,
    /// Only when the request asked for tracking. Absent, never zero, when there is no upstream.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ahead: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub behind: Option<u32>,
}

/// What went wrong reading a repository that exists.
///
/// An enum rather than a sentence, on the same reasoning that shaped [`crate::files::FileError`]:
/// `NotFound` means stop asking, `Corrupt` means say so and offer nothing, `Denied` means mark it,
/// `Interrupted` means a newer answer is on its way.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "reason")]
pub enum GitError {
    NotFound,
    Corrupt,
    Denied,
    Interrupted,
    Failed(String),
}

impl std::fmt::Display for GitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GitError::NotFound => write!(f, "the repository is gone"),
            GitError::Corrupt => write!(f, "the repository is corrupt"),
            GitError::Denied => write!(f, "the repository could not be read"),
            GitError::Interrupted => write!(f, "a newer answer is on its way"),
            GitError::Failed(reason) => write!(f, "{reason}"),
        }
    }
}
