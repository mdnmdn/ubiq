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
| G7 | The MCP surface Ubiq exposes to hosted agents is a module header. What tools it offers is undecided | [`tech/architecture.md`](./tech/architecture.md) |
| G9 | One file's diff against `HEAD` or the index is all that reads version control. The explorer's status marks draw unfilled, the working-tree counts and the branch readout are absent from the status bar, and the branch picker is gone rather than empty — none of them has a host answer behind it | [`features/workbench.md`](./features/workbench.md) |
| G10 | The chat has no transport family, so its composer sends to nothing and its reply is canned | [`features/chat.md`](./features/chat.md) |
| G11 | Two of the six rail modes — Control and KB — render an empty page, and the explorer and the chat leave with IDE mode. The terminals and the console stay, because a panel's visibility is its kind's rule rather than the frame's | [`features/workbench.md`](./features/workbench.md) |
| G12 | Ubiq ships no icon set, so the history and status glyphs borrow the nearest icon from the component library's bundle | [`tech/ui-and-design.md`](./tech/ui-and-design.md) |
| G14 | `just verify` is red on three clippy lints in `crates/agent-manager`, so the project gate cannot pass from a clean checkout | [`tech/operations.md`](./tech/operations.md) |
| G24 | `just test` is red on `codex_bridge_round_trips_events_and_terminates`: under the workspace run its `initialize` handshake times out after 10s, while the test passes on its own | [`tech/operations.md`](./tech/operations.md) |
| G16 | The titlebar's command field accepts text and does nothing with it — no file search, no command palette | [`features/workbench.md`](./features/workbench.md) |
| G17 | The thinking-budget selection is recorded and never sent anywhere, like the rest of the composer's pickers | [`features/chat.md`](./features/chat.md) |
| G18 | `selected` and `border_focus` are drawn only as specimens on the kitchen sink's style reference: no row is filled with `selected` and no pane shows focus on its edge, because focus across split panes is designed ahead of the code. A specimen is evidence a token has a value in both palettes, not evidence anything uses it | [`tech/ui-and-design.md`](./tech/ui-and-design.md) |
| G19 | Of the session family, only `SpawnWorkspace`, `WorkspaceSpawned` and `CloseWorkspace` are implemented (the project family is complete). `ListSessions`/`SessionList`, `CreateSession`/`SessionCreated`, `AttachToSession`/`SessionAttached`, `DetachFromSession`, `ListAgentTypes`/`AgentTypes`, `Status`, `Error` and the `SessionInfo` and `AgentTypeInfo` records exist in the transport contract document and in no code. The work family's `WorkSession` is a session on the wire, seen from the other side, and the two records merge when the session family lands | [`tech/transport-contract.md`](./tech/transport-contract.md) |
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
| G41 | `crates/ubiq-host/src/files/path.rs` is named in no document's `code_anchors`, so a change to the one security boundary the file family has is told it owes no document an update. The walk, the read, the save and the diff are anchored by [`tech/architecture.md`](./tech/architecture.md) | [`tech/transport-contract.md`](./tech/transport-contract.md) |
| G42 | `just docs-lint` is red on thirteen failures in `_docs/inbox/`, so the project gate cannot pass from a clean checkout for a third reason beyond `G14` and `G24`. Eleven are proposals no `INDEX.md` entry links, one has no frontmatter, and two are over the length ceiling | [`INDEX.md`](./INDEX.md) |
| G53 | `crates/ubiq-host/src/store/` is named in no document's `code_anchors`, so a change to a trait, to the on-disk format or to what a corrupt file does is told it owes no document an update. The same shape as `G41` | [`tech/architecture.md`](./tech/architecture.md) |
| G46 | The catalogue, the view state and a project's tasks are all written from the coordinator's run loop, which every pane's keystrokes pass through, so a `--config-root` on a network mount blocks them behind the write. This is the catalogue's own accepted pattern rather than a new defect — `G38` is about the *spawn* path probing a user's folder — and it holds for three stores | [`tech/architecture.md`](./tech/architecture.md) |
| G47 | The host answers work messages and never pushes one, so nothing tells a window that an agent moved on its own. A live agent needs an unsolicited variant the work family does not have | [`tech/transport-contract.md`](./tech/transport-contract.md) |
| G48 | The mock's five session ids and eleven agent ids are the same literals in every project, because a mock session is not a host object and nothing keeps a catalogue of them | [`features/sessions-and-workspaces.md`](./features/sessions-and-workspaces.md) |
| G49 | A step cannot be handed to an agent. `Step.owner` is on the record and read when a step is drawn, and nothing sets it | [`features/workbench.md`](./features/workbench.md) |
| G50 | Steps cannot be reordered from the interface, though `MoveStep` names a step and the place it should end up in on the wire for exactly that | [`features/workbench.md`](./features/workbench.md) |
| G51 | `cmd-s` and `ctrl-s` are bound in `crates/ubiq/src/app.rs` to a `"Workbench"` key context no element declares, and `save_active_file()` is never registered with `cx.on_action`, so the keystroke does nothing and the save path cannot be reached from the keyboard | [`features/workbench.md`](./features/workbench.md) |
| G55 | The component library's dock places edge regions left, right and bottom only, so there is no top region: "docked on top" is a split at the top of the centre, and takes its width from the centre rather than spanning under the explorer | [`features/workbench.md`](./features/workbench.md) |
| G56 | A panel dropped in a region its class forbids is moved back on the same edit rather than refused under the pointer, because the library's drop is region-blind. The drop reads as refused; the drag offers no indicator saying it would be | [`features/workbench.md`](./features/workbench.md) |
| G57 | Panels cannot be moved between, or focused from, the keyboard. `D17`'s reversal makes it possible and nothing builds it | [`features/workbench.md`](./features/workbench.md) |
| G58 | The library's free-floating tiles canvas is reachable — Ubiq names a tiles renderer because a dock renderer must — and nothing builds one, so the renderer draws nothing | [`tech/ui-and-design.md`](./tech/ui-and-design.md) |
| G59 | A panel name a saved layout carries that this build does not know is dropped rather than kept as a placeholder, so a layout written by a later build loses those panels on the round trip. The version check catches the case that matters; a hand-edited file is the one that does not | [`features/workbench.md`](./features/workbench.md) |
| G60 | The session family's table documents `ListSessions`, `CreateSession`, `AttachToSession`, `DetachFromSession`, `ListAgentTypes`, `SessionList`, `SessionCreated`, `SessionAttached`, `AgentTypes`, `Status` and `Error`, none of which are variants of `Message` — the table is ahead of `crates/ubiq-proto/src/messages.rs`, which `G19` reads from the other side | [`tech/transport-contract.md`](./tech/transport-contract.md) |
| G61 | A diff compares the blob as it is stored with the file as it is on disk, so a repository using `core.autocrlf` or a clean/smudge filter shows a whole-file change where git shows none. Running the filters means running programs a merely-opened folder configures, which is why the host does not | [`tech/architecture.md`](./tech/architecture.md) |
| G62 | A path in the middle of a merge has no stage-zero entry in the index, so `DiffBase::Index` answers it as wholly added rather than saying it is conflicted. The contract has no conflicted state for a `FileDiff` to carry | [`tech/transport-contract.md`](./tech/transport-contract.md) |
| G63 | A diff is answered only when it is asked for. Nothing tells a window that a pane's agent changed a file it is showing a diff of, which is the same watch `G34` needs for the tree | [`features/workbench.md`](./features/workbench.md) |
| G64 | `AppState::open_diff()` opens a tab on a file's change and asks the host for it, and nothing calls it: the explorer's rows have one click and no modifier reading, so a comparison can only be reached from a saved arrangement that held one | [`features/workbench.md`](./features/workbench.md) |
| G65 | A dirty file panel closed from its tab returns to the first group of the centre to ask, not to the group and the position it was closed from. The dock takes a closed tab out before the window hears about it, so the question is asked by putting the panel back | [`features/workbench.md`](./features/workbench.md) |
| G52 | The inspector's composer sends into a thread nothing answers. `SendToAgent` reaches the host, which appends the line to the mock agent's thread and answers with the agent carrying it, and nothing generates a reply — a reply needs a live harness behind the mock | [`features/workbench.md`](./features/workbench.md) |
| G66 | Nothing bounds the interface's workarea. The host reserves `projects/<ulid>/ui/` and never looks inside, so no size ceiling, no eviction and no age limit exists anywhere — the only thing that empties it is forgetting the project, which removes the directory wholesale | [`tech/architecture.md`](./tech/architecture.md) |
| G67 | The workarea is reserved by a `create_dir_all` inside `Projects::snapshot`, so every listing of the catalogue makes one syscall per project on the coordinator's run loop, beside the health probe on the same line. The same class as `G46` | [`tech/architecture.md`](./tech/architecture.md) |

