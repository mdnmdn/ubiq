---
id: inbox-mission
title: Proposal — the mission as a first-class entity
kind: proposal
status: proposal
summary: Promote today's mission — a task carrying `Level::Mission`, its plan, its children and the new-mission dialog — into an entity with a lifecycle of its own. Four phases (requirements, refining, in progress, completed) with user-gated transitions, a mission record beside the task keyed by the same `TaskId`, a dockable mission panel, a Missions section in the Agents sidebar, a fence with a drag handle on both Teams canvases, spawn and attach of coordinator and worker agents, a document set beyond the plan, a journal, and a `ubiq-mission` / `use-mission` MCP pair. Everything that exists — the task record, parent/child, attachments and references, the plan store and annotations, `ubiq-plan`, `NewMissionForm`, `WorkAgent::task` — is reused rather than replaced. Every fork is a numbered decision with a recommendation and a cost.
read_when: you are working on missions, the mission panel, the mission fence on the Teams graph, mission phases, or the mission MCP servers
updated: 2026-09-24
depends_on: [feat-workbench-tasks, feat-workbench-teams, feat-workbench-agents, feat-chat, feat-workbench, tech-transport, tech-decisions, wip-planning-system, inbox-hitl-dialogs, inbox-graph-layout, inbox-agent-graph-final]
---

# Proposal — the mission as a first-class entity

**A proposal to work on, not a settled design. Nothing here is built.** Every fork is a numbered
decision **M1–M18** with a recommendation and a cost; §11 stages the work; §12 lists what is still
open for discussion. A decision that lands takes a `Dnn` row from **D164** upward.

**Posture: reuse the mission that exists.** The planning flow (T-56–T-62, T-94) already ships most of
the parts: a mission is a `TaskRecord` with `level: Some(Level::Mission)`; it may have children at
depth one; it carries a markdown plan with block-anchored annotation threads and per-line
provenance; the `ubiq-plan` MCP server reads and writes that plan; `NewMissionForm` creates the
card and launches an assistant profile in one act. This proposal adds what a *tracked, long-running
piece of work* needs on top of those parts — a phase, a roster, documents, a journal, a surface of
its own and a tool surface for its agents. It adds no parallel task model, no second plan store and
no second id space.

## 1. What exists, and what is missing

| Already there | Where | Reused as |
|---|---|---|
| `Level::Mission` on `TaskRecord` | `ubiq-proto/src/work.rs` | the mission's anchor task, and its identity |
| `description`, `attachments` (path or `kb:`), `references` | `TaskRecord` | the brief, its attachments, its linked tasks |
| `parent` at depth one, orphan on delete | `Work::parent_refusal`, `orphan_children` | the mission's tasks |
| `steps`, `comments` with host-stamped author | `TaskRecord` | the anchor's own checklist and a discussion |
| Plan at `plans/<TaskId>.md`, annotations, provenance, `PlanConflict` | `store/plan.rs`, `plan/`, `D157`–`D161` | the mission's plan, unchanged |
| `DocumentHandle` keyed family | `ubiq-proto/src/plan.rs` | extended with one arm for mission documents |
| `ubiq-plan`, `manage-ubiq-tasks`, `use-task`, `ubiq-ask` | `ubiq-host/src/mcp/` | composed with, not duplicated |
| `NewMissionForm`, `mission_briefing()`, `ProfileInfo::mission_assistant` | `state/new_mission.rs`, `app/new_mission.rs` | the new-mission dialog, widened |
| `WorkAgent::task`, `WorkAgent::parent`, `AssignAgent`, `SendToAgent` | `work.rs`, work family | the roster, derived |
| Task containers, delegate ring fences, project fences | `state::layout::fence`, `TeamsView` | the mission fence, one level up |
| `mission_term` (app default and project override) | `HostSettings`, `ProjectRecord` | the entity's name everywhere it is drawn |

