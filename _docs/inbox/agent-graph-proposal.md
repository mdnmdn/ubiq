---
id: inbox-agent-graph
title: Proposal — the agent graph
kind: proposal
status: proposal
summary: Agents as a graph rather than a row of terminals — the session that owns a worktree, the task that owns a goal, the role that owns a permission, the four channels an agent's activity can honestly be known through, the MCP surface agents reach each other by, and the two screens that draw all of it.
read_when: you are deciding how several agents cooperate on one goal, what an agent knows about the others, or what the Agents and Tasks rail modes are for
updated: 2026-08-31
depends_on: [feat-sessions, feat-workbench, tech-transport, tech-agent-manager]
---

# Proposal — the agent graph

> Superseded by [`agent-graph-final.md`](./agent-graph-final.md), which merges this document with
> [`graph-harness-brainstorm.md`](./graph-harness-brainstorm.md). Kept as source material.

Ubiq today is a multiplexer: panes side by side, each an independent harness, related only by
sharing a window. This proposes the layer above — several agents working one goal, knowing about
each other, reporting what they are doing, drawn as a graph instead of a row.

It is a deep dive on [`graph-harness-ideas.md`](./graph-harness-ideas.md), and answers the questions
that idea does not: how Ubiq learns what an agent is doing without parsing its terminal, which agents
it can drive and which it can only watch, what a task is, and where the line falls between
orchestration Ubiq performs and orchestration it hosts.

**The line, stated once:** Ubiq hosts orchestration; it does not perform it. The one exception is the
pipeline sequencer in §5, and only because it needs no model to decide anything. Everything else —
who to spawn, what to delegate, when a thing is done — is an agent's call, and Ubiq's job is to make
it visible, addressable and stoppable.

## 1. The vocabulary, before anything else

Four words are already spoken for and the idea introduces two more. The whole design is a nesting,
and a nesting whose levels share a name cannot be discussed.

| Level | What it is | Owns |
|---|---|---|
| **Project** | A repository Ubiq remembers. A window is opened on one | The catalogue record, the config root |
| **Session** | A working area with a goal: one folder, one branch, one worktree | A directory, a git ref, its tasks |
| **Task** | A unit of intent inside a session — a goal, a shape, a state | Its agents, its subtasks, its artifacts |
| **Agent** | One running harness with an identity | A conversation, a pane, a working directory |
| **Role** | A named bundle an agent is spawned from | A prompt fragment, a harness, a tool policy |

Three of those exist. **Project** is [`project-handling-proposal.md`](./project-handling-proposal.md).
**Session** is [`../features/sessions-and-workspaces.md`](../features/sessions-and-workspaces.md) —
"a named piece of work that owns a folder", exactly the idea's "working area with a goal", and it
grows two fields rather than a new concept. **Agent** is the workspace: one harness, one directory,
one terminal; this document says *agent* for identity and *workspace* for process, and they are one
record. **Task** and **role** are new, and they are the entire addition.

The word *session* itself should not survive, because Ubiq spends it twice —
[`session-naming-proposal.md`](./session-naming-proposal.md) is that question, with the candidates
and a recommendation. This document keeps saying "session" for the thing being renamed and
**working area** where the sentence would otherwise be ambiguous.

The collision this design does resolve: the harness library's *session* is a resumable conversation,
Ubiq's is a folder full of work, and [`../backlog.md`](../backlog.md) Q4 asks which maps onto which.
**The library's session belongs to the agent.** One agent is one resumable conversation, so resuming
a task is a fan-out over its agents, not a lookup.

```
   project ──┬── area "auth-refactor"   (worktree, branch feat/auth)
             │     ├── task "extract the token store"  [pipeline]
             │     │     ├── agent implementer (codex)  tool: edit
             │     │     └── agent reviewer    (claude) idle
             │     └── task "write the migration note" [solo]
             └── area "main"            (the repo itself)
                   └── agent pm         (claude) awaiting input
```

## 2. Where a session grows, and where it does not

A session gains three durable fields and one derived one: **`goal`**, one or two sentences on what
this area is for, injected into every agent's identity brief; **`git_ref`**, the branch it works on,
absent meaning whatever the folder is already on; **`state`**, `Draft` until its brief is accepted
and `Open` after (§7); and derived, whether the folder is a linked **worktree** the host created or
one that was already there.

