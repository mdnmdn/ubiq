---
id: inbox-git
title: Proposal — version control, read-only
kind: proposal
status: proposal
summary: Git as something the host observes and the interface draws — repository discovery through worktrees and submodules, a working-tree map proportional to the change set rather than the tree, a bounded commit log, and the message family that carries all three. Nothing writes.
read_when: you are deciding how Ubiq learns what git knows, where that work runs, or what the explorer and the status bar are actually reading
updated: 2026-09-01
depends_on: [tech-architecture, tech-transport, feat-workbench]
---

# Proposal — version control, read-only

The workbench already draws version control: the explorer tints and badges every row with a git
state, each editor tab's dot carries one, and the status bar prints a branch with ahead and behind
counts beside the working tree's totals. None of it is true. This proposes making it true, in the
host, behind the message set, using gitoxide — and **only reading**. Staging, committing, branching
and fetching are a later proposal that this one is deliberately shaped to accept.

## 1. Where it stands

`crates/ubiq/src/state/explorer.rs` defines `GitStatus` — `Modified`, `Untracked`, `Conflict`,
`Staged`, `Ignored` — with a badge letter each, and `FileNode::git` is an `Option<GitStatus>` whose
header already says the right thing: **`None` is not "clean"**, it is "nothing has been read".
`crates/ubiq/src/state/workbench.rs` holds `branches`, `branch`, `ahead`, `behind`, `modified`,
`untracked` and `conflicts`, and `crates/ubiq/src/ui/status_bar.rs` prints them. Every one of those
values is a literal in `crates/ubiq/src/state/sample.rs`, including the branch names. This is `G9`
in [`../backlog.md`](../backlog.md), and phase 8 of
[`project-handling-proposal.md`](./project-handling-proposal.md) is the row this document expands.

Nothing else exists. No crate declares gitoxide, `crates/ubiq-host/src/lib.rs` has no version-control
module, and no message in `crates/ubiq-proto/src/messages.rs` mentions a repository.

Two things about the current shape are worth keeping. The interface's status enum is close to right
and only needs a stated projection rule behind it, and `Option` on the row is exactly the
distinction a real implementation needs — "no repository here" and "clean" must never draw the same.

## 2. Read-only, and what that buys

**Ubiq observes the repository and never modifies it.** No stage, no commit, no branch, no checkout,
no fetch. The user's own agents run `git` in their panes, and they are the ones changing the tree;
Ubiq's job in this version is to say what happened.

Three payoffs, and they are why the line is worth drawing here rather than halfway into a write API.
**`D30` survives literally** — Ubiq writes nothing inside a project's folder, and every question
about locks, concurrent index writes and a half-finished operation is simply absent. **There is no
confirmation surface to design** — a destructive action needs undo, a dialog, an error path and a
story for the agent in pane two who is mid-rebase, and none of that stands between here and an
honest status bar. **The read model is what a write model needs anyway** — discovery, the repository
cache, the worker, the refresh discipline and the staleness generation are all required by writes
too, and a write becomes one more request on the same worker, ending by bumping the generation.

One consequence has to be stated because gitoxide offers the opposite by default. A `gix` status
computes stat information it would like to write back into the index, and
`status::Outcome::write_changes()` is how you give it to it. **Ubiq never calls it.** The cost is
real: every status pays to re-hash files whose stat cache would have been refreshed, so a large
repository is slower than `git status` on the second run. It is paid on purpose, because the
alternative is Ubiq touching `.git/index` behind a user whose agent may be holding it.

## 3. What a repository is, for Ubiq

A project is a folder. A folder and a repository are not the same thing, and the gap between them is
most of the difficulty. Six cases, all of which occur in the projects Ubiq is for.

| Case | What is at the project's path | What Ubiq says |
|---|---|---|
| No repository | An ordinary folder, nothing above it | Not a repository. No error, no rows marked |
| Repository root | `.git/` directory beside the files | The ordinary case |
| Inside a repository | The repository root is some way above the project | The repository is named, and the project's root is a prefix inside it |
| Linked worktree | `.git` is a *file* pointing at `…/worktrees/<name>` | A worktree of a repository, named, with its siblings listed |
| Repositories below | A monorepo of independent checkouts, or submodules | Named as nested repositories; **never merged into the outer one** |
| Bare | No working tree at all | A repository with no working-tree status to give |

