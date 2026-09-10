---
id: tech-version-control
title: Version control
kind: tech
status: current
summary: How the host reads a project's repositories — the rule that Ubiq creates a repository or reads one and never writes into one, where a clone runs, upward discovery and scope, the bounded downward walk that finds the repositories inside a project and merges them into one map, the git worker's two queues and its per-project caches, the three shapes it answers with, the commit-graph lane engine, the refresh discipline that narrows the staleness window, and the ceilings and assumptions the model rests on.
read_when: you are extending version control, adding the write family, touching how a clone runs, working on a project that holds more than one repository, or wondering why the commit graph's lane engine is hand-rolled rather than a dependency
updated: 2026-09-10
verified: 2026-09-10
code_anchors: [crates/ubiq-proto/src/git.rs, crates/ubiq-host/src/git/mod.rs, crates/ubiq-host/src/git/observe.rs, crates/ubiq-host/src/git/nested.rs, crates/ubiq-host/src/git/history.rs, crates/ubiq-host/src/git/graph.rs, crates/ubiq-host/src/files/diff.rs, crates/ubiq-host/src/watch/mod.rs, crates/ubiq/src/state/git.rs, crates/ubiq/src/app/git.rs, crates/ubiq-host/src/repos/mod.rs, crates/ubiq-host/src/repos/clone.rs, crates/ubiq-host/src/repos/list.rs]
depends_on: [tech-architecture, tech-transport, tech-decisions, feat-workbench]
review_cycle: monthly
---

# Version control

The subsystem model: what Ubiq reads out of a repository, where that work runs, and what it
refuses to do. The message table belongs to
[`transport-contract.md`](./transport-contract.md); what the Git screen draws with these answers
belongs to [`../features/workbench.md`](../features/workbench.md). This document is the layer under
both.

## 1. Ubiq creates a repository, or reads one; it never writes into one

**The agents in the panes are what mutate the repository. Ubiq only observes.** That is the domain
fact the whole subsystem is shaped around, and it is why the read-only line was worth drawing:
Ubiq is a window onto a working tree that several harnesses are editing at once, and a second
writer in that room is a correctness problem rather than a feature.

Two decisions hold the line. `D30` — Ubiq writes nothing inside a project's folder — covers the git
directory, because the git directory is inside that folder: no ref written, nothing staged, and the
status walk runs with libgit2's index-stat refresh turned off so a read cannot touch the index
either. `D43` — the host links libgit2 and computes hunks itself — makes `git2` the one reader:
no `git` subprocess whose output would have to be parsed, and gitoxide is not a second reader. Both
are in [`decisions.md`](./decisions.md); `D9` is why the harness library, and not Ubiq, decides how
an agent is launched into that same folder.

**Cloning is the one write, and it is a write that has no repository to corrupt.** A clone brings a
repository into existence at a path where none was — nothing is staged, no ref is moved, no working
tree another writer is in is touched — and from the moment it registers the project, everything above
applies to it unchanged. That is the whole of why `git2` is compiled with `https` (`D72`); a
transport was left out while Ubiq only read, and it is in for exactly one operation. The line the
rule draws is the useful one: **no message in this family mutates a repository Ubiq did not just
make.** The clone itself belongs to
[`../features/workbench.md`](../features/workbench.md) and its wire form to
[`transport-contract.md`](./transport-contract.md).

The consequence to hold on to: **every fact this family reports is about a moment that has passed.**
The machinery that narrows that window (§6) is the feature. The walk being fast is not.

## 2. What a repository is, and what it is not

**A project has one repository of its own, and may hold more inside it.** The one of its own is
discovered **upward from the project's root** with `Repository::discover`, at the worker, once per
project. Three shapes follow:

- **No repository above the project** is an ordinary answer, not a failure: the overview is absent,
  and no branch and no badges are drawn.
- **The project is the repository root.** The common case. The scope is empty.
- **The project is a folder inside a larger repository.** `scope()` computes the project's prefix
  inside the repository, and every path that crosses the bus is made relative to the project by
  `project_rel()`. No absolute path leaves the host, and a change outside the project's prefix is
  not the project's business.

