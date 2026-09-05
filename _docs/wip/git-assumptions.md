---
id: wip-git-assumptions
title: Version control — assumptions and considerations
kind: wip
status: current
summary: What the landed read-only git work assumes rather than verifies, and where it sits in the rest of Ubiq — three independent libgit2 readers, three independent ignore-rule readers, a staleness guard the log and refs families do not have, four untested ceilings, and a commit graph with no lane engine. The lane engine survey closes it — nothing embeddable exists, and the fix is ~80 lines behind one wire change.
read_when: you are extending version control, adding the write family, or wondering why the commit graph draws one lane
updated: 2026-09-05
depends_on: [wip-git-integration, tech-architecture, tech-decisions, feat-workbench]
---

# Version control — assumptions and considerations

Companion to [`git-integration-plan.md`](./git-integration-plan.md), which says what was built. This
says what it assumes, and where it touches the rest of the application. Every gap below is either a
row in [`../backlog.md`](../backlog.md) or a candidate for one; the numbers and citations are from
the tree as it stands.

## 1. Ubiq is not the only thing holding this repository

The domain fact that shapes everything: **the agents in the panes are the ones mutating the
repository, and Ubiq only watches.** That is `D43` and `D30` working together, and it is why the
read-only line was worth drawing. But it means every fact the git family reports is a fact about a
moment that has already passed, and the machinery that narrows that window is the feature — not the
walk being fast.

What exists: five triggers — project opened (`app/shell.rs:53`), the user asks
(`app/git.rs:22`), an editor save lands (`app/wire.rs:434`), a pane exits (`app/wire.rs:257`), and
the git-directory watch fires on `HEAD`/`MERGE_HEAD`/`index`/`refs/**` (`watch/mod.rs:130`). Full
refreshes coalesce (`git/mod.rs:127`) behind one per-project generation counter (`git/mod.rs:94`),
and the interface discards an older reply (`app/wire.rs:598`).

**What that machinery does not cover, and this is the sharpest finding in this document:**
`Request::Refs` and `Request::Log` carry **no generation and no staleness guard**. `git/mod.rs:190`
answers them unconditionally and `app/wire.rs:673` writes the reply straight in. Worse,
`app/wire.rs:692` decides replace-versus-append from `log_cursor.is_none()` alone, so a first-page
reply that lands *after* a later request already advanced the cursor is appended instead of
replacing — the history list silently doubles. A `refresh_git()` racing an in-flight page is the
ordinary way to hit this, and nothing on the wire carries the request identity that would detect it.
The generation the overview and the working tree already have is the fix; refs and log were built
without it because phase C predated phase D.

Two smaller holes in the same area. `should_interrupt` mid-walk is not built — a superseded status
runs to completion. And `GitError::Interrupted` is constructed nowhere (`G126`): a wire variant with
no producer.

## 2. Three readers of the same repository, and three readers of the same ignore rules

Version control is not the only subsystem that opens this repository, and the duplication is worth
naming before someone adds a fourth.

| Reader | Where | Caching |
|---|---|---|
| The git worker | `git/mod.rs:86` | One `Repository` per project, cached |
| The diff builder | `files/diff.rs:60` | `Repository::discover` **per request**, uncached |
| Content search | `search/mod.rs` | No git at all — its own `WalkBuilder` |

`D43` accepted the second reader knowingly (`decisions.md:640`), but it accepted a second
*comparison engine*, not a second discovery walk on every diff request. Caching there is a small
change against a cost that is paid on every file the user opens.

The ignore rules are the same story from a different angle: the git worker's `StatusOptions`, the
watch's own `Gitignore` build, and search's `WalkBuilder` each read `.gitignore` independently.
`G110` already records that search and the watch disagree; the git worker is the third party to that
disagreement and is not mentioned in the row.

## 3. What the rest of the application does not know about git

- **The project catalogue's health probe is filesystem-only** (`health.rs`, called at
  `projects.rs:94`). A corrupt `.git` probes healthy: the picker and the titlebar say the project is fine while the git worker is
  returning `GitError::Corrupt` to a different part of the screen. Two truths, no reconciliation.
- **An agent's turn and the commit it produced are unconnected facts.** `work/mod.rs` and `links.rs`
  contain no git reference. This is the most interesting thing Ubiq could know and does not: it is
  the one application in the category that watches both the agent and the repository, and it
  currently declines to join them. Not a bug — an unclaimed opportunity, and the reason to keep the
  log family even though no screen needs it yet.
- **The explorer and the status bar agree by convention, not by type.** `explorer/tree.rs::apply_git`
  is the only consumer of `GitWorkingTree`; `ui/status_bar.rs:337` the only consumer of the
  overview's head and operation. The shared `operation_label`/`capped` helpers are what keep them
  saying the same thing.

## 4. Assumed, not verified

Four ceilings, all arbitrary, none load-tested against a real large repository:

| Constant | Where | Value |
|---|---|---|
| `MAX_WORKING_TREE` | `ubiq-proto/src/git.rs:21` | 2 000 entries |
| `AHEAD_BEHIND_CAP` | `ubiq-proto/src/git.rs:18` | 99 |
| `MAX_LOG_PAGE` | `ubiq-proto/src/git.rs:207` | 200 commits |
| `PATH_SCAN_CEILING` | `git/history.rs:15` | 5 000 commits scanned |

They are honest bounds with a `truncated` story behind them, which is the right shape; what is
missing is one measurement on a repository of the size Ubiq is actually opened on.

