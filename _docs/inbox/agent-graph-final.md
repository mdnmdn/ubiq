---
id: inbox-agent-graph-final
title: Proposal — the agent graph, final
kind: proposal
status: proposal
summary: The merged and settled version of the agent-graph work — sessions as working areas, tasks with three shapes, hosted and reported agents, four activity channels, the brief as a gate, the MCP surface, the two screens, and every fork the earlier drafts left open, closed.
read_when: you are deciding how several agents cooperate on one goal, what an agent knows about the others, or what the Agents and Tasks rail modes are for
updated: 2026-08-31
depends_on: [feat-sessions, feat-workbench, feat-chat, tech-transport, tech-agent-manager]
---

# Proposal — the agent graph, final

Ubiq today is a multiplexer: panes side by side, each an independent harness, related only by
sharing a window. This proposes the layer above — several agents working one goal, knowing about
each other, reporting what they are doing, drawn as a graph instead of a row.

This document supersedes [`agent-graph-proposal.md`](./agent-graph-proposal.md) and
[`graph-harness-brainstorm.md`](./graph-harness-brainstorm.md), which both expand
[`graph-harness-ideas.md`](./graph-harness-ideas.md). It takes the proposal's model — the
distinctions, the channels, the contract — and folds in what the brainstorm had that the proposal
dropped: the shape of the brief, work without a task, the PM's guardrails, the scenarios, the
alternatives that were considered and rejected, and the decisions the mockups made. Every fork the
two drafts left open is closed here, and §15 lists the closures.

**The line, stated once:** Ubiq hosts orchestration; it does not perform it. The one exception is
the pipeline sequencer in §5, and only because it needs no model to decide anything. Everything
else — who to spawn, what to delegate, when a thing is done — is an agent's call, and Ubiq's job is
to make it visible, addressable and stoppable.

## 1. The vocabulary, before anything else

The whole design is a nesting, and a nesting whose levels share a name cannot be discussed.

| Level | What it is | Owns |
|---|---|---|
| **Project** | A repository Ubiq remembers. A window is opened on one | The catalogue record, the config root |
| **Session** | A working area: one folder, one branch, one worktree, optionally a goal | A directory, a git ref, its tasks |
| **Task** | A unit of intent inside a session — a goal, a shape, a state | Its agents, its subtasks, its artifacts |
| **Agent** | One running harness with an identity | A conversation, a pane, a working directory |
| **Role** | A named bundle an agent is spawned from | A prompt fragment, a harness, a tool policy |

The compass for all five: **a session is a place; an agent is a who; a task is a why, when there is
one; a role is a what-for; and the library's session is a conversation.** The graph is the who's,
grouped by the place and the why, coloured by what each is doing.

Three of those exist. **Project** is [`project-handling-proposal.md`](./project-handling-proposal.md).
**Session** is [`../features/sessions-and-workspaces.md`](../features/sessions-and-workspaces.md) —
"a named piece of work that owns a folder" — and it grows fields rather than becoming a new
concept. **Agent** is today's workspace: one harness, one directory, one terminal; this document
says *agent* for identity and *workspace* for process, and they are one record. **Task** and
**role** are new, and they are the entire addition.

The word *session* itself should not survive — Ubiq spends it twice, and
[`session-naming-proposal.md`](./session-naming-proposal.md) is that decision on its own:
`session` → `workspace` and `workspace` → `agent` if the cascade is taken, `session` → `lane` if it
is not. This document keeps saying "session" for the thing being renamed and **working area** where
the sentence would otherwise be ambiguous.

The collision this design does resolve: the harness library's *session* is a resumable
conversation, Ubiq's is a folder full of work, and [`../backlog.md`](../backlog.md) Q4 asks which
maps onto which. **The library's session belongs to the agent.** One agent is one resumable
conversation, so resuming a task is a fan-out over its agents, not a lookup.

```
   project ──┬── area "auth-refactor"   (worktree, branch feat/auth)
             │     ├── task "extract the token store"  [pipeline]
             │     │     ├── agent implementer (codex)  tool: edit
             │     │     └── agent reviewer    (claude) queued
             │     └── (two agents, no task — the user is just coding here)
             └── area "main"            (the repo itself)
                   └── agent pm         (claude) awaiting input
```

## 2. Where a session grows, and where it does not

A session gains three durable fields and one derived one: **`goal`**, one or two sentences on what
this area is for, injected into every agent's identity brief; **`git_ref`**, the branch it works
on, absent meaning whatever the folder is already on; **`state`**, `Draft` until its brief is
accepted and `Open` after (§7); and derived, whether the folder is a linked **worktree** the host
created or one that was already there.

**A session is a worktree when it needs to be, and Ubiq never converts one silently.** Creating one
offers three placements: the project's own folder, an existing folder, or a new linked worktree on
a new branch — the third is what makes parallel areas safe. Removing a session never removes a
worktree Ubiq did not create. The UI never runs `git`; the worktree is the host's move.

**A session without a task is first-class, not a degenerate state.** Opening an area and coding
with agents — no brief, no board row — is today's product with a name on the place, and it must
stay one click. A task does not own its session; it uses it, and closing a task does not destroy
the place.