Three decisions come out of that table.

**One repository is the project's repository: the one that contains the project's root.** Found by
discovery upwards from the project's path, exactly as `git` itself would. Everything the status bar
says — branch, ahead, behind, totals — is that repository's and nothing else's. Blending two
repositories' counts into one number is a lie the user cannot unpick, and there is no second place
on screen to put a second branch.

**A project inside a repository is scoped to its own subtree.** With the repository root above the
project's path, the project's prefix is recorded, the map is filtered to paths under it, and the
totals count only those. The branch is still the repository's — there is one HEAD — and the status
bar says the project is part of a larger repository, so the numbers are never read as the whole
repository's.

**Nested repositories are listed, not merged.** A submodule and an independent checkout below the
project's root are the same thing to the outer repository — a directory it does not look inside —
and Ubiq says the same about both: here is a repository, its HEAD, and whether it is dirty. Neither
is walked into to produce the outer project's counts, which is what `git status` does.

**"Subremotes" resolve into two separate facts**, and conflating them is a trap worth naming. A
repository's **remotes** are named URLs on one repository — `origin`, a fork, a mirror — and there
may be several. A **submodule** is a different repository, with its own remotes, pinned at a commit
by the outer one. The overview carries both lists, and never flattens one into the other.

## 4. The three shapes

Version control produces three things at three different rates, and giving them one message would
force the expensive one's cadence on the cheap ones.

### The overview — cheap, whole, refreshed often

What the status bar and the titlebar read. Everything here comes from reading refs and a handful of
files in the git directory; no tree is walked.

| Field | Meaning |
|---|---|
| `repo_root` | The repository root **relative to the project**, or the project's path relative to the repository root when the repository is above. One of the two is empty |
| `scoped_to` | The project's prefix inside the repository. Empty when the project *is* the root |
| `head` | `Branch(name)`, `Detached { short_id, describes }`, or `Unborn(name)` — a fresh `git init` has a branch name and no commit, and drawing it as detached would be wrong |
| `upstream` | The remote-tracking ref the branch is configured against, absent when there is none |
| `ahead`, `behind` | Commits either side of the merge base with `upstream`. Absent, not zero, when there is no upstream |
| `operation` | `Merge`, `Rebase`, `RebaseInteractive`, `CherryPick`, `Revert`, `Bisect`, `ApplyMailbox` — or none. A repository mid-rebase is the single most useful thing a status bar can say |
| `remotes` | Name and URL each, with which is the fetch default |
| `worktree` | `Main`, or `Linked { name }` when the project sits in a linked worktree |
| `worktrees` | The repository's other worktrees: name, whether locked, and whether one of them is a project Ubiq already knows |
| `submodules` | Name, path relative to the project, URL, and `Active`/`Uninitialised`/`Dirty`/`Clean` |
| `nested` | Repositories below the project that are not submodules, by relative path |
| `is_bare` | No working tree. The counts below are absent rather than zero |
| `counts` | Staged, modified, untracked and conflicted totals for the project's scope |
| `generation` | See §7 |

`ahead` and `behind` deserve a note: gitoxide has no counting helper. They are two revision walks
from the merge base — `Repository::merge_base` then `rev_walk` — and **capped**, so a repository
whose upstream is a year behind draws `99+` rather than walking to the root.

### The working-tree map — expensive, whole-repository, refreshed on a signal

What the explorer's badges and the editor tabs' dots read.

**The map carries only paths that have something to say.** A row not in the map is clean. That is
the property that makes the payload proportional to the change set rather than to the repository,
and it is what makes this affordable on a tree with a hundred thousand files.

An entry is a **pair**, not a single state:

| Field | Meaning |
|---|---|
| `rel_path` | Project-relative, forward-slashed, on the same discipline as the file family |
| `index` | How the index differs from HEAD: `Added`, `Modified`, `Deleted`, `Renamed { from }`, `TypeChange`, or nothing |
| `worktree` | How the worktree differs from the index: the same set, plus `Untracked` |
| `conflicted` | The path has unmerged stages. Overrides both sides for display |
| `ignored` | Set only on the collapsed directory entries described below |

**The pair is the model; the interface's single `GitStatus` is a projection of it**, and the
projection rule is stated once so that two windows cannot disagree:

> conflicted → `Conflict`; else worktree `Untracked` → `Untracked`; else worktree change present →
> `Modified`; else index change present → `Staged`; else ignored → `Ignored`.

A file both staged and modified draws as `Modified`: the unstaged edit is the newer fact and the one
about to be lost track of. Keeping the pair on the wire rather than the projection is what lets a
later two-column view — the one every git UI eventually grows — arrive without a contract change.

**Directories get a rollup.** The explorer expands one level at a time and cannot derive a folder's
badge from children it has not asked for, so the host sends, alongside the file entries, the status
of every ancestor directory of every changed path. Rolling up is the same projection applied to the
children's worst case, and the host does it because two windows must not compute it differently.

**Ignored paths are collapsed directories, and that is the whole trick.** A repository's ignored set
is unbounded — `target/`, `node_modules/`, `.venv/` — and enumerating it would dwarf everything else
on the wire. gitoxide's directory walk emits ignored entries in `CollapseDirectory` mode, yielding
`target/` as *one* entry instead of forty thousand, and the same for untracked directories. A row is
ignored when it or any ancestor is one of those entries, so expanding into an ignored directory
inherits the state with no further request and no per-row check. The fixed `NOT_DESCENDED` set in
`crates/ubiq-host/src/files/mod.rs` stays what it is — a depth bound, not a hiding rule.

The map is bounded like everything else in the host: past an entry ceiling it is marked truncated,
and the interface says the working tree has more changes than it is drawing rather than drawing a
number that is wrong. A rebase across ten thousand files is the case this exists for.

### The log — bounded, paged, asked for

What a history view reads. Not drawn today, and the family carries it because the walk is the same
walk and designing it later would mean revisiting the worker.

| Field | Meaning |
|---|---|
| `id`, `short_id` | The commit, and the abbreviation the repository's own config would use |
| `summary` | The first line of the message. The body is fetched per commit, not per page |
| `author` | Name, email, and time with its offset — the committer's time is carried separately, because they differ after a rebase and the difference is the point |
| `parents` | Parent count, so a merge is drawn as one without a second request |
| `refs` | Branch and tag names pointing at this commit, for the decorations |

Paged with a cursor rather than an offset: a page is a bounded walk from a starting commit, and the
next cursor is the commit after the last returned. Offsets would re-walk from HEAD every page and be
wrong the moment the tree moved underneath. A page filters by path — which is what "history of this
file" is — and by `first_parent`.

Branches and tags are a fourth, small shape: name, kind, target, and — **only when asked for** —
ahead and behind against each one's upstream. That last part is `n` merge-base computations, so it
is a separate request and never rides along with the branch list the picker needs.

## 5. Where the work runs

**On its own thread, never the coordinator's** — the rule
`crates/ubiq-host/src/files/mod.rs` already states in its header, and version control is the sharper
case: a cold status on a large repository is seconds, and seconds on the coordinator's thread is
every pane's keystrokes stalled behind it. The shape `files::Files` proved is copied rather than
reinvented: a `crates/ubiq-host/src/git/` module, one worker thread, a `Job { project_id, root,
request, reply_to: Mailbox }`, and a coordinator that looks the record up in memory, submits, and
answers nothing itself.

Three things are different enough to state.

**Repositories are opened once and cached.** Discovery walks upward through the filesystem and
opening reads configuration; doing it per request would put that cost on every expand. The worker
keeps a `gix::ThreadSafeRepository` per project and takes a thread-local handle per job. The cache
is invalidated by `LocateProject`, `ForgetProject`, and a refresh that finds the repository gone.

