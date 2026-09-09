---
id: feat-sessions
title: Sessions and workspaces
kind: feature
status: draft
summary: A session is a named piece of work that owns a folder and outlives the agents inside it; a workspace is one running agent within it, and the two have separate lifecycles.
read_when: you are changing how sessions are created, attached to, persisted, or how an agent is spawned into one
updated: 2026-09-08
verified: 2026-09-08
code_anchors: [crates/ubiq-host/src/coordinator.rs, crates/ubiq-host/src/agent.rs, crates/ubiq-host/src/conversation.rs, crates/agent-manager/src/session.rs]
depends_on: [tech-transport]
review_cycle: monthly
---

# Sessions and workspaces

## Purpose

A developer running several agents needs somewhere to put them. A **session** is that place: a named
piece of work with a home folder, holding the agents serving it. A **workspace** is one of those
agents — a single running harness with its own directory and its own terminal. The user attaches to
a session to see and drive its workspaces, and detaches without ending anything.

## Behaviour

**A session is a grouping, not a process.** It owns a name, a home folder and a creation time, and
holds zero or more workspaces. Creating one starts nothing; it makes a place for workspaces to be
started in. Several sessions exist at once and are independent of each other.

**A session's home folder is created if it is absent.** The default is the workspace folder under
the application's own directory.

**A workspace's working directory is its project's, not its session's.** `SpawnWorkspace` names a
project, and the host resolves the folder from that project's record; an optional path relative to it
starts the harness in a subdirectory. The session's home folder is what a session with no project
would fall back to, and no caller does that — a window with no project spawns nothing.

**Attaching is how the user sees a session.** Attaching returns the session and the full list of its
workspaces, so the UI can rebuild the panes from one message rather than reconstructing state
incrementally. Detaching removes the view and touches nothing else — the agents keep running, and
reattaching finds them where they were.

**A workspace is one agent, one project, one terminal.** It carries its agent type, the project it
runs in and where inside it, its terminal dimensions, and whether the process is alive. It carries no
absolute path: the interface is told which project, and holds the name and colour for that already. Spawning one creates a pseudo-terminal,
launches the harness inside it, and starts streaming output.

**A workspace never outlives its session in the user's model.** Closing a session closes its
workspaces. The reverse does not hold: a workspace can exit while its session continues, and the
session is still there to spawn another into.

**The agent type must be registered.** Spawning names an agent type; an unregistered name is
rejected with an error the user sees, rather than a failed process spawn they have to interpret.
The register is the harness library's own list, so a harness it learns about is offered without a
change here, and each row says whether that harness can be started here — its own binary found on
this machine, or a command override configured for it — a row that cannot start says so before it
is picked. A name the library does not know is a program, which is how a shell reaches a pane.

**An agent is composed, not executed.** Starting one provisions a throwaway configuration directory
for that run, and the harness is launched against it with the environment the library computed —
the user's own `~/.claude` and its siblings are read-only for the duration. The directory belongs to
the pane: it is named by it, and it is deleted when the pane closes, credentials seeded into it
included. One left behind by a process that was killed is deleted at the next start.

**The record outlives the run directory.** A run is recorded under Ubiq's own root the moment it is
composed — one directory per run, named by the pane's or the agent's id, holding the metadata of
what was launched, where, as whom and in which face. Before that run directory is deleted — on a
pane close, on a conversation being retired, and on the startup sweep of what a killed process left
— the harness's own transcript is copied in beside that metadata and the record is stamped as
finished. So a conversation's full record survives the pane it was held in, and a run that crashed
leaves one too: its finish time is the sweep's, and its exit code is unknown rather than invented.
Which files are a harness's record is the harness library's answer, not Ubiq's, and a harness that
does not answer leaves metadata alone.
**A conversation the user marks persistent survives a restart, and the run directory is how.** That
directory is not scaffolding around the conversation — it holds the harness's own session store,
because every harness's config lever is pinned to it — so keeping it is what lets `--resume`, or
ACP's `session/load`, work after Ubiq has been quit and reopened. A marked conversation is therefore
*parked* rather than retired when its window closes or its harness exits: the harness stops, the
directory stays, and every credential seeded into it is scrubbed on the way out and re-seeded at the
next launch. Ubiq's own row beside the metadata carries what a relaunch needs and the library's
record does not — the profile, the picks, where the message sequence had reached, the title, and the
flag itself. Copying that directory instead of keeping it is what forks a conversation onto a second
agent. `D97` is the decision and its costs.

Nothing is kept for a conversation nobody marked: it is retired exactly as before, which is what
stops the run root growing without bound. The archived transcript beside the metadata is still data
rather than a resume — the resume is the store in the run directory, not the copy of it — and the
one thing it is still owed is a redraw for Claude Code's native bridge, which restores its memory
and replays no transcript. Old records for unmarked conversations are collected after
`HostSettings.retain_conversations_days`; a marked one is never collected on a timer, only by an
explicit delete.

**Unload is not delete.** A conversation's harness can be killed without ending the conversation —
`UnloadConversation`, answered by `ConversationUnloaded` — and unlike a pane close or
`EndConversation`, that keeps everything: the transcript, the `WorkAgent`, and the run directory with
whatever was seeded into it. Only the harness process and its pump go. What is kept is exactly what a
resume (`ResumeConversation`, or the next `PromptAgent`) needs to start the harness again under the
same `agent_id`, continuing the same message sequence rather than starting a new one. Delete
(`EndConversation`) is still what removes the run directory and its credentials, and takes the
conversation with it.