**Sessions are the isolation boundary; tasks and crews are not.** Two sessions on two worktrees
cannot collide. Two tasks in one session share a folder, and so do two agents in one crew. §12 says
what that costs — it is the hazard this design does not remove.

## 3. Two kinds of agent, and why it decides the UI

This is the load-bearing distinction, and every screen decision follows from it.

**A hosted agent is one Ubiq spawned.** It has a pane, a pseudo-terminal, a harness the user
picked, an identity Ubiq injected, and an MCP surface Ubiq exposes. Ubiq can watch it, type into
it, resize it, message it and kill it. Everything in the existing contract already applies.

**A reported agent is a harness's own subagent** — Claude Code's task tool and its kin. It runs
*inside* the parent's process. It has no pseudo-terminal, no pane, no process Ubiq can see, and no
MCP connection of its own. Ubiq knows it exists only because the parent said so, in a hook or a
tool event.

| | Hosted | Reported |
|---|---|---|
| Spawned by | Ubiq, through the harness library | The parent harness, internally |
| Pane | A terminal pane | None — a transcript pane at most |
| Input | Keystrokes, prompts, messages | None. It cannot be addressed |
| Harness | Any Ubiq supports, independent of the parent's | Always the parent's |
| Activity | Observed on four channels (§4) | Whatever the parent reported, when it reported it |
| Ending | An exit code from the coordinator | The parent saying so, or silence |

Two consequences the idea asked for without naming the cause. **A reported agent's chat is
read-only by construction** — a composer on a node that cannot receive input would be a lie. For a
hosted agent, read-only is instead a *view flag*: the user watching without steering, while the
node's parent may still be messaging it over MCP. And **a mixed-harness crew is necessarily
hosted**: a Claude Code subagent is a Claude Code subagent, so "a codex reviewer under a claude
implementer" is two hosted agents with a delegation edge.

**Reported agents are drawn, never inferred.** A node appears when the parent reports it and is
finalised when the parent reports it finished. A parent that dies mid-flight leaves its reported
children in `Unknown`, never in `Ended` — Ubiq did not watch them and must not claim it did.

## 4. How Ubiq knows what an agent is doing

A node should say *queued, thinking, running a tool, waiting for input, ended, error*. The
architecture forbids the obvious route: terminal bytes are opaque, and inferring "idle" from a lull
in output is the screen-scraping the boundary exists to prevent. So activity arrives on four
channels, and **each value names the channel it came from** — they are not equally trustworthy.

| # | Channel | Knows | Trust |
|---|---|---|---|
| 1 | **Process** — the coordinator's own supervision | Started, alive, exited with a code, stream closed | Ground truth. Ubiq owns the child |
| 2 | **Hooks** — injected by the harness library into the run's config | A tool is about to run, a tool finished, the harness stopped, the harness wants the user | Fires whether the model cooperates or not |
| 3 | **Structured events** — `AgentEvent` from the library's `IoBridge` | Thinking, assistant text, tool call and result, approval request, token usage, result | Complete, and mutually exclusive with a terminal (see below) |
| 4 | **Self-report** — the agent calling Ubiq's own MCP tool | Its own account of what it is doing and why | Cooperative. An agent that stops calling it goes quiet, not idle |

**The mutual exclusion is the constraint that shapes everything.** The library's structured mode
*replaces the tty* — a run is either passthrough on a pseudo-terminal or driven over the harness's
own wire protocol, never both. An agent that streams rich events has no terminal to watch; an agent
in a terminal pane has channels 1, 2 and 4 only. That is a per-agent choice, and the role makes it:

- **A terminal agent** — the default, watched and typed into. Liveness, hooks, self-report: enough
  for every state the graph needs.
- **A structured agent** — no terminal, a transcript pane, full event detail, driven by prompt. The
  shape for a crew member nobody watches keystroke by keystroke, and the only faithful token count.

**Hooks are the rung that matters most**: the only non-cooperative source of tool-level detail, and
the library already injects them per harness. A `PreToolUse` hook posting the tool name to Ubiq's
loopback endpoint turns `Tool { name }` from a claim into an observation. Which harnesses expose
which hook points is the library's fact, and a harness with none degrades to channels 1 and 4 —
and the node says which channel it is on, rather than inventing "thinking" from silence.

**Staleness is rendered, not stored.** Every activity carries `observed_at`; a node whose last
observation is old draws faint, and one silent past a threshold reads `Unknown`. Ubiq never
promotes silence to `Idle` — "it finished" and "it stopped telling us" is the distinction the user
most needs.

```
Starting · Queued · Idle · Thinking · Responding · Tool{name} · AwaitingApproval
AwaitingInput · Blocked{on} · Ended{code} · Failed{error} · Unknown
```

each carried with `since`, `observed_at`, `source`, and an optional note the agent supplies. Two of
these are not harness states. **`Queued`** is a node that exists and has not started — a pipeline's
successor, drawn so the path is legible before the token reaches it. **`Blocked{on}`** is an agent
saying it waits on another agent, a lock or a human, and it is what makes a stalled crew legible at
a glance. `AwaitingInput` is the state that earns the whole graph: it is the node the user should
click.