**A superseded status is interrupted, not queued behind.** gitoxide's status platform takes a
`should_interrupt` flag, which is exactly what it is for: a second refresh for a project whose
status is still running cancels the first. Without it, saving three files in a monorepo queues three
full walks, two of them answers nobody will look at.

**Cheap requests do not wait behind expensive ones.** An overview is milliseconds and a status can
be seconds, so one strict queue would make the branch name arrive after the badges. Two queues on
the one thread — overviews and log pages ahead of working-tree walks — is enough, and is cheaper
than a pool, which would reorder replies and cost a sequence number on the wire. The generation in
§7 already covers the only reordering that matters.

## 6. The message family

A fourth family, alongside pane, session, project and file. Every variant names a project, because
the interface holds no repository identity of its own — a repository is a fact *about* a project,
discovered by the host.

| Message | Direction | Payload | Answers |
|---|---|---|---|
| `ProjectGit` | UI → host | `project_id` | `GitOverview` or `GitError` |
| `RefreshProjectGit` | UI → host | `project_id`, `full` | `GitOverview`, and `GitWorkingTree` when `full` |
| `ProjectGitLog` | UI → host | `project_id`, `cursor?`, `count`, `rel_path?`, `first_parent` | `GitLogPage` or `GitError` |
| `ProjectGitRefs` | UI → host | `project_id`, `with_tracking` | `GitRefs` or `GitError` |
| `GitOverview` | host → UI | `project_id`, `overview: Option<RepoOverview>` | — |
| `GitWorkingTree` | host → UI | `project_id`, `generation`, `entries[]`, `rollups[]`, `truncated` | — |
| `GitLogPage` | host → UI | `project_id`, `commits[]`, `next_cursor?` | — |
| `GitRefs` | host → UI | `project_id`, `branches[]`, `tags[]` | — |
| `GitError` | host → UI | `project_id`, `error: GitError` | — |

**`overview: Option<RepoOverview>` is the load-bearing signature.** `None` means the project is not
in a repository, and that is an ordinary answer, not a failure: most of what Ubiq is opened on is a
repository, and a folder that is not one must draw as *unknown*, never as clean, and never as an
error the user has to dismiss. `GitError` is for a repository that exists and could not be read — a
corrupt index, an unreadable object database, a permission refusal.

**These are addressed to the asker, not broadcast**, unlike the project family: a project is open in
exactly one window, so a second window has nothing to be told. The family broadcasts if that rule
changes.

**No absolute path crosses.** Every path here is relative to the project, and the facts genuinely
outside its root — a linked worktree's git directory, a repository root above the project — cross as
a *name* and a *relative prefix*. That is architecture rule 2 applied to the filesystem, and it is
what keeps a detached host a transport change: a project id and a relative path do not say which
machine answered.

**`GitError` is an enum, not a sentence**, on the same reasoning that shaped `FileError`: `NotFound`
means stop asking, `Corrupt` means say so and offer nothing, `Denied` means mark it, `Interrupted`
means a newer answer is on its way and the interface should draw nothing at all.

Two rows in [`../tech/transport-contract.md`](../tech/transport-contract.md) need adding when this
lands, and one correction is owed independently: that document's tables predate the file family, so
`ProjectTree`, `ReadProjectFile`, `WriteProjectFile` and their four replies are in the code and not
in the contract. Filing that is not this proposal's job, but it is worth naming so it is not
discovered by someone trusting the table.

## 7. Refresh, and the staleness rule

Version control state goes stale the moment an agent in a pane runs a command, which is constantly.
Getting the refresh discipline right matters more than the walk being fast.

**Four triggers, in v1.** A project is opened in a window. The interface asks explicitly — the
status bar's readout is clickable. An editor save lands, which is a `WriteProjectFile` the host
already sees. And a pane exits, because a harness that just finished almost certainly changed
something.

**A watcher is not in v1** — the same shape as `G30`, where health is probed when somebody asks and
nothing watches the filesystem. Two would eventually be right: the git directory, for `HEAD`,
`MERGE_HEAD` and the index, which is a handful of files and would make the overview live for almost
nothing; and the working tree, which is expensive and needs the ignore rules to avoid drowning in
`target/`. Both are backlog rows, and the cheap one is worth taking first.

