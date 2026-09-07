---
id: inbox-agents-definitions
title: Proposal — the dimensions of an agent, and a standard/advanced interface over them
kind: proposal
status: proposal
summary: Seven independent dimensions — definition vs instance, process ownership, parentage, voice, delegation, resumability and persistence, and spawn origin — that model everything from a one-click harness pick to a defined, resumable, persistent crew member, plus the progressive-disclosure interface that keeps the newcomer path a single button while the power-user path stays fully explicit.
read_when: you are deciding what a profile, a role or a defined agent should carry, how an agent instance relates to the definition it was spawned from, or how to keep the agent-creation UI simple for a first-time user while staying expressive for a power user
updated: 2026-09-07
depends_on: [wip-agent-setup, wip-agent-vocabulary, inbox-agent-graph-final, feat-sessions, tech-agent-manager]
---

# Proposal — the dimensions of an agent, and a standard/advanced interface over them

"Agent" already names three different things in this tree: a reusable definition to spawn from, a
live running instance, and a harness's own internal subagent that Ubiq only hears about. Each is
real and each is needed. The risk is not that the word is overloaded — `agent-graph-final.md`
already resolves the running-instance sense — it is that the *properties* of an agent get invented
one at a time, on whichever screen needs them next, and drift into an interface that a newcomer
cannot open without seeing a form with twelve fields. This proposes the opposite order: name the
independent dimensions first, so that "simple for a newcomer" and "complete for a power user" are
the same model shown through two doors, not two designs that happen to share a database.

## 1. The seven dimensions

Each is independent of the others — none is a special case of another, and none needs the others
to be meaningful on its own.

### Definition vs. instance