**A session is a worktree when it needs to be, and Ubiq never converts one silently.** Creating one
offers three placements: the project's own folder, an existing folder, or a new linked worktree on a
new branch — the third is what makes parallel areas safe. Removing a session never removes a worktree
Ubiq did not create.

**Sessions are the isolation boundary; tasks and crews are not.** Two sessions on two worktrees
cannot collide. Two tasks in one session share a folder, and so do two agents in one crew. §12 says
what that costs — it is the hazard this design does not remove.

## 3. Two kinds of agent, and why it decides the UI

This is the load-bearing distinction, and every screen decision follows from it.

**A hosted agent is one Ubiq spawned.** It has a pane, a pseudo-terminal, a harness the user picked,
an identity Ubiq injected, and an MCP surface Ubiq exposes. Ubiq can watch it, type into it,
resize it, message it and kill it. Everything in the existing contract already applies.

**A reported agent is a harness's own subagent** — Claude Code's task tool, grok's subagents. It runs
*inside* the parent's process. It has no pseudo-terminal, no pane, no process Ubiq can see, and no
MCP connection of its own. Ubiq knows it exists only because the parent said so, in a hook or a tool
event.

| | Hosted | Reported |
|---|---|---|
| Spawned by | Ubiq, through the harness library | The parent harness, internally |
| Pane | A terminal pane | None — a transcript pane at most |
| Input | Keystrokes, prompts, messages | None. It cannot be addressed |
| Harness | Any Ubiq supports, independent of the parent's | Always the parent's |
| Activity | Observed on four channels (§4) | Whatever the parent reported, when it reported it |
| Ending | An exit code from the coordinator | The parent saying so, or silence |

Two consequences the idea asked for without naming the cause. **The read-only chat is not a
preference — it is what a reported agent can offer**, and a composer on a node that cannot receive
input would be a lie. And **a mixed-harness crew is necessarily hosted**: a Claude Code subagent is a
Claude Code subagent, so "a codex reviewer under a claude implementer" is two hosted agents with a
delegation edge.

**Reported agents are drawn, never inferred.** A node appears when the parent reports it and is
finalised when the parent reports it finished. A parent that dies mid-flight leaves its reported
children in `Unknown`, never in `Ended` — Ubiq did not watch them and must not claim it did.

## 4. How Ubiq knows what an agent is doing

The idea wants a node to say *idle, thinking, running a tool, waiting for input, ended, error*. The
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
  for every state the idea listed.
- **A structured agent** — no terminal, a transcript pane, full event detail, driven by prompt. The
  shape for a crew member nobody watches keystroke by keystroke, and the only faithful token count.

**Hooks are the rung that matters most**: the only non-cooperative source of tool-level detail, and
the library already injects them per harness. A `PreToolUse` hook posting the tool name to Ubiq's
loopback endpoint turns `Tool { name }` from a claim into an observation. Which harnesses expose
which hook points is the library's fact, and a harness with none degrades to channels 1 and 4.

**Staleness is rendered, not stored.** Every activity carries `observed_at`; a node whose last
observation is old draws faint, and one silent past a threshold reads `Unknown`. Ubiq never promotes
silence to `Idle` — "it finished" and "it stopped telling us" is the distinction the user most needs.

```
Starting · Idle · Thinking · Responding · Tool{name} · AwaitingApproval
AwaitingInput · Blocked{on} · Ended{code} · Failed{error} · Unknown
```

each carried with `since`, `observed_at`, `source`, and an optional note the agent supplies.
`Blocked{on}` is not a harness state: it is an agent saying it waits on another agent, a lock or a
human, and it is what makes a stalled crew legible at a glance.

## 5. The three shapes, and the one Ubiq performs

The idea's three ways of starting work are three shapes of task.

| Shape | Who decides the next step | Agents | Ends when |
|---|---|---|---|
| **Solo** | The user, in conversation | One | The user says so, or the agent reports the task done |
| **Pipeline** | The task's declared stage list | One at a time, in order | The last stage passes, or a stage fails |
| **Crew** | A coordinator agent | Several, concurrent, each with a role | The coordinator reports done |

**Pipeline is the shape Ubiq owns.** A stage list is a state machine — start stage *n*, wait for its
agent to finish, hand its result to stage *n+1* — and running it in the host needs no model, costs
no tokens, and cannot hallucinate a step. That is why it is worth having as a distinct shape rather
than as "a crew whose coordinator was told to go in order".

