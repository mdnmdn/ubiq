---
id: backlog
title: Backlog
kind: tech
status: current
summary: Every open question, known gap and deferred item across the project, in one register.
read_when: you are planning the next piece of work, or you hit something unresolved and need somewhere to put it
updated: 2026-08-31
verified: 2026-08-31
review_cycle: monthly
---

# Backlog

One register for everything unresolved. A `TODO` in a document, an `Open questions` heading, or a
"not decided yet" aside all belong here instead — that is what keeps the rest of the library
readable as a statement of what holds.

Each row names the document it affects, so resolving an item tells you what to update. An item that
ships or is dropped **leaves this file**; its outcome lives in git, or in the decision register if
it settled something structural.

Documentation-structure proposals go to `_meta/feedback.md`, not here. The test: does resolving it
change what Ubiq does (here), or where a document lives (there)?

## Gaps — the tree lacks something the documentation describes

| # | Item | Affects |
|---|---|---|
| G1 | The application declares no dependency on the harness library. The boundary is designed; the edge is absent | [`tech/agent-manager.md`](./tech/agent-manager.md) |
| G5 | The agent-type registry has no source. The five supported harnesses are documented, not registered | [`features/sessions-and-workspaces.md`](./features/sessions-and-workspaces.md) |
| G6 | Only the single layout mode is drawn; the split and grid modes exist as an enum | [`features/panes-and-terminals.md`](./features/panes-and-terminals.md) |
| G7 | The MCP surface Ubiq exposes to hosted agents is a module header. What tools it offers is undecided | [`tech/architecture.md`](./tech/architecture.md) |
| G9 | The whole workbench renders fixtures from `state/sample.rs`. No folder is read, no file is opened, no branch is queried | [`features/workbench.md`](./features/workbench.md) |
| G10 | The chat has no transport family, so its composer sends to nothing and its reply is canned | [`features/chat.md`](./features/chat.md) |
| G11 | Four of the five rail modes — Control, Agents, KB, Tasks — render an empty page, and every panel including the chat leaves with IDE mode | [`features/workbench.md`](./features/workbench.md) |
| G12 | Ubiq ships no icon set, so the branch and history glyphs borrow the nearest icon from the component library's bundle | [`tech/ui-and-design.md`](./tech/ui-and-design.md) |
| G13 | Panel sizes and visibility are not persisted; every launch starts from the defaults in `theme.rs` | [`features/workbench.md`](./features/workbench.md) |
| G14 | `just verify` is red on three clippy lints in `crates/agent-manager`, so the project gate cannot pass from a clean checkout | [`tech/operations.md`](./tech/operations.md) |
| G24 | `just test` is red on `codex_bridge_round_trips_events_and_terminates`: under the workspace run its `initialize` handshake times out after 10s, while the test passes on its own | [`tech/operations.md`](./tech/operations.md) |
| G15 | A second window gets its own fixtures rather than its project's real tree, so two windows differ only by name, letter and colour | [`features/workbench.md`](./features/workbench.md) |
| G16 | The titlebar's command field accepts text and does nothing with it — no file search, no command palette | [`features/workbench.md`](./features/workbench.md) |
| G17 | The thinking-budget selection is recorded and never sent anywhere, like the rest of the composer's pickers | [`features/chat.md`](./features/chat.md) |
| G18 | `border_focus`, `accent_muted`, `selected` and `info_soft` have values in both palettes and no call site; the conventions they serve are drawn ahead of the code | [`tech/ui-and-design.md`](./tech/ui-and-design.md) |
| G19 | Of the session family, only `SpawnWorkspace`, `WorkspaceSpawned` and `CloseWorkspace` are implemented. `ListSessions`/`SessionList`, `CreateSession`/`SessionCreated`, `AttachToSession`/`SessionAttached`, `DetachFromSession`, `ListAgentTypes`/`AgentTypes`, `Status`, `Error` and the `SessionInfo` and `AgentTypeInfo` records exist in the contract and in no code | [`tech/transport-contract.md`](./tech/transport-contract.md) |
| G20 | One coordinator per window, started when the window is built. Two windows cannot see each other's panes, and a session is whatever a single window groups | [`tech/architecture.md`](./tech/architecture.md) |
| G21 | A harness is started as a plain command — an agent type is a program name, defaulting to the user's shell — rather than composed through the harness library | [`tech/agent-manager.md`](./tech/agent-manager.md) |
| G22 | The emulator offers no mouse text selection and no scrollback navigation, so what a harness draws can be read but not copied or scrolled back through | [`features/panes-and-terminals.md`](./features/panes-and-terminals.md) |
| G23 | `tokio` and `futures` are declared by the application and nothing consumes them; its async is GPUI's executor and the bus's channels | [`tech/project-structure.md`](./tech/project-structure.md) |
| G25 | The harness library emits no `tracing` events, so the log console's Harness subsystem has nothing to show | [`features/logs.md`](./features/logs.md) |
| G26 | The log ring lives in memory and is not written anywhere, so diagnostics die with the process and a bug report cannot carry them | [`features/logs.md`](./features/logs.md) |

## Open questions — a decision nobody has made

| # | Question | Affects |
|---|---|---|
| Q1 | Do sessions survive a restart? If so, what is persisted — the arrangement, the folders, or the conversations too? | [`features/sessions-and-workspaces.md`](./features/sessions-and-workspaces.md) |
| Q3 | Does a crashed harness restart automatically, on request, or never? | [`features/panes-and-terminals.md`](./features/panes-and-terminals.md) |
| Q4 | Which of Ubiq's two session meanings maps onto the library's resumable session, and where does the mapping live? | [`tech/agent-manager.md`](./tech/agent-manager.md) |
| Q5 | The bus is unbounded, so nothing blocks and nothing is dropped. Should the queue be bounded instead, and if it is, what goes: the oldest chunks, a coalesced screen, or the whole pane's backlog? | [`tech/transport-contract.md`](./tech/transport-contract.md) |
| Q6 | How does a subagent's pane show its parentage, and who decides where it opens? | [`features/panes-and-terminals.md`](./features/panes-and-terminals.md) |
| Q7 | Is scrollback owned by the emulator, or does the coordinator keep a buffer so a reattaching UI can be repainted? This one is load-bearing for detach | [`tech/architecture.md`](./tech/architecture.md) |
| Q8 | A detached coordinator cannot write into the window's log ring. Does it carry its records over the transport as a message, keep its own ring the console queries, or write to a file the console reads? | [`features/logs.md`](./features/logs.md) |

## Deferred — decided to wait

| # | Item | Why it waits |
|---|---|---|
| D1 | Splitting the coordinator into its own process | The contract makes it cheap later; nothing today needs it |
| D2 | Harnesses on remote hosts | Same reason, one step further out |
| D3 | Windows support | The pseudo-terminal layer is cross-platform; nothing has been run there |
| D4 | Packaging and distribution | The application is run from source |
| D5 | Automated tests for the application's window | The pane path is covered end to end over the bus and the window registry is covered without a frame; driving `AppState` itself needs a headless window |

## Related docs

- [`tech/decisions.md`](./tech/decisions.md) — the choices that are settled, and what they cost
- [`INDEX.md`](./INDEX.md) — which document owns which fact
- `_meta/feedback.md` — proposals about the documentation rather than the product
