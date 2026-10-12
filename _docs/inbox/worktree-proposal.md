---
id: inbox-workspace
title: Proposal — workspaces, a folder and a branch, for a mission, a task or nothing at all
kind: inbox
status: proposal
summary: A workspace is a folder plus a branch. Every project has one already — its main working directory, the default workspace — and may add more, each a git linked worktree, free-standing or bound to a mission or a task. The project owns the list; on the wire a workspace is addressed by the id the rooted families already carry, the main workspace's id being the project's own, so the explorer, search, the Git screen, terminals and agents follow the active workspace with no new field and every existing path keeps working. The host owns every git operation (create from a new or existing branch, adopt from `git worktree list`, remove, prune); linked worktrees live outside the project folder; work (tasks, missions, plans) stays the project's. A branch already checked out elsewhere is refused with git's reason and offered as a new branch or a detached head at its tip. The glossary's "workspace = one running agent" retires in favour of "agent". Merge-back is surfaced, never performed. Every fork is a numbered decision with a recommendation.
read_when: you are designing how Ubiq creates, opens or removes a workspace or git worktree, how an agent is started on a branch other than the project's checkout, or how a mission or task gets an isolated working tree
updated: 2026-10-12
depends_on: [feat-workbench, feat-workbench-git, feat-workbench-agents, feat-sessions, tech-version-control, tech-transport, tech-architecture, tech-agent-manager, inbox-mission, inbox-agent-graph-final]
---

# Proposal — workspaces

**A proposal to work on, not a settled design. Nothing here is built.** Every fork is a numbered
decision **W1–W12** with a recommendation; §9 lists them. A decision that lands takes a `Dnn` row.

**Settled with the user (2026-10-12): the concept.** A workspace is a folder plus a specific
branch. The project's main working directory is a workspace — the default one, which every project
has. Linked git worktrees are additional workspaces of the same project, free-standing or bound to a
mission or a task.

> Everything in Ubiq that roots on a project — the explorer, search, the Git screen, a terminal, an
> agent — roots on a workspace instead, and a project that never adds one sees no difference.

## 1. What the tree does today

| Fact | Where |
|---|---|
| Every rooted family keys on `ProjectId` + a project-relative `rel_path`; the host resolves against the record's `path` | `ubiq-files` skill, `crates/ubiq-host/src/files/path.rs` |
| A pane's and a conversation's working directory **is its project's** — `SpawnWorkspace` and `StartConversation` name a `project_id` and an optional `rel_path`, never a folder | [`../features/sessions-and-workspaces.md`](../features/sessions-and-workspaces.md) |
| A window holds several projects (`OpenProject` each) — explorer tree, buffers, panes, Git screen, view state — switching is a lookup, an unseen project's terminals keep running, and a project is open in one window at a time | [`../features/workbench.md`](../features/workbench.md) `## What a window owns` |
| `Missing` / Locate, view-state collection and the watch are per `ProjectId` | `crates/ubiq-host/src/projects.rs`, `gc.rs`, `watch/` |
| `WorkSession { id, name, branch, worktree: bool }` is on the wire and drawn — the Agents sidebar labels a session "worktree", an agent column reads `identity · branch · worktree`, the task detail prints `(worktree)` — but the host always sends `branch: ""`, `worktree: false`, and the other sessions come from `work/mock.rs` | `crates/ubiq-proto/src/work.rs`, `crates/ubiq-host/src/coordinator.rs`, `crates/ubiq/src/ui/agents/{sidebar,column}.rs`, `ui/board/detail.rs` |
| The glossary's **workspace** is one running agent; the wire says so (`SpawnWorkspace`, `WorkspaceSpawned`, `CloseWorkspace`, `WorkspaceInfo`). No UI string says "workspace" — the interface already says **agent** ("New agent") and **pane** | [`../product/glossary.md`](../product/glossary.md), `crates/ubiq/src/ui/new_agent.rs` |
| The nested-repo walk recognises a linked worktree's gitlink file, but the overview reads it as if it were the only repository, and its real `HEAD`/index under `<common>/.git/worktrees/<name>/` sit outside the watched root (`G125`) | [`../tech/version-control.md`](../tech/version-control.md) §6 |
| Ubiq writes nothing inside a project's folder but its reserved workarea (`D30`) | [`../tech/decisions.md`](../tech/decisions.md) |
| `agent-graph-final` sketched "workspace = folder + branch + worktree" with a `may_create_workspace` grant, phase 11, never built | [`./backlog/agent-graph-final.md`](./backlog/agent-graph-final.md) §2, §16 |