A stage hands off through an **artifact, not a conversation**: it writes a result file into the
task's scratch directory and returns a summary line, and the next stage's prompt names both. No stage
inherits the previous one's context window — a pipeline exists to spend a fresh context per step.

**Crew is the shape an agent owns.** The coordinator is an ordinary hosted agent with a role that
permits spawning, and it spawns and messages its members through the MCP surface in §8. Ubiq
supervises: it enforces the budgets, draws the graph, and can stop the whole task. It does not
second-guess the coordinator's plan, and it has no opinion about what a good crew looks like.

**Solo is not a degenerate case to be folded away.** It is what most work is, it needs no task
record until the user wants one, and every pane running today is already one, untitled.

## 6. The PM is a role, not a component

The idea's project orchestrator is tempting to build as a special thing. It should not be.

**A PM is an ordinary hosted agent with a wide tool policy, in a session bound to the project's main
worktree.** It has a pane like everything else, so the user can watch it. It holds the conversation
where the user says "start a task", it creates tasks, sessions and agents through tools, and it is
the default recipient of an escalation. It is a row in the role table, not a new subsystem.

Three consequences worth stating so they are not rediscovered:

**Every agent has a session, including the PM.** The idea asked whether a PM might have none. It
cannot: every agent has a working directory, and a null session would hole every node, edge and
query. The PM's is the project's main worktree, and it is read-mostly there.

**A project without a PM is a working project.** The user drives tasks directly, which is the whole
product today. A PM pays for itself on a large project and is overhead on a small one.

**"Which agent may do what" is one table, not scattered checks.** A role carries its policy, and the
PM is simply the role with everything on:

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

A new working area — from the `+` button, or created by the PM — does not open on an empty prompt. It
opens on an **analyst**: a hosted agent whose only job is turning what the user wants into something
a task can be built from. A form cannot do that job, because the three questions that decide whether
work succeeds — what changes, how we will know it worked, what must not change — have their good
answers in the repository, and an analyst that has read it asks better ones than a field labelled
"Goal".

**It produces one artifact: the brief** — the goal, the requirements it must satisfy, the acceptance
checks that say it worked, what is explicitly out of scope, the constraints (which files, which
branch, what must not change), and a proposed shape, with the roles it would use. The brief becomes
the area's `goal`, the first task's prompt, and the text §8's identity brief injects into every
agent that follows. Written once, quoted everywhere.

**The brief is a gate.** An area stays `Draft` until the user accepts it, and accepting is what
creates the first task and lets anything spawn. So the analyst's policy is deliberately narrow — read
anything, spawn nothing, write nothing but the brief — which is what makes it safe to start
automatically on every new area. **It proposes; it never starts.** An analyst suggesting a crew of
nine is making a suggestion, and the budget that crew needs is the user's to grant.

**Skipping is one click, and must stay one click.** A user who already knows what they want types it
into the same box and gets a solo task. Being interviewed about a one-line fix is exactly the
friction that makes people stop using a tool.

**When the PM opens the area it hands over a draft brief**, not a blank one — it has the conversation
the user just had — and the analyst refines that with the user, so nobody is shown a working area
whose goal only an agent understands. **The interview is kept**: the brief links to the transcript
that produced it, because in a month "why is this scoped this way" has an answer and that
conversation is it.

## 8. Identity, and the surface agents reach each other by

[`../backlog.md`](../backlog.md) G7 says the MCP surface Ubiq exposes to hosted agents is a module
header and what it offers is undecided. This is the answer, and it is the mechanism the whole idea
runs on: `crates/ubiq-host/src/mcp_server.rs`, hosted through the library's `inproc-mcp` feature and
injected into every hosted run as an ordinary remote MCP server.

**Identity is injected, not discovered.** Every hosted run is composed with an identity brief: the
agent's name and role, its area and that area's brief, its task, its parent, the names and roles of
its siblings, and how to reach them. It is Ubiq-generated text handed to the run spec through the
library's instructions path — the one `--instructions` uses — so no new mechanism is needed and
harness-specific placement stays the library's problem.

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
worth having a record at all.

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

**A task never spans working areas**, because an area is a folder and a task that ran in two folders
has no coherent diff. The idea's "a PM works across sessions" holds — it creates and follows tasks in
several — but one task stays in one.

**Two registers, one screen, one authority.** Tasks and subtasks Ubiq created are Ubiq's record; the
todo list a harness keeps in its own head is the harness's, and Ubiq mirrors what an agent reports
through `task.subtask` without pretending to own it. A mirrored subtask is marked reported and
disappears with its agent — reconciling the two would make Ubiq wrong about both.