A **profile** is a reusable, named bundle: the harness, the account, the model, the mode, and —
where this proposal extends what is landed — the prompt, skills and MCP servers a run is composed
from. Nothing is running. An **agent** (today's `Workspace`, per `inbox-agent-graph-final` §1's
rename) is one spawn of a profile, or of no profile at all: a live pseudo-terminal, a live process,
a working directory. `crates/ubiq-host/src/agent.rs`'s `ProfileInfo { id, agent_type, account,
model, mode }` is today's profile, already landed (`wip-agent-setup` P4); it carries no prompt, no
skills, no MCP — that gap is `G122`, and closing it is what turns a profile into the fuller **role**
`inbox-agent-graph-final` §6 proposes (a profile plus a tool policy: `may_spawn`, `may_write`,
`may_message`, `mcps`, `skills`). This document treats "profile" and "role" as the same dimension at
two levels of completeness, not two concepts.

### Process ownership and parentage

Earlier drafts of this reasoning reached for a single three-way "nature" enum — independent, child,
internal — and it does not hold up: a "child" can itself talk to the user or not, which means voice
is not a property of the bucket. What actually varies is two independent facts, both already present
in the design in different words:

- **`has_own_process`** — hosted (its own harness, its own pseudo-terminal, Ubiq owns and can kill
  it) or internal (shares its parent's process — a harness's own subagent, Claude Code's Task tool
  and its kin). This is exactly `inbox-agent-graph-final` §3's Hosted/Reported split. An internal
  agent has no pane of its own, no MCP connection of its own, and ends only when its parent says so
  or goes silent.
- **`parent: Option<AgentId>`** — none, or the agent that spawned it. Orthogonal to the above: a
  hosted agent can be top-level (what a newcomer spawns) or a hosted child (what a crew coordinator
  spawns through `agents.spawn`, §8 of the same proposal). An internal agent always has a parent —
  it cannot exist without one.

Four combinations exist, not three, and all four are real: a top-level hosted agent (today's whole
product), a hosted child (a crew member with its own harness and pane), an internal agent (a
subagent nested in its parent's process), and — the one nothing today produces — a top-level
internal agent, which is not meaningful and should simply never be constructible.

### Voice — who may address the user

`may_message_user`, a policy flag independent of parentage. A child does not inherit muteness from
having a parent; whether it may escalate straight to the human or must go through its parent (or
the PM, per `inbox-agent-graph-final` §8's `ask_user`) is a choice the role makes. Folding this into
the process/parentage dimension is what made the three-way enum leak in the first place.

### Delegation — may this agent spawn its own children

`may_spawn`, a second policy checkbox, independent of Voice and independent of whether *this* agent
is itself a child. It answers one question only: may this agent create new **hosted** children of
its own through `agents.spawn`? It says nothing about internal subagents — a harness's own Task-tool
mechanism spawns inside the harness's own process, outside Ubiq's policy surface entirely, and no
flag Ubiq holds can permit or refuse it. A role that forbids `may_spawn` still cannot stop its
harness from spawning internal subagents; it can only stop Ubiq from letting it ask for hosted ones.
That is the distinction worth keeping sharp when this checkbox is read: it governs delegation *Ubiq
can see and kill*, not delegation the harness does on its own.

Two fields travel with it once it is on: `max_children`, a budget rather than a boolean (per-task
budgets already exist in `inbox-agent-graph-final` §13), and `may_create_workspace`, a related but
separate checkbox — spawning a child in the same workspace is cheap and common, opening a new
worktree for it is a bigger grant and defaults off even when spawning itself is on.

### Resumability — two operations, not one

**Resume**, already shipped ([`agent-vocabulary.md`](../wip/agent-vocabulary.md) §4): restart the harness under the same
`agent_id`, continuing the same conversation. Nothing about the goal changes; only the process
does. **Retask** — hand a finished or idle agent a *new* goal while keeping its accumulated
transcript and context — is a different operation and nothing in the tree does it today. The
difference matters because they answer different questions: Resume answers "the harness died or was
unloaded, keep going"; Retask answers "this agent's work is done, but its knowledge of this
codebase is valuable, reuse it for something else." Naming both under "resumable" is exactly the
kind of collapse this document exists to avoid.

### Persistence — two different things kept alive

Two axes, easy to conflate because both use the word "persistent":

- **Profile-level identity persistence.** A defined agent — one spawned from a named profile — gets
  a persistent `$HOME` keyed to that profile, so its caches and its login survive to the next run
  ([`agent-setup.md`](../wip/agent-setup.md) P4, landed). An ad-hoc, profile-less run gets an ephemeral `$HOME`, discarded
  with the run. This is a property of the **profile**.
- **Instance-level record persistence.** Whether a *specific* agent's conversation record survives
  an application restart. The record is already written — the harness's transcript is copied out of
  the run directory before it is deleted ([`sessions-and-workspaces.md`](../features/sessions-and-workspaces.md)) — but nothing reads it back on
  startup: `G97` and `G120` name this exactly, and today every conversation starts blank after a
  restart regardless of what its profile says. This document's addition is framing it as a **flag on
  the instance, settable while it is open** — "keep this one running/resumable across restarts" —
  rather than a fixed property decided at spawn time, since a newcomer typing a throwaway question
  and a power user mid-refactor both start from the same spawn path and diverge only in whether the
  work is worth carrying forward.

An agent can be ephemeral-identity (no profile) and instance-persistent (flagged to survive a
restart), or defined (persistent identity) and instance-ephemeral (a one-off run of a saved role).
The two axes are genuinely independent and both are needed.

### Spawn origin

Who issued the spawn: the user, through the UI (`SpawnWorkspace` / a task's `CreateTask`), or an
agent, through the MCP surface (`agents.spawn`, `inbox-agent-graph-final` §8). Worth tracking as its
own field rather than inferring from parentage, because a user can ask a PM to spawn a specific
child by name (parent set, user-issued) and, later, a role itself could be agent-authored (a
coordinator saving a role it composed for a crew it plans to reuse) — a fact about the *profile*,
not just the instance. The second case is speculative and not proposed for building now; the field
is proposed now because retrofitting provenance onto instances after the fact is the harder order.

## 2. The dimensions together

| Case | Definition | Process / parent | Voice | Delegation | Resumable | Persistent | Origin |
|---|---|---|---|---|---|---|---|
| Newcomer's first agent | none | hosted, no parent | yes | no | Resume only | ephemeral identity, ephemeral record | user |
| "Just code," today's product | none or a saved profile | hosted, no parent | yes | no | Resume only | either | user |
| A crew coordinator | a role | hosted, parent optional | policy-decided | yes, with a budget | Resume only | ephemeral identity unless its role is a saved definition | user or agent |
| A crew member | a role | hosted, parent set | policy-decided | usually no | Resume only | ephemeral identity unless its role is a saved definition | agent (`agents.spawn`) |
| A harness's own subagent | none — inherits the parent's | internal, parent set | usually no | n/a — not Ubiq's to grant | tied to parent | n/a — no separate process | agent (internal to harness) |
| A standing project assistant | a saved profile/role | hosted, no parent | yes | policy-decided | Resume + flagged instance-persistent | persistent identity + persistent record | user |

No row needed an eighth dimension. That is the check this model has to pass: every scenario raised
across this design's discussion — a template to spawn several independent agents, an always-on
project assistant, a crew coordinator's children, Claude Code's own subagents — is a point in this
seven-dimensional space, not a new case bolted on.

## 3. Standard and advanced: one model, two doors

The dimensions are the whole model; they must not be the whole interface. Every dimension has a
default that reproduces exactly what a newcomer already gets — pick a harness, type a prompt, go —
so the standard surface never renders a field for any of them:

| Dimension | Standard default | Advanced control |
|---|---|---|
| Definition | none (a bare harness pick, today's `HarnessChoice`) | The profile/role editor: save the current pick, then add prompt, skills, MCP, policy |
| Process / parent | hosted, no parent | Set when a role is spawned as a crew member — never a field the user fills by hand |
| Voice | yes | `may_message_user` in a role's policy table |
| Delegation | no | `may_spawn` (with `max_children`) and `may_create_workspace` in a role's policy table |
| Resumable | Resume, offered only once an agent has exited | Retask, once built, as an explicit action distinct from Resume |
| Persistent | ephemeral both ways | A per-instance toggle in the agent's own menu, and "save as profile" for identity persistence |
| Origin | user | Read-only, shown in an agent's inspector, never a control |

**The one control that bridges the two doors is "save as profile."** A newcomer never sees the
model; they see a harness picker and a prompt box, which is the entire standard surface. The moment
they save what they typed as a named, reusable thing, they have opted into the advanced surface for
that one definition — and only that one. This mirrors how `ProfileInfo` and the settings overlay
already work ([`agent-setup.md`](../wip/agent-setup.md) P4): a bare harness row starts with no profile at all, and a
saved profile is additive, never a mode switch for the whole application. The advanced surface is
not a settings toggle a user turns on globally; it is where a definition someone chose to save
lives, discoverable the moment there is one, invisible until then.

The same principle answers group behaviour. A crew, a pipeline and a PM (`inbox-agent-graph-final`
§§5–6) are all just roles with `may_spawn` and a policy, composed by another agent — nothing about
them needs a separate "advanced mode" beyond the role editor already described. The graph screen
that visualises them is additive, the same way the profile editor is: it has something to draw only
once a role or a task exists to produce one.

## 4. What this changes about what exists

Nothing here proposes a new message family beyond what `inbox-agent-graph-final` §10 already lays
out (the profile fields extending `ProfileInfo`, and `origin` / `parent_id` already present on the
proposed `AgentInfo`). What this document adds on top:

- Decomposing `inbox-agent-graph-final`'s Hosted/Reported into `has_own_process` × `parent`, so a
  hosted child is not modelled as a variant of Reported or bolted on as a special case of Hosted.
- Splitting Resume from a not-yet-designed Retask, so the latter is not silently implied by the
  former when it eventually gets built.
- Splitting profile-identity persistence (landed, P4) from instance-record persistence (`G97`,
  unbuilt), and proposing the latter as a per-instance flag rather than a property fixed at spawn.
- Naming spawn origin as an explicit, tracked field rather than an inference from `parent_id`.
- Promoting `may_spawn` out of `inbox-agent-graph-final` §6's policy table into a named dimension
  (Delegation) alongside Voice, and drawing the line it and Voice share: both are checkboxes on the
  *role*, both are independent of parentage, and Delegation's line is specifically that it can only
  ever govern hosted children — an internal subagent is never something Ubiq's policy can grant or
  refuse.
- The standard/advanced framing in §3, which none of the existing documents state as a rule — they
  describe the pieces; this says which door a newcomer never has to open.

## 5. Open questions

- **Retask is unbuilt.** Whether it forks a new `agent_id` and copies context in, or reuses the same
  id with a new instruction and the old transcript as a preamble, is undecided and probably wants
  its own proposal once Resume and Unload have more mileage on them.
- **Instance-record persistence across a restart is `G97`/`G120`, unbuilt.** The per-instance flag
  proposed here is a UI framing on top of that gap, not a substitute for closing it.
- **Whether an agent-authored profile (a coordinator saving a role it composed) is ever wanted** is
  speculative; §1's spawn-origin dimension is proposed now only because it is cheap to carry from
  the start, not because agent-authored profiles are proposed for building.
- **Where `may_message_user` is actually enforced** — refused at the MCP tool boundary, or only a UI
  affordance that is hidden — is not decided here and should follow whatever `inbox-agent-graph-final`
  §8's tool-policy enforcement ends up being.

## Related docs

- [`../wip/agent-setup.md`](../wip/agent-setup.md) — P4, the landed `Profile` this extends
- [`../wip/agent-vocabulary.md`](../wip/agent-vocabulary.md) — §4, the Stop/Unload/Resume/Delete
  lifecycle Resumability builds on
- [`backlog/agent-graph-final.md`](./backlog/agent-graph-final.md) — §§3, 6, 8, 10, the Hosted/
  Reported split, the role policy table and the MCP surface this document assumes
- [`../features/sessions-and-workspaces.md`](../features/sessions-and-workspaces.md) — the run
  record and the confinement rules a profile's identity persistence relies on
- [`../backlog.md`](../backlog.md) — `G97`, `G120`, `G122`, the gaps named above