## Open questions — a decision nobody has made

| # | Question | Affects |
|---|---|---|
| Q1 | Do sessions survive a restart? If so, what is persisted — the arrangement, the folders, or the conversations too? The window's own arrangement does, and drops its terminal panels on load; whether a restored layout should offer to respawn the harnesses it dropped is the half this question still owes | [`features/sessions-and-workspaces.md`](./features/sessions-and-workspaces.md) |
| Q3 | Does a crashed harness restart automatically, on request, or never? | [`features/panes-and-terminals.md`](./features/panes-and-terminals.md) |
| Q4 | Which of Ubiq's two session meanings maps onto the library's resumable session, and where does the mapping live? | [`tech/agent-manager.md`](./tech/agent-manager.md) |
| Q5 | The bus is unbounded, so nothing blocks and nothing is dropped. Should the queue be bounded instead, and if it is, what goes: the oldest chunks, a coalesced screen, or the whole pane's backlog? | [`tech/transport-contract.md`](./tech/transport-contract.md) |
| Q6 | How does a subagent's pane show its parentage, and who decides where it opens? | [`features/panes-and-terminals.md`](./features/panes-and-terminals.md) |
| Q7 | Is scrollback owned by the emulator, or does the coordinator keep a buffer so a reattaching UI can be repainted? This one is load-bearing for detach | [`tech/architecture.md`](./tech/architecture.md) |
| Q9 | A workspace is its pane today, so `WorkspaceInfo` carries a `PaneId` while `WorkspaceId` reaches the wire only as the work family's `AgentId`. Does a workspace ever keep an id distinct from its pane's, and if so does the record carry both? | [`tech/transport-contract.md`](./tech/transport-contract.md) |
| Q8 | A detached coordinator cannot write into the window's log ring. Does it carry its records over the transport as a message, keep its own ring the console queries, or write to a file the console reads? | [`features/logs.md`](./features/logs.md) |
| Q10 | The workarea is an absolute path on the host's machine, and the interface uses it directly. A detached host on another machine hands over a path the interface cannot open — does the interface fall back to a root of its own, does the host learn to serve the directory over the bus, or does a remote host simply mean no cached renders? | [`tech/architecture.md`](./tech/architecture.md) |

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