`Repository::discover` finds the nearest `.git` and stops, so the upward walk answers exactly one
repository — the project's own, and the only one the overview, the refs and the log are about.

**The repositories inside the project are found by walking down.** `git/nested.rs`'s `discover()`
starts at the project's root and names every folder holding a `.git` — a directory, or the gitlink
*file* a submodule and a linked worktree both appear as. It skips `WALK_SKIP`'s names, never
descends into a repository once it has found one — a repository inside a repository inside the
project is that repository's business — and is bounded twice: `MAX_NESTED_REPOS` (32) roots and
`MAX_NESTED_DEPTH` (8) levels. Past either it stops looking, and the map arrives `truncated`.

Each one found is **walked and merged into the project's one map** (`D99`): the nested repository's
status walk runs with an empty scope, its paths are prefixed with its own project-relative root,
the outer repository's own entry at that folder is dropped first, and the rollups are recomputed
over the merged set. `GitNested` is the boundary — one row per repository inside the project,
carrying its `HEAD`, whether the outer repository pins it as a submodule, and its own counts. A
nested repository that will not open is reported there with **absent counts** and contributes no
entries; one broken clone is not an error for the whole project.

Nothing above requires a repository of the project's own. A folder holding several independent
clones answers `overview: None` and a working tree all the same, which is why the downward walk runs
before the upward walk's result is consulted.

Two things are still **listed rather than merged**. A remote is a name and a URL the project's own
repository fetches from. A submodule is a row on the overview: `GitSubmodule` is the outer
repository's own account of what it pins — the commit, the URL, the state — and it stays what it
was, because the status walk excludes submodules and a submodule outside the project's scope is
omitted the way a file outside it never appears in a listing. The pin and the repository are two
facts about the same folder, so an initialised submodule appears on **both** lists: named as a pin
on the overview, and walked as a `GitNested` in the map.

`git::web_url` turns a remote's URL into the page a browser would open, and is the whole of what
Ubiq knows about providers: strip the scheme, the credentials and the `.git`, and every host but
Azure DevOps puts the repository at `https://<host>/<path>` — GitHub, GitLab, Gitea, Bitbucket and
a self-hosted instance alike. Azure is the one exception, because its web path is
`org/project/_git/repo` while its ssh path is `v3/org/project/repo`. It is not a URL parser: a
local path, a single-segment path, or anything carrying a query is not a page and answers `None`,
which is how the titlebar decides whether to draw the link at all.

## 3. Where the work runs

`crates/ubiq-host/src/git/` is a worker thread of its own, on the shape the files worker proved: a
`Job` carries the project's root rather than a way to look one up, and the coordinator resolves the
record in memory, submits, and answers nothing itself. A cold status on a large repository is
seconds, and seconds on the coordinator's thread is every pane's keystrokes stalled behind it.

**Two queues on the one thread.** The cheap queue holds everything that is refs and a handful of
files in the git directory — the overview, the refs list, a log page, a forget. The full queue holds
working-tree walks. Cheap work is drained ahead of a walk, so the branch name in the status bar is
never stuck behind the explorer's badges. A second full refresh for a project that is still walking
**replaces** the queued one rather than lining up behind it.

**A clone never runs on this worker** (`D73`). The worker is one shared thread whose `answer()` is
synchronous, so a clone of a large repository sitting in either queue would be minutes of
head-of-line blocking on the branch name and the explorer's badges of every project in every window.
`crates/ubiq-host/src/repos/` takes a thread per clone instead, copying the connector flows' shape:
an id in every message, the asker's mailbox for progress, and a `flume` receiver as the sole cancel
mechanism. Progress is throttled to one message every 250ms, because libgit2's `transfer_progress`
fires per object. The clone thread cannot reach the project catalogue, so it hands the finished
folder back over a channel the coordinator drains in `register_clones()` beside
`reap_conversations()`, and the run loop's wait is capped while `Repos::busy()`. Nothing about
`repos/` touches this worker's repository cache — a clone opens no cached handle, which is why
`G84`'s un-mutexed cache is untouched by it.

Three pieces of per-project state live on the worker:

| State | What it holds | Dropped by |
|---|---|---|
| The repository cache | One open `Repository` per project, from `ensure_repo()` | `Request::Forget` — the folder moved, or the record is gone |
| The generation counter | A `u64` bumped when a full refresh starts | `Request::Forget` |
| The lane cache | The commit-graph columns a page ended on, keyed by the cursor and filters the next page must arrive with (§5) | `Request::Forget`, a fresh walk, or a filter change |

**The generation is a staleness guard, not a version.** It rides out on the overview and the
working-tree reply, and the interface discards a reply older than the one it holds — a superseded
walk still runs to completion, and its answer is dropped on arrival rather than interrupted
mid-flight. The log has a guard of its own on a narrower mechanism: `GitLogPage` echoes the `cursor`
its request carried, and the interface accepts a page only when that echo matches the request the
view is waiting on, so a first page landing after a later request advanced the cursor is discarded
instead of appended. Refs have neither (`G127`).

Only the window that asked is answered. The family is addressed to the asker rather than broadcast,
which rides on the invariant that a project is open in exactly one window — an invariant the
workbench enforces by kicking a project out of its previous window when it is opened elsewhere. The
worker's own state is keyed by project id with no client awareness, so it has no second line of
defence if that invariant is ever violated (`G135`).

## 4. The three shapes

**The overview** is cheap: `HEAD` as a branch name, a detached short id or an unborn branch name;
the upstream and the ahead/behind pair when there is one; an operation in progress; whether the
repository is bare; the remotes; the submodules in scope. Working-tree counts ride with a full
refresh, and are absent rather than zero until a walk has run. They are the project's **own**
repository's counts — the folders a nested repository owns are dropped and nothing nested is summed
in, so the status bar reads the branch it names. This is what the status bar reads.

**The working-tree map** is the status walk, and its rule is that it carries only paths that have
something to say: **a path not in the map is clean**, once a map has arrived. An entry is the pair —
how the index differs from `HEAD`, how the worktree differs from the index — plus whether the path
is conflicted or ignored, and the single badge the explorer paints is a projection of that pair
computed host-side so two windows cannot disagree. Directory rollups are computed by `rollups_of()`
and sent with the map, because the explorer expands one level at a time and cannot derive a folder's
badge from children it has not asked for.

Ignored directories are **collapsed**: the walk does not recurse into an ignored tree, so
`target/` and `node_modules/` are one entry each rather than an unbounded fan-out. That is what
keeps the map proportional to the change set rather than to the repository.

The map is **one map for the project, however many repositories it holds**. Every repository §2
found below the project is walked and merged into it, `MAX_WORKING_TREE` is applied to the merged
set and `truncated` says whether anything was cut, and `repos` on `GitWorkingTree` names the
repositories whose entries are in there. A noisy nested clone can therefore crowd the outer
repository's own paths out of the map, which is the price of one map (`D99`).

**The log** is a bounded, cursor-paged walk. A page starts at `HEAD` or at the cursor it was given,
and the next cursor is the commit after the last one the page carried — absent at the end. An offset
would re-walk from `HEAD` for every page and be wrong the moment the tree moved underneath. A page
carries commit ids, the summary line, author and committer with their own clocks, the real parent
ids, ref decorations built once per page by `decorations_of()`, and whether the commit's author is
the repository's configured identity. `rel_path` narrows a page to one path's history, which
`touches_path()` answers per commit by diffing against the first parent — git2's revwalk has no
pathspec — bounded by `PATH_SCAN_CEILING` so a path with no history cannot walk to the root. An
unborn `HEAD` answers with an empty page, not an error. A log with no `rel_path` walks the whole
repository rather than the project's prefix (`G124`). The cursor has a caller: the interface asks
for the next page from the foot of its own history through `AppState::load_more_git_log`
(`crates/ubiq/src/app/git.rs`), so the walk goes as far as the reader scrolls.

**Refs** are one reply for four sections — local branches, remote-tracking branches, tags and
stashes — because a sidebar with five sections has no use for five walks when the repository is
open. `with_tracking` adds the ahead/behind pair per local branch, one merge-base walk each, so a
caller that wants names alone skips the cost. Stashes are read through the `refs/stash` reflog
rather than libgit2's stash iterator, which needs a mutable repository the shared cache cannot
hand out. The fifth section, submodules, comes from the overview, because a submodule is a
repository and not a ref.