## 5. The three shapes, and the one Ubiq performs

Three ways of starting work are three shapes of task. They are topologies of one graph, not three
products.

| Shape | Who decides the next step | Agents | Ends when |
|---|---|---|---|
| **Solo** | The user, in conversation | One | The user says so, or the agent reports the task done |
| **Pipeline** | The task's declared stage list | One at a time, in order | The last stage passes, or a stage fails |
| **Crew** | A coordinator agent | Several, concurrent, each with a role | The coordinator reports done |

**Pipeline is the shape Ubiq owns.** A stage list is a state machine — start stage *n*, wait for
its agent to finish, hand its result to stage *n+1* — and running it in the host needs no model,
costs no tokens, and cannot hallucinate a step. That is why it is worth having as a distinct shape
rather than as "a crew whose coordinator was told to go in order". A pipeline is sequential on
purpose: parallelism inside a step is a crew, not a pipeline with extra arrows.

A stage hands off through an **artifact, not a conversation**: it writes a result file into the
task's scratch directory and returns a summary line, and the next stage's prompt names both. No
stage inherits the previous one's context window — a pipeline exists to spend a fresh context per
step.

**Crew is the shape an agent owns.** The coordinator is an ordinary hosted agent with a role that
permits spawning, and it spawns and messages its members through the MCP surface in §8. Ubiq
supervises: it enforces the budgets, draws the graph, and can stop the whole task. It does not
second-guess the coordinator's plan, and it has no opinion about what a good crew looks like. The
coordinator is not the PM: the PM *starts* tasks and picks shapes; a coordinator *runs* one, and a
project can have a PM and several coordinators at once.

**Solo is not a degenerate case to be folded away.** It is what most work is. Distinguish it from
"just code in this session": a solo task has a brief and a board row; just-code has neither, and
both are legitimate.

## 6. The PM is a role, not a component

The project orchestrator is tempting to build as a special thing. It should not be.

**A PM is an ordinary hosted agent with a wide tool policy, in a session bound to the project's
main worktree.** It has a pane like everything else, so the user can watch it. It holds the
conversation where the user says "start a task", it creates tasks, sessions and agents through
tools, and it is the default recipient of an escalation. It is a row in the role table, not a new
subsystem.

**Every agent has a session, including the PM — and the PM's is always main.** The earlier drafts
forked on this: a PM with no working tree (it plans, it cannot edit) against a PM on the main
checkout (it can see the tree it directs). The fork closes on *main*, for two reasons. Every agent
has a working directory, and a null session would hole every node, edge and query. And a PM that
sometimes has a tree and sometimes does not is how the user loses track of whether "the PM" can
touch files — the occupancy must be one thing, always. What the "none" option was protecting is
kept by policy instead: the PM's role is read-mostly on main, and `may_write` is the switch.

**A project without a PM is a working project.** The user drives tasks directly, which is the whole
product today. A PM pays for itself on a large project and is overhead on a small one.

Four things the PM must never become, so they are not rediscovered:

- **The only agent that may talk to the user.** Selecting a worker still opens that worker's chat.
- **The owner of the processes.** The PM asks; the host spawns; the coordinator owns the children;
  the UI draws. The PM is composed through the library like every other agent.
- **A harness the user did not knowingly hire.** The node shows the harness; so does the composer.
- **A spawn menu with a chat skin.** Orchestration without a brief is the user, typing, which they
  can already do. The brief (§7) is the point of having a PM at all.

**"Which agent may do what" is one table, not scattered checks.** A role carries its policy, and
the PM is simply the role with everything on:

| Policy | Meaning |
|---|---|
| `may_spawn` | May create hosted agents, up to `max_children` |
| `may_create_session` | May open a new working area, worktree included |
| `may_message` | May address other agents in the project |
| `may_write` | Whether its run is composed read-only |
| `mcps`, `skills`, `model`, `harness` | Bindings handed to the run spec |

A role is a **binding**, in the sense
[`config-persistence-proposal.md`](./config-persistence-proposal.md) uses: Ubiq stores the choice,
the harness library owns what the choice refers to. A role names skills and MCP ids; it never
contains one.

## 7. Shaping the work before any of it runs

A new working area — from the `+` button, or created by the PM — does not open on an empty prompt.
It opens on an **analyst**: a hosted agent whose only job is turning what the user wants into
something a task can be built from. A form cannot do that job, because the questions that decide
whether work succeeds have their good answers in the repository, and an analyst that has read it
asks better ones than a field labelled "Goal". Three hats, one conversation: an assistant that asks
and restates, an analyst that turns vague intent into structure, a refiner that proposes and is
corrected until the description is sharp enough to act on — or to walk away from.

**It produces one artifact: the brief.** Four parts, and they are the four things a later agent
should not have to invent:

| Part | Is |
|---|---|
| **Requirements** | What has to be true when this is done. Testable where it can be. Including what is out of scope and what must not change |
| **Goal** | One or two sentences the board, the graph and every identity brief can quote |
| **What to do** | The breakdown: subtasks, order, which files, which branch |
| **How to do it** | The proposed shape — solo, pipeline, crew — with the roles and harnesses it would use |

The brief becomes the area's `goal`, the first task's prompt, and the text §8's identity brief
injects into every agent that follows. Written once, quoted everywhere.

**The brief is a gate.** An area stays `Draft` until the user accepts it, and accepting is what
creates the first task and lets anything spawn. So the analyst's policy is deliberately narrow —
read anything, spawn nothing, write nothing but the brief — which is what makes it safe to start
automatically on every new area. **It proposes; it never starts.** An analyst suggesting a crew of
nine is making a suggestion, and the budget that crew needs is the user's to grant.

**A brief can live without a task.** Refine, then just code: the user accepts the brief, opens the
area, spawns agents by hand, and the brief is in every prompt as context with no board row. The
brief is a description of work, not a commitment to orchestrate it.

**Skipping is one click, and must stay one click.** A user who already knows what they want types
it into the same box and gets a solo task — or an untasked area. Being interviewed about a one-line
fix is exactly the friction that makes people stop using a tool.

**When the PM opens the area it hands over a draft brief**, not a blank one — it has the
conversation the user just had — and the analyst refines that with the user, so nobody is shown a
working area whose goal only an agent understands. The analyst hat never comes off: a task that
goes sideways goes back into refinement, in the same conversation, while other tasks run. **The
interview is kept**: the brief links to the transcript that produced it, because in a month "why is
this scoped this way" has an answer and that conversation is it.

## 8. Identity, and the surface agents reach each other by

[`../backlog.md`](../backlog.md) G7 says the MCP surface Ubiq exposes to hosted agents is a module
header and what it offers is undecided. This is the answer, and it is the mechanism the whole idea
runs on: `crates/ubiq-host/src/mcp_server.rs`, hosted through the library's `inproc-mcp` feature
and injected into every hosted run as an ordinary remote MCP server.

**Identity is injected, not discovered.** Every hosted run is composed with an identity brief: the
agent's name and role, its area and that area's brief, its task, its parent, the names and roles of
its siblings, and how to reach them. It is Ubiq-generated text handed to the run spec through the
library's instructions path — the one `--instructions` uses — so no new mechanism is needed and
harness-specific placement stays the library's problem. Identity is not flavour text: if two agents
are both "Claude" and neither knows its name, `agents.send` has nothing to address. The short name
on the node is the short name in the prompt is the short name in `agents.list`. One string.

**The id is a ULID; the address is a name.** Agents refer to each other as `reviewer-2`, unique
within a project and readable in a prompt. Making agents pass 26-character ids to each other would
be correct and unusable.

The tool surface, minimal on purpose:

| Tool | Who may call it | Does |
|---|---|---|
| `agents.list` | Anyone | The agents in my task and my session: name, role, harness, activity |
| `agents.send` | `may_message` | Puts a message in another agent's inbox. Never interrupts them |
| `agents.inbox` | Anyone | Drains my messages |
| `agents.spawn` | `may_spawn` | Creates a hosted agent from a role, in my session, under me |
| `status.set` | Anyone | My own account of what I am doing — channel 4 of §4 |
| `task.subtask` | Anyone | Declares or updates a subtask under my task |
| `task.done` | Anyone | Reports my part finished, with a summary and the artifacts |
| `ask_user` | Anyone | Escalates a question to the human, through the PM if there is one |
| `session.create` | `may_create_session` | A new working area, optionally a new worktree and branch |

`agents.spawn` is worth pausing on, because it is a new direction on the bus: a message that
originates in a hosted agent, crosses the MCP into the host, and comes back out as a spawned pane
the UI did not ask for. The contract has to allow that without letting the agent dictate layout or
focus — where the pane opens is the UI's decision, which is Q6's answer.

**Messages are a mailbox, not a channel, and that is a fact about harnesses rather than a design
preference.** A harness is a turn loop driven by a prompt; there is no way to hand it an incoming
message mid-turn except by writing to its input, which corrupts the conversation it is having. So
delivery is: the sender's `agents.send` returns immediately; the message sits in the recipient's
inbox; the recipient collects it by calling `agents.inbox`, which its identity brief tells it to do
between steps. For a recipient that has gone idle, Ubiq may additionally *nudge* — write a short
prompt naming the waiting messages — and it does that **only on an idle transition**, never into a
running turn. An agent that never checks its inbox is a role-prompt problem, and the graph shows it
as an undrained inbox count on the node, which is the diagnosis.

**Every tool call is a graph event.** A spawn draws an edge, a send draws a transient one, a
`status.set` repaints a node, a `task.subtask` adds a row in Tasks mode. The surface and the graph
are the same data seen twice, which is what keeps the picture honest.

## 9. Tasks, and the register that holds them

A task is the durable half of the idea — the thing that survives its agents, which is what makes it
worth having a record at all. A finished stage's agent is `Ended`; the task's row still counts it.

