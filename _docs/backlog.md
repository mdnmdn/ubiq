---
id: backlog
title: Backlog
kind: tech
status: current
summary: Every open question, known gap and deferred item across the project, in one register.
read_when: you are planning the next piece of work, or you hit something unresolved and need somewhere to put it
updated: 2026-09-01
verified: 2026-09-01
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
| G9 | Nothing reads version control. The explorer's status marks draw unfilled, the working-tree counts and the branch readout are absent from the status bar, and the branch picker is gone rather than empty. The tree, a file's bytes and a save are the host's | [`features/workbench.md`](./features/workbench.md) |
| G10 | The chat has no transport family, so its composer sends to nothing and its reply is canned | [`features/chat.md`](./features/chat.md) |
| G11 | Four of the five rail modes — Control, Agents, KB, Tasks — render an empty page, and every panel including the chat leaves with IDE mode | [`features/workbench.md`](./features/workbench.md) |
| G12 | Ubiq ships no icon set, so the history and status glyphs borrow the nearest icon from the component library's bundle | [`tech/ui-and-design.md`](./tech/ui-and-design.md) |
| G14 | `just verify` is red on three clippy lints in `crates/agent-manager`, so the project gate cannot pass from a clean checkout | [`tech/operations.md`](./tech/operations.md) |
| G24 | `just test` is red on `codex_bridge_round_trips_events_and_terminates`: under the workspace run its `initialize` handshake times out after 10s, while the test passes on its own | [`tech/operations.md`](./tech/operations.md) |
| G16 | The titlebar's command field accepts text and does nothing with it — no file search, no command palette | [`features/workbench.md`](./features/workbench.md) |
| G17 | The thinking-budget selection is recorded and never sent anywhere, like the rest of the composer's pickers | [`features/chat.md`](./features/chat.md) |
| G18 | `border_focus`, `accent_muted`, `selected` and `info_soft` have values in both palettes and no call site; the conventions they serve are drawn ahead of the code | [`tech/ui-and-design.md`](./tech/ui-and-design.md) |
| G19 | Of the session family, only `SpawnWorkspace`, `WorkspaceSpawned` and `CloseWorkspace` are implemented (the project family is complete). `ListSessions`/`SessionList`, `CreateSession`/`SessionCreated`, `AttachToSession`/`SessionAttached`, `DetachFromSession`, `ListAgentTypes`/`AgentTypes`, `Status`, `Error` and the `SessionInfo` and `AgentTypeInfo` records exist in the transport contract document and in no code | [`tech/transport-contract.md`](./tech/transport-contract.md) |
| G21 | A harness is started as a plain command — an agent type is a program name, defaulting to the user's shell — rather than composed through the harness library | [`tech/agent-manager.md`](./tech/agent-manager.md) |
| G22 | The emulator offers no mouse text selection and no scrollback navigation, so what a harness draws can be read but not copied or scrolled back through | [`features/panes-and-terminals.md`](./features/panes-and-terminals.md) |
| G25 | The harness library emits no `tracing` events, so the log console's Harness subsystem has nothing to show |
| G27 | Ubiq's four crates pin `serde`, `tracing` and `flume` independently; the workspace has no `[workspace.dependencies]` table to make a skew impossible | [`tech/project-structure.md`](./tech/project-structure.md) |
| G28 | The config root moves Ubiq's own stores but not the embedded library's, so a development run is only self-contained as far as the catalogue and view state. Deriving `agent-manager`'s catalogue, accounts and credentials roots from it waits on the dependency in `G1` | [`tech/agent-manager.md`](./tech/agent-manager.md) |
| G29 | Two hosts on one config root are last-writer-wins over `projects.toml`; an advisory lock around the read-modify-write would close it | [`tech/architecture.md`](./tech/architecture.md) |
| G30 | Health is probed at load, on open and on request. Nothing watches the filesystem, so a folder that goes away is only noticed the next time somebody asks | [`features/workbench.md`](./features/workbench.md) |
| G31 | A project binds to no profile, harness or account yet: the composer's `HARNESSES`, `MODELS` and `MODES` are still constants rather than a projection of the library's catalogue | [`tech/agent-manager.md`](./tech/agent-manager.md) | [`features/logs.md`](./features/logs.md) |
| G32 | Add and Locate open the platform's folder dialog, which browses the interface's filesystem — the one place the two halves are assumed to share a machine. A detached host needs a host-side listing behind it | [`tech/decisions.md`](./tech/decisions.md) |
| G26 | The log ring lives in memory and is not written anywhere, so diagnostics die with the process and a bug report cannot carry them | [`features/logs.md`](./features/logs.md) |
| G33 | The set of folders a deep walk will not descend into is fixed in `crates/ubiq-host/src/files/mod.rs`. Reading a project's `.gitignore` instead is what the interface's users will expect, and is the one thing that would justify the `ignore` crate | [`tech/transport-contract.md`](./tech/transport-contract.md) |
| G34 | Nothing watches a project's folder, so a file created, deleted or renamed outside Ubiq is invisible until the folder is collapsed and expanded again. The tree's merge is written to accept an unsolicited listing, so the watch is the only missing half | [`features/workbench.md`](./features/workbench.md) |
| G35 | `Row::loading` and `Row::truncated` are carried through the tree and drawn nowhere, so a folder whose listing is in flight looks empty and one cut short at the host's entry ceiling looks complete | [`features/workbench.md`](./features/workbench.md) |
| G36 | The file worker is one thread, so a request against a hung mount holds up the ones behind it. A pool would fix that and reorder the replies, which needs a sequence number on the wire before the interface could trust them | [`tech/transport-contract.md`](./tech/transport-contract.md) |
| G37 | A listing joins a child's path with a forward slash, which is the interface's own path shape and not Windows'. Nothing has been run there | [`tech/transport-contract.md`](./tech/transport-contract.md) |
| G38 | `spawn_workspace` looks a record up, probes its folder and resolves a path on the coordinator's thread, which every pane's keystrokes pass through. The file family was moved off that thread and the spawn path was not | [`features/panes-and-terminals.md`](./features/panes-and-terminals.md) |
| G39 | The chat belongs to the window rather than to a project, so switching projects keeps the conversation. It moves when the chat gets a transport family and a conversation is about something | [`features/chat.md`](./features/chat.md) |
| G40 | `FileLanguage` has no JavaScript arm, so a `.js` or `.mjs` file opens unhighlighted. It needs a variant and the matching arm in `ui/editor.rs`'s highlighter mapping | [`features/workbench.md`](./features/workbench.md) |
| G41 | `crates/ubiq-host/src/files/` is named in no document's `code_anchors`, so a change to the walk, the read or the save is told it owes no document an update | [`tech/transport-contract.md`](./tech/transport-contract.md) |
| G42 | `just docs-lint` is red on nine failures in `_docs/inbox/`, so the project gate cannot pass from a clean checkout for a third reason beyond `G14` and `G24`. Eight are proposals no `INDEX.md` entry links, and one has no frontmatter | [`INDEX.md`](./INDEX.md) |

