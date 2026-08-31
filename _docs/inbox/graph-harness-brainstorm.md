---
id: inbox-agent-graphs
title: Brainstorm — agent graphs and project orchestration
kind: proposal
status: proposal
summary: A project-level graph of agents — a PM that first refines the brief, workspaces as working areas (for a task or just to code), three run shapes, and a screen that shows who is who, what they are doing, and opens their chats.
read_when: you are thinking about the Agents rail, multi-agent orchestration, the graph of a run, or how a PM starts work across workspaces
updated: 2026-08-31
depends_on: [prod-overview, feat-sessions, feat-chat, feat-workbench, tech-architecture, tech-agent-manager, inbox-projects, inbox-panels]
---

# Brainstorm — agent graphs and project orchestration

> Superseded by [`agent-graph-final.md`](./agent-graph-final.md), which merges this document with
> [`agent-graph-proposal.md`](./agent-graph-proposal.md). Kept as source material.

An expansion of [`graph-harness-ideas.md`](./graph-harness-ideas.md). The notes are the intent;
this file is the description — what the pieces are, how they nest, what the graph has to show, and
what in Ubiq already points this way. Nothing here is a decision. It is the picture the notes
sketch, drawn large enough to argue with.

## 1. The picture in one paragraph

A **project** holds one to many **workspaces**. A workspace is a working area — the main
checkout, or a worktree, maybe on a named branch. It can be opened just to code with
agents, or spawned for a specific **task**. Talking to the project happens through a
**PM**, and the PM's first job is not to hire anyone: it is to help describe the work.
An assistant, an analyst, a refiner — it proposes requirements, a goal, what to do and
how to do it, and the user sharpens that until it is a **brief**. From a brief the user
can sit down and code, or the PM can start a task. A task runs in one of three shapes:
a single agent that just starts; a **pipeline** of agents that hand off in sequence; or
a **coordinator** that spawns specialists and keeps them moving. The PM can also open
new workspaces rather than stuffing everything into the current one.

The thing the user *watches* once more than one agent is live is a **graph**: who each
one is, which role, which harness, which workspace and task, how it relates to the
others, and what it is doing right now. Selecting a node opens that agent's **chat** —
read-only if wanted, several at once if wanted.

That is the whole of the notes. The rest of this file is what that actually names, in
Ubiq's existing words, and where it does not yet have a word. **Session** is not a
working area — see §3.

## 2. Where it stands

Ubiq today is a multiplexer of independent terminals. A window is one project. A pane is one
harness. The user is the orchestrator: they spawn, they watch, they type into whichever has
focus. The product overview already lists "agent-to-agent orchestration: a main agent that
spawns subagents into their own panes" as in-scope, last of six. The glossary already has
**subagent** — an agent spawned by another agent, and Ubiq's interest is that it gets its own
pane. That is as far as the library goes.

Four empty rail modes wait for the rest. **Agents**, **Tasks** and **KB** sit under PROJECT and
render a named empty page (`G11`). The chat panel exists as IDE furniture with no transport
family (`G10`) and two run states, Idle and Working. The session family is designed and almost
entirely unimplemented (`G19`). The MCP surface Ubiq exposes to hosted agents is a module
header (`G7`). How a subagent's pane shows its parentage is an open question (`Q6`). One
coordinator per window means two windows cannot see each other's panes (`G20`).

The earlier subagents wireframe (`_docs/design/_old/wireframe-opus/03-subagents.png`) already
drew the *layout* version of this: a parent "orchestrator" pane and two children, parentage in
the chrome (`subagent of main`), status on the tab. It did not draw a graph. The current
target layout (`ubiq-layout.png`) put Agents on the rail and left the middle of that mode
blank. This brainstorm is what fills it.

Two sibling proposals change the floor this would stand on.
[`project-handling-proposal.md`](./project-handling-proposal.md) moves the project record to
the host, so a workspace and a graph have somewhere durable to live.
[`movable-panels-proposal.md`](./movable-panels-proposal.md) turns every area into a dockable
panel, which is how "open several chats at once" stops being a special-case split.

## 3. Vocabulary — the notes mapped onto Ubiq

The notes called the working area a "session". That word is spent twice already, and neither
spending is this:

| Who says session | What they mean |
|---|---|
| Ubiq (`D11`) | A named grouping of workspaces with a home folder. The user attaches and detaches — the tmux meaning. |
| The harness library | A resumable conversation with one agent. The industry meaning, and `Q4`. |
| The notes | A working area: the main repo, or a worktree, maybe a branch. Several of these run at once in one project. |

Three meanings is how you get a sentence nobody can parse. This brainstorm leaves **session**
for the library's conversation — the meaning a hosted harness already uses, and the one `Q4`
has been waiting to keep — and calls the working area a **workspace**.

A project has a main workspace (the checkout the window opened on) and may grow more: a
worktree, a branch, an isolated folder, each with the agents on it. A workspace can exist
**just to code** — spawn agents, type, watch — which is today's product with a name on the
place. Or it can be **spawned for a task**, after a brief exists, so the graph and the board
have somewhere to put that work. Several workspaces are live together; you do not attach to
one and detach from the others. That is the other reason not to reuse Ubiq's session:
attach-one-at-a-time is the tmux shape, and the notes want parallel working areas on one
graph.

This steals **workspace** from Ubiq's current glossary, where it means one running agent.
That meaning is the odd one out: VS Code, Git, and the notes all use workspace for a place
with a tree. The running harness is an **agent** (hosted, with a pane; or reported, without).
The graph node is the agent. Filing this would rename two layers at once — Ubiq's session
becomes workspace, Ubiq's workspace becomes agent — and until then `feat-sessions` and the
session family on the bus (`G19`) stay as they are.

The rest of this file says **workspace**. That is the current pick, not a closed decision.
Every name that was proposed for this layer is in the table below, so a later pass can
pick a different one without hunting the conversation.