| Field | Meaning |
|---|---|
| `id` | A ULID |
| `session_id` | The working area it runs in. A task never spans sessions |
| `title`, `goal` | What it is called, and the prompt every agent in it inherits |
| `shape` | `Solo`, `Pipeline { stages[] }`, `Crew { coordinator_role }` |
| `state` | `Draft`, `Running`, `Blocked`, `Review`, `Done`, `Failed`, `Cancelled` |
| `budget` | Agent count, wall clock, tokens. §13 |
| `created_at`, `started_at`, `ended_at` | |
| `artifacts` | Branch, files touched, documents written, the summary each finished agent left |
| `brief` | The accepted brief, with a link to the interview that produced it |

**A task never spans working areas**, because an area is a folder and a task that ran in two
folders has no coherent diff. The earlier draft's counter-case — research on main, implement on a
worktree — dissolves once read and write are separated: every stage *runs* in the task's session,
and a stage that needs to consult another checkout reads those files, which requires no second
session. Work that must *write* two areas is two tasks, and the PM working across areas — creating
and following tasks in several — is exactly how that is expressed.

**Two registers, one screen, one authority.** Tasks and subtasks Ubiq created are Ubiq's record;
the todo list a harness keeps in its own head is the harness's, and Ubiq mirrors what an agent
reports through `task.subtask` without pretending to own it. A mirrored subtask is marked reported
and disappears with its agent — reconciling the two would make Ubiq wrong about both.

**Tasks mode is that register, and the Tasks drawer is the same register docked.** Tasks mode is a
list per area, grouped by state, each row expanding to its subtasks, agents and artifacts, with the
graph one click away. While Agents mode is selected, the same records project into a drawer under
the canvas — shape, title, progress, the agents with their activity — so graph and board are two
lenses on one run. Two views are fine; two *boards* would be the mistake.

Documentation is not a special case — a documentation stage is a stage whose artifact is a
document, a docs agent is an agent whose role is documentation, and KB mode reads the artifacts
back. It is a surface fed by the same agents the graph shows, not a fourth shape.

## 10. The message families

Two new families beside the pane and session families, and three additions to what exists. Every
variant that names an agent names it by id, and **the host sends a graph, not a drawing** — no
coordinates, no layout, no colour crosses the bus.

**Additions to existing records.** `WorkspaceInfo` grows `name`, `role`, `parent_id`, `task_id`,
`origin` (`Hosted` | `Reported`) and `activity`. `SessionInfo` grows `goal`, `git_ref` and `state`.
Those two changes alone are most of the value, and they are phase 1.

### The task family

| Message | Direction | Payload | Responds with |
|---|---|---|---|
| `ListTasks` | UI → host | `session_id?` | `TaskList` |
| `CreateTask` | UI → host | `session_id`, `title`, `goal`, `shape` | `TaskChanged` |
| `StartTask` | UI → host | `task_id` | `TaskChanged`, then agent events |
| `StopTask` | UI → host | `task_id` | `TaskChanged` |
| `UpdateTask` | UI → host | `task_id`, `title?`, `goal?`, `state?` | `TaskChanged` |
| `TaskList` | host → UI | `tasks[]` | — |
| `TaskChanged` | host → UI | `task` | — |

### The agent family

| Message | Direction | Payload | Responds with |
|---|---|---|---|
| `RequestGraph` | UI → host | `project_id` | `GraphSnapshot` |
| `GraphSnapshot` | host → UI | `sessions[]`, `tasks[]`, `agents[]`, `edges[]` | — |
| `AgentAppeared` | host → UI | `agent` | — |
| `AgentActivity` | host → UI | `agent_id`, `activity`, `since`, `observed_at`, `source`, `note?` | — |
| `AgentGone` | host → UI | `agent_id`, `outcome` | — |
| `AgentTraffic` | host → UI | `from`, `to`, `at`, `kind` | — |
| `SendToAgent` | UI → host | `agent_id`, `text` | — |
| `RequestTranscript` | UI → host | `agent_id`, `after?` | `TranscriptPage` |
| `TranscriptPage` | host → UI | `agent_id`, `entries[]`, `more` | — |
| `TranscriptAppended` | host → UI | `agent_id`, `entries[]` | — |

Four rules on top of the existing framing rules:

**A snapshot is always available, and the deltas are an optimisation.** A graph rebuilt only from a
stream drifts, and a UI that reconnects — a second window, a reattaching client — needs one message
that says everything. This is `SessionAttached`'s existing shape, for the same reason.

**Activity is coalesced by the host.** A crew mid-flight produces hundreds of tool events a second,
and a graph repainting per event flickers and floods the bus. The host emits at most one
`AgentActivity` per agent per interval, carrying the latest state; the transcript keeps the
history. Coalescing host-side is what stops a slow window from being the thing that decides.

**Traffic is an event, not state.** `AgentTraffic` exists to animate an edge and to be forgotten.
The durable record of a message is in the recipient's transcript.

**The UI infers nothing.** It never decides an agent is idle, never derives an edge from timing,
never promotes a stale observation. If it is not in a message, it is not on the screen.

## 11. The screen

