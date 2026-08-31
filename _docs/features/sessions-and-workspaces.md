---
id: feat-sessions
title: Sessions and workspaces
kind: feature
status: draft
summary: A session is a named piece of work that owns a folder and outlives the agents inside it; a workspace is one running agent within it, and the two have separate lifecycles.
read_when: you are changing how sessions are created, attached to, persisted, or how an agent is spawned into one
updated: 2026-08-31
verified: 2026-08-31
code_anchors: [crates/ubiq-host/src/coordinator.rs, crates/ubiq-host/src/agent.rs]
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
the application's own directory. Every workspace in the session inherits it unless given a folder of
its own.

**Attaching is how the user sees a session.** Attaching returns the session and the full list of its
workspaces, so the UI can rebuild the panes from one message rather than reconstructing state
incrementally. Detaching removes the view and touches nothing else — the agents keep running, and
reattaching finds them where they were.

**A workspace is one agent, one directory, one terminal.** It carries its agent type, its folder,
its terminal dimensions, and whether the process is alive. Spawning one creates a pseudo-terminal,
launches the harness inside it, and starts streaming output.

**A workspace never outlives its session in the user's model.** Closing a session closes its
workspaces. The reverse does not hold: a workspace can exit while its session continues, and the
session is still there to spawn another into.

**The agent type must be registered.** Spawning names an agent type; an unregistered name is
rejected with an error the user sees, rather than a failed process spawn they have to interpret.

**A failed spawn is an error about a pane, not about the application.** The user asked for an agent
in a place, and the error belongs where they were looking.

**Terminal dimensions default to 80×24** and are corrected by the first resize once the pane knows
how big it actually is. A harness that starts at the wrong size and is immediately resized behaves
correctly; one that never learns its size does not.

## Contract

The session family of the transport contract carries all of this: `CreateSession`, `AttachToSession`,
`DetachFromSession`, `ListSessions`, `SpawnWorkspace`, `ListAgentTypes`, and the responses to each.
`SpawnWorkspace` also names the project the pane belongs to, so the catalogue can count what is
running in it.
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

`crates/ubiq-host/src/agent.rs` holds the agent-type registry the spawn path validates against. Its
launch facts come from the embedded harness library rather than a hard-coded table — see
[`../tech/agent-manager.md`](../tech/agent-manager.md).

The spawn path, in order: resolve the agent type, falling back to what the session starts by
default; resolve the folder; open a pseudo-terminal pair at 80×24; build the command with its
arguments, its working directory and the `TERM` and `COLORTERM` a harness reads before it decides
what it may draw; spawn the child; take a writer and a reader from the master; start the reader
thread and the one that waits for the child; and answer with the workspace, which is what makes the
pane appear.

## Failure

| What happens | Result |
|---|---|
| The agent binary is missing or fails to execute | `PaneError` naming the pane; the session is unaffected |
| The home folder cannot be created | `Error`; the session is not created |
| A spawn names an unknown session or agent type | `Error`; nothing is created |
| The harness exits | `PaneExited` with its code. The pane stays visible and stops accepting input |
| The pseudo-terminal stream ends | Treated as an exit |

Sessions and workspaces live in memory for the lifetime of the process. Persisting them across
restarts, and what "reattach" means once the coordinator is a separate process, are open — see
[`../backlog.md`](../backlog.md).

## Related docs

- [`panes-and-terminals.md`](./panes-and-terminals.md) — the visible half of a workspace
- [`../tech/transport-contract.md`](../tech/transport-contract.md) — the message set, in full
- [`../tech/architecture.md`](../tech/architecture.md) — why the coordinator holds all of this
- [`../product/glossary.md`](../product/glossary.md) — session, workspace, pane, agent type

## Next steps

- Persist sessions across restarts, so reattaching survives a quit.
- Rename and delete a session from the UI.
- Spawn a workspace into a folder chosen per workspace rather than inherited.
