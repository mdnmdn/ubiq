---
id: wip-git-integration
title: Git integration implementation plan
kind: wip
status: current
summary: Working notes for the read-only version-control work. Phases A (remotes and submodules in the overview), B (collapsed ignored directories) and D (refs and the log) have landed on git2 per D43; phase C was already in the tree. The gitoxide the source proposal named is not used and is not coming.
read_when: you are picking up the version-control work or asking why the proposal's gitoxide never arrived
updated: 2026-09-05
depends_on: [tech-architecture, tech-transport, feat-workbench]
---

# Git integration — implementation plan (working notes)

Informal working notes, kept until the version-control work is finished and then deleted.
Source: `_docs/inbox/backlog/git-integration-proposal.md`.

---

## 0. Read this before doing anything

**Most of the proposal has already shipped.** Do not re-implement it. What follows is the residue.

| Proposal §12 phase | State in the tree |
|---|---|
| 1 — discovery + overview | **Shipped**, minus remotes, submodules, worktrees, nested repos, `repo_root` |
| 2 — working tree | **Shipped**, minus collapsed ignored directories (`G69`) |
| 3 — refresh + generation | **Shipped in full.** Five triggers, coalescing, two generation counters |
| 4 — refs + log | **Not started.** `G70`, `G83`. This is the actual work |
| 5 — git-directory watch | **Shipped ahead of schedule** — `crates/ubiq-host/src/watch/mod.rs:147` classifies `HEAD`/`MERGE_HEAD`/`index`/`refs/**` and sets `ProjectFilesChanged.repository`, which `crates/ubiq/src/app/wire.rs:484` turns into a full refresh |

What exists today:

- `crates/ubiq-proto/src/git.rs` — `RepoOverview`, `GitHead`, `GitOperation`, `GitCounts`, `GitEntry`, `GitPathChange`, `GitMark`, `GitRollup`, `GitError`, `AHEAD_BEHIND_CAP`, `MAX_WORKING_TREE`.
- `crates/ubiq-proto/src/messages.rs:472-508` — `ProjectGit`, `RefreshProjectGit`, `GitOverview`, `GitWorkingTree`, `GitError`.
- `crates/ubiq-host/src/git/mod.rs` — the worker thread, the two queues, per-project repo cache, per-project generation.
- `crates/ubiq-host/src/git/observe.rs` — discovery, scope, head, tracking, operation, the status walk, rollups, counts.
- `crates/ubiq-host/src/coordinator.rs:941-954, 1655-1683` — dispatch, `git_job`, `git_forget`.
- `crates/ubiq/src/app/wire.rs:585-654` — `receive_git`, with stale-generation guards.
- `crates/ubiq/src/state/explorer/tree.rs:87-132` — `apply_git` / `clear_git`.
- `crates/ubiq/src/ui/status_bar.rs:337-395` — the readout.
- `crates/ubiq-host/tests/git.rs` — 14 green tests against real scratch repos.

### 0.1 The gitoxide question is already settled: **use `git2`, not `gix`**

The proposal's §9 asked for `gix = "0.87"`. It was overruled during implementation, and the
overruling is a written decision:

- `_docs/tech/decisions.md:623` — **D43**, "The host links libgit2 and computes hunks itself": the
  host reads version control through `git2`, and *"gitoxide is not a second reader."*
- `_docs/tech/architecture.md:157` — "Version control is read in two places, both in the host, both
  through `git2`."
- `Cargo.lock` — `git2 0.21.0`, `libgit2-sys 0.18.8+1.9.7`, already built.

Migrating ~570 working lines and 14 passing tests to `gix` changes nothing on screen. **Do not do
it under this plan.** If someone wants `gix`, that is a new decision row amending D43, filed
separately.

For the record, since it was asked: `gix = "0.87"` is a real published version (0.87.1 current).
This machine has no network and no vendored copy, so nothing about its API surface was verified
here. Anyone reviving that path must check every call against `cargo doc -p gix` first — the crate
renames surface between minors (`work_dir` → `workdir`, the `status` platform builder, `rev_walk`
options) and the proposal's §5/§7 sketches predate at least one of those. `refs/rgitui` is on
`git2 0.20` and is no help for `gix` at all.

**Everything below is `git2 0.21`.**

---

## Phase A — the overview's missing discovery fields

Small. Retires two of the Git screen's five fixture sections.

### A.1 YAGNI filter, applied