Beyond the ceilings: `submodule_state` (`observe.rs:159`) defaults to `Clean` when
`submodule_status` errors, so a broken submodule reports as a healthy one — a silent wrong answer
rather than an absent one, and the only place in the family that does that. `mine` is `false` when
`user.email` is unset (`history.rs:228`), untested. The stash list is read through
`repo.reflog("refs/stash")` rather than `stash_foreach`, because the latter needs `&mut Repository`
and the cache holds a shared one — untested, and the first sign of §5's problem.

The 20 host tests cover host logic only. Nothing exercises the `app/wire.rs` generation gap in §1.

## 5. What a write family would break

`G84` records that fetch, pull, push, branch, stash, commit and undo are already drawn and inert
behind a `read-only` chip. When they become real:

**Survives** — project-relative addressing, per-client `reply_to` routing (`coordinator.rs:1683`),
the cheap/full queue split, and the `truncated` bound as a shape for a bounded write queue.

**Does not survive** — the shared, un-mutexed `Repository` cache (`git/mod.rs:86`), which is safe
only because nothing mutates; a stage or commit needs `&mut Repository`, which is exactly the
collision the stash reflog workaround already dodged. The two independent repository handles in §2
become a correctness hazard rather than a cost the moment one of them can write. And the missing
staleness guard in §1 stops being a display glitch and becomes a write racing a read.

The read model was the right thing to build first, and none of the above argues otherwise — but the
repository cache is the one design choice that has to change before the first write lands, and it is
better changed while nothing depends on its current shape.

## 6. One project, one window — an invariant with no second line of defence

The family is addressed to the asker rather than broadcast (`transport-contract.md:319`), which
rides on "a project is open in exactly one window". That invariant is *enforced* — opening a project
elsewhere kicks it out of its previous window (`workbench.md:2027`) — so the choice is sound. But
the worker's `State { repos, generation }` (`git/mod.rs:92`) is keyed by `ProjectId` alone with no
client awareness, so if the invariant is ever violated even transiently during that handoff, two
clients share one repository handle and one generation counter and nothing in `git/mod.rs` would
notice. Untested seam, not a known bug.

## 7. The commit graph has no lane engine

`state/git.rs:174` puts every commit at lane 0 and synthesises `merges: vec![1]` for a merge — a
placeholder with a `ponytail:` comment on it, tracked as `G123`. This was surveyed properly:

**No embeddable crate exists.** Everything crates.io offers for commit-graph layout — `git-graph`,
`serie`, `git-igitt`, `gitloom-tui`, `keifu` — is an end-user binary, not a library: none exposes a
lane-layout API over bare ids and parents, all bind a full git read internally, none pages. (Note
that `gix-chunk` and friends read git's own on-disk `commit-graph` *file*, which is a pack-level
ancestry index and an unrelated concept.) Adopting any of them means taking a dependency and still
writing the ~100 lines.

**The sibling project's engine does not solve our hard part.** `refs/rgitui/crates/rgitui_git/src/graph.rs`
is a working single-pass lane allocator with a "keep main on lane 0" heuristic and a bounded
ancestor cache — but it takes the *entire loaded commit slice* and recomputes from scratch on every
call. It has no cursor and no resumable lane state. It works because rgitui loads a growing window;
Ubiq's log is deliberately cursor-paged, which is precisely the case it does not handle.

**The algorithm is small.** Keep a `Vec<Option<CommitId>>` of lanes, each holding the parent id it
is waiting for. Per commit: claim the lane waiting for this id, or open one; replace that lane's
waiting id with the first parent; match or open a lane for each additional parent; free a lane when
nothing waits on it. That is the whole thing — 60 to 100 lines. rgitui's ancestor cache exists only
for the cosmetic main-branch heuristic, which is polish, not correctness.

**The blocker is on the wire, and it is one field.** `GitCommit.parents` is a `u32` *count*. No lane
algorithm can run on a count — matching a child to the lane its parent occupies needs parent
identity. `ubiq-proto/src/git.rs` has to carry `parents: Vec<String>`, which
`git2::Commit::parent_ids()` gives for free in `history.rs`.

**Recommendation:** write it ourselves — a pure function in a new `git/graph.rs`, sibling to
`history.rs`, taking a page of commits and returning per-row lane and edges. Host-side, because the
host computes and the interface renders, and because two windows must not lay out the same history
differently. Page continuity is an in-process lane table threaded through consecutive `log()` calls
for one walk, keyed by cursor and filters — nothing further on the wire.

Order of work: the wire field, then the function, then the renderer. Roughly 80 lines plus the
per-walk cache, and it retires `G123`.

## 8. Also true, and smaller

`workbench.md:431` says reaching the bottom of the history asks for the next page. **It does not** —
`log_cursor` and `log_done` are stored, but no call site in `ui/git/history.rs` ever requests page
two. Either the document or the code is wrong; the document is the one that is currently lying.

`G124` (a log with no `rel_path` walks the whole repository rather than the project's prefix) and
`G125` (linked worktrees and nested repositories are not accounted for — `Repository::discover`
finds the nearest `.git` and stops) are filed and unchanged by anything here.

## Related docs

- [`git-integration-plan.md`](./git-integration-plan.md) — what was built, and why on `git2`
- [`../tech/decisions.md`](../tech/decisions.md) — `D9`, `D30`, `D43`
- [`../tech/architecture.md`](../tech/architecture.md) — the two halves these rules serve
- [`../features/workbench.md`](../features/workbench.md) — the Git screen, the explorer, the status bar
- [`../backlog.md`](../backlog.md) — `G84`, `G110`, `G123`–`G126`
