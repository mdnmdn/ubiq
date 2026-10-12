---
id: inbox-worktree
title: Proposal — worktrees as linked projects, for a mission, a task or nothing at all
kind: inbox
status: proposal
summary: A git worktree becomes a linked project — a catalogue record with a `worktree` link to its parent project, a branch and an optional binding to a mission or a task — so the explorer, search, the Git screen, terminals and agents all root there through the `ProjectId` they already key on, with no new rooting concept on the wire. The host owns every git worktree operation (create from a new or existing branch, adopt from `git worktree list`, remove, prune), worktrees live outside the project folder by default, the parent's work (tasks, missions, plans) is shared, the titlebar gets a worktree switcher and every pane a branch chip, and a mission or task can provision one automatically. Merge-back is surfaced, never performed. Every fork is a numbered decision with a recommendation.
read_when: you are designing how Ubiq creates, opens or removes a git worktree, how an agent is started on a branch other than the project's checkout, or how a mission or task gets an isolated working tree
updated: 2026-10-12
depends_on: [feat-workbench, feat-workbench-git, feat-workbench-agents, feat-sessions, tech-version-control, tech-transport, tech-architecture, tech-agent-manager, inbox-mission, inbox-agent-graph-final, inbox-isolation]
---

# Proposal — worktrees as linked projects

**A proposal to work on, not a settled design. Nothing here is built.** Every fork is a numbered
decision **W1–W12** with a recommendation; §9 lists the ones the user has to confirm. A decision
that lands takes a `Dnn` row.

> A worktree is a project that happens to share a repository with another one. Everything in Ubiq
> that roots on a project — the explorer, search, the Git screen, a terminal, an agent — then roots
> on the worktree without learning a new word.

## 1. What the tree does today

| Fact | Where |
|---|---|
| Every rooted family keys on `ProjectId` + a project-relative `rel_path`; the host resolves against the record's `path` | `ubiq-files` skill, `crates/ubiq-host/src/files/path.rs` |
| A pane's and a conversation's working directory **is its project's** — `SpawnWorkspace` and `StartConversation` name a `project_id` and an optional `rel_path`, never a folder | [`../features/sessions-and-workspaces.md`](../features/sessions-and-workspaces.md) |
| A window holds several projects (`OpenProject` each), switching is a lookup, and an unseen project's terminals keep running; a project is open in one window at a time | [`../features/workbench.md`](../features/workbench.md) `## What a window owns` |
| `WorkSession { id, name, branch, worktree: bool }` is on the wire and drawn — the Agents sidebar labels a session "worktree", an agent column reads `identity · branch · worktree`, the task detail prints `(worktree)` — but the host always sends `branch: ""`, `worktree: false`, and the other sessions come from `work/mock.rs` | `crates/ubiq-proto/src/work.rs`, `crates/ubiq-host/src/coordinator.rs`, `crates/ubiq/src/ui/agents/{sidebar,column}.rs`, `ui/board/detail.rs` |
| The sink mocks a "Sessions & worktrees" settings section with a checkbox wired to nothing | `crates/ubiq/src/ui/sink/settings.rs` |
| The nested-repo walk recognises a linked worktree's gitlink file, but the overview reads it as if it were the only repository, and its real `HEAD`/index under `<common>/.git/worktrees/<name>/` sit outside the watched root (`G125`) | [`../tech/version-control.md`](../tech/version-control.md) §6 |
| Ubiq writes nothing inside a project's folder but its reserved workarea (`D30`) | [`../tech/decisions.md`](../tech/decisions.md) |
| `agent-graph-final` already sketched a "workspace = folder + branch + worktree" with a `may_create_workspace` grant and a `workspace.create` tool, phase 11, never built | [`./backlog/agent-graph-final.md`](./backlog/agent-graph-final.md) §2, §16 |

So the visible half of "an agent works in a worktree" exists and lies; the mechanism does not.
The host uses `git2` 0.21, which has `Repository::worktree`, `worktrees()`, `find_worktree`,
`Worktree::{validate, is_prunable, prune, lock}` — nothing new to link.

## 2. The concept

A **worktree** (user-facing word; see W2 for why not "workspace") is a scope bound to one git
linked worktree — a folder plus the branch checked out in it — belonging to one project. Its
explorer, search, Git screen, terminals and agents all root in that folder. It has one of three
origins, which only change who created it and what it is labelled with:

| Origin | Created by | Bound to | Typical branch |
|---|---|---|---|
| Free-standing | The user, from the worktree switcher | nothing | anything the user types or picks |
| Task | The user from a board card, or the mission runner | one `TaskId` | `ubiq/<task-key>-<slug>` |
| Mission | The mission's settings, at Start mission | the mission's parent `TaskId` | `ubiq/<mission-slug>` |

The project's own checkout is the **main worktree** and is never one of these records; it is the
parent.

### W1 — A worktree is a linked project record, not a new entity and not a Ubiq session

**Recommended: a `ProjectRecord` with an optional `worktree: Option<WorktreeLink>`.**

```rust
pub struct WorktreeLink {
    pub parent: ProjectId,          // the project whose repository this is a worktree of
    pub name: String,               // git's worktree name (the admin dir under .git/worktrees)
    pub branch: Option<String>,     // None = detached HEAD
    pub base: Option<String>,       // the ref it was branched from, for ahead/behind and hand-off
    pub origin: WorktreeOrigin,     // Created (Ubiq ran `worktree add`) | Adopted (it was already there)
    pub binding: Option<WorktreeBinding>, // Mission(TaskId) | Task(TaskId)
}
```

Why this and not the alternatives:

| Option | Cost | Verdict |
|---|---|---|
| **Linked project record** | One additive field on the catalogue record (the `runs_on` precedent: no version bump). Explorer, search, index, watch, git, spawn, conversations, view state, `Missing`/Locate already work per `ProjectId` | **Chosen** |
| New `WorkspaceId` inside a project | Every rooted message (`ProjectTree`, `ReadFile`, `EditProjectPath`, `SearchProject`, every `Git*`, `SpawnWorkspace`, `StartConversation`, watch) grows a field; `OpenProject` grows a per-workspace sub-state that is exactly a second `OpenProject` | Rejected — rebuilds the project inside the project |
| Extend Ubiq's session | A session has a folder, but a pane's cwd is deliberately its *project's*, not its session's, and nothing roots files or git on a session; sessions are still mocks | Rejected — would invert a settled rule for no gain |
| `rel_path` into the main checkout | Worktree would live inside the project: breaks `D30`, and the nested walk would merge its status into the parent's map | Rejected |

What the link changes, and nothing else: the picker nests a worktree under its parent rather than
listing it top-level; the work families resolve to the parent (W6); the git worker knows the
common dir (W9); and `WorkSession` finally carries the truth (W8).

### W2 — The word

"Workspace" is spent: the glossary's workspace is one running agent, and `SpawnWorkspace` says so
on the wire. **Recommended: "worktree" everywhere** — UI, proto, code. It is the git word, users
already know it, and it is what the existing labels already print.

## 3. Lifecycle

### W3 — Creation has three entries