| Proposal field | Verdict |
|---|---|
| `remotes` | **Build.** `ui/git/refs.rs` draws a Remotes section from `sample.rs` today |
| `submodules` | **Build.** Same — a Submodules section from `sample.rs` |
| `repo_root` | **Delete the field.** It is `String::new()` at `observe.rs:73` and read nowhere in `crates/ubiq`. `Repository::discover` only ever walks *up*, so a repository root below the project cannot occur, and the above case is already `scoped_to`. The field is a lie with no caller |
| `worktree`, `worktrees` | **Skip.** Nothing on screen would draw them. Backlog row |
| `nested` | **Skip.** Same. Backlog row |

### A.2 `crates/ubiq-proto/src/git.rs`

Delete `RepoOverview::repo_root` (line 60 and its doc comment). Add:

```rust
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
```

On `RepoOverview`, in place of `repo_root`:

```rust
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub remotes: Vec<GitRemote>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub submodules: Vec<GitSubmodule>,
```

### A.3 `crates/ubiq-host/src/git/observe.rs`

Drop `repo_root:` from the `RepoOverview` literal at line 73; add `remotes:` and `submodules:`.
Both are refs-and-config reads, so they belong on the cheap path (`full == false` too).

```rust
/// The repository's named URLs. `origin` is the default when it exists, else the first named.
fn remotes(repo: &Repository) -> Vec<GitRemote>

/// The submodules whose path falls inside the project's scope.
fn submodules(repo: &Repository, scoped_to: &str) -> Vec<GitSubmodule>
```

`git2` calls — all present in 0.21, all `&self`:

- `repo.remotes() -> Result<git2::string_array::StringArray>` (names), then
  `repo.find_remote(name)?.url() -> Option<&str>`. A remote with no UTF-8 URL is skipped, not an error.
- `repo.submodules() -> Result<Vec<git2::Submodule>>`; per submodule `sm.name() -> Option<&str>`,
  `sm.path() -> &Path`, `sm.url() -> Option<&str>`.
- State: `repo.submodule_status(name, git2::SubmoduleIgnore::None) -> Result<git2::SubmoduleStatus>`.
  `WD_UNINITIALIZED` → `Uninitialised`; else any of
  `WD_INDEX_MODIFIED | WD_WD_MODIFIED | WD_UNTRACKED | WD_MODIFIED` → `Dirty`; else `Clean`.
- Reuse `project_rel(sm.path()…, scoped_to)` — it already strips the scope and normalises slashes.
  `None` means outside the scope: omit the row.

Failures here are never fatal to the overview. A submodule that will not stat is dropped from the
list; `GitError` stays for the repository as a whole.

### A.4 UI

**None in this phase.** The rows are consumed in Phase D, where `GitView.refs` is rebuilt in one
pass. Shipping half a rebuild would mean touching `ui/git/refs.rs` twice.

### A.5 Check

`cargo test -p ubiq-host --test git`, then `just verify`. New tests in
`crates/ubiq-host/tests/git.rs`, using the existing `git(&[…])` / `repository()` helpers:

- `a_remote_is_named_in_the_overview` — `git remote add origin https://example/x`, assert one
  remote, `is_default`.
- `an_uninitialised_submodule_is_listed_not_walked` — add a submodule, do not `update --init`;
  assert `Uninitialised` and that the outer `entries` do not contain paths from inside it.

---

## Phase B — collapsed ignored directories (`G69`)

`_docs/backlog.md:43` claims git2 cannot collapse an ignored tree. That is wrong:
`StatusOptions::recurse_ignored_dirs(false)` is exactly the collapse — libgit2 reports
`node_modules/` as one entry. Two lines plus tests.

### B.1 `crates/ubiq-host/src/git/observe.rs`, in `working_tree` (line ~196)

```rust
    opts.include_untracked(true)
        .recurse_untracked_dirs(false)
        .exclude_submodules(true)
        .update_index(false)
        // An ignored tree is unbounded — `target/`, `node_modules/`, `.venv/`. libgit2 yields it
        // as one entry when it is told not to recurse, which is what keeps the map proportional
        // to the change set. `project_rel` already strips the trailing slash.
        .include_ignored(true)
        .recurse_ignored_dirs(false);
```

Nothing else changes. The plumbing downstream already handles it:

- `project_rel` (observe.rs:291) strips libgit2's trailing slash on a directory entry.
- `GitEntry::mark` returns `Ignored` last, and `GitMark::rank` gives it 1 — the lowest — so an
  ignored sibling never outranks a modified one in `rollups_of`.
- `counts_of` (observe.rs:347) counts nothing for an ignored entry, so the status bar is unchanged.
- `crates/ubiq/src/state/explorer/tree.rs` already seeds `git_inherit` for `Ignored`, so expanding
  into `target/` inherits the badge with no further request.

One risk to watch: ignored entries now consume the `MAX_WORKING_TREE` budget. Keep the ignored
entries *after* the changed ones is not worth the sort — a repository with 2 000 top-level ignored
directories does not exist. Leave it; the ceiling already reports `truncated`.

### B.2 Check

New test in `crates/ubiq-host/tests/git.rs`:

- `an_ignored_directory_is_one_entry` — `.gitignore` with `target/`, create `target/a`, `target/b`;
  assert exactly one entry, `rel_path == "target"`, `ignored == true`.
- `an_ignored_directory_does_not_outrank_a_modified_sibling` — assert the root rollup, if any,
  is not `Ignored`.

Then `_docs/backlog.md`: delete row `G69`.

---

## Phase C — refresh and the generation

**Nothing to do.** Verified present:

| Proposal trigger | Where |
|---|---|
| Project opened | `crates/ubiq/src/app/shell.rs:53-57` — `ProjectGit`, then `RefreshProjectGit { full: true }` |
| Asked explicitly | `crates/ubiq/src/app/git.rs:28-32` — `AppState::refresh_git` |
| Editor save | `crates/ubiq/src/app/wire.rs:434-437` |
| Pane exit | `crates/ubiq/src/app/wire.rs:257-260` |
| Explorer edit | `crates/ubiq/src/app/wire.rs:1252-1255` |
| Git directory moved | `crates/ubiq/src/app/wire.rs:484-487`, from the watcher's `repository` flag |

Coalescing: `crates/ubiq-host/src/git/mod.rs:116-131` — a second `Full` for a project replaces the
queued one. Generation: bumped at `mod.rs:135-142`, discarded UI-side at `wire.rs:601-604`
(overview) and `explorer/tree.rs:92-94` (tree).

One known gap, **not worth fixing**: `should_interrupt` mid-walk. libgit2's status has no
interrupt, so a superseded walk runs to completion and its reply is dropped by generation. That is
the same outcome one message later. Leave it; `GitError::Interrupted` stays unused.

---

## Phase D — refs and the log

The real work. Retires `G70` and `G83` and deletes `crates/ubiq/src/state/sample.rs`.

### D.1 `crates/ubiq-proto/src/git.rs`

```rust
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
    /// How many parents. Two or more is a merge, drawn as one without a second request.
    pub parents: u32,
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
```

### D.2 `crates/ubiq-proto/src/messages.rs`

Extend the `use crate::git::…` line (line 14) with `GitCommit, GitRef`.

In the "Git family: UI → host" block after `RefreshProjectGit` (line ~487):

```rust
    /// A page of history. **Cursor-paged, not offset-paged**: a page is a bounded walk from a
    /// starting commit and the next cursor is the commit after the last returned. An offset would
    /// re-walk from HEAD every page and be wrong the moment the tree moved underneath.
    ///
    /// Answered with [`Message::GitLogPage`], or [`Message::GitError`].
    ProjectGitLog {
        project_id: ProjectId,
        /// Where the walk starts. Absent is HEAD.
        cursor: Option<String>,
        /// How many commits, clamped to [`git::MAX_LOG_PAGE`].
        count: u32,
        /// The history of one path, project-relative. Absent is the whole repository.
        rel_path: Option<String>,
        first_parent: bool,
    },
    /// Branches, remote-tracking branches, tags and stashes. `with_tracking` adds ahead and behind
    /// per branch, which is one merge-base walk each — the branch picker asks without it.
    ///
    /// Answered with [`Message::GitRefs`], or [`Message::GitError`].
    ProjectGitRefs {
        project_id: ProjectId,
        with_tracking: bool,
    },
```

In the "Git family: host → UI" block after `GitWorkingTree` (line ~503):

```rust
    /// One page of history. `next_cursor` absent is the end.
    GitLogPage {
        project_id: ProjectId,
        commits: Vec<GitCommit>,
        next_cursor: Option<String>,
    },
    /// Every ref the sidebar draws, in one reply — five sections would otherwise be five walks.
    GitRefs {
        project_id: ProjectId,
        refs: Vec<GitRef>,
    },
```