**Every reply carries a generation, and the interface discards an older one.** A monotonic counter
per project, bumped when a refresh starts, needed because an interrupted-and-re-run status is not
ordered the way the file worker's FIFO guarantees. Without it a slow walk landing after a fast one
repaints the explorer with what was true thirty seconds ago, and nothing on screen says so.

**Refreshes coalesce.** A request arriving while one runs marks the project dirty; exactly one more
starts when it finishes. An agent writing forty files produces two walks, not forty.

## 8. What the interface does with it

Small, because the drawing already exists.

`ExplorerState`'s `FileNode::git` stops being seeded from `sample.rs` and is applied from a
`GitWorkingTree`: paths in the map get a status, every other row gets `None`. The `Option` stays as
it is, and its header comment becomes true instead of aspirational.

`WorkbenchState`'s seven git fields become one `Option<RepoOverview>`. `branches` as a `Vec<String>`
with an index goes — the branch list is a `GitRefs` reply asked for when the picker opens, and the
*current* branch is in the overview. The status bar gains three readings it has never had: the
in-progress operation, that a project is scoped inside a larger repository, and that a working-tree
map was truncated. `state/sample.rs` loses its git fixtures; the tree, the file fixtures and the
chat stay until their own transports arrive.

## 9. The dependency

`gix = "0.87"` in `crates/ubiq-host` only. It never appears in `crates/ubiq-proto`, because the
protocol crate is both halves' dependency and nothing that touches disk may enter it — the wire
types here are plain records, and the `gix` types are converted at the worker's edge. It never
appears in `crates/ubiq`, and `just ui` already fails if it does.

**`default-features = false`.** The default feature set pulls in network transports, credentials
prompting and TLS, none of which a read-only local reader needs, and all of which would be dead
weight in the binary and a supply-chain surface for nothing. What is needed:
`max-performance-safe` (parallel walks and the pack cache), `index`, `excludes`, `attributes`,
`status`, `dirwalk`, `revision`, `blob-diff`, `comfort`. Networking features are the ones a *fetch*
would add, and adding them is part of the write proposal, not this one.

**Cost, stated plainly:** gitoxide is a large tree of small crates and easily the biggest dependency
Ubiq has taken outside GPUI. The alternatives are worse. `git2` binds libgit2 — a C toolchain in the
build and a threading model that is not Rust's. Shelling out to `git` means parsing porcelain, a
process per query, and assuming a binary whose version you know; it also contradicts `D9`, which
chose embedding a library over shelling out to a CLI for these reasons. gitoxide is the only one of
the three that is straightforwardly interruptible, which §5 depends on.

## 10. Failure

| What happens | Result |
|---|---|
| The project is not in a repository | `overview: None`. No badges, no branch, no error |
| The repository is above the project | The project is scoped to its prefix; counts cover that prefix only |
| The project is in a linked worktree | Drawn as that worktree by name, with its siblings listed |
| A submodule is uninitialised | Listed, marked uninitialised, not walked into |
| A repository below the project is not a submodule | Listed as nested. It contributes nothing to the outer counts |
| The repository is bare | Overview without counts; the explorer shows no badges |
| HEAD is unborn | The branch name is drawn; ahead, behind and the counts are absent |
| HEAD is detached | The short id is drawn in place of a branch name |
| A rebase or merge is in progress | The status bar says which, beside the branch |
| The branch has no upstream | Ahead and behind are absent, and the bar draws nothing rather than `0/0` |
| Ahead or behind exceeds the cap | Drawn as `99+`. The walk stops at the cap |
| The working tree has more changes than the ceiling | The map is marked truncated and the bar says so |
| The index is locked by an agent's own `git` | Read what is readable; the next refresh is the repair |
| The object database is corrupt | `GitError::Corrupt`. Badges clear rather than freeze at the last good answer |
| A status is superseded mid-walk | Interrupted; nothing is drawn from it; the newer one lands |
| A reply arrives with a stale generation | Discarded by the interface |
| The project's folder goes away | The repository cache is dropped; the existing health probe marks the row |