| Word | Why it was proposed | Why it struggles |
|---|---|---|
| **workspace** | Ordinary name for a working area. "Open a workspace", "main workspace", "a workspace on `feat/auth`". VS Code's word. Current pick. | Steals Ubiq's current glossary, where it means one running agent. Filing has to rename that too. |
| **lane** | Parallel 1–n is the point. Graph groups as labelled bands. Not git-specific. "Main lane", "open a lane for the migration". First pick in this file. | Reads as a kanban swimlane next to the Tasks rail. |
| **session** | The notes' word. Same layer as Ubiq `D11`. | Spent twice: Ubiq's attachable grouping (tmux) and the library's resumable conversation. The whole reason to rename. |
| **worktree** | The motivating case: main repo or a worktree, maybe a branch. Git people already say "main worktree". | A lie for a folder that is not git. |
| **checkout** | A working copy of a branch. "Main checkout." | Same git-only problem as worktree. |
| **room** | Ordinary place-word. "The agents in this room." | Then every chat is a room. |
| **camp** | "Make camp on this branch." Vivid for a temporary worktree. | Cute. "Spawned a camp" is a wilderness programme. |
| **desk** | A place you sit and work, with your files. Window is the workbench. | "Opened a desk" is odd. Collides with the agent as the one at the desk. |
| **bay** | Short, industrial, not taken. "Work bay." | Jargon until the product has it. |
| **base** | "Main base", "a base on this worktree." Short. | Gamey. "Spawn a base." |
| **island** | Isolation: a worktree as an island off main. | Cute. Main is then the mainland, whether you want the metaphor or not. |
| **area** / **work area** | The gloss this file already uses. Honest. | Two words, and vague without "work". |
| **workstream** | PM language for a line of work with a goal. | Does not imply a folder. "Stream" is already a byte stream in the architecture. |
| **track** | A line of work. "The auth track." | Same: a path, not a place with a tree. |
| **front** | Parallel work. "Open a front." | Military tone. |
| **wing** | A part of the project. "The auth wing." | A building, not a checkout. Main is not a wing. |
| **outpost** | A working area away from main. | Asymmetric: main is not an outpost. |
| **site** | "Work site." | Sounds like a website. |
| **floor** | Graph grouping as floors of the project. | A building again. "Open a floor" is odd. |

A name that survives has to do four jobs in speech: *open a ___ for the migration*, *the agents in this ___*, *main ___*, *which ___ is this agent in*. Workspace, lane, room, and worktree all can. Session cannot, because it already answers a different question.

| In the notes | In Ubiq today | What this brainstorm treats it as |
|---|---|---|
| Project | A window's colour and folder. Headless-host proposal wants it durable. | The container. One PM, many workspaces, one graph that can span them. |
| Session (working area, repo or worktree, branch) | **Session** (`D11`): a named piece of work with a home folder, holding workspaces. | A **workspace** — also proposed as lane, worktree, checkout, room, camp, desk, bay, base, island, area, workstream, track, front, wing, outpost, site, floor. Same layer as today's session. Optional **goal**, optional **worktree / branch**. Not only for tasks. |
| Session (a harness conversation you can resume) | The library's session. Mapping onto Ubiq's is `Q4`. | **Session**, only this. A property of an agent, not of a workspace. |
| Agent / subagent | **Workspace** (the running harness) + **pane** (what you see) + **subagent** (spawned by another agent). | An **agent** is a role that may occupy a hosted run with a pane — or, for the PM, occupy none. The graph node is the agent, not the pane. |
| PM / project orchestrator | Nothing. The user is the PM. | A project-level agent. First an analyst that refines the brief; then, if asked, an orchestrator that starts work. See §5. |
| Task | Nothing. The Tasks rail is empty. | A unit of work with a brief, a run shape, and a set of agents. Optional: a workspace can have agents and no task. High-level tasks are the user's; agent tasks are the sub-work those agents take on. |
| Harness (claude…) | **Agent type**: which binary to launch. | Unchanged. The node shows it; the library launches it. |
| Chat window | The chat panel. Fixtures, one conversation at a time, IDE-only. | A view onto one agent's conversation. Opened from a graph node. Several may be open. |
| "What it is doing" | Pane `running` / exited. Chat `Idle` / `Working`. | A richer **activity** on the node. See §7. Idle, working, thinking, in a tool, waiting for input, ended, error. |

The load-bearing distinction: **a workspace is a place; an agent is a who; a task is a why,
when there is one; a session is a conversation.** The graph is the who's, laid out by how
they relate, coloured by what they are doing, grouped by the place and the why.

A pane remains a terminal. That rule does not move. The graph is not a replacement for the
terminal and not a scrolling log of its output. It is a map of the people in the room. Opening
a node can still reveal the terminal; it can also reveal the chat. They are two views of the
same agent.

## 4. Containment

```
project
├── PM                         optional. project-scoped, not workspace-scoped
├── briefs[]                   what the PM and the user have refined. a brief may become a task
├── tasks[]                    the board. optional. a task may span workspaces
├── documentation              the KB. produced and read by agents, not a fourth kind of agent
└── workspaces[]
    ├── folder | worktree
    ├── branch?
    ├── goal?                  present when this workspace was opened for a task; absent when it is just a place to code
    └── agents[]
        ├── identity           who it is — a name, injected into the run
        ├── role               what it is for — reviewer, implementer, PM, …
        ├── harness            which agent type
        ├── occupancy          which workspace it works in; the PM may have none
        ├── parent / children  the graph edges
        ├── task?              which task (and which subtask) it is on, if any
        ├── activity           what it is doing right now
        └── hosted?            the running harness + pane, when it has one
```