## Open questions — a decision nobody has made

| # | Question | Affects |
|---|---|---|
| Q1 | Do sessions survive a restart? If so, what is persisted — the arrangement, the folders, or the conversations too? | [`features/sessions-and-workspaces.md`](./features/sessions-and-workspaces.md) |
| Q3 | Does a crashed harness restart automatically, on request, or never? | [`features/panes-and-terminals.md`](./features/panes-and-terminals.md) |
| Q4 | Which of Ubiq's two session meanings maps onto the library's resumable session, and where does the mapping live? | [`tech/agent-manager.md`](./tech/agent-manager.md) |
| Q5 | The bus is unbounded, so nothing blocks and nothing is dropped. Should the queue be bounded instead, and if it is, what goes: the oldest chunks, a coalesced screen, or the whole pane's backlog? | [`tech/transport-contract.md`](./tech/transport-contract.md) |
| Q6 | How does a subagent's pane show its parentage, and who decides where it opens? | [`features/panes-and-terminals.md`](./features/panes-and-terminals.md) |
| Q7 | Is scrollback owned by the emulator, or does the coordinator keep a buffer so a reattaching UI can be repainted? This one is load-bearing for detach | [`tech/architecture.md`](./tech/architecture.md) |
| Q9 | A workspace is its pane today, so `WorkspaceInfo` carries a `PaneId` and `WorkspaceId` is defined and unused. Does a workspace ever keep an id distinct from its pane's, and if so does the record carry both? | [`tech/transport-contract.md`](./tech/transport-contract.md) |
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