Add all four to the `project_id` accessor arm at `messages.rs:850-854`.

### D.3 `crates/ubiq-host/src/git/mod.rs`

`Request` gains two variants (line ~34):

```rust
    /// Branches, tags and stashes. Cheap: refs only, no tree walk.
    Refs { with_tracking: bool },
    /// One page of history.
    Log {
        cursor: Option<String>,
        count: u32,
        rel_path: Option<String>,
        first_parent: bool,
    },
```

- `enqueue` (line 116): both join the cheap queue — `Request::Overview | Request::Forget |
  Request::Refs { .. } | Request::Log { .. } => overviews.push_back(job)`. Rename the local
  `overviews` to `cheap` while there; the name stopped being true.
- `answer` (line 133): two new arms. **Neither bumps the generation** — neither is a working-tree
  walk, and a log page is not something a stale reply can corrupt.
- Both arms need the cached `Repository`: reuse `ensure_repo`, and on `Ok(false)` (no repository)
  answer with an empty `GitRefs` / an empty `GitLogPage`, not a `GitError` — the same reasoning as
  `overview: None`.

### D.4 New file `crates/ubiq-host/src/git/history.rs`

Keeps `observe.rs` from growing a second subject. Declare `pub mod history;` in `git/mod.rs`.

```rust
/// Every ref the sidebar draws. `with_tracking` costs one merge-base walk per local branch.
pub fn refs(repo: &Repository, with_tracking: bool) -> Result<Vec<GitRef>, GitError>

/// One page of history, newest first.
///
/// Returns the page and the id of the commit after it, which is the next cursor.
pub fn log(
    repo: &Repository,
    scoped_to: &str,
    cursor: Option<&str>,
    count: u32,
    rel_path: Option<&str>,
    first_parent: bool,
) -> Result<(Vec<GitCommit>, Option<String>), GitError>
```

Make `observe::short_id`, `observe::tracking`, `observe::map_error` and `observe::project_rel`
`pub(crate)` rather than copying them.

`git2 0.21` calls:

**refs**
- Local: `repo.branches(Some(BranchType::Local))?`; per branch `branch.name()?`,
  `branch.is_head()`, `branch.get().target()`. With tracking, reuse `observe::tracking(repo, name)`
  — it already returns the capped pair.
- Remote: `repo.branches(Some(BranchType::Remote))?`. Never tracked.
- Tags: `repo.tag_names(None)?`, then
  `repo.revparse_single(&format!("refs/tags/{name}"))?.peel_to_commit()?.id()`. A tag that will not
  peel (a tag on a tree) is skipped.
- Stashes: **read `repo.reflog("refs/stash")`**, not `repo.stash_foreach` — the latter takes
  `&mut Repository` and the worker holds the cache immutably. Per `ReflogEntry`: `entry.id_new()`
  for the target, `entry.message()` for the name, and index `i` gives `stash@{i}`. A repository
  with no stash reflog returns `NotFound`; treat it as an empty list.

**log**
- `let mut walk = repo.revwalk()?;`
  `walk.set_sorting(git2::Sort::TIME | git2::Sort::TOPOLOGICAL)?;`
- Start: `match cursor { Some(id) => walk.push(Oid::from_str(id)?)?, None => walk.push_head()? }`.
  `push_head` on an unborn HEAD errors — map to an empty page, not a `GitError`.
- `if first_parent { walk.simplify_first_parent()?; }`
- Decorations, built **once** per page before the loop:
  `HashMap<Oid, Vec<String>>` from `repo.references()?`, keyed by `r.peel_to_commit()?.id()`, value
  `r.shorthand()`. Skip `refs/stash` and `HEAD`.
- Per commit: `repo.find_commit(oid)?`; `commit.summary()`, `commit.parent_count()`,
  `commit.author()` / `commit.committer()` → `GitWho { name, email, time: sig.when().seconds(),
  offset: sig.when().offset_minutes() }`. `short_id` via `observe::short_id`.
- `mine`: compare the author email against `repo.signature()?.email()`, case-insensitively.
  `repo.signature()` errors when `user.email` is unset — then `mine` is always `false`.