**A project has 1–n workspaces.** One of them may be "main" — the checkout the window opened
on. The others are working areas that should not share a tree or a branch with main: a
worktree, a feature branch, an isolated folder. Creating a workspace does not start an
agent (`feat-sessions` already says this of today's session). It makes a place.

A workspace is spawned for a **task**, or it is opened **just to code**. Both are first-class.
The second is today's product: a place, some agents, no board row. The first is the same
place with a brief pinned to it. A task does not own the workspace; it uses it. Closing the
task does not have to destroy the place.

**A workspace has 0–n agents.** Zero is a place waiting. One is today's default. Several is
the graph.

**An agent belongs to at most one workspace at a time**, with one exception: the PM. The
notes say the PM "could have none or always the main." Those are the two occupancies worth
taking seriously:

- **None.** The PM is a conversation without a working tree. It plans, it refines, it starts
  workspaces, it does not edit. Its node sits above the workspaces on the graph.
- **Main.** The PM is an agent in the main workspace that also has permission to spawn
  other workspaces. It can look at the tree it is directing.

Either is a product choice. Mixing them — a PM that sometimes has a tree and sometimes does
not — is how the user loses track of whether "the PM" can touch files.

**A task is not nested inside a workspace.** The notes put high-level tasks in "a specific
area" and let a PM spawn working areas to carry them. So a task *uses* workspaces; it does
not live in one. A pipeline that reviews on main and implements on a worktree is one task,
two workspaces. The Tasks rail is the list of those; the graph is who is currently on them.
A workspace with agents and no task does not appear on the board. It appears on the graph
as a quiet group.

**Documentation is a surface, not a node.** Agents write it and read it. The KB rail is where
the user follows that work. A "docs" agent is an agent whose *role* is documentation, sitting
on the graph like any other.

## 5. The PM

The PM is the person the user talks to about work, not the person who does it. Today the
user *is* the PM: they pick a harness, they type a prompt, they spawn. Replacing that with
an agent does not remove the user. It gives them someone to think with first, and only
then someone who can press the buttons.

Two phases. The first is the one to start with, and it does not need the graph.

### 5.1 Analyst, assistant, refiner

Before anything is hired, the user talks to the PM about the work. The PM's first job is
to help write the **brief** — not to spawn a coordinator. Three hats, one conversation:

- **Assistant** — a partner in the chat. It asks, it restates, it notices what was not
  said.
- **Analyst** — it takes a vague intent and turns it into structure: what has to be true
  when this is done, what is in scope, what is out, what depends on what.
- **Refiner** — it proposes a version of that structure, the user corrects it, they go
  around until the description is sharp enough to act on, or to walk away from.

The brief it is aiming at has four parts, and they are the four things a later agent
should not have to invent:

| Part | Is |
|---|---|
| **Requirements** | What has to be true when this is done. Testable where it can be. |
| **Goal** | One sentence the board and the graph can show. |
| **What to do** | The breakdown. Subtasks, order, what is not this work. |
| **How to do it** | The approach: sit down and code in this workspace; one agent; a pipeline; a coordinator; a new worktree; which harnesses, which roles. |

The PM proposes. The user does not have to accept the first proposal. A run that starts
from a vague prompt is the current product; this is the step that sits in front of it.
The output is a brief, not a running graph. A brief can:

- become a **task** on the board, with a run shape and workspaces, which the PM then
  orchestrates (§5.2, §6);
- be handed to agents in a workspace **as context**, with no task row — the user takes
  the brief and codes, the agents have the same description in their prompt;
- sit there until the user is ready.

This phase is a chat with the PM. It does not need Agents mode, Tasks mode, MCP between
workers, or parentage on panes. That is why it is a good first slice of the whole picture:
ship the analyst, keep the multiplexer, add the orchestrator later.

### 5.2 Orchestrator

Once there is a brief the user wants executed as a task, the PM grows a second verb list:

- Turn the brief into a task, or refuse and keep refining.
- Pick a run shape for that task (§6), or take the one the brief already chose under
  *how*.
- Pick or create the workspace(s) the task needs — main, a new worktree, a branch.
- Pick identities, roles and harnesses for the agents the shape requires.
- Spawn those agents, inject identity, the brief, and the cross-agent MCP, and point
  them at the task.
- Watch the graph and intervene: stop, redirect, spawn another, open a workspace the
  user did not ask for.
- Talk to the user about progress without the user having to read every pane.

The analyst does not disappear when the orchestrator starts. A task that has gone sideways
goes back into refinement: the brief was wrong, or the how was wrong, and the PM is the
place that conversation happens.

### 5.3 What the PM must not do

- Become the only agent that may talk to the user. Selecting a worker still opens that
  worker's chat.
- Own the terminals. The PM starts agents; the coordinator owns the processes; the UI
  draws the graph. The PM is another hosted agent, composed through the library like the
  rest.
- Silently be a different harness from the one the user thinks they hired. The node shows
  the harness. The composer that talks to the PM shows it too.
- Skip the brief and spawn a tree of agents from a one-line prompt as if that were
  refinement. Orchestration without a brief is the user, typing, which they can already do.

The PM is itself a node on the graph. If it has no workspace, it still has identity, role
(`pm`), harness, activity, and a chat. It has no folder and no pane, or it has a pane that
is a conversation-only run — structured I/O without a working tree. That last form is new
for Ubiq: every hosted agent today has a folder and a terminal. A PM-without-a-tree is the
first agent that might not. It is also the form the analyst phase wants: talking about
the work without a default to editing it.

## 6. Three run shapes, one graph

A workspace does not require a task. Opening one and coding with agents is a use of the
place, not a degenerate run shape. The notes' three shapes apply when a brief has become
a task and someone — the PM, or the user skipping the PM — has to start it.

They are topologies of the same graph, not three products. The graph is how you tell
them apart at a glance.

### 6.1 Direct — one agent, on a task

The user, or the PM, starts a generic agent on the task. It works in one workspace. The
graph is a single node, or a PM plus that node. This is today's spawn, with a brief
pinned to it.

Use it when the work is one person's job and splitting it would just be theatre. Do not
confuse it with "just code in this workspace": that has no task and no board row. This
has both.

### 6.2 Pipeline — a chain that hands off

A sequence of agents, each with a role, each finishing before the next starts. Review then
implement then document. Research then plan then patch. The graph is a path. The edge is a
**handoff**: the outgoing agent's result is the incoming agent's brief.

The workspace may stay the same the whole way (everyone on main) or change at a step
(implement on a worktree, document back on main). The task is the path; the workspaces
are the stages' floors.

A pipeline is sequential on purpose. Parallelism inside a step is a coordinator, not a
pipeline with extra arrows.

### 6.3 Coordinator — specialists under one activity lead

An activity coordinator spawns agents with different responsibilities and keeps them
moving. The graph is a star, or a shallow tree. The edges are **parentage**: spawned by,
reporting to. Children may run in parallel, in the same workspace or in workspaces the
coordinator opened for them.

This is the shape the old subagents wireframe already drew, and the shape the glossary's
"subagent gets its own pane" is for. The coordinator is not the PM. The PM *started* this
task and picked this shape; the coordinator *runs* it. A project can have a PM and several
coordinators, each owning a task.

### 6.4 The PM spawning workspaces

The notes add one more move, which is not a fourth shape: the PM may start a new
workspace rather than a new agent. "Go do the migration in its own worktree" is a
workspace spawn; what then runs inside it is one of the three shapes, or just agents
coding against the brief. The graph at project scope therefore has a layer the
workspace-scoped graph does not: workspace grouping, with agent nodes inside.

A useful default: the graph the user stares at is **the project**, grouped by
workspace, with the PM at the top. Filtering to one workspace, or one task, is a
lens, not a different screen.

## 7. The graph, as a screen

This is the Agents rail mode. The middle of the window, in that mode, is the graph. It is
not a terminal layout and not a chat list. It is a map.

### 7.1 A node

For each agent the user can see, without opening anything:

| On the node | Why |
|---|---|
| **Who** — identity, the short name | So two Claudes are not interchangeable. Injected into the run so the agent uses the same name when it talks to the others. |
| **Role** — PM, coordinator, implementer, reviewer, docs, … | So the graph is readable as a team, not as a process list. |
| **Harness** — Claude Code, Codex, Grok, … | So mixed-harness work is visible. The library launched it; the node just says which. |
| **Workspace** — main, `feat/auth`, none | So a node that is editing a worktree is not mistaken for one on main. The PM's "none" is a first-class occupancy. |
| **Task** — the high-level task, and the subtask if it has one | So "what is this for" does not require reading the chat. |
| **Activity** — idle, queued, thinking, tools, writing, waiting for input, ended, error | So the user watches the room, not the screens. Status is a colour from the status group, never wording alone — the same convention as the rest of the chrome. The mockups in §17 name these on the node. |
| **Relation** — parent, children, the pipeline predecessor / successor | Drawn as edges; also available as a line of chrome on the node, because a graph that has to be *read* as topology will fail when it is large. |

Activity is richer than Idle/Working and richer than running/exited. The notes' list is
the right grain. **Waiting for input** is the one that earns the graph: it is the node
the user should click. **Error** and **ended** stay on the graph; an exited harness
leaves its pane, and it should leave its node too.

### 7.2 An edge

Three kinds, and mixing them on one canvas without a legend is how the graph becomes
spaghetti:

- **Parentage.** Spawned by. The coordinator shape. A tree.
- **Handoff.** Pipeline succession. A path. The work product moved.
- **Talk.** Cross-agent communication over the injected MCP. A mesh. Drawn on demand,
  or as a pulse, not as a permanent thicket — otherwise every coordinator-plus-children
  is a complete graph.

Parentage and handoff are structural; they are the run shape. Talk is traffic. The
default view shows structure. Traffic is a toggle, or an animation on an existing
edge.

### 7.3 Grouping and scale

At one or two agents the graph is a vanity. At a PM, a coordinator, three children and
a pipeline leftover, it is the only honest view. Two groupings are honest, and they
are a fork: **by workspace** (this file's first instinct — labelled bands of place)
or **by task** (what the mockups in §17 actually draw — a labelled group per run
shape). Do not put every ended agent from last week on the same canvas; ended nodes
collapse into the task that owned them.

A workspace with one idle agent and no task is a place to code, not a graph of a
run. Show it as a quiet group, not as a solitary node in a sea of whitespace. It is
the current product, visible from Agents mode.

### 7.4 Interaction

- **Click a node** — select it. The rest of the mode's chrome (a side detail, a status
  strip) follows the selection.
- **Open chat** — the agent's conversation, in a panel. Optionally read-only: the user
  is watching, not steering. Read-only is a property of the *view*, not of the agent;
  the PM can still be talking to it over MCP while the user is only reading.
- **Open more than one chat** — pin, split, or dock. This is why the movable-panels
  proposal matters here: several chats is several panels, not a tab strip that hides
  all but one. The notes are explicit that several should be visible together.
- **Open the terminal** — the pane, because some of what an agent is doing is only on
  the alternate screen. The graph does not replace the multiplexer; it sits above it.
- **Click an edge** — the handoff brief, the spawn reason, the last MCP message. Cheap
  to skip in a first version; expensive to never have, because "why is this arrow
  here" is the question a graph exists to answer.

Focus still means one thing in a window: the pane that receives keystrokes. Opening a
chat as read-only must not steal that. Opening a chat as interactive *is* taking
focus, and the terminal of that agent should not also think it has it. Exactly one
place types at a time, even when several conversations are on screen.

## 8. Chat, from the graph

The chat panel is already the conversation beside the work. This brainstorm asks it
to become the conversation beside a *node*, and to allow more than one.

That collides with two statements already in the library.

The product overview's non-goal: *Ubiq is not a chat client. It does not talk to a
model API, does not own a conversation, and does not render messages. It hosts the
harness that does.* The chat feature document already lives in the gap that non-goal
left — a panel that renders a transcript the host does not yet supply (`G10`). This
brainstorm takes the same side as the chat document: Ubiq still does not call a
model. It *does* render a conversation the host extracted from a run, because
otherwise the graph has nowhere to open.

The second collision is **D2** — a pane is a terminal, not a text buffer. The chat
is not a pane. It is a projection of structured events from a run: user turns,
assistant markdown, tool blocks, diffs. Those events do not come from parsing VT.
They come from a side channel the library already knows how to speak: ACP, JSONL,
AG-UI, the structured I/O half of a composed run. A graph-opened chat is that
projection, pointed at one agent. The terminal remains the terminal. An agent
can have both; a PM-without-a-tree may have only the chat.

Read-only follows for free once the chat is a projection: do not send. Interactive
is: the composer's send goes to that agent over the chat family the transport
does not yet have. Several chats open is several projections, not several composers
fighting for one draft — each open chat has its own composer, disabled in
read-only.

Native subagent transcripts are the awkward case. A Claude (or Grok) subagent that
the harness spawned *inside itself* may never appear as a hosted agent. Its conversation
lives inside the parent's run. The graph can still show it as a child node if the
parent reports it over MCP or structured I/O, and the chat opened from that node
is a slice of the parent's transcript, or a nested stream, not a second process.
That is a different occupancy again: **reported, not hosted.** The user can watch;
they may not be able to type; there may be no pane. The node should say so.

## 9. Identity, and talking across harnesses

The notes want two kinds of agent on the same graph:

- **Native subagents** — Claude spawning Claude, Grok spawning Grok, the harness's
  own subtree. Ubiq's job is to notice them, give them nodes, give them panes when
  it can.
- **Cross-harness agents** — a Codex child of a Claude coordinator, a Gemini docs
  agent on a Grok task. They cannot use the parent's native spawn. They communicate
  through **MCP injected by the host**, and each has a **precise identity in the
  initial prompt**.

The second kind is the one Ubiq is in a position to do well, because composing a
run is already the library's job: skills, MCP servers, initial instructions, an
account, a throwaway config. Identity is an instruction. The cross-agent bus is an
in-process MCP service the embedder registers (`inproc-mcp`), which is also the
undecided surface in `mcp_server.rs` (`G7`).

A sketch of that MCP, as a verb list an agent actually needs:

- `who_am_i` — identity, role, workspace, task, parent.
- `list_peers` — the other nodes on this task, with role and activity.
- `message` — send to a named peer.
- `spawn` — ask the host to start a peer (harness, role, workspace, brief). The
  host starts a hosted agent; the parent does not shell out.
- `handoff` — finish this step, pass the brief to the named successor.
- `report` — activity, so the graph does not have to guess.
- `ask_user` — mark this node waiting-for-input and put the question on the chat.

The PM's tools are a superset: refine brief, create workspace, create task, pick
a run shape, assign a workspace to a task. Workers do not get those.

Identity injection is not a flavour text. If two agents are both "Claude" and
neither knows its name, MCP `message` has nothing to address. The short name on
the node is the short name in the prompt is the short name in `list_peers`. One
string.

A native subagent that the harness spawned without asking Ubiq will not have this
MCP unless the host also wraps *that* process. First version can treat native
subagents as reported nodes (parent says they exist) and wrapped agents as hosted
nodes (Ubiq spawned them, injected identity and MCP, owns the hosted run). The
graph shows both; the chrome says which.

## 10. Tasks, and the documentation

The graph is the people. The Tasks rail is the work. The KB rail is the writing.

**High-level tasks** are what the user or the PM started from a brief: a goal, a
run shape, the workspaces involved, the agents currently on it, a state (planned,
running, waiting, done, failed). The board is that list. Opening a task lenses
the graph. A workspace that is just for coding does not appear here.

**Agent tasks** are the sub-work: what a node is doing *within* the high-level
task. They appear on the node, in the task's own breakdown, and in the chat as
the thing the agent accepted. They should not be a second board the user has to
reconcile with the first. One task, with a tree of subtasks the agents own.

**Documentation** is both a product of work and an input to it. A docs-role agent
writes; everyone reads; the KB rail is the place the user follows that without
standing in the graph. This brainstorm does not design the KB. It only claims
that documentation is not a fourth run shape and not a special workspace. It is a
surface fed by the same agents the graph already shows. A task can have a docs
subtask; the node that takes it is a node.

## 11. What the graph cannot see, unless something else exists

The graph as described is not a skin on the PTY. Terminal bytes stay opaque. A
node that shows "thinking", "in a tool", "waiting for input", a role, a subtask,
and a chat transcript is reading a **side channel**. Three candidates, and the
product probably wants two of them:

1. **Host-owned facts.** Identity, role, harness, workspace, parentage, the task
   the PM assigned. True at spawn, no cooperation from the agent. Enough to draw
   a labelled tree. Not enough to draw activity, chat, or native subagents.
2. **Structured I/O.** The library's ACP / JSONL / AG-UI path. Turns, tool
   calls, diffs, perhaps thinking. Enough to drive the chat projection and a
   decent activity enum. Requires the run to be composed with that I/O mode
   rather than pure passthrough. A pipeline of mixed harnesses only works as
   far as each harness can speak a structured mode.
3. **Injected MCP reports.** `report`, `ask_user`, native-subagent notices.
   Enough to fill the gaps structured I/O does not, and the only way a
   passthrough TUI agent participates in the graph at all.

A first Agents mode that only had (1) would still be worth it: a map of who was
started, in which workspace, on which task, with parentage. Activity would be
running/exited. Chat would not open. That is a multiplexer with a roster. The
notes want more than a roster, so (2) or (3) is in the same product, not a
sequel.

Passthrough terminals do not go away. The user still opens the pane. The graph
is honest about what it does not know — a hosted agent on passthrough shows
harness, role, running, and no tool-level activity, rather than inventing
"thinking" from silence.

## 12. Scenarios

### 12.1 Just code, no task

The user is in the project, one workspace (main). They spawn Claude Code and
start typing. There is no brief, no board row, no PM. The graph, if they open
Agents mode, is one quiet node. This is today's product with a name on the place.

### 12.2 Refine, then code

The user tells the PM: "the host crate is a mess." The PM asks what "mess"
means, proposes a goal ("split the host so the UI crate cannot import it"), a
list of requirements, a breakdown, and a how: stay on main, two agents, no
worktree. The user cuts the breakdown, adds a requirement the PM missed, and
accepts the brief. They do not start a task. They open the main workspace, spawn
two agents, and the brief is in both prompts. The Tasks rail is empty. The graph
is the PM plus two nodes on main.

### 12.3 Refine, then a task on one agent

Same brief as 12.2, but the user asks the PM to run it. The PM turns the brief
into a task, shape direct, Claude Code on main. The graph shows one worker.
The Tasks rail has one row. They work in the terminal as today, with the brief
on the node.

### 12.4 Coordinator, same workspace

The user tells the PM: "refactor the host crate, keep the UI compiling." After
refinement, the PM creates a task, picks coordinator, stays on main. It spawns
a coordinator and then a Codex implementer and a Claude reviewer as children,
same workspace, identity injected, MCP shared, brief in every prompt. The graph
is a star. The user reads the reviewer's chat read-only, types into the
implementer's terminal. The coordinator waits on both and merges.
Waiting-for-input lights up on the coordinator when it wants the user to
accept the merge.

### 12.5 Pipeline across workspaces

The PM is asked for a breaking change that should not land on main. After
refinement, it creates a workspace from a worktree on `feat/auth`, then a
pipeline: research (Claude, main, read-only tree) → implement (Codex, the
worktree) → docs (Grok, the worktree) → review (Claude, main, reading the
worktree). The graph is a path that changes colour as the token moves. Two
workspace bands. One task. The user opens research and implement chats side by
side at the handoff, because that is where pipelines go wrong.

### 12.6 Native child plus foreign child

A Claude coordinator natively spawns a Claude subagent for a narrow edit, and
asks the host (MCP `spawn`) for a Codex child for the tests. The graph shows
three nodes: coordinator (hosted), native child (reported, no pane, chat is a
slice), Codex (hosted, own pane). The chrome on the native child says it is
inside Claude, not a hosted agent. The user can open its chat read-only and cannot
focus a terminal it does not have.

### 12.7 PM with no tree, several live tasks

The PM occupancy is none. Two tasks are running in two workspaces. The project
graph is the PM plus two groups. The user talks to the PM about a third brief
without walking into either workspace. Selecting a worker still opens that
worker's chat. The PM never has a folder; it cannot "helpfully" edit.

## 13. Alternatives worth keeping in the room

**Graph versus roster versus layout.** The old wireframe used layout (big parent,
small children) and chrome parentage. A roster (a list with indent and status)
gets 80% of the information at none of the layout cost. The notes ask for a
graph specifically. A first version that is a grouped roster, with a graph as
the mode's other lens, is a cheaper way to find out whether anyone looks at
the arrows.

**PM as hosted agent versus PM as UI.** A UI-only PM is a wizard: pick a shape,
pick a harness, spawn. No identity, no MCP, no chat with the PM. It is smaller
and it is not what the notes describe. The notes want to *talk* to the PM about
the work. That is an agent. The analyst phase makes the UI-wizard even less of
a substitute: a form cannot refine a brief.

**PM as coordinator, always.** Collapse §5.2 and §6.3. Simpler graph, worse
scaling: a project with three live tasks then has one agent wearing three hats,
or three PMs. Keeping them distinct is the whole reason to have a project-level
node. The analyst hat stays on the PM either way.

**Skip the brief.** Spawn from a one-line prompt, the way the user does today.
Cheaper. Then every orchestrated run inherits a vague goal, and the PM is just
a spawn menu with a chat skin. The brief is the point of having a PM at all.

**Chat is the terminal.** Do not. D2. Opening "chat" by focusing the pane is
today's product, and it does not give read-only, does not give several
conversations visible, and does not give a transcript the KB can cite.

**One chat panel, node selection swaps it.** Smaller than several chats at once.
The notes want several. Movable panels make several honest; a single panel with
a pin-to-side is a stepping stone.

**Worktrees as folders.** Today's session model already has a home folder
and a next-step of "spawn into a folder of its own." A worktree may just be a
folder the host created with `git worktree add`. It does not need a new object
if the workspace record carries the branch and the fact that the host made the
directory. It does need the host; the UI must not run `git`.

## 14. What this leans on, and what it would cost

Leans on, already in the library or the inbox:

- Workspaces as the same layer as today's sessions — a grouping with a home
  folder — under the ordinary word for a working area (`D11`, `feat-sessions`,
  mostly `G19`). Today's "workspace" (one running agent) becomes just **agent**.
- Subagents getting panes (overview §6, glossary, `Q6`).
- Composed runs: instructions, MCP, identity as a prompt (`tech/agent-manager.md`).
- In-process MCP for embedder tools (`G7`, `inproc-mcp`).
- Structured I/O in the library (ACP, JSONL, AG-UI) if chat and activity are real.
- Chat as a projection (`feat-chat`, `G10`).
- Empty Agents / Tasks / KB modes (`G11`).
- A durable project on the host (project-handling proposal).
- Several panels visible (movable-panels proposal).
- One coordinator that can see every pane of the project — today's per-window
  coordinator (`G20`) is the wrong grain once a PM spans workspaces inside one
  project, but it is the right grain if a window *is* a project and workspaces
  live inside it. That one is close: keep one coordinator per project window,
  let it own every workspace of that project. Do not split coordinators per
  workspace.

Would cost, if taken as written:

- The overview's "not a chat client" non-goal has to be rewritten as "does not
  call a model, does render a hosted conversation." The chat document already
  needs that rewrite; this makes it unavoidable.
- Activity and native-subagent nodes require a side channel. Pure passthrough
  as the only I/O mode cannot feed the graph the notes want. Passthrough stays
  for the pane; it loses the monopoly.
- A PM-without-a-tree is an agent without a hosted run as currently defined.
  Occupancy grows a conversation that has no folder. That is also what the
  analyst phase wants.
- Cross-harness spawn is a host verb (`spawn` on the MCP), not a harness verb.
  The coordinator learns to start an agent *because another agent asked*,
  not because the UI did. That is a new direction on the bus: a message that
  originates in a hosted agent, crosses the MCP into the host, and comes back
  out as a spawn acknowledgement. The UI did not click. The contract has to
  allow that without letting the agent dictate layout or focus (`Q6` is
  exactly this).
- The PM's analyst phase is a real conversation that has to land somewhere.
  The chat family (`G10`) is no longer optional even before Agents mode exists.
- The Agents mode needs the chat as shared furniture, which the workbench
  already says it is written for and does not do. Tasks and KB probably do
  too.

## 15. Open questions this file does not settle

These are product forks, not implementation leftovers. Filing this as a feature
would turn each into a backlog row or a decision.

1. **PM occupancy.** None, or always main. Not both.
2. **Is the PM ever the coordinator**, or does even a one-task project spawn a
   coordinator node distinct from the PM?
3. **Graph, roster, or layout-with-chrome** as the first Agents surface.
4. **Activity source.** Structured I/O, MCP reports, or host-owned running/exited
   only, in which combination, for a first version.
5. **Native subagents.** Reported nodes in v1, or wait until Ubiq can wrap the
   child process and give it a pane.
6. **Who may open a workspace** — only the PM, or any coordinator, or the user
   from the UI as today.
7. **Read-only chat.** A view flag, or a distinct occupancy (reported vs hosted).
8. **Several chats.** Wait for movable panels, or a cheap split inside Agents
   mode.
9. **Task identity.** Does a task survive the agents that ran it, the way a
   workspace survives its agents? (Almost certainly yes — otherwise the board
   is a process list.)
10. **The library session.** Does opening a chat with a hosted agent attach to
    that agent's resumable conversation (`Q4`), or is the projection live-only?
11. **Does a brief live without a task?** This file says yes — refine, then code
    without a board row. If a brief is always a task in draft, the board has
    to show drafts, and "just code" loses a pinned description.
12. **The place-word.** Workspace is the current pick. The full list is in §3.
    Lane is the strongest runner-up if stealing Ubiq's "workspace" is too
    expensive. Pick before filing. The mockups' status bar still says
    `sessions` (§17).
13. **Group the graph by task, or by workspace.** §7.3 said workspace-first.
    The mockups draw task-first (§17.4). Pick before the canvas is built.
14. **Tasks as a drawer on Agents mode, or as its own rail, or both.** The
    second mockup docks the board under the graph. §10 put it on the Tasks
    rail. Same records; the question is which screen owns the first view.

## 16. What filing this would mean

Not now. When it leaves inbox, it is not one document:

- A **feature** for the PM's analyst phase — a chat that produces a brief
  (requirements, goal, what, how). This is the first slice: it does not need
  the graph. Deleting that conversation from Ubiq would delete the document.
- A **feature** for the Agents graph (the screen, the node, the edges, the
  selection → chat/terminal). Later than the analyst.
- Behaviour on **Tasks** as its own feature, or a section of the graph's if
  the board is not a screen without it. The placement rule decides.
- A rename of today's **session** layer to whichever place-word §3 settles
  (**workspace** is the current pick; **lane** the strongest runner-up), and
  of today's **workspace** (one running agent) to **agent** if that word is
  taken. Plus the fields the place does not yet carry: optional goal,
  worktree/branch, occupancy without a folder, spawn-requested-by-an-agent.
  **Session** is then only the library's resumable conversation, which is
  what `Q4` has been asking.
- An extension of **chat**: pointed at a node, read-only, several open, a
  transport family — and the PM's brief conversation before any of that.
- A decision on **G7** (the MCP verb list in §9).
- A decision on **Q6** (parentage, and who decides where a spawned pane opens).
- A rewrite of the overview non-goal about chat, if the product is going to
  mean it.

Until then this file is the detailed description of the notes, and the notes
stay the raw intent.

## 17. The agent-view mockups

Two screens in the inbox draw this proposal rather than describe it:

- [`agent-view-proposal.png`](./agent-view-proposal.png) — Agents mode, the graph, the
  PM's chat open on the right.
- [`agent-view-proposal-with-tasks.png`](./agent-view-proposal-with-tasks.png) — the
  same canvas with a **Tasks** drawer pulled up from the bottom.

They are not a third model. They are this file's graph, PM, three run shapes, and
chat-from-a-node, drawn in the workbench shell. What follows is where they sit on
the sections above, and the forks they pick without saying so.

A sibling write-up, [`agent-graph-proposal.md`](./agent-graph-proposal.md), covers
the same ground with a different vocabulary (it still says *session* for the
working area). The mockups belong to both.

### 17.1 What the canvas is showing

The window is IDE chrome with the **Agents** rail selected. The title reads
`Agents · orchestration graph`. The middle is a pannable dotted canvas, not a
terminal layout and not a roster. That is §7.

One node sits above the rest: **Orchestrator / PROJECT MANAGER**, activity
**Needs you**, path `main`. Three labelled groups hang off it:

| Group label on the canvas | This file | What is inside |
|---|---|---|
| `DIRECT · Guard the 0×0 resize callback` | §6.1 Direct | One agent, **Fixer** / implementer, in `fix/terminal-refit` |
| `CHAIN · Migrate the session store` | §6.2 Pipeline | **Spec** (ended) → **Builder** (writing) → **Reviewer** (queued, second mockup) in `feat/session-store` |
| `COORDINATED · Cut cold start under 800 ms` | §6.3 Coordinator | **Perf lead** (activity coordinator, thinking) and four children — profiler, rust dev, bench, scribe — in `spike/cold-start` |

Three tasks, three shapes, one PM, one canvas. That is the sentence in §6 that
the shapes are topologies of the same graph, not three products, and the sentence
in §5.2 that a project can have a PM and several live tasks at once. The mockup
is that sentence, drawn.

The status bar counts `4 sessions · 10 agents · 5 running · 2 need you · 1 error`.
Under this file's words those four "sessions" are four **workspaces**: `main`,
`fix/terminal-refit`, `feat/session-store`, `spike/cold-start`. Ten agents is the
PM plus the nine workers the drawer names (`3 tasks · 9 sub-agents`).

### 17.2 A node, against §7.1

Each card is the node table, with two fields moved and one added.

| §7.1 | On the card | Notes |
|---|---|---|
| Who | **Fixer**, **Spec**, **Perf lead**, … | Short name, unique on the canvas. |
| Role | `IMPLEMENTER`, `ANALYST`, `ACTIVITY COORDINATOR`, `INVESTIGATOR`, `VERIFIER`, `DOCUMENTATION`, `REVIEWER`, `PROJECT MANAGER` | Richer than the examples in §7.1. A catalog of roles, not free text. |
| Harness | Not on the card | Lives in the inspector (`Claude Code · Opus 4.6`). Mixed-harness work is not glanceable on the graph. |
| Workspace | A path chip: `main`, `feat/session-store`, `spike/cold-start` | Place is a git path, not a band. Token count sits next to it (`18.9K`). |
| Task | The group label around the card, not a field on it | Direct / chain / coordinated plus the goal sentence. |
| Activity | A coloured pill: Tools, Writing, Thinking, Ended, Error, Needs you, Queued | Left edge of the card takes the same colour — the shell's identifying edge, from the status group. |
| Relation | Edges, plus nesting inside the coordinated group | Parentage is a dashed tree; handoff is a numbered arrow (`1` from Spec to Builder). |
| Chat | A `chat` affordance on every card | Opens the inspector. |

**Queued** is an activity this file did not name and the pipeline needs: the
successor that exists as a node but has not started. **Needs you** is waiting-for-input
in four fewer letters. **Tools** / **Writing** / **Thinking** are the grain of
"doing something" that §7.1 left as one bucket.

The coloured left edge is the one convention the rest of Ubiq already has
(`ACCENT_EDGE`, status tokens). A graph of rounded-enough cards still obeys it.

### 17.3 Edges, against §7.2

The mockup draws exactly two of the three kinds, which is the default this file
asked for:

- **Parentage.** Orchestrator down to each task group; Perf lead down to its
  four workers (dashed).
- **Handoff.** Spec → Builder, numbered. Reviewer waits on Builder (queued),
  which is a handoff that has not fired yet.
- **Talk.** Not drawn. Traffic stays off the canvas.

No MCP mesh. The canvas stays a structure diagram.

### 17.4 Grouping — the fork the mockups pick

§7.3 said group by workspace first, then by task. The mockups group **by task**.
The workspace is a chip on the node, not a band. That is the better default
once several tasks are in flight: you came here to see the work, not the
checkouts. A pipeline that already shares one worktree (`feat/session-store`)
would be two bands with one agent each if grouped by place, and one labelled
chain if grouped by task. The mockup picks the chain.

The cost: two tasks on the same worktree would sit in two groups and the shared
place would only be readable by comparing path chips. The inverse cost of
workspace-first is what the mockup avoids — a chain split across the canvas.

"Just code" (§6, §12.1) has no task group to sit in. On this canvas it would be
an unlabelled cluster, or a fourth group without a DIRECT/CHAIN/COORDINATED
prefix. The mockups do not show that state. They show a project that is already
being orchestrated.

### 17.5 The PM, against §5

The Orchestrator card is the PM. The mockup picks the occupancy fork in §4
without discussing it: the PM sits on **main** (`main 18.9K`), not on none. It
can see the tree. It also spawned other workspaces — the chat says so in as
many words:

> I opened spike/cold-start as a worktree and put a coordinator on it with four
> workers: profiler, Rust dev, bench and a scribe.

That is §5.2 and §6.4, in a turn. The user-side turn above it is the analyst
phase still running *while three tasks are already in flight*:

> Cold start feels slow since the plugin registry landed. Look into it.

Refine and orchestrate are one conversation, not a wizard that ends when the
graph appears. The composer placeholder is `Describe a task for the
orchestrator…` — the PM is still taking work. §5.1 said the analyst does not
need the graph; the mockup says the analyst *keeps talking* once the graph
exists. Both can be true. The first slice is still the chat. The mockup is the
slice after that, with the chat still there.

The inspector header shows harness, model, thinking budget, mode, context
percentage — the chat composer's pickers from `feat-chat`, pointed at this
node. The PM is a hosted agent like the others, which is what §5.3 required.

### 17.6 Chat, against §8

Selecting the Orchestrator opens its conversation on the right. That is
"click a node → open chat". The mockup picks the **one panel, selection
swaps it** alternative in §13, not several chats visible at once. The status
bar's `1 chat open` is that choice, stated. Several chats remain a later
move, and they still want the movable-panels proposal.

There is no read-only in the mockup. Every card has `chat`, the composer is
live, Send is enabled. Watching a worker without steering it is not drawn.
Easy to add (composer disabled, or a view flag) and easy to forget, which is
why §8 bothered.

The inspector also has a **tasks 3·9** tab. That is the board, scoped to this
node: the PM's three tasks, nine agents. Opening the PM is how you see the
whole run as a list without leaving the graph. Opening Fixer would presumably
scope that tab to Fixer's own work. The mockup does not show a worker's
inspector, so that last bit is a guess.

### 17.7 The Tasks drawer, against §10

The second mockup pulls a strip up from `TASKS · Orchestrator · 3 tasks · 9
sub-agents`. Three cards, one per task: shape, title, workspace path, progress
(`0/1`, `1/3`), and the agents with their activity.

This is the Tasks rail's board **docked under the graph**, not a different
rail mode. The Tasks rail still sits in the activity rail, empty in these
frames. Two honest homes for the same list:

- **A drawer on Agents mode**, as drawn. Graph and board are two lenses on
  one run, and you never leave the canvas to see progress as numbers.
- **The Tasks rail**, as §10. A dedicated board, the graph a click away.

They are not exclusive. The drawer can be the board's projection while Agents
is selected, and the Tasks rail the same records full-page. What would be a
mistake is two *different* boards.

The drawer also answers "does a task survive its agents": Spec is **Ended**
and still on the chain card at `1/3 done`. The task outlives the step.

### 17.8 What the mockups need from underneath

Nothing on this canvas is readable from PTY bytes. Token counts, Tools /
Writing / Thinking, Queued, Needs you, the brief in the chat, "I opened a
worktree" — all of that is §11's side channel. The mockup is the argument
that (1) host-owned facts plus (2) structured I/O or (3) MCP reports are in
the same product as the screen, not a sequel. A first graph that only had
running/exited would not look like this, and would not be worth the canvas.

Filter chips along the top (`running`, `waiting`, `ended`, `error`) are a
cheap extra: activity as a lens, not a new object.

### 17.9 What they leave unset, or pick silently

| Fork in this file | What the mockup does |
|---|---|
| PM occupancy (§4, Q1) | Always **main**, not none. |
| Group by workspace vs by task (§7.3) | By **task**. Workspace is a path chip. |
| One chat vs several (§8, §13) | **One** inspector. Selection swaps it. |
| Read-only chat (§8) | Not drawn. |
| Tasks rail vs drawer (§10) | Drawer on Agents mode. Tasks rail unused in the frame. |
| Chain vs pipeline (§6.2) | The canvas says **CHAIN**. Same shape. |
| Just-code workspace with no task (§6, §12.1) | Not on the canvas. |
| Native vs hosted chrome (§9) | Not distinguished. Every node looks hosted. |
| Harness on the node (§7.1) | Inspector only. |
| Analyst as a first slice without the graph (§5.1) | The chat is drawn *with* the graph. The composer still works as that slice if the canvas is empty but the Orchestrator node. |

The mockups are the target picture for Agents mode once more than one task is
live. They do not replace the analyst-first slice, and they do not settle the
place-word: the status bar still says `sessions`. Under this file that count
is workspaces.