Tasks mode is that register: a list per area, grouped by state, each row expanding to its subtasks,
agents and artifacts, with the graph one click away. Documentation is not a special case — a
documentation stage is a stage whose artifact is a document, and KB mode reads those back, which is
all of the idea's "work with the documentation" that belongs here.

## 10. The message families

Two new families beside the pane and session families, and three additions to what exists. Every
variant that names an agent names it by id, and **the host sends a graph, not a drawing** — no
coordinates, no layout, no colour crosses the bus.

**Additions to existing records.** `WorkspaceInfo` grows `name`, `role`, `parent_id`, `task_id`,
`origin` (`Hosted` | `Reported`) and `activity`. `SessionInfo` grows `goal` and `git_ref`. Those two
changes alone are most of the value, and they are phase 1.

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
`AgentActivity` per agent per interval, carrying the latest state; the transcript keeps the history.
Coalescing host-side is what stops a slow window from being the thing that decides.

**Traffic is an event, not state.** `AgentTraffic` exists to animate an edge and to be forgotten. The
durable record of a message is in the recipient's transcript.

**The UI infers nothing.** It never decides an agent is idle, never derives an edge from timing,
never promotes a stale observation. If it is not in a message, it is not on the screen.

## 11. The screen

Two rail modes that currently render an empty page — G11 — and one extension to panes.

**Agents mode is the graph.** Sessions are containers, tasks are groups inside them, agents are
nodes, and edges are three kinds: delegation (solid, parent to child), handoff (a pipeline's arrow
between stages), and traffic (transient, animated, gone in a second). A node carries its name and
role, a harness badge, the activity chip in its status colour, elapsed time in the current state,
and an undrained-inbox count when it has one. Colour comes from the theme's status group — the same
tokens the chat's run pill and the explorer's git states use — so a red node means the same thing it
means everywhere else.

**Layout is stable, not pretty.** Nodes are placed by depth from the task's root and keep their slot
for its life; a new sibling appends rather than reflowing its neighbours; a finished agent leaves a
dimmed node until the task ends. A graph that rearranges while the user is reading it is worse than
an ugly one — the user is tracking motion, and every avoidable movement is noise. Above a threshold a
crew collapses to one node with a member count.

**Selecting a node opens an inspector**: identity, role, area, task, parent, harness, model,
account, budget consumed, tools run, and the actions — open its chat, open its terminal, message it,
stop it.

**Opening several chats at once is a pane, not a window.** Ubiq already arranges rectangles with
exactly one focused: the pane layout, whose split and grid modes exist as an enum and are not drawn
(G6). So a pane gains a **kind** — `Terminal`, backed by an emulator, or `Transcript`, backed by an
agent's event history — and nothing else about panes changes: one agent one pane, exactly one
focused, an ended agent's pane keeps its last state. A reported agent can only have a transcript
pane, and one with no input path draws no composer. That makes "open several chats, some read-only"
fall out of machinery that exists rather than a second window manager; it answers Q6, since a pane
names an agent and the agent names its parent; and it gives [`../features/chat.md`](../features/chat.md)
something real behind it — G10.

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
rule, and a tidy graph is a tempting way to break it. A finished agent leaves a node, a failed stage
leaves its artifacts, an orphaned crew runs until someone says otherwise.

## 13. Cost, and the stop button

A crew is the first thing in Ubiq that can spend real money while the user is looking away.

**A budget is per task, and it is three numbers**: agents, wall clock, tokens. Structured runs
report usage exactly through `AgentEvent::Usage`; terminal runs report what hooks or self-report
give, and the node says which. Hitting one **pauses** — no more spawning, no more nudging, the task
goes `Blocked` and asks — it never kills work in progress. **Stop** is the other half, and it is
honest: it kills every hosted agent in the task, leaves every pane showing its last screen, leaves
the worktree as the agents left it, and reverts nothing. It asks first, being the only multi-pane
kill in the product.

## 14. Phases

Each is useful on its own, and the graph is on screen before any orchestration exists.