What is missing: a **phase** (a mission shares the generic seven-value `Status`), a **coordinator**
distinct from "whichever profile the dialog launched", **documents** beyond the one plan, a record of
**what happened** (the contract has no variant for an agent reporting progress —
`transport-contract.md`), a **surface** (a mission is a card plus a modal), a place in **Agents** and
**Teams**, a way for an agent to **spawn** another Ubiq-level agent (`StartConversation` is UI-only
and no MCP reach can issue it), and a **tool surface scoped to one mission**.

## 2. The shape

```
TaskRecord (level = Mission)          ← unchanged; the board's card, the identity (TaskId)
 ├─ description / attachments / references   → the brief
 ├─ children (parent = this)                  → the mission's tasks
 └─ plans/<TaskId>.md + .annotations.json     → the plan (ubiq-plan, unchanged)

MissionRecord  missions/<TaskId>/mission.toml  ← new; keyed by the same TaskId
 ├─ phase, phase history
 ├─ coordinator: Option<AgentId>, roles
 ├─ settings (spawn policy, gates)
 missions/<TaskId>/docs/*.md (+ sidecars)      ← new; further documents, annotatable
 missions/<TaskId>/journal.jsonl               ← new; append-only events and progress

Roster  = agents whose WorkAgent::task is the mission or one of its children   ← derived
```

### M1 — Identity: the anchor task's id, or a `MissionId` of its own

- **(a) Recommended: `TaskId`.** A mission *is* its anchor task; the mission record is a sidecar
  keyed by the same id, exactly as the plan already is. Every existing message, handle, MCP tool and
  board affordance keeps working; `DocumentHandle::Plan` needs no change.
- (b) A new `MissionId`, with `MissionRecord::task: Option<TaskId>`. Lets a mission exist with no
  card. Costs a second id space, a link table to keep consistent in both directions, a migration of
  every plan, and a rewrite of `ubiq-plan`'s addressing — against the brief to reuse.

**Cost of (a):** "tracked independently" means an independent *record, lifecycle and surface*, not an
independent id; a mission can never exist without a card on the board. Demoting the task
(`Level` → `None`) must be refused while a mission record exists, or archive it (M3).

### M2 — Where the mission's state lives

- **(a) Recommended:** a `MissionStore` beside `FilePlanStore`, one directory per mission under
  `<config root>/projects/<ProjectId>/missions/<TaskId>/`, held behind a `mission::Handle`
  (`Arc<Mutex<Missions>>`) that — like `plan::Handle` — also holds a `work::Handle`, and is shared by
  the coordinator and the MCP listener (`D120`'s pattern, third holder).
- (b) New fields on `TaskRecord`. Puts phase and roster into `tasks.toml`, which every task pays for
  and every window reloads on each poll.

The plan stays at `plans/<TaskId>.md` (M9). Under `D30`, nothing lands in the project tree except
what the user exports or what an agent writes there as work.

### M3 — Existing missions, promotion and demotion

- **Recommended:** a mission record is created lazily — the first time a `Level::Mission` task is
  opened in the mission panel, listed, or touched by `ubiq-mission` — with the phase inferred:
  `Done` → Completed, `Abandoned` → Abandoned, a plan exists → Refining, else Requirements. No
  migration pass. Promoting a task on the panel's Level control creates the record the same lazy
  way. Demoting a mission whose record has a non-empty roster or journal is refused with a sentence;
  an empty record is deleted with the demotion. Deleting the task deletes the mission directory
  (plan deletion already rides task deletion) and orphans children, as today.

## 3. Phases

| Phase | Who works | What happens | Anchor `Status` |
|---|---|---|---|
| **Requirements** | coordinator (as assistant) + user | the coordinator asks questions (HITL), writes the brief out, may draft a plan | `InProgress` |
| **Refining** | user + coordinator | the plan is refined through annotation threads, `write_plan`, extra documents | `InReview` |
| **In progress** | coordinator + workers | tasks are created and worked; long-running; HITL pauses; user feedback | `InProgress` (`Blocked` while it needs you) |
| **Completed** | — | everything is done; the journal closes; agents may be unloaded | `Done` |
| *Abandoned* | — | terminal, from any phase | `Abandoned` |

### M4 — The phase is its own field; `Status` is derived from it

- **Recommended:** `MissionRecord::phase` is the truth; the anchor's `Status` is written from it by
  the host on each transition (table above), so the board, the Teams tasks drawer and every
  existing filter stay right with no change. Dragging a mission card on the board to another column
  becomes a phase request (M5), not a raw status write — `Done` means "request Completed".
- Alternative: reuse `Status` alone and map Requirements/Refining onto `Backlog`/`Ready`. Costs the
  meaning of both columns and has nowhere to put the gate.

### M5 — Who moves a phase

- **Recommended: the agent requests, the user confirms, the step back is free.**
  - Requirements → Refining: the coordinator calls `request_phase`; confirmed by the user, or
    automatic when the mission's `auto_refine` setting is on (default on — nothing is at stake).
  - **Refining → In progress is always a human gate** — it is *plan approval*, the one moment the
    user commits agents and spend. This replaces the `require plan` reminder: the gate refuses
    while the plan is empty or while it has open blocking threads, which answers the open question
    in `wip/planning-system.md` mechanically.
  - In progress → Completed: requested by the coordinator (with a summary), confirmed by the user.
  - Any phase → an earlier one (re-plan, re-scope) and any → Abandoned: the user, any time.
- A pending request shows in the panel's *Needs you* section and as a notification (link to the
  panel); confirming or declining sends the answer to the coordinator as its next prompt.