## 5. The commit-graph lane engine

`crates/ubiq-host/src/git/graph.rs` holds `assign_lanes()`: a pure function over one page of commits
plus the lanes carried in from the page before it. No repository access, no ancestor walk. A commit
claims the lane waiting for its id or opens the lowest free one; that lane then waits for the
commit's first parent, and each additional parent claims or opens a lane of its own — those are the
commit's `merges`, the columns the merge lines behind it draw from.

Lane assignment is **host-side**, on the same reasoning as the rollups: two windows must not lay out
the same history differently. It is also why a commit's parents cross the bus as ids rather than as
a count — a lane algorithm matches a child to the lane its parent occupies, and a count cannot say
which lane that is. `lanes_for()` keeps page continuity: a request whose cursor, `rel_path` and
`first_parent` all match the cached entry resumes its lanes, and anything else starts empty, so a
branch does not visually collapse and reopen at a page boundary.

**Why it is hand-rolled rather than a dependency.** The survey found nothing embeddable. Everything
crates.io offers for commit-graph layout — `git-graph`, `serie`, `git-igitt`, `gitloom-tui`,
`keifu` — is an end-user binary: none exposes a lane-layout API over bare ids and parents, all bind
a full git read internally, none pages. (`gix-chunk` and its neighbours read git's own on-disk
`commit-graph` file, which is a pack-level ancestry index and an unrelated concept.) The nearest
working engine, in other engines, is a single-pass allocator with a "keep the main line
on lane 0" heuristic that takes the entire loaded commit slice and recomputes from scratch on every
call — no cursor, no resumable lane state. It works there because that interface loads a growing
window; Ubiq's log is deliberately cursor-paged, which is the case it does not handle.
`assign_lanes()` is under sixty lines because it skips that heuristic and the ancestor cache it
needs; the lane cache is page continuity instead.

## 6. The refresh discipline

Five triggers ask for a refresh, all from the interface, all through the same message:

| Trigger | Where |
|---|---|
| A project is opened | `crates/ubiq/src/app/shell.rs` |
| The user asks | `crates/ubiq/src/app/git.rs`, `refresh_git()` |
| An editor save lands | `crates/ubiq/src/app/wire.rs` |
| A pane exits | `crates/ubiq/src/app/wire.rs` |
| The git directory changes | `crates/ubiq/src/app/wire.rs`, from the watcher |

The last is the one that catches an agent. `crates/ubiq-host/src/watch/mod.rs` classifies writes to
`HEAD`, `MERGE_HEAD`, `index` and `refs/**` and raises the `repository` flag on the change
notification; the interface turns that into a full refresh. It finds the `.git` component
**anywhere** in the project, so a nested clone's `sub/.git/HEAD` and a submodule's real git
directory at `.git/modules/<name>/HEAD` both raise it, and nothing under any `.git` ever reaches
`changed` as a file path. The flag stays a `bool`: a plumbing move anywhere asks for one full
refresh, which is the right coarseness when the answer is one merged map. The repository *above*
the project keeps its `.git` outside the watched root and is still undetected (`G125`). A pane
exiting is the coarser version of
the same idea — a harness that has finished is a harness that has finished writing.

Refreshes coalesce at the queue (§3), and the answers are guarded by generation. Nothing polls.

## 7. The ceilings

Every bound is honest — the answer says it was cut short rather than pretending to be whole — and
none is measured against a repository of the size Ubiq is opened on (`G133`).

| Constant | Bounds | Value | What passing it looks like |
|---|---|---|---|
| `MAX_WORKING_TREE` | Entries in one working-tree map, after the merge | 2 000 | The map arrives `truncated` |
| `MAX_NESTED_REPOS` | Repositories one downward walk may find | 32 | The map arrives `truncated` |
| `MAX_NESTED_DEPTH` | Levels below the project the walk looks | 8 | The map arrives `truncated` |
| `AHEAD_BEHIND_CAP` | The ahead/behind walk | 99 | The interface draws `99+` |
| `MAX_LOG_PAGE` | Commits in one log page | 200 | A larger request is clamped, not refused |
| `PATH_SCAN_CEILING` | Commits scanned for a path-filtered page | 5 000 | The page comes back short |