- Path filter: **git2's revwalk has no pathspec.** Filter manually — diff the commit's tree against
  its first parent's with `DiffOptions::pathspec(full_path)` and keep the commit when
  `diff.deltas().len() > 0`. A root commit diffs against `None`. `full_path` is `scoped_to` joined
  with `rel_path` when the project sits inside a larger repository.
  ```rust
  // ponytail: a path with no history would walk to the root. The scan is bounded at
  // PATH_SCAN_CEILING commits and the page simply comes back short; if that reads badly in a
  // history view, the upgrade is a commit-graph bloom filter, not a longer walk.
  const PATH_SCAN_CEILING: usize = 5_000;
  ```
- `next_cursor`: pull `count.min(MAX_LOG_PAGE) + 1` commits and use the extra one's id, dropping it
  from the page. `None` when the walk ran out.
- The unfiltered log is **repository-wide even for a scoped project**. There is one HEAD and one
  history; scoping the log would need the same per-commit diff as the path filter, for every commit,
  and no screen asks for it. Backlog row if it ever matters.

### D.5 `crates/ubiq-host/src/coordinator.rs`

Two arms after `RefreshProjectGit` (line ~953). `git_job` is unchanged — the parameters ride on
`Request`.

```rust
            Message::ProjectGitRefs {
                project_id,
                with_tracking,
            } => {
                self.git_job(client, project_id, git::Request::Refs { with_tracking });
            }
            Message::ProjectGitLog {
                project_id,
                cursor,
                count,
                rel_path,
                first_parent,
            } => {
                self.git_job(
                    client,
                    project_id,
                    git::Request::Log {
                        cursor,
                        count,
                        rel_path,
                        first_parent,
                    },
                );
            }
```

### D.6 UI

`crates/ubiq/src/state/git.rs` — keep `RefRow` and `CommitRow`. They carry `lane`, `merges`, `when`
and `section`, which are drawing facts the wire deliberately does not have. Add two converters and
one field:

```rust
/// The sidebar's rows, from the host's refs and the overview's submodules. Local, Remotes, Tags
/// and Stashes come from the refs reply; Submodules from the overview, because a submodule is a
/// repository and not a ref.
pub fn ref_rows(refs: &[GitRef], submodules: &[GitSubmodule]) -> Vec<RefRow>

/// The history's rows. Lane assignment is topology the interface computes for itself — the wire
/// carries a parent *count*, which is all the drawing needs to know a merge from a commit.
pub fn commit_rows(commits: &[GitCommit]) -> Vec<CommitRow>
```

- `when`: relative wording from `GitWho { time, offset }`. Grep `crates/ubiq/src/ui/kit` for an
  existing "…ago" helper before writing one; the logs and the conversation screens both print
  relative times.
- `lane` / `merges`: the minimal honest version is lane 0 for every commit and
  `merges: vec![1]` when `parents > 1`, which is what `history.rs:219 lane_gutter` needs to draw a
  hollow dot. A real lane allocator is a backlog row, not this task.
- `mine`: straight from `GitCommit::mine`.
- On `GitView`, add `pub log_cursor: Option<String>` and `pub log_done: bool`.

`crates/ubiq/src/app/wire.rs`, in `receive_git` (line 585) — two arms:

```rust
Message::GitRefs { project_id, refs } => {
    let open = self.projects.get_mut(&project_id)?;
    open.git_view.refs = git_state::ref_rows(
        &refs,
        open.git.as_ref().map(|o| o.submodules.as_slice()).unwrap_or(&[]),
    );
    open.git_view.selected_ref = open.git_view.refs.iter().position(|r| r.current);
}
Message::GitLogPage { project_id, commits, next_cursor } => {
    let open = self.projects.get_mut(&project_id)?;
    let rows = git_state::commit_rows(&commits);
    if open.git_view.log_cursor.is_none() {
        open.git_view.commits = rows;      // first page replaces
    } else {
        open.git_view.commits.extend(rows); // a later page appends
    }
    open.git_view.log_done = next_cursor.is_none();
    open.git_view.log_cursor = next_cursor;
}
```

The `GitOverview` arm must also rebuild `refs` when `submodules` changed, or the Submodules section
lags a refresh behind. Cheapest: after storing the overview, re-run `ref_rows` if `refs` is
non-empty.

`crates/ubiq/src/app/git.rs` — `refresh_git` sends the two new asks alongside the existing pair:

```rust
self.bus.send(Message::ProjectGitRefs { project_id, with_tracking: true });
self.bus.send(Message::ProjectGitLog {
    project_id,
    cursor: None,
    count: 100,
    rel_path: None,
    first_parent: false,
});
```