### M6 — Attention is a flag, not a phase

"Needs you" (a pending ask, a phase request, a spawn request under M13's policy) is derived from what
is pending and drawn as the mission's hexagon inner mark `NeedsYou` — the same dictionary cards use
(`subagent-status-dictionaries-and-hexagon.md`). The mission's hexagon is the worst of its roster's,
the rule a session group's left bar already follows.

## 4. The brief, attachments and documents

### M7 — The brief is the anchor task's own fields

- **Recommended:** requirements = the task's `description` (markdown), attachments = the task's
  `attachments` (project path or `kb:`), linked tasks = the task's `references`. The dialog (§5)
  writes them with messages that exist (`UpdateTask`, `SetTaskField`), exactly as
  `start_new_mission()` already composes. Nothing new crosses the bus for the brief.
- Pasted pictures go to `.ubiq/pasted/` via `WriteProjectFile` and are attached as ordinary files —
  the chat composer's path. Pasted text longer than a threshold becomes a mission document
  (`docs/pasted-<n>.md`) and is attached, instead of bloating the description.

### M8 — Documents beyond the plan

- **Recommended:** `missions/<TaskId>/docs/<name>.md`, markdown only, flat, addressed by a new arm
  `DocumentHandle::MissionDoc { project_id, task_id, name }`. Because the family is already keyed by
  a document (`D161`), the arm inherits annotations, provenance, the conflict rule and the native
  editor surface for free; the host only resolves one more path. Typical documents: requirements
  (the Q&A the assistant distils), decisions, notes, research, review reports.
- Agents write them through `ubiq-mission` (M16); the user through the panel. Export uses the plan's
  one-shot `ExportPlan` path.

### M9 — The plan stays where it is

`plans/<TaskId>.md` and `ubiq-plan` are unchanged; the panel lists the plan first among the documents
and opens it on the existing plan surface. A mission may be *started from a plan* (§5): the chosen
markdown (project file, KB document, or paste) is copied in as the first plan revision, stamped
`Human`, and the mission starts in Refining.

## 5. Starting a mission — the dialog

`NewMissionForm` widened, not replaced; still `Layer::NewMission`, still composed from existing
messages, now also raisable from the Agents sidebar and the Teams toolbar, not only the board.

| Row | Today | Proposed |
|---|---|---|
| Title | yes | unchanged (and generated from the description when blank, as a task already is) |
| Description | plain field | the **requirements** editor: markdown, paste (text and pictures) |
| Attachments | — | chip row, the task attachment picker (project + KB roots) |
| Linked tasks | — | chip row, the task panel's references picker |
| `require plan` | reminder sentence | removed — the Refining → In progress gate (M5) is the mechanism |
| Assistant | mission-assistant profiles | **coordinator**: the same picker, plus "attach a running agent" |
| Plan | — | optional: none / project file / KB document / paste → starts in Refining |
| Start | creates + launches | unchanged sequence + `CreateMission` (M17), briefing mentions `ubiq-mission` |

### M10 — Coordinator and assistant are one role

- **Recommended:** one role, **coordinator**, held by at most one agent at a time; in Requirements
  and Refining it behaves as the assistant (its briefing says so), in In progress it plans work and
  delegates. The mission record keeps `coordinator: Option<AgentId>` and a history. Handing off —
  e.g. a cheap assistant model for Requirements, a stronger one for In progress — is "spawn
  coordinator" on the panel with a different profile; the previous coordinator is detached, not
  killed, and the new one's briefing points at the brief, plan, documents and journal.
- Alternative: two roles. Costs a second picker and a rule for who answers what, for a difference a
  profile already expresses.

## 6. The mission panel

`PanelKind::Mission(TaskId)`, class `Free` — dockable in any region, in any mode, like `Chat`. Opened
from the board card, the task panel ("Open mission"), the Agents sidebar, the Teams fence handle, a
notification link, and the command palette. Several may be open.

```
┌ ⬡ Mission  UBQ-42  Payment retries                       [⋯] ┐
│ Requirements ─ Refining ─ ●In progress ─ Completed            │
├ Needs you (2) ────────────────────────────────────────────────┤
│  ? coordinator asks: "retry cap per tenant?"        [Answer]  │
│  ⇢ spawn request: reviewer (sonnet) for UBQ-47  [Allow][Deny] │
├ Agents (3) ─────────────────────────── [Spawn ▾] [Attach…] ───┤
│  ⬡ coordinator  claude · opus      writing plan.md    [Chat]  │
│  ⬡ worker       codex              UBQ-45             [Chat]  │
│  ⬡ worker       claude · sonnet    idle               [Chat]  │
├ Tasks  4 todo · 2 in progress · 5 done   ▓▓▓▓▓░░░░ ───────────┤
│  … grouped rows, click opens the task panel                   │
├ Plan & documents ─────────────────────────────────────────────┤
│  plan.md  3 open threads    requirements.md   decisions.md    │
├ Brief ────────────────────────────────────────────────────────┤
│  description · 3 attachments · linked UBQ-12, UBQ-19          │
├ Activity ─────────────────────────────────────────────────────┤
│  journal feed (phase changes, progress, spawns, feedback)     │
│  [ feedback to the mission…                        ] [Send]   │
└───────────────────────────────────────────────────────────────┘
```

- **Header:** mission term, key, title, the phase stepper (clicking a phase requests it, M5),
  the hexagon (M6), `⋯` for complete / abandon / pause all / open on board / open on Teams.
- **Agents:** the roster (M11) with each card's hexagon and note. `Chat` opens that agent's
  conversation in a `Chat` panel (Teams' `open_teams_agent_panel` path). **Spawn ▾**: coordinator,
  worker (profile picker, optional task), any agent (the New agent form, pre-filled with this
  mission). **Attach…**: pick a running agent — it joins the roster (`AssignAgent` onto the mission).
  When the coordinator exists but is not loaded, its row offers **Resume** (the library session) and
  **Replace**.
- **Tasks:** the anchor's children grouped by status, with a progress bar; `+` creates a child.
- **Plan & documents:** plan first, then `docs/`; open on the plan surface. Counts of open threads.
- **Activity:** the journal (M12), and a feedback field — feedback is journaled *and* delivered to
  the coordinator as a prompt (queued if it is working; `message-queue-and-steering.md`).

The existing task panel keeps working for the anchor task; it gains one "Open mission" row.

## 7. Agents

### M11 — The roster is derived from `WorkAgent::task`

- **Recommended:** an agent belongs to a mission when its `WorkAgent::task` is the mission or one of
  its children — the link Teams already draws containers from, and `AssignAgent` already writes. The
  mission record only adds *role* (coordinator vs worker) and a history of who has been in it. So an
  agent belongs to at most one mission, attaching is `AssignAgent`, and moving an agent between
  task containers inside a fence keeps it in the mission.
- `AgentFacts` gains `mission: Option<TaskId>`, filled at launch and updated on `AssignAgent`, so
  `ubiq-mission` resolves "which mission am I in" from the URL identity (`D102`) with no argument.
- Harness-native subagents (Claude Code's `Task` tool) stay what they are (`D47`): read off the
  stream, drawn as delegate rings inside the fence, counted in the mission's spend, not rostered.

### M12 — The journal

- **Recommended:** `missions/<TaskId>/journal.jsonl`, append-only, one event per line: phase
  requested / changed, agent joined / left / role changed, task created / finished, document
  written, progress reported, feedback given, ask raised / answered. Written by the host at the
  point each of those already happens, plus the explicit `report_progress` tool. The panel reads
  the tail and pages back. This is the "agent making progress of its own" variant the transport
  contract says it lacks, scoped to missions.

### M13 — Spawning an agent from a mission

The user spawns from the panel: the window composes `StartConversation` exactly as the New agent
form does, with the mission's `project_id`/`session_id`, the mission's MCP servers ticked, and an
opening prompt from a briefing template; then `AssignAgent` onto the mission (or onto a child task).

An agent spawns through `ubiq-mission::spawn_agent`. `StartConversation` is UI-only and the window
mints the `AgentId`; that rule is kept:

- **Recommended: the host relays, the window launches.** The tool posts `MissionSpawnRequest`
  (profile, task, prompt, reason) to the window that owns the project and returns a request id at
  once. The window applies the mission's **spawn policy** — `ask` (default: a row in *Needs you*),
  `auto up to N concurrent`, or `never` — and launches with the same composition the panel uses. The
  outcome reaches the requesting agent as its next prompt and in the journal; `list_agents` shows it.
- Alternative: let the host mint and launch. Breaks the one-minter rule and bypasses the window's
  profile resolution for no gain; the project is open in its window whenever its agents run.

Today's instruction to the coordinator — "create tasks and invoke subagents to process them" — keeps
working unchanged in the first stages (harness subagents); Ubiq-level workers are the later stage
(§11, S5), which is where "this will change" lands.

## 8. Agents mode — the sidebar

### M14 — A Missions section above the sessions

- **Recommended:** the sidebar (`ui/agents/sidebar.rs`) gains a **Missions** section at the top: one
  row per mission not Completed/Abandoned (a toggle shows those too), carrying the hexagon, phase chip,
  agent count and a *needs you* dot. A row expands to its roster; clicking the row opens the mission
  panel; clicking an agent opens its column as today. The session groups below stay exactly as they
  are, and an agent inside a mission carries a small mission chip there rather than being moved out —
  one conversation is never listed as two different things. `New mission` sits beside `New agent`.

## 9. Teams and All Teams — the fence

```
 ┌■ UBQ-42 Payment retries · In progress ⬡┐ ← handle: mini block, top-left
 ┆  ┌ coordinator ┐                        ┆   drag = move everything inside
 ┆  └─────────────┘                        ┆
 ┆  ┌╌ UBQ-45 ╌╌╌╌╌╌╌╌┐  ┌╌ UBQ-47 ╌╌╌╌╌╌╌┐ ┆ ← task containers, as today
 ┆  ┆ [worker] [ring] ┆  ┆ [worker]       ┆ ┆
 ┆  └╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌┘  └╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌┘ ┆
 └┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┘
```

### M15 — The fence is derived; the handle is the only new object

- **Recommended:** the fence is the union of the mission's own task container and its children's,
  padded by `GROUP_PAD`, computed each frame like every enclosure today (`D41`: position is the
  interface's fact, membership the host's). Nesting, outermost first: project fence (All Teams) →
  **mission fence** → task container → delegate ring; the "not fenced twice" rule (T-105) applies —
  the mission's own anchor container merges into the fence rather than drawing inside it.
- **The handle** is a small block at the fence's top-left: mission term chip, key, short title,
  phase, hexagon. Dragging it translates every card inside (the same translation a task container's
  ground drag already applies). Clicking it selects the mission and opens the mission panel in the
  right dock. Its right-click menu mirrors the panel's `⋯`.
- The coordinator is drawn in a band at the top of the fence — `agent-graph-layout-proposal.md`'s
  coordinator band, at the mission scale.
- **A mission with no agent** still draws its handle with a minimum fence, so it can be moved and can
  be dropped onto.
- Dropping a card inside the fence (on no child's container) assigns it to the mission itself
  (`AssignAgent`), i.e. attaches it; dropping it out onto open ground stays a no-op, as today.
- The arrangements lay a mission out as one group, innermost first — the layout proposal's
  frame-owns-its-box rule, with a mission as the first frame kind built. Until that proposal lands,
  the twelve arrangements place fences by their members and the handle drag is the correction.
- A "Missions" filter joins Sessions and States in the toolbar; the tasks drawer groups by mission.

## 10. MCP

### M16 — Two servers, the `manage` / `use` split the task servers already have

`ubiq-mission` for the coordinator, `use-mission` for workers. Both resolve the mission from the
calling agent's `AgentFacts::mission` (M11); a call from an agent outside a mission answers with a
sentence, not an error. Neither duplicates `ubiq-plan` (plan and annotations) or the task servers —
the coordinator's launch ticks `ubiq-mission`, `ubiq-plan`, `manage-ubiq-tasks`, `ubiq-ask`; a
worker's ticks `use-mission`, `use-task`, `ubiq-ask`.

| Tool | `ubiq-mission` | `use-mission` | What it does |
|---|:-:|:-:|---|
| `mission_overview` | ✓ | ✓ | phase, brief, roster, task counts, documents, pending items, last journal events |
| `read_brief` | ✓ | ✓ | description, attachments (paths / `kb:`), linked tasks' titles and keys |
| `list_documents` / `read_document` | ✓ | ✓ | `docs/*.md` (plan is `ubiq-plan::read_plan`) |
| `write_document` | ✓ | — | create / replace a document, `expected_revision` like `write_plan`, stamped `Agent` |
| `create_mission_task` | ✓ | — | a child of the mission, with todos, optionally for a role; thin over `Work` |
| `report_progress` | ✓ | ✓ | a journal line, optionally against a task; drawn on the panel and the card note |
| `request_phase` | ✓ | — | ask to move phase with a summary; answered by the user (M5) |
| `spawn_agent` | ✓ | — | a `MissionSpawnRequest` (M13); returns a request id |
| `list_agents` | ✓ | ✓ | the roster with role, task, activity |
| `message_agent` | ✓ | — | a prompt to a roster member, over `SendToAgent` (queued if working) |
| `read_feedback` | ✓ | — | user feedback since the last read (also delivered as prompts) |

Asking the user stays `ubiq-ask::ask_user_question`; long waits move to the arm-and-fire mode of
`inbox/hitl-registered-dialogs.md` once it lands — a mission's In-progress phase is exactly the
long-running case that proposal is for. A pending ask from a roster member shows in the panel's
*Needs you*.

Built the way the catalogue already builds servers: slugs and `ServerSpec`s in `mcp/catalogue.rs`,
a `mcp/mission.rs` module, a `MissionReach` holding the `mission::Handle`, a `Voice` for the spawn
relay, and the dispatch arm in `mcp/tools.rs`. Mutations broadcast to every window (`D120`).

### M17 — The wire

A **mission family** beside the work and plan families, keyed by `(project_id, task_id)`:

- UI → host: `ListMissions`, `CreateMission` (record for a task; phase, coordinator, settings),
  `RequestPhase`, `SetPhase` (the user's confirm / step back), `SetMissionField` (coordinator,
  spawn policy, `auto_refine` — one variant with a field enum, `SetTaskField`'s shape),
  `AddFeedback`, `LoadJournal { before }`, `AnswerSpawn`.
- Host → UI: `MissionList`, `MissionChanged`, `MissionDeleted`, `JournalAppended`, `Journal`,
  `MissionSpawnRequest`, `MissionError`.
- Documents ride the plan family with `DocumentHandle::MissionDoc` (M8) — no new document messages.
- `StartConversation` gains nothing: a mission launch is a launch plus `AssignAgent`.

### M18 — Naming

The entity takes the project's `mission_term` everywhere it is drawn (panel title, sidebar section,
fence handle, dialog), as the board chip already does. Code, wire and MCP names stay `mission`.

## 11. Staging

Each stage ships on its own and leaves the tree coherent.

| Stage | Contents | Reuses |
|---|---|---|
| **S1 — record and panel** | `MissionStore`, lazy records and phase inference (M3), mission family on the wire, `PanelKind::Mission` with header, tasks, plan link, brief, roster derived from `WorkAgent::task`; board "Open mission" | task record, plan store, task panel pieces |
| **S2 — phases and dialog** | phase transitions and gates (M4, M5), `Status` derivation, widened new-mission dialog (attachments, references, plan seed, attach running coordinator), *Needs you* section | `NewMissionForm`, `start_new_mission()`, task attachment and reference pickers |
| **S3 — MCP core, documents, journal** | `ubiq-mission` / `use-mission` without spawn, `DocumentHandle::MissionDoc`, journal and `report_progress`, `AgentFacts::mission` | `ubiq-plan`'s handle pattern, `D120`, `D161` |
| **S4 — Agents and Teams** | Missions section in the sidebar (M14), the fence and its handle on both canvases, drop-to-attach, Missions filter | `state::layout::fence`, task containers, hexagon mark |
| **S5 — spawning and feedback** | spawn / attach / replace coordinator from the panel, `spawn_agent` relay with spawn policy, `message_agent`, feedback delivery; arm-and-fire asks once that proposal lands | New agent form composition, `SendToAgent`, message queue |
| **S6 — later** | Ubiq-level workers as the default instead of harness subagents; arrangements laying missions out as frames (with the layout proposal) | — |

## 12. Open for discussion

1. **Anchor task optional?** M1 makes the card mandatory. Is there a real case for a mission with
   no card (e.g. an exploratory mission that should stay off the board)?
2. **Depth one.** A mission's tasks cannot have children. Enough for In progress, or do workers
   need sub-tasks (today they have todos, `steps`)?
3. **Pause.** Is "pause all" (stop every roster agent, keep the phase) a phase of its own, or just an
   action? Proposed: an action plus a journal line.
4. **Spend.** Should a mission carry a budget (tokens / cost) that the spawn policy and a gate
   respect? The usage store already splits spend per agent and delegate.
5. **Completion.** Should Completed produce anything — a summary document the coordinator must
   write, an export of the plan and documents into the project?
6. **Cross-project missions.** All Teams draws several projects; a mission stays inside one project
   here. Needed?
7. **Mission templates.** Saved briefs + coordinator profile + policy ("bug hunt", "feature spike")?
8. **Child inheritance** (`wip/planning-system.md`): should a mission's children inherit labels,
   colour or session? The fence makes the colour question more visible.

## 13. Related docs

- [`../features/workbench-tasks.md`](../features/workbench-tasks.md) — missions, children, the plan surface and the new-mission dialog as built
- [`../wip/planning-system.md`](../wip/planning-system.md) — the open items this proposal answers (`require plan`) or leaves open (inheritance, T-85)
- [`../features/workbench-teams.md`](../features/workbench-teams.md) — task containers, fences, the drag model, the hexagon
- [`../features/workbench-agents.md`](../features/workbench-agents.md) — the sidebar, columns, the New agent form
- [`../tech/transport-contract.md`](../tech/transport-contract.md) — the work and plan families the mission family sits beside
- [`../tech/decisions.md`](../tech/decisions.md) — `D39`, `D41`, `D47`, `D102`, `D120`, `D157`–`D161`
- [`hitl-registered-dialogs.md`](./hitl-registered-dialogs.md) — the ask mode long-running missions want
- [`backlog/agent-graph-layout-proposal.md`](./backlog/agent-graph-layout-proposal.md) — frames with handles and coordinator bands
- [`backlog/agent-graph-final.md`](./backlog/agent-graph-final.md) — Task, Role and coordinator vocabulary

## Next steps

- Walk §12 with the user and settle M1, M5, M11 and M13 first — the rest follows from them.
- Then cut S1 into cards on the board.