## 8. Assumed, not verified

- **A broken submodule reports as a healthy one.** `submodule_state()` returns `Clean` when
  libgit2's submodule status errors — a silent wrong answer rather than an absent one, and the only
  place in the family that does that (`G132`).
- **The repository is opened twice.** The git worker caches one handle per project; the one-file
  diff in `crates/ubiq-host/src/files/diff.rs` runs its own discovery per request, uncached
  (`G130`). `D43` accepted a second comparison engine, not a second discovery walk on every file
  opened.
- **A nested repository is opened on every full refresh.** Nothing caches those handles: the
  worker's cache is keyed by `ProjectId` alone, and `Repository::open` on an exact root takes no
  upward walk, so the cost is one open plus one status walk per nested repository per refresh and
  has not been measured against a project holding many (`G226`).
- **The ignore rules are read three times** — by libgit2 for the status walk, by the watch's own
  matcher, and by search's walker — and the three do not agree (`G110`).
- **The project catalogue's health probe is filesystem-only.** `probe()` looks at the path, not at
  `.git`, so a corrupt repository probes healthy while the worker returns an error to a different
  part of the screen (`G131`).
- **An agent's turn and the commit it produced are unconnected facts** (`G134`). Ubiq watches both
  the agent and the repository and joins neither.
- **Untested seams:** whether the author check falls back correctly when `user.email` is unset, and
  the stash reflog path.

## 9. Next steps

Ordered. The first two are cheap and pay off inside this subsystem; the rest are load-bearing —
they change a shape rather than fill a hole.

1. **A generation on refs (`G127`).** Two fields and one guard, matching what the overview and the
   working tree do. Cheap, and it stops being cosmetic the moment writes land (item 5).
2. **Cache the diff builder's repository handle (`G130`).** One cache, against a discovery walk paid
   on every file the user opens. Cheap, and it removes one of the two handles item 5 has to reason
   about.
3. **Measure the four ceilings (`G133`).** Not code — one run against a large repository. Every
   number in §7 is a guess until then, and each later item is easier to size once they are real.
4. **The silent submodule default (`G132`), then `GitError::Interrupted` (`G126`).** The first is a
   wrong answer and should become an absent one. The second is a wire variant with no producer:
   either a cancellable walk gives it one, or it leaves the contract. Both are small; neither
   blocks anything.
5. **The write family (`G84`).** Load-bearing, and it forces a design change first: the shared,
   un-mutexed repository cache is safe only because nothing mutates, and staging or committing needs
   a mutable repository — the same collision §4's stash reflog sidesteps. Two independent
   handles (item 2) become a correctness hazard rather than a cost, and the missing staleness guard
   (item 1) stops being a display glitch and becomes a write racing a read. **Change the cache
   before the first write lands**, while nothing depends on its current shape.
6. **What is left of `G125`.** The working-tree map is done: repositories below the project are
   found, walked and merged (`D99`). What is open is a linked worktree read as if it were the only
   repository, the repository *above* the project whose `.git` sits outside the watched root, and
   the Git screen's refs, history and commit box, which are single-repository with no way to choose
   which one. The last is the one that changes a shape, and it is worth doing after the write
   family rather than before: a write into the wrong repository is worse than a read from it.
7. **Joining an agent's turn to the commit it produced (`G134`).** The most interesting thing Ubiq
   could know: it is the one application in the category watching both the agent and the repository.
   It needs the log family it has and a link the work family does not carry.

## Related docs

- [`transport-contract.md`](./transport-contract.md) — the git family's messages, payloads and
  record types
- [`../features/workbench.md`](../features/workbench.md) — the Git screen, the explorer's badges and
  the status bar's readout
- [`architecture.md`](./architecture.md) — the two halves, and the second `git2` reader in the file
  family
- [`decisions.md`](./decisions.md) — `D9`, `D30`, `D43`
- [`../backlog.md`](../backlog.md) — every row cited above