Reset `git_view.log_cursor = None` before sending, or the first page will append to the old one.

`crates/ubiq/src/app/shell.rs:53-57` — same two, on project open.

`crates/ubiq/src/state/sample.rs` — **delete the file.** `git_refs` and `git_history` are its only
public items. Remove `mod sample;` from `crates/ubiq/src/state/mod.rs`, and change
`crates/ubiq/src/app/mod.rs:293` to `git_view: GitView::default()`.

`crates/ubiq/src/ui/git/refs.rs`, `history.rs` — **no change**. They read `GitView.refs` and
`GitView.commits`, which now arrive from the host. Confirm by grepping for `sample::` in `ui/`.

### D.7 Docs, in the same commit

- `_docs/backlog.md` — delete `G70` and `G83`. Add rows for: linked worktrees and nested
  repositories in the overview; scoping the log to a project's prefix; a real history lane
  allocator; `GitError::Interrupted` unused because libgit2 status has no interrupt.
- `_docs/tech/transport-contract.md` — four rows in the git-family table (§"The git family",
  line ~322), plus a paragraph on cursor paging and on refs being one reply for four sections.
- `_docs/tech/project-structure.md` — a row for `ubiq-host/src/git/history.rs`; the `sample.rs`
  row goes if it has one.
- `_docs/tech/decisions.md` — no new row. D43 already covers the library.
- Run `just docs-touched` and update whatever it names.

### D.8 Checks

New file `crates/ubiq-host/tests/git_history.rs` (a new file, so it does not collide with the
existing `tests/git.rs`), reusing the same scratch-repo helper style:

- `the_refs_list_names_the_current_branch`
- `a_tag_is_in_the_refs_list`
- `a_log_page_returns_a_cursor_and_the_next_page_continues`
- `a_path_filtered_log_returns_only_commits_touching_the_path`
- `first_parent_skips_the_merged_side`
- `an_unborn_head_returns_an_empty_page_not_an_error`

`crates/ubiq/tests/git.rs` additions: `ref_rows` sorts into sections and marks the current row;
`commit_rows` marks a two-parent commit as a merge.

`just verify` is the gate. Expect `docs-lint` to fail at the repo's known 164-failure baseline —
compare against `main`, do not chase zero.

---

## Ordering and partition

Parallel agents share one working tree, so ownership is **by file**, and no two agents ever hold
the same path.

### Wave 1 — serial gate: **agent P (protocol)**

Owns, exclusively:

- `crates/ubiq-proto/src/git.rs`
- `crates/ubiq-proto/src/messages.rs`

Does A.2 and D.1 + D.2 in one pass — both phases touch the same two files, so splitting them across
agents is exactly the collision to avoid. Gate: `cargo check -p ubiq-proto` is green.
Nothing else starts until this lands.

### Wave 2 — three agents in parallel, disjoint trees

| Agent | Owns | Does |
|---|---|---|
| **H** (host) | `crates/ubiq-host/**` — `src/git/mod.rs`, `src/git/observe.rs`, `src/git/history.rs` (new), `src/coordinator.rs`, `tests/git.rs`, `tests/git_history.rs` (new) | A.3, B.1, D.3, D.4, D.5, and every host test |
| **U** (interface) | `crates/ubiq/**` — `src/app/wire.rs`, `src/app/git.rs`, `src/app/shell.rs`, `src/app/mod.rs`, `src/state/git.rs`, `src/state/mod.rs`, `src/state/sample.rs` (deleted), `tests/git.rs` | D.6 and the client tests |
| **D** (docs) | `_docs/**` only | B.2's backlog edit, D.7 |

H and U cannot collide: the architecture rule already forbids the crates from naming each other,
and both compile against agent P's output alone. D touches no `.rs` at all.

### Wave 3 — one agent, `just verify`

Whoever finishes last. `cargo check --workspace` first to catch a proto field one side spelled
differently, then the full recipe.

### Notes for the agents

- Foreground cargo runs only. A background build stalls a subagent.
- No `git stash`, `git checkout`, `git commit` — the working tree is shared.
- `just host` and `just ui` are boundary checks: `git2` must never reach `crates/ubiq`, and
  `crates/ubiq` must never name `crates/ubiq-host`.
- Phases A and B are host-only once P has landed, so they are not separate waves — they are
  agent H's first two commits.