Two rail modes that currently render an empty page — G11 — and one extension to panes. The mockups
[`agent-view-proposal.png`](./agent-view-proposal.png) and
[`agent-view-proposal-with-tasks.png`](./agent-view-proposal-with-tasks.png) are the target
picture: three tasks in three shapes under one PM, one canvas.

**Agents mode is the graph** — a map of the people in the room, not a terminal layout, not a
roster, not a scrolling log. Sessions are containers, tasks are labelled groups inside them, agents
are nodes. That nesting is legal *because* a task never spans sessions (§9), which is what
dissolves the drafts' group-by-place versus group-by-task fork: a pipeline never splits across
bands, and an untasked area draws as a quiet group — the current product, visible from Agents mode,
not a solitary node in whitespace.

**Edges are three kinds, and the default view shows structure**: delegation (dashed, parent to
child), handoff (a pipeline's numbered arrow between stages), and traffic (transient, animated,
gone in a second — a toggle, never a permanent thicket, or every crew is a complete graph).
Clicking an edge answers "why is this arrow here": the spawn reason, the handoff artifact, the last
message.

**A node carries** its name and role, a harness badge, the path chip of its session, the activity
chip in its status colour with elapsed time in state, and an undrained-inbox count when it has one.
The card's left edge takes the status colour — the shell's existing `ACCENT_EDGE` convention — and
colour comes from the theme's status group, so a red node means the same thing it means everywhere
else. Status is never wording alone. Filter chips along the top — running, waiting, ended, error —
are activity as a lens, not a new object.

**Layout is stable, not pretty.** Nodes are placed by depth from the task's root and keep their
slot for its life; a new sibling appends rather than reflowing its neighbours; a finished agent
leaves a dimmed node until the task ends. A graph that rearranges while the user is reading it is
worse than an ugly one — the user is tracking motion, and every avoidable movement is noise. Above
a threshold a crew collapses to one node with a member count.

**Selecting a node opens an inspector**: identity, role, area, task, parent, harness, model,
account, budget consumed, tools run, and the actions — open its chat, open its terminal, message
it, stop it. Hosted or reported is stated in the chrome, never left to be deduced from a missing
button.

**Opening several chats at once is a pane, not a window.** Ubiq already arranges rectangles with
exactly one focused: the pane layout, whose split and grid modes exist as an enum and are not drawn
(G6). So a pane gains a **kind** — `Terminal`, backed by an emulator, or `Transcript`, backed by an
agent's event history — and nothing else about panes changes: one agent one pane, exactly one
focused, an ended agent's pane keeps its last state. A reported agent can only have a transcript
pane, and one with no input path draws no composer; a hosted agent's transcript pane may be opened
read-only, and a read-only view never steals focus. That makes "open several chats, some
read-only" fall out of machinery that exists rather than a second window manager; it answers Q6,
since a pane names an agent and the agent names its parent; and it gives
[`../features/chat.md`](../features/chat.md) something real behind it — G10.

The chat is a projection of structured events, never a parse of the terminal — D2 stands. This
costs the overview's non-goal one clause: Ubiq still does not call a model or own a conversation;
it *does* render a conversation the host extracted from a run, because otherwise the graph has
nowhere to open.

## 12. Failure

Multi-agent work fails in ways single-agent work does not, and most of them are quiet.

| What happens | Result |
|---|---|
| A crew's coordinator exits mid-task | Its children keep running and are marked orphaned. Nothing is killed on the user's behalf |
| A hosted agent's harness dies | `Ended` with its code. Its pane stays showing its last screen. Its task goes `Blocked`, not `Failed` |
| A reported agent's parent dies | The child is finalised `Unknown`. Ubiq never watched it and does not claim it did |
| An agent stops reporting | The node goes faint, then `Unknown`. Never `Idle` |
| An agent messages one that has ended | The send returns a tool error naming the state. Messages are never silently dropped |
| An agent never drains its inbox | The node shows the count. No nudge is written into a running turn |
| A spawn would exceed the task's agent budget | Refused, as a tool error the agent can read and reason about, not a crash |
| Delegation cycles — A spawns B spawns A | A depth cap and a per-task budget. The cap is a refusal, not a deadlock |
| Two agents in one crew edit the same file | Nothing prevents it. §2's mitigations: separate directories by role, or a worktree per member with a merge stage |
| The analyst is never answered | The area stays `Draft`. Nothing was spawned and nothing was spent |
| The user rewrites the brief by hand | It is text, and it wins. The analyst is a convenience, not an authority |
| A pipeline stage fails | The task stops at that stage, keeps every prior artifact, and offers a retry from there rather than from the top |
| A worktree the session owns is gone | The session is `Missing`, like a project. Spawning into it is refused before a pseudo-terminal exists |
| The graph outgrows the canvas | Crews collapse to a count; sessions collapse to a header. No node is dropped |
| A harness supports no hooks | Activity degrades to process liveness plus self-report, and the node says which channel it is on |
| The user stops a task | Every agent in it is killed. This is the only action that ends more than one pane, and it asks first |