## 11. Rules this adds

Three, if this is taken.

**Ubiq never runs the `git` binary.** Version control is read through the embedded library, in
process. The same rule and the same reasoning as `D9`.

**Ubiq never writes into a repository.** Not the index, not a ref, not a config, not the stat cache
gitoxide offers to refresh. `D30` said Ubiq writes nothing inside a project's folder; this says the
git directory is inside it.

**The interface learns no repository path.** A repository crosses as relative prefixes and names, so
the interface cannot form an absolute path from what it is told — the file family's rule extended to
the one family that would most naturally break it.

## 12. Phases

1. **Discovery and the overview.** The `git` module, the worker, the repository cache, `ProjectGit`
   and `GitOverview`; the status bar's branch, upstream, ahead/behind and operation stop being
   fixtures. Worktrees, submodules, nested repositories and remotes are all here — all discovery,
   none of it a tree walk.
2. **The working tree.** `RefreshProjectGit` with `full`, `GitWorkingTree`, the pair model, the
   rollups, the collapsed ignored entries, the ceiling. The explorer and the editor tabs stop being
   fixtures and `sample.rs` loses its git data.
3. **Refresh and the generation.** The four triggers, coalescing, interruption, and the interface
   discarding a stale reply.
4. **Refs and the log.** `ProjectGitRefs`, `ProjectGitLog`, the cursor, path filtering. No screen
   needs these yet; once they exist the history view is a UI-only change.
5. **The git-directory watch.** `HEAD`, `MERGE_HEAD` and the index, so the overview is live without
   a tree walk. Only after 1–3 are settled.

Phases 1 and 2 are the ones that retire `G9`. Phase 4 can be skipped indefinitely without leaving
anything half-built, which is the point of it being last.

## 13. What this asks to be decided

Seven decision rows:

- Version control is read-only in this version, and Ubiq never writes into a repository — including
  the index stat cache gitoxide offers to refresh.
- Version control is read through gitoxide, embedded, with default features off. Ubiq never runs the
  `git` binary and never links libgit2.
- A project has exactly one repository — the one containing its root — and repositories nested below
  it are listed, never merged. A project inside a larger repository is scoped to its own prefix.
- The working-tree map carries only paths that have something to say; absence means clean. Ignored
  and untracked directories cross collapsed, never enumerated.
- A path's state is a pair — index against HEAD, worktree against index — and the interface's single
  status enum is a stated projection of it, computed the same way in every window.
- Version-control work runs on its own thread with a per-project repository cache, and a superseded
  status is interrupted rather than queued.
- Every version-control reply carries a per-project generation, and the interface discards a reply
  older than what it holds.

One rule amendment: `D30` is restated to say the repository is inside the project's folder, so that
"Ubiq writes nothing there" is unambiguous about `.git`.

Backlog rows left open by this: a watch on the working tree, and the ignore rules that make one
affordable; whether the explorer should offer a two-column staged/unstaged view now that the pair is
on the wire; whether a submodule should be openable as a project of its own from the picker; whether
ahead and behind are worth computing for every branch in the picker or only for HEAD; what a bounded
transport does with a working-tree map for a ten-thousand-file rebase; and the write family itself,
which is a separate proposal this one is shaped to accept without changing anything above.

## Related docs

- [`../tech/architecture.md`](../tech/architecture.md) — the two halves, and the rules §5 and §6 obey
- [`../tech/transport-contract.md`](../tech/transport-contract.md) — where the family in §6 lands
- [`../features/workbench.md`](../features/workbench.md) — the explorer, the tabs and the status bar this makes true
- [`project-handling-proposal.md`](./project-handling-proposal.md) — whose phase 8 this expands
- [`file-viewers-proposal.md`](./file-viewers-proposal.md) — the file family this borrows its worker shape from
- [`../backlog.md`](../backlog.md) — `G9`, and the rows §13 leaves open