| Entry | Git operation (host, `git2`) | Refused when |
|---|---|---|
| **New branch** from a base (default: the parent's current branch) | `branch(name, base_commit)` then `worktree(name, path, reference)` | branch exists; path exists and is non-empty |
| **Existing branch** | `worktree(name, path, reference)` | branch already checked out in another worktree (git's own rule) |
| **Adopt** a worktree from `git worktree list` | none — a record is written with `origin: Adopted` | already linked to a record |

Adopt is also how worktrees made by hand, by Claude Code (`.claude/worktrees/`) or by another tool
become visible: the switcher lists every worktree git knows about, linked or not, and an unlinked
row offers **Open** (which adopts). Creation opens the new record in the asking window by default.

### W4 — Where worktrees live

**Recommended default: beside the repository, `<repo-parent>/<repo-name>.worktrees/<slug>`**, where
`<slug>` is the branch with `/` turned into `-`. Configurable once in host settings
(`worktree_root`: `Sibling` | `ConfigRoot` | `Custom(template)`) with a per-project override.

- Outside the project folder: `D30` holds and the nested walk never sees it.
- Visible and conventional: an IDE, a shell or `git worktree list` finds it where a user looks.
- `ConfigRoot` (`<config root>/worktrees/<ProjectId>/<slug>`) is the option for a read-only parent
  folder or a user who wants nothing next to their repos.
- On a drone the host resolves the same rule on its own machine; the UI never sees the choice made.

### W5 — Close, remove, forget, prune

| Action | Effect | Guard |
|---|---|---|
| **Close** (window's project row) | Drops it from the window, exactly as for a project | Existing "terminals running" question |
| **Remove** | `git worktree remove` + drops the record and its view state | Refused while any pane or conversation runs in it. Dirty tree → confirm that names the count of changed and untracked files. `origin: Adopted` → the confirm says Ubiq did not create it |
| **Forget** | Drops the record only; the folder and branch stay (the default for an adopted one) | none — it deletes nothing |
| **Prune** | `Worktree::prune` for every worktree git reports prunable (folder gone, not locked) | A record whose folder vanished goes `Missing` first, like a project, with Locate / Forget; prune is the explicit sweep |

**Branch policy on remove: keep by default.** The remove dialog offers *Delete branch if merged into
`<base>`* (checked when it is merged), and never force-deletes an unmerged branch without a second
explicit tick. A worktree bound to a task or mission that is still open asks once more.

### W6 — Work is shared with the parent

Tasks, boards, missions, plans, the KB and the run history are the **project's**, not a checkout's.
The host resolves every work-family message from a worktree through `Projects::work_home(id)` to its
parent, answers under the id that asked, and broadcasts changes to every window holding either.
Per-checkout state stays per record: explorer tree, open files, panes, Git screen, view prefs, index.
`search_excludes`, `tools`, `managed_repos` and `lanes` are copied from the parent at creation and
then the worktree's own.

### Merge-back is a hand-off, not a tool

When a bound task reaches `in review`, a mission reaches Review, or the user asks, Ubiq shows one
row: `feat/x · 4 ahead, 1 behind main · clean`. Its actions are what already exists — **Open in
Git** (the parent's Git screen with the branch selected in refs, where `Merge` already lives),
**Copy branch name**, **Push** (the existing remote write), and the project's web link for opening a
pull request. No merge UI, no conflict UI beyond what the Git screen already has.

## 4. UI surface

**The worktree switcher sits in the titlebar, after the project's name**: a chip `⎇ main` for the
main checkout, `⎇ feat/auth` for a worktree. Its menu lists the main checkout first, then linked
records (bound ones carry the mission or task title in faint text), then git's unlinked worktrees
under *Not opened*, then **New worktree…** and **Prune…**. Picking a row points the window at that
record — a project switch, so it is a lookup and the other checkout's terminals keep running.

| Surface | Change |
|---|---|
| Rail project badges | A worktree's badge uses the parent's colour and initials with a small branch mark in its corner; tooltip `project · branch` |
| Project picker | Worktrees nest under their parent, indented, with branch and binding; never top-level |
| Explorer, search, file picker, Git screen | Nothing — they already follow the active `ProjectId` |
| New terminal / new agent | Nothing — they spawn into the active project, which is now the worktree |
| Pane and chat tab | A branch chip in the pane's title row when its project is a worktree, so a pane moved or detached still says where it runs |
| Agents / Teams modes on the parent | List agents of the parent **and** its worktrees, grouped by `WorkSession` = worktree, with the branch label the column already draws |
| Board card / task detail | **Create worktree** and **Open worktree** actions; the existing `(worktree)` mark becomes the branch name |
| Mission panel | The mission's worktree chip, Open, and the merge-back row at Review |
| New worktree dialog | Branch: new (name + base picker) / existing (picker over local and remote branches) / adopt; location preview read-only; bind to: none / task / mission; open here or in a new window |

Two windows on two worktrees of one project are the side-by-side case and need nothing new: a
record is open in one window at a time, and these are different records.

## 5. Agents

**Spawning into a worktree is spawning into its project.** `SpawnWorkspace` and `StartConversation`
already name a `project_id`; passing the worktree's id is the whole change. `agent.rs` hands the
record's path to `RunFlags` as the folder, as it does today.

### W7 — Missions and tasks provision worktrees

The mission's settings gain `worktree: None | PerMission | PerTask` (default `None`; the dialog
suggests `PerMission`).

- `PerMission`: at **Start mission** the host creates `ubiq/<mission-slug>` from the parent's current
  branch, bound to the mission. The planner and every worker launch there.
- `PerTask`: each worker gets its own worktree branched from the mission's branch (or the base when
  there is none), bound to its task; a reviewer launches in the task it reviews. Useful only with
  `parallel > 1`. Merging the task branches back is the hand-off row on each task, done by the user
  or by a later task assigned to an agent that runs `git merge` itself.
- The runner's launch request gains a host-chosen `project: ProjectId` — the worktree, or the parent
  when `None` — and the window starts the conversation there. Creation happens host-side before the
  launch is sent; a failure becomes a `LaunchFailed` comment, as any launch failure does.
- A task-bound worktree is never reused for another task while dirty.

A board card's **Create worktree** is the same `PerTask` path started by hand.

### W8 — `WorkSession` tells the truth

The host fills `WorkSession` for an agent from its project record: `name` = the worktree's name or
the project's, `branch` = the link's branch (or the main checkout's `HEAD` name), `worktree` = the
record is linked. No proto change: the field exists and three screens already draw it.

### MCP

- **`project-info`** (already in every worker's set) adds to `project_info`: `worktree` (bool),
  `branch`, `base`, `bound_to` (task or mission id). An agent learns where it is without a new tool.
- **A `worktrees` tool group, later**: `list_worktrees()` for anyone with `project-info`, and
  `create_worktree(branch, base?, bind_task?)` only for a definition granted `may_create_worktree`
  (the `agents-definitions` grant, default off). It returns the new record's id; it never spawns into
  it — spawning stays the runner's or the window's, consistent with the simplified mission MCP having
  dropped `spawn_agent`.

## 6. Architecture

| Concern | Owner |
|---|---|
| `git worktree add / list / remove / prune`, branch create/delete, ahead/behind, dirty count | `crates/ubiq-host/src/git/worktree.rs`, on the git worker thread — never the coordinator's |
| The record, `work_home`, nesting in snapshots, location policy | `crates/ubiq-host/src/projects.rs` |
| Wire types and messages | `crates/ubiq-proto/src/projects.rs` (`WorktreeLink`), `messages.rs` |
| Switcher, chips, dialogs, picker nesting | `crates/ubiq` |
| Harness launch | `crates/agent-manager`, unchanged — it is handed a folder, as now |

The UI holds a `ProjectId` and display strings (branch, name, the record's `path` as a label, as
projects already do). It never opens, composes or passes a path; a drone answers the same messages
from its own disk.

### W9 — The git worker learns about the common dir (closes `G125`'s worktree half)

A linked worktree's `HEAD`, index and reflog live in `<common>/.git/worktrees/<name>/`, outside the
record's root. The git worker opens it with `Repository::open` (git2 follows the gitlink), reads the
overview from that repository, and the watch adds a second, non-recursive watch on that admin dir so
a commit by an agent refreshes the Git screen. The nested walk keeps treating a gitlink *inside* a
project as before.

### W10 — A confined agent needs the common dir

An isolated run is granted its project folder. In a worktree, `git commit` writes objects and refs
into the parent's `.git` — outside the grant — and fails inside the harness, in prose Ubiq did not
write. `agent.rs` adds the common git dir as a read-write grant through the existing
`IsolateOptions` extra-grant path when the record is a worktree. One line of policy, and the
difference between isolation and worktrees working together or not at all.

### Messages (sketch, not final)

| Message | Direction | Carries |
|---|---|---|
| `ListWorktrees { project_id }` | UI → host | any record of the repository; answers for the whole repository |
| `Worktrees { project_id, rows: Vec<WorktreeRow> }` | host → asker | `name`, `branch?`, `head`, `linked: Option<ProjectId>`, `locked`, `prunable`, `dirty: Option<u32>`, `ahead_behind: Option<(u32, u32)>` |
| `CreateWorktree { project_id, branch: BranchSpec, binding?, open: bool }` | UI → host | `BranchSpec::New { name, base }` \| `Existing { name }` |
| `AdoptWorktree { project_id, name }` | UI → host | — |
| `RemoveWorktree { project_id, branch: BranchDelete, force_dirty: bool }` | UI → host | `BranchDelete::Keep` \| `IfMerged` \| `Force` |
| `PruneWorktrees { project_id }` | UI → host | — |
| `WorktreeCreated { project_id, worktree: ProjectId }` | host → asker | then the catalogue snapshot broadcasts the new record as for any project |
| `WorktreeError { project_id, error: WorktreeError }` | host → asker | `BranchExists`, `CheckedOutElsewhere`, `PathOccupied`, `Dirty(u32)`, `Live`, `Git(String)` |

All carry `project_id`, so they route like the project family. Binding changes ride
`UpdateProject`, the way every other record field does.

### Persistence

The record and its link live in the catalogue (`projects.toml`), additive, no version bump. View
state lives under `<config root>/projects/<worktree ProjectId>/` like any project's and goes with
Remove or Forget. The binding's other side is **not** written on the task: the task finds its
worktree by asking the catalogue, so there is one owner. Git is the authority on the worktree itself;
on load the host reconciles each record with `git worktree list` and marks a vanished one `Missing`.

## 7. Phases

| # | Slice | Delivers |
|---|---|---|
| **1 (MVP)** | Free-standing worktrees | `WorktreeLink`, `ListWorktrees`/`CreateWorktree`/`AdoptWorktree`/`RemoveWorktree`, the titlebar switcher and New worktree dialog, picker nesting, W4 default location, W6 shared work, W8 truthful `WorkSession`, W9 git overview and watch. Explorer, search, Git screen, terminals and agents follow for free |
| 2 | Identification and safety | Pane/tab branch chip, rail badge mark, dirty and live guards, branch-delete policy, Prune, W10 isolation grant, `project_info` fields |
| 3 | Tasks | Board card Create/Open worktree, task binding, merge-back row on the card and task detail |
| 4 | Missions | `worktree: PerMission` setting, launch request carries the target project, mission panel chip and Review hand-off |
| 5 | Parallel and agent-driven | `PerTask` provisioning, `list_worktrees` / `create_worktree` MCP behind `may_create_worktree` |

Phase 1 is shippable alone and is most of the user's value: "open this branch next to main and run
agents on it".

## 8. Costs and hazards

- **Disk and build time.** Each worktree is a full checkout plus its own `target/`, `node_modules/`;
  a Rust project's first build in a worktree is a cold one. Ubiq says nothing about that, and the
  remove dialog showing the folder's size is a cheap kindness worth doing in phase 2.
- **One branch, one worktree.** Git refuses to check out a branch twice; `CheckedOutElsewhere` names
  the worktree that holds it and offers to switch to it.
- **Index and search per worktree.** Every worktree gets its own index under its own id; the
  project's `index_level` applies, so a user with ten worktrees pays ten indexes. Acceptable;
  indexing level is already a project setting.
- **Hooks and submodules.** `git worktree add` runs no checkout hooks through `git2`, and submodules
  are not initialised in the new tree. Said in the dialog's note, not fixed.

## 9. Decisions to confirm

| # | Decision | Recommendation |
|---|---|---|
| W1 | Entity | A linked `ProjectRecord` (`worktree: Option<WorktreeLink>`), not a new `WorkspaceId`, not a session extension |
| W2 | Name | "Worktree" in UI, proto and code; "workspace" stays the glossary's running agent |
| W3 | Creation entries | New branch from base, existing branch, adopt from `git worktree list`; open in the asking window |
| W4 | Default location | `<repo-parent>/<repo-name>.worktrees/<branch-slug>`, host setting `Sibling`/`ConfigRoot`/`Custom` with per-project override |
| W5 | Branch on remove | Keep by default; offer delete-if-merged; force only with a second explicit tick |
| W6 | Work sharing | Tasks, missions, plans, KB shared with the parent through `work_home`; explorer, panes, Git screen, view prefs, index per worktree |
| W7 | Mission/task provisioning | Mission setting `None`/`PerMission`/`PerTask`, default `None`, dialog suggests `PerMission`; `PerTask` only in phase 5 |
| W8 | `WorkSession` | Filled from the record — no proto change |
| W9 | Git overview / watch | Open via gitlink, watch the worktree's admin dir; closes `G125`'s worktree half |
| W10 | Isolation | Grant the common git dir read-write to a confined run in a worktree |
| W11 | Git engine | `git2`, on the git worker, no `git` CLI subprocess — consistent with every other repository write |
| W12 | Agent MCP | `project_info` gains worktree facts in phase 2; `create_worktree` only behind `may_create_worktree` (default off), never spawns |

## Related docs

- [`../features/workbench.md`](../features/workbench.md) — the window's projects, the picker, the titlebar the switcher joins.
- [`../features/workbench-git.md`](../features/workbench-git.md) and [`../tech/version-control.md`](../tech/version-control.md) — the git worker, the nested walk and `G125`.
- [`../features/sessions-and-workspaces.md`](../features/sessions-and-workspaces.md) — why a pane's cwd is its project's.
- [`../features/workbench-agents.md`](../features/workbench-agents.md) — the sidebar and columns that already draw branch and worktree.
- [`../tech/transport-contract.md`](../tech/transport-contract.md) — where the new family and `WorkSession` land.
- [`./mission-proposal.md`](./mission-proposal.md) — the mission this provisions worktrees for.
- [`./isolation-proposal.md`](./isolation-proposal.md) — the confinement W10 has to grant past.
- [`./agents-definitions.md`](./agents-definitions.md) — the `may_create_workspace` grant W12 renames and uses.
- [`./backlog/agent-graph-final.md`](./backlog/agent-graph-final.md) — the earlier workspace-as-worktree sketch this replaces.