**One rule stands over the table: nothing disappears from under the user.** It is already the pane
rule, and a tidy graph is a tempting way to break it. A finished agent leaves a node, a failed
stage leaves its artifacts, an orphaned crew runs until someone says otherwise.

## 13. Cost, and the stop button

A crew is the first thing in Ubiq that can spend real money while the user is looking away.

**A budget is per task, and it is three numbers**: agents, wall clock, tokens. Structured runs
report usage exactly through `AgentEvent::Usage`; terminal runs report what hooks or self-report
give, and the node says which. Hitting one **pauses** — no more spawning, no more nudging, the task
goes `Blocked` and asks — it never kills work in progress. **Stop** is the other half, and it is
honest: it kills every hosted agent in the task, leaves every pane showing its last screen, leaves
the worktree as the agents left it, and reverts nothing. It asks first, being the only multi-pane
kill in the product.

## 14. Five scenarios, as a check on the model

Each exercises a piece the sections above claim; a design change that breaks one of these sentences
has changed the product.

**Just code.** One area (main), one spawned harness, typing. No brief, no board row, no PM. Agents
mode shows one quiet node. Today's product, with a name on the place.

**Refine, then code.** The user tells the PM "the host crate is a mess." The PM asks what mess
means, proposes a brief, the user cuts a subtask and adds a requirement, accepts. No task: they
spawn two agents into the area by hand and the brief is in both prompts. The Tasks register is
empty; the graph is the PM plus two nodes.

**A crew in one area.** After refinement the PM creates a task, shape crew, on main. A coordinator
spawns a codex implementer and a claude reviewer — identity injected, MCP shared, brief in every
prompt. The graph is a star. The user reads the reviewer's transcript pane read-only and types into
the implementer's terminal. `AwaitingInput` lights on the coordinator when it wants the merge
accepted.

**A pipeline on a worktree.** A breaking change that must not land on main: the PM creates an area
from a worktree on `feat/auth` and a pipeline in it — research → implement → document → review.
Every stage runs in that area; the research stage reads main's checkout without owning it. The
graph is a path that changes colour as the token moves; the reviewer sits `Queued` until the
builder's artifact lands. The user opens the two transcript panes at the handoff, because that is
where pipelines go wrong.