| # | Phase | What lands |
|---|---|---|
| 1 | **Identity on what already runs** | `name`, `role`, `parent_id`, `task_id`, `origin` on the agent record; `goal` and `git_ref` on the area; the rename, if it is taken |
| 2 | **The graph** | Agents mode, `RequestGraph`/`GraphSnapshot`, node, inspector, delegation edges. Activity is process liveness, and already worth looking at |
| 3 | **Activity from hooks** | Channel 2 through the harness library; the activity record with its source and `observed_at`; staleness as a rendering |
| 4 | **The MCP surface** | `crates/ubiq-host/src/mcp_server.rs` on `inproc-mcp`: identity brief, `agents.list`, `status.set`, `agents.send`, `agents.inbox`. Closes G7 |
| 5 | **Tasks** | The record, Tasks mode, `Solo` and `Pipeline`, the stage sequencer, the artifact handoff, budgets |
| 6 | **The analyst and the brief** | The intake role, the brief document, the `Draft` gate, the skip path |
| 7 | **Transcript panes** | The pane kind, the split and grid modes G6 leaves undrawn, several chats at once — G10 |
| 8 | **Crews** | `agents.spawn`, the role table and its policy, depth caps, orphan handling |
| 9 | **Reported agents** | Parent-reported nodes from hook and event streams; the read-only transcript |
| 10 | **The PM** | A role, an area on the main worktree, `area.create` and `ask_user` |
| 11 | **Areas as worktrees** | Creation, branch binding, removal that never removes what it did not create. Waits on version control in the host |

Phases 1–3 are independent of the project-handling proposal. From phase 4 on, this assumes the
harness library is wired in — G1 and G21 — because injecting an MCP surface needs a composed run.

## 15. What this asks to be decided

- Ubiq hosts orchestration and does not perform it, with exactly one exception: the pipeline stage
  sequencer, which needs no model.
- A session grows a goal and a git ref, and is the isolation boundary. A task never spans one.
- An agent is either hosted or reported: a contract fact, not a UI mode. Reported agents cannot be
  addressed and their transcripts are read-only by construction.
- Activity is observed on four named channels, every value carries its channel, and silence is never
  promoted to idle.
- A terminal and a structured event stream are mutually exclusive per agent, and the role chooses.
- The harness library's session belongs to the agent, not to Ubiq's session — which settles Q4.
- Cross-agent messaging is a mailbox with pull delivery, plus a nudge that only lands on an idle
  transition.
- The MCP surface in §8 is what Ubiq exposes to hosted agents — which settles G7.
- A PM is a role with a wide tool policy, not a subsystem, and a project without one works.
- A pane gains a kind, `Terminal` or `Transcript`; multi-chat reuses the pane layout rather than
  introducing a second one — which settles Q6.
- The host sends a graph and never a drawing; layout, colour and collapse are the UI's alone, and
  layout is stable across ticks — nodes keep their slot, finished nodes persist to the task's end.
- A task carries a three-number budget, and exceeding it pauses rather than kills.
- Ubiq's `session` is renamed: to `workspace` if the `workspace` → `agent` rename lands with it,
  otherwise to `lane`. The word stops carrying two senses.
- A working area opens on an analyst, and its brief is a gate — nothing spawns until the brief is
  accepted, and skipping the analyst is one click.
- The analyst may read anything and spawn nothing. It proposes a shape; it never starts one.

Backlog rows this opens: what a bounded transport does with a crew's event volume; whether a
transcript is persisted or dies with the process; whether a task survives a restart, when each
agent's resume is the library's; how a nudge reaches a harness with no prompt-injection path; what
happens when two crew members stage conflicting edits in one worktree; and whether roles are per
project, per user, or both.

## Related docs

- [`graph-harness-ideas.md`](./graph-harness-ideas.md) — the raw idea this details
- [`../features/sessions-and-workspaces.md`](../features/sessions-and-workspaces.md) — the session and workspace this extends
- [`../features/panes-and-terminals.md`](../features/panes-and-terminals.md) — the pane rules a transcript pane inherits
- [`../features/chat.md`](../features/chat.md), [`../features/workbench.md`](../features/workbench.md) — the panel a transcript pane reuses, and the rail modes this fills
- [`../tech/transport-contract.md`](../tech/transport-contract.md) — where the two families land
- [`../tech/agent-manager.md`](../tech/agent-manager.md) — hooks, structured I/O and in-process MCP are all its side of the line
- [`project-handling-proposal.md`](./project-handling-proposal.md) — the host, the project, and the ids this assumes
- [`../backlog.md`](../backlog.md) — G7, G10, G11, Q4 and Q6, which this answers