**An abort is an unload that does not ask.** `UnloadConversation` asks the harness to shut down
and waits for it to; `AbortConversation` kills its process and reaps it afterwards, for the harness
that does not act on the ask. The two are the same unload otherwise, down to the
`ConversationUnloaded` that answers both, and the interface reads them identically.

**An agent runs confined unless the settings say otherwise.** The policy grants the project's folder
and that run's own directory and denies the rest of the machine. `$HOME` is the user's own, and the
policy's toolchain grants are what makes the home usable: they name `~/.cargo`, `~/.npm` and their
siblings, so a compiler on the user's `PATH` reaches its own caches and nothing else in the home is
reachable. A replaced `$HOME` — a scratch one per run, or one kept by name — is offered and is not
the default, because a `~`-relative grant expands against the run's home, so replacing it aims
every one of those grants at a directory nothing populated and the agent cannot build. Which
`$HOME` a run gets, and which folders it may reach beyond the policy, are settings; what a policy
grants and which layers it stacks belong to the harness library — see
[`../tech/agent-manager.md`](../tech/agent-manager.md). A process that is itself confined cannot
confine anything — a sandbox does not nest — and says so once at startup rather than as an error on
every pane.

**A failed spawn is an error about a pane, not about the application.** The user asked for an agent
in a place, and the error belongs where they were looking.

**Terminal dimensions default to 80×24** and are corrected by the first resize once the pane knows
how big it actually is. A harness that starts at the wrong size and is immediately resized behaves
correctly; one that never learns its size does not.

## Contract

The session family of the transport contract carries all of this: `CreateSession`, `AttachToSession`,
`DetachFromSession`, `ListSessions`, `SpawnWorkspace`, `ListAgentTypes`, and the responses to each.
`SpawnWorkspace` names the project the pane belongs to — which is where its working directory comes
from, and what lets the catalogue count what is running in it — and answers `ProjectError` instead
when that project's folder cannot be worked in.
Variant names, payload fields and the `SessionInfo`, `WorkspaceInfo` and `AgentTypeInfo` records are
owned by [`../tech/transport-contract.md`](../tech/transport-contract.md).

Nothing in a `WorkspaceInfo` is a handle. The process, the writer and the pseudo-terminal stay in the
coordinator; what crosses the bus is a description.

## Implementation

`crates/ubiq-host/src/coordinator.rs` is the single source of truth. It runs on a thread of its own and
holds one set of I/O resources per workspace, keyed by the ID the pane and the workspace share: the
pseudo-terminal master for resizing, the writer for input, and a killer for closing. What crosses
the bus is a description built at spawn — splitting that description from the resources is what
lets it serialise. The session table itself, and the attach path over it, are a gap rather than a
design change; both are listed in [`../backlog.md`](../backlog.md).

`crates/ubiq-host/src/agent.rs` holds the agent-type registry the spawn path validates against, and
the composer behind it. Its launch facts come from the embedded harness library rather than a
hard-coded table — see [`../tech/agent-manager.md`](../tech/agent-manager.md). `Composed::exec` is
what the coordinator asks for: it answers the harness under its policy when the run is confined, and
the harness itself when it is not, so there is no field to reach for that would start a confined run
unconfined.

The spawn path, in order: look the project's record up and probe its folder, refusing before
anything is opened if it cannot be worked in; resolve the working directory from that record and the
optional path below it; resolve the agent type, falling back to what the session starts by default;
compose the run when the library knows that type, which resolves what it is composed of — the
account and model a profile names included — provisions its configuration directory, and resolves
the policy it runs under; open a pseudo-terminal pair at 80×24; build the command with its
arguments, its working directory, the environment the composition produced, and the `TERM` and
`COLORTERM` a harness reads before it decides what it may draw; spawn the child; take a writer and a
reader from the master; start the reader thread and the one that waits for the child; and answer
with the workspace, which is what makes the pane appear.

A composition that fails, and a policy that cannot be applied, both fail before a pseudo-terminal
exists — so both are a `PaneError` naming a pane the interface was never told about, and the run
directory the attempt created is removed.

## Failure

| What happens | Result |
|---|---|
| The agent binary is missing or fails to execute | `PaneError` naming the pane; the session is unaffected |
| The run cannot be composed, or its policy cannot be applied | `PaneError` naming the pane; the run directory is removed |
| The home folder cannot be created | `Error`; the session is not created |
| A spawn names an unknown session or agent type | `Error`; nothing is created |
| The harness exits | `PaneExited` with its code. The pane's tab is closed |
| The pseudo-terminal stream ends | Treated as an exit |

Sessions and workspaces live in memory for the lifetime of the process. A **conversation** marked
persistent is the exception and the only one: its recipe and its harness's store are on disk, so it
comes back. What "reattach" means once the coordinator is a separate process — and how a window
learns of a conversation it did not start — are still open; see [`../backlog.md`](../backlog.md).

## Related docs

- [`panes-and-terminals.md`](./panes-and-terminals.md) — the visible half of a workspace
- [`../tech/transport-contract.md`](../tech/transport-contract.md) — the message set, in full
- [`../tech/architecture.md`](../tech/architecture.md) — why the coordinator holds all of this
- [`../product/glossary.md`](../product/glossary.md) — session, workspace, pane, agent type

## Next steps

- Persist *sessions* across restarts. A marked conversation already does; the named grouping of
  panes around it does not.
- Rename and delete a session from the UI.
- Let the user choose which folder inside a project a workspace starts in.