**A native child beside a foreign one.** A claude coordinator spawns a claude subagent internally
for a narrow edit, and asks the host — `agents.spawn` — for a codex child for the tests. Three
nodes: coordinator (hosted), native child (reported, no pane, transcript is a slice of the
parent's), codex (hosted, own pane). The chrome says which is which; the user can read the native
child and cannot focus a terminal it does not have.

## 15. What the drafts left open, closed here

The two source documents forked on real questions. The closures, with the reasoning where it is not
already in a section above:

- **The place-word** goes to [`session-naming-proposal.md`](./session-naming-proposal.md):
  `workspace` via the cascade, `lane` without it. Not re-argued here.
- **PM occupancy** is *main*, never none, never both — §6.
- **Group the graph by area or by task** dissolves: areas contain, tasks group inside, because a
  task never spans an area — §11.
- **A task never spans areas**; a stage may read other checkouts, and cross-area *writes* are two
  tasks — §9.
- **One chat or several** is answered by pane kinds, not by a second window manager — §11. Movable
  panels remain a separate proposal this does not depend on.
- **Read-only chat** is two things: a construction fact for reported agents, a view flag for hosted
  ones — §3, §11.
- **Tasks rail or drawer** is both, one record set, two projections — §9.
- **Graph, roster or layout-with-chrome**: the graph, as the mockups draw it. A roster was the
  cheaper 80% answer; the mockups exist and the arrows carry the shapes, so the fork is spent.
- **Native subagents in v1** are reported nodes; wrapping them into hosted panes is not attempted.
- **Does a task survive its agents** — yes; it is the durable record — §9.
- **Does a brief live without a task** — yes — §7.
- **Activity grain**: the enum in §4, including `Queued` and `AwaitingInput`, sourced per channel.
- **The library session** maps to the agent, settling Q4 — §1.

And the alternatives considered and rejected, kept so they are not re-proposed: a UI-wizard PM (a
form cannot refine a brief); the PM as the only coordinator (three live tasks would give one agent
three hats); skipping the brief for orchestrated runs (a vague goal inherited by every agent);
chat-as-the-terminal (D2, and it gives neither read-only nor several-at-once nor a citable
transcript).

## 16. Phases

Each is useful on its own. Two tracks run independently until tasks tie them together: the **graph
track** (1–3), which needs no orchestration to be worth shipping, and the **conversation track**
(4, 6, 7), which is the brainstorm's analyst-first slice — a chat that produces a brief needs the
MCP surface and a transcript pane, not the graph.

| # | Phase | What lands |
|---|---|---|
| 1 | **Identity on what already runs** | `name`, `role`, `parent_id`, `task_id`, `origin` on the agent record; `goal` and `git_ref` on the area; the rename, if it is taken |
| 2 | **The graph** | Agents mode, `RequestGraph`/`GraphSnapshot`, node, inspector, delegation edges. Activity is process liveness, and already worth looking at |
| 3 | **Activity from hooks** | Channel 2 through the harness library; the activity record with its source and `observed_at`; staleness as a rendering |
| 4 | **The MCP surface** | `crates/ubiq-host/src/mcp_server.rs` on `inproc-mcp`: identity brief, `agents.list`, `status.set`, `agents.send`, `agents.inbox`. Closes G7 |
| 5 | **Transcript panes** | The pane kind, the split and grid modes G6 leaves undrawn, several chats at once — G10 |
| 6 | **The analyst and the brief** | The intake role, the four-part brief, the `Draft` gate, the skip path |
| 7 | **Tasks** | The record, Tasks mode and the drawer, `Solo` and `Pipeline`, the stage sequencer, the artifact handoff, budgets |
| 8 | **Crews** | `agents.spawn`, the role table and its policy, depth caps, orphan handling |
| 9 | **Reported agents** | Parent-reported nodes from hook and event streams; the read-only transcript |
| 10 | **The PM** | A role, an area on the main worktree, `session.create` and `ask_user` |
| 11 | **Areas as worktrees** | Creation, branch binding, removal that never removes what it did not create. Waits on version control in the host |

Phases 1–3 are independent of the project-handling proposal. From phase 4 on, this assumes the
harness library is wired in — G1 and G21 — because injecting an MCP surface needs a composed run.

## 17. What this asks to be decided

- Ubiq hosts orchestration and does not perform it, with exactly one exception: the pipeline stage
  sequencer, which needs no model.
- A session grows a goal, a git ref and a `Draft`/`Open` state, and is the isolation boundary. A
  task never spans one; an area without a task is first-class.
- An agent is either hosted or reported: a contract fact, not a UI mode. Reported agents cannot be
  addressed and their transcripts are read-only by construction; a hosted agent's transcript may be
  opened read-only as a view.
- Activity is observed on four named channels, every value carries its channel, and silence is
  never promoted to idle.
- A terminal and a structured event stream are mutually exclusive per agent, and the role chooses.
- The harness library's session belongs to the agent, not to Ubiq's session — which settles Q4.
- Cross-agent messaging is a mailbox with pull delivery, plus a nudge that only lands on an idle
  transition.
- The MCP surface in §8 is what Ubiq exposes to hosted agents — which settles G7 — and
  `agents.spawn` is a host verb: an agent asks, the host spawns, the UI decides where the pane
  opens — which is Q6's other half.
- A PM is a role with a wide tool policy, sitting on the project's main worktree, and a project
  without one works.
- A pane gains a kind, `Terminal` or `Transcript`; multi-chat reuses the pane layout rather than
  introducing a second one — which settles Q6.
- The host sends a graph and never a drawing; layout, colour and collapse are the UI's alone, and
  layout is stable across ticks — nodes keep their slot, finished nodes persist to the task's end.
- The graph nests area → task → agent; the Tasks drawer and Tasks mode are two projections of one
  register.
- A task carries a three-number budget, and exceeding it pauses rather than kills.
- Ubiq's `session` is renamed, per the session-naming proposal. The word stops carrying two
  senses.
- A working area opens on an analyst; its four-part brief is a gate — nothing spawns until the
  brief is accepted — and a brief may also be taken without a task, as context for hand-driven
  work. Skipping the analyst is one click.
- The analyst may read anything and spawn nothing. It proposes a shape; it never starts one.
- The overview's "not a chat client" non-goal is reworded: Ubiq does not call a model; it does
  render a conversation the host extracted from a run.

Backlog rows this opens: what a bounded transport does with a crew's event volume; whether a
transcript is persisted or dies with the process; whether a task survives a restart, when each
agent's resume is the library's; how a nudge reaches a harness with no prompt-injection path; what
happens when two crew members stage conflicting edits in one worktree; and whether roles are per
project, per user, or both.

## Related docs

- [`agent-graph-proposal.md`](./agent-graph-proposal.md), [`graph-harness-brainstorm.md`](./graph-harness-brainstorm.md) — the two drafts this merges and supersedes
- [`graph-harness-ideas.md`](./graph-harness-ideas.md) — the raw intent behind both
- [`session-naming-proposal.md`](./session-naming-proposal.md) — the place-word, decided on its own
- [`../features/sessions-and-workspaces.md`](../features/sessions-and-workspaces.md) — the session and workspace this extends
- [`../features/panes-and-terminals.md`](../features/panes-and-terminals.md) — the pane rules a transcript pane inherits
- [`../features/chat.md`](../features/chat.md), [`../features/workbench.md`](../features/workbench.md) — the panel a transcript pane reuses, and the rail modes this fills
- [`../tech/transport-contract.md`](../tech/transport-contract.md) — where the two families land
- [`../tech/agent-manager.md`](../tech/agent-manager.md) — hooks, structured I/O and in-process MCP are all its side of the line
- [`project-handling-proposal.md`](./project-handling-proposal.md), [`config-persistence-proposal.md`](./config-persistence-proposal.md) — the host, the project, the ids and the bindings this assumes
- [`../backlog.md`](../backlog.md) — G7, G10, G11, Q4 and Q6, which this answers