The host uses `git2` 0.21, which has `Repository::worktree`, `worktrees()`, `find_worktree`,
`Worktree::{validate, is_prunable, prune, lock}` — nothing new to link.

## 2. The concept

A **workspace** is one folder with one branch checked out, belonging to one project. Its explorer,
search, Git screen, terminals and agents all root in that folder. **Worktree** is only the git
mechanism behind a non-main workspace.

| Workspace | Folder | Created by | Bound to | Typical branch |
|---|---|---|---|---|
| **Main** (default, implicit) | the project's own path | adding the project | nothing | whatever is checked out there |
| Free-standing | a linked worktree | the user, from the switcher | nothing | anything the user types or picks |
| Task | a linked worktree | the user from a board card, or the mission runner | one `TaskId` | `ubiq/<task-key>-<slug>` |
| Mission | a linked worktree | the mission's settings, at Start mission | the mission's parent `TaskId` | `ubiq/<mission-slug>` |

The main workspace's branch is read from git, never stored: switching branch there (the Git
screen's `Checkout`) changes the main workspace's branch, which is what the user means.

### W1 — The project owns its workspaces; a workspace is addressed by the id the wire already carries

**Recommended: `ProjectRecord.workspaces: Vec<WorkspaceRecord>` for the linked ones, the main one
implicit, and every workspace addressed by an id of the `ProjectId` type — the main workspace's
being the project's own.**

```rust
pub struct WorkspaceRecord {
    pub id: ProjectId,              // minted by the host; the address every rooted message carries
    pub name: String,               // display; also git's worktree name
    pub path: String,               // host-resolved, like ProjectRecord::path — a label to the UI
    pub branch: Option<String>,     // None = detached HEAD (W5)
    pub base: Option<String>,       // the ref it was branched from, for ahead/behind and hand-off
    pub origin: WorkspaceOrigin,    // Created (Ubiq ran `worktree add`) | Adopted
    pub binding: Option<WorkspaceBinding>, // Mission(TaskId) | Task(TaskId)
}
```

The rooted families' `project_id` field then reads "the workspace's root". The host resolves it
through one function, `Projects::root(id) -> (project, path)`, and the work families through
`Projects::project_of(id)`. A project that never adds a workspace sends exactly what it sends today.

The honest weighing:

| Option | What it costs | Verdict |
|---|---|---|
| **List on the project, addressed by a `ProjectId`-typed workspace id** | One additive catalogue field (the `runs_on` precedent, no version bump); `root()` and `project_of()` in the host; the window builds one `OpenProject` per open workspace, which is what it already builds per id. Explorer, search, index, watch, git, panes, conversations, view state and `Missing`/Locate work per id unchanged | **Chosen** |
| `ProjectId` + `WorkspaceId` pair on every rooted message, main = `None` | `messages.rs` names `project_id` ~320 times; every file, search, git, watch, spawn and conversation message grows a field, and `OpenProject` grows a per-workspace sub-state that is a second `OpenProject`. Explicit, but nothing it buys the chosen option lacks | Rejected for now; renaming the field `root` is a later mechanical card if the double reading confuses |
| Each worktree a top-level project record with a parent link (this proposal's first draft) | Same engine as the chosen option, but the catalogue lies: a workspace would look like a project, and the main checkout would be the odd one out | Rejected — contradicts "the main directory is a workspace" |
| Extend Ubiq's session | A pane's cwd is deliberately its project's, not its session's; nothing roots files or git on a session; sessions are still mocks | Rejected |

Per-workspace: explorer tree, open files, panes, Git screen, view prefs, index, `Missing`.
Per-project: name, colour, tasks, boards, missions, plans, KB, run history, settings
(`search_excludes`, `tools`, `managed_repos`, `lanes` apply to every workspace of the project).

Out of scope: no sandbox, isolation, home or remote field on a workspace. They could attach to the
record later without changing its shape.

### W2 — "Workspace" is the user's word; the old meaning becomes "agent"

**Recommended:** the glossary's *Workspace — one running instance of one agent* retires, and that
meaning is called **agent** (its terminal view stays **pane**), which is what the interface already
says everywhere. "Workspace" then means only this proposal's folder-plus-branch, in UI, docs and new
code. The wire's `SpawnWorkspace`, `WorkspaceSpawned`, `CloseWorkspace` and `WorkspaceInfo` keep
their names until a separate mechanical rename card (to `SpawnAgentPane`-style names); new types in
this proposal avoid the clash by being `WorkspaceRecord`, `WorkspaceRow`, `*Workspace` verbs on the
workspace family only. The glossary edit lands with the first phase, not with this proposal.

## 3. Lifecycle

### W3 — Creating a workspace has three entries

| Entry | Git operation (host, `git2`, git worker thread) | Refused when |
|---|---|---|
| **New branch** from a base (default: the main workspace's current branch) | `branch(name, base_commit)` then `worktree(name, path, reference)` | branch exists; path exists and is non-empty |
| **Existing branch** | `worktree(name, path, reference)` | branch checked out in another workspace — see W5 |
| **Adopt** a worktree from `git worktree list` | none — a record is written with `origin: Adopted` | already a workspace |

Adopt is how worktrees made by hand or by another tool (Claude Code's `.claude/worktrees/`) become
workspaces: the switcher lists every worktree git knows about, and an unadopted row offers **Open**.
Creation opens the new workspace in the asking window.

### W4 — Where linked worktrees live

**Recommended default: beside the repository, `<repo-parent>/<repo-name>.worktrees/<slug>`**, `<slug>`
being the branch with `/` turned into `-`. One host setting (`workspace_root`: `Sibling` |
`ConfigRoot` | `Custom(template)`) with a per-project override.

- Outside the project folder: `D30` holds and the nested walk never sees it.
- Visible and conventional: an IDE, a shell or `git worktree list` finds it where a user looks.
- `ConfigRoot` (`<config root>/worktrees/<ProjectId>/<slug>`) serves a read-only parent folder.

### W5 — A branch already checked out elsewhere

Git refuses to check out one branch in two worktrees, because two indexes moving one ref would
corrupt each other's view of it. The common case is the obvious one: the user wants a workspace "on
`main`" while the main workspace is on `main`.

**Recommended: refuse, say why in one line, and offer two ways forward in the same dialog:**

1. **New branch from its tip** (default, focused): `<branch>-<slug>` prefilled, base = that branch.
   This is what the user almost always wants — work that can be committed and later merged back.
2. **Detached at its tip**: a workspace with `branch: None`, labelled `⎇ main@a1b2c3d (detached)`,
   for reading, running or reviewing. Committing there is allowed by git; the workspace's chip turns
   to the warning colour and the Git screen offers *Create branch here* so the work is not orphaned.

If the branch is held by another *linked* workspace, a third row offers **Switch to it**. Ubiq never
passes git's force flag.

### W6 — Close, remove, forget, prune

| Action | Effect | Guard |
|---|---|---|
| **Close** | Drops the workspace from the window, exactly as closing a project | Existing "terminals running" question |
| **Remove** | `git worktree remove` + drops the record and its view state | Refused while any pane or conversation runs in it. Uncommitted changes → confirm naming the count. `Adopted` → the confirm says Ubiq did not create it |
| **Forget** | Drops the record only; folder and branch stay (default for an adopted one) | none — it deletes nothing |
| **Prune** | `Worktree::prune` for every worktree git reports prunable | A workspace whose folder vanished goes `Missing` first, with Locate / Forget, as a project does |

The main workspace has none of these: it goes when the project is forgotten.

**Branch on remove: keep by default.** The dialog offers *Delete branch if merged into `<base>`*
(checked when merged) and never force-deletes an unmerged branch without a second explicit tick. A
workspace bound to an open task or mission asks once more.

### Merge-back is a hand-off, not a tool

When a bound task reaches `in review`, a mission reaches Review, or the user asks, Ubiq shows one
row: `feat/x · 4 ahead, 1 behind main · clean`. Its actions are what already exists — **Open in
Git** (the main workspace's Git screen with the branch selected in refs, where `Merge` lives),
**Copy branch name**, **Push**, and the project's web link for a pull request. No merge UI.

## 4. UI surface

**The workspace switcher sits in the titlebar, after the project's name**: a chip `⎇ main` on the
main workspace, `⎇ feat/auth` on another. Its menu lists the main workspace first, then the linked
ones (bound ones carry the mission or task title, faint), then git's unadopted worktrees under *Not
opened*, then **New workspace…** and **Prune…**. Picking a row points the window at that workspace
— a lookup, so the other workspace's terminals keep running.

| Surface | Change |
|---|---|
| Rail project badges | Unchanged — one per project. On a non-main workspace the badge gets a small branch mark; tooltip `project · branch` |
| Project picker | A project row expands to its workspaces; the row's click opens the main one |
| Explorer, search, file picker, Git screen | Nothing — they already follow the active id |
| New terminal / new agent | Nothing — they spawn into the active workspace |
| Pane and chat tab | A branch chip in the title row when not on the main workspace, so a moved or detached pane still says where it runs |
| Agents / Teams modes | List agents of every workspace of the project, grouped by workspace, with the branch label the column already draws |
| Board card / task detail | **Create workspace** and **Open workspace**; the existing `(worktree)` mark becomes the branch name |
| Mission panel | The mission's workspace chip, Open, and the merge-back row at Review |
| New workspace dialog | Branch: new (name + base) / existing (local and remote branches) / adopt; W5's refusal inline; location preview read-only; bind to: none / task / mission |

Two windows on two workspaces of one project are the side-by-side case: a workspace is open in one
window at a time, and these are two workspaces.

## 5. Agents

**Spawning into a workspace is spawning with its id.** `SpawnWorkspace` and `StartConversation`
already carry `project_id`; the active workspace's id is the whole change. `agent.rs` hands
`root(id)`'s path to `RunFlags` as the folder; `agent-manager` is untouched.

### W7 — Missions and tasks provision workspaces

The mission's settings gain `workspace: Main | PerMission | PerTask` (default `Main`; the dialog
suggests `PerMission`).

- `PerMission`: at **Start mission** the host creates `ubiq/<mission-slug>` from the main
  workspace's branch, bound to the mission. The planner and every worker launch there.
- `PerTask`: each worker gets its own workspace branched from the mission's branch (or the base),
  bound to its task; a reviewer launches in the task it reviews. Useful only with `parallel > 1`.
  Merging task branches back is the hand-off row on each task.
- The runner's launch request gains a host-chosen target workspace id; the window starts the
  conversation there. Creation happens host-side before the launch; a failure is a `LaunchFailed`
  comment like any other.
- A task-bound workspace is never reused for another task while it has uncommitted changes.

A board card's **Create workspace** is the `PerTask` path started by hand.

### W8 — `WorkSession` tells the truth

The host fills `WorkSession` for an agent from the workspace it runs in: `name` = the workspace's,
`branch` = its branch (or `HEAD`'s name on main), `worktree` = not the main workspace. No proto
change: the field exists and three screens draw it.

### MCP

- **`project-info`** (in every worker's set) adds to `project_info`: `workspace` (name), `branch`,
  `base`, `main` (bool), `bound_to`. An agent learns where it is without a new tool.
- **Later**: `list_workspaces()` for anyone with `project-info`, and
  `create_workspace(branch, base?, bind_task?)` only for a definition granted `may_create_workspace`
  (`agents-definitions`, default off). It returns the id and never spawns — spawning stays the
  runner's or the window's, consistent with the mission MCP having no `spawn_agent`.

## 6. Architecture

| Concern | Owner |
|---|---|
| `worktree add / list / remove / prune`, branch create/delete, ahead/behind, change count | `crates/ubiq-host/src/git/worktree.rs`, on the git worker thread |
| The records, `root()`, `project_of()`, location policy, reconciliation with git on load | `crates/ubiq-host/src/projects.rs` |
| Wire types and messages | `crates/ubiq-proto/src/projects.rs` (`WorkspaceRecord`), `messages.rs` |
| Switcher, chips, dialogs, picker rows | `crates/ubiq` |
| Harness launch | `crates/agent-manager`, unchanged — handed a folder, as now |

The UI holds ids and display strings (branch, name, the path as a label, as projects already do). It
never opens, composes or passes a path.

### W9 — The git worker reads a linked worktree properly (closes `G125`'s worktree half)

A linked worktree's `HEAD`, index and reflog live in `<common>/.git/worktrees/<name>/`, outside its
folder. The git worker opens it with `Repository::open` (git2 follows the gitlink) and the watch adds
a non-recursive watch on that admin dir, so a commit made by an agent refreshes the Git screen.

### Messages (sketch, not final)

| Message | Direction | Carries |
|---|---|---|
| `ListWorkspaces { project_id }` | UI → host | any workspace id of the project |
| `Workspaces { project_id, rows: Vec<WorkspaceRow> }` | host → asker | `id?` (None = unadopted), `name`, `branch?`, `head`, `main`, `locked`, `prunable`, `changes: Option<u32>`, `ahead_behind: Option<(u32, u32)>` |
| `CreateWorkspace { project_id, branch: BranchSpec, binding? }` | UI → host | `BranchSpec::New { name, base }` \| `Existing { name }` \| `Detached { at }` |
| `AdoptWorkspace { project_id, name }` | UI → host | — |
| `RemoveWorkspace { project_id, branch: BranchDelete, discard_changes: bool }` | UI → host | `Keep` \| `IfMerged` \| `Force` |
| `PruneWorkspaces { project_id }` | UI → host | — |
| `WorkspaceCreated { project_id, workspace: ProjectId }` | host → asker | then the catalogue snapshot broadcasts the record |
| `WorkspaceError { project_id, error }` | host → asker | `BranchExists`, `CheckedOutIn(name)`, `PathOccupied`, `Uncommitted(u32)`, `Live`, `Git(String)` |

All carry `project_id`, so they route like the project family. A binding change rides
`UpdateProject`.

### Persistence

The linked workspaces live on their project's catalogue record (`projects.toml`), additive. View
state lives under `<config root>/projects/<workspace id>/` like a project's and goes with Remove or
Forget. The binding is **not** written on the task: the task finds its workspace by asking the
catalogue, one owner. Git is the authority on the worktree; on load the host reconciles each record
with `git worktree list` and marks a vanished one `Missing`.

## 7. Phases

| # | Slice | Delivers |
|---|---|---|
| **1 (MVP)** | Free-standing workspaces | `WorkspaceRecord`, `root()`/`project_of()`, list / create / adopt / remove, W5's refusal and offers, titlebar switcher and dialog, picker rows, W4 location, W8 truthful `WorkSession`, W9 git overview and watch, the glossary edit. Explorer, search, Git screen, terminals and agents follow for free |
| 2 | Identification and safety | Pane/tab branch chip, rail badge mark, change and live guards, branch-delete policy, Prune, `project_info` fields |
| 3 | Tasks | Board card Create/Open workspace, task binding, merge-back row |
| 4 | Missions | `workspace: PerMission`, the launch request's target workspace, mission panel chip and Review hand-off |
| 5 | Parallel and agent-driven | `PerTask`, `list_workspaces` / `create_workspace` behind `may_create_workspace` |

Separate, mechanical, any time: the `SpawnWorkspace` family rename (W2).

## 8. Costs and hazards

- **Disk and build time.** Each linked workspace is a full checkout with its own `target/` or
  `node_modules/`; its first build is cold. The remove dialog shows the folder's size (phase 2).
- **Index per workspace.** The project's `index_level` applies to each; ten workspaces, ten indexes.
- **Hooks and submodules.** `git2`'s `worktree add` runs no checkout hooks and initialises no
  submodules. Said in the dialog's note, not fixed.
- **A confined agent committing in a linked workspace** writes into the main repository's `.git`,
  outside its folder; the isolation grant has to include the common dir. A line for the isolation
  work, not a field here.

## 9. Decisions

| # | Decision | Recommendation |
|---|---|---|
| W1 | Model and address | The project owns its workspaces (main implicit, linked ones on the record); each addressed by a `ProjectId`-typed id, main's = the project's, so rooted messages are unchanged. No `WorkspaceId` pair on the wire |
| W2 | Naming | "Workspace" = folder + branch; the glossary's running-agent meaning becomes "agent" (the UI's word already); `SpawnWorkspace` family renamed by a separate mechanical card, wire unchanged until then |
| W3 | Creation entries | New branch from base, existing branch, adopt from `git worktree list`; open in the asking window |
| W4 | Default location | `<repo-parent>/<repo-name>.worktrees/<branch-slug>`; `Sibling`/`ConfigRoot`/`Custom` with per-project override |
| W5 | Branch checked out elsewhere | Refuse with git's reason; offer a new branch from its tip (default) or a detached head at its tip; switch-to if a linked workspace holds it; never force |
| W6 | Remove and branches | Remove refused while live, confirm on changes; keep branch by default, delete-if-merged offered, force only with a second tick |
| W7 | Mission/task provisioning | Mission setting `Main`/`PerMission`/`PerTask`, default `Main`, dialog suggests `PerMission`; `PerTask` in phase 5 |
| W8 | `WorkSession` | Filled from the agent's workspace — no proto change |
| W9 | Git overview / watch | Open via gitlink, watch the admin dir; closes `G125`'s worktree half |
| W10 | Shared vs per-workspace state | Tasks, missions, plans, KB, settings per project; explorer, panes, Git screen, view prefs, index, `Missing` per workspace |
| W11 | Git engine | `git2` on the git worker, no `git` subprocess |
| W12 | Agent MCP | `project_info` gains workspace facts in phase 2; `create_workspace` only behind `may_create_workspace` (default off), never spawns |

## Related docs

- [`../features/workbench.md`](../features/workbench.md) — the window's projects, the picker, the titlebar the switcher joins.
- [`../features/workbench-git.md`](../features/workbench-git.md) and [`../tech/version-control.md`](../tech/version-control.md) — the git worker, the nested walk and `G125`.
- [`../features/sessions-and-workspaces.md`](../features/sessions-and-workspaces.md) — why a pane's cwd is its project's, and the old meaning of "workspace" W2 retires.
- [`../product/glossary.md`](../product/glossary.md) — the definition W2 changes.
- [`../features/workbench-agents.md`](../features/workbench-agents.md) — the sidebar and columns that already draw branch and worktree.
- [`../tech/transport-contract.md`](../tech/transport-contract.md) — where the new family and `WorkSession` land.
- [`./mission-proposal.md`](./mission-proposal.md) — the mission this provisions workspaces for.
- [`./agents-definitions.md`](./agents-definitions.md) — the `may_create_workspace` grant W12 uses.
- [`./backlog/agent-graph-final.md`](./backlog/agent-graph-final.md) — the earlier workspace sketch this replaces.
