---
id: backlog
title: Backlog
kind: tech
status: current
summary: Every open question, known gap and deferred item across the project, in one register.
read_when: you are planning the next piece of work, or you hit something unresolved and need somewhere to put it
updated: 2026-09-03
verified: 2026-09-03
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
| G92 | A conversation runs unconfined and auto-approved. Structured I/O refuses isolation, because a bridge owns its child's descriptors and the sandbox needs them; and the Claude launch passes `bypassPermissions` while every bridge answers each approval itself. So an agent in a column edits any file the user can, with no prompt. Deliberate for the first end-to-end slice and the next thing to close | [`tech/decisions.md`](./tech/decisions.md), [`tech/agent-manager.md`](./tech/agent-manager.md) |
| G93 | `AnswerPermission` and `SetAgentConfig` are on the wire and refused. Nothing emits a permission request a human could answer, and no harness advertises its models as config options yet | [`tech/transport-contract.md`](./tech/transport-contract.md) |
| G94 | Live agents sit beside mock ones in the same list. `crates/ubiq-host/src/work/mod.rs` holds both, and the sidebar draws them together, so a screen can show an agent that answers next to eleven that cannot | [`features/workbench.md`](./features/workbench.md) |
| G95 | A conversation is only Claude's. The Codex bridge takes input and is not offered; `opencode` and `copilot` are one-shot and would look broken the moment a second turn was typed, which is what `IoBridge::input()` answering `None` is for | [`tech/agent-manager.md`](./tech/agent-manager.md) |
| G96 | Only Claude reports a context window, so only Claude draws a ring. Codex, opencode and Copilot report tokens with no window, and a ratio with an invented denominator is worse than none | [`features/workbench.md`](./features/workbench.md) |
| G97 | A conversation does not survive a restart, and its transcript is held only in the window. The harness writes a richer record of its own inside the run directory the agent deletes when it closes | [`tech/agent-manager.md`](./tech/agent-manager.md) |
| G98 | Ubiq's conversation vocabulary is the Agent Client Protocol's v1. A v2 draft reshapes diffs into structured file changes, makes the message id required, and removes the mode methods outright — `refs/acp-protocol.md` records what is coming | [`tech/transport-contract.md`](./tech/transport-contract.md) |
| G99 | Raw harness frames reach the log console only when `RUST_LOG` asks for them by name, because they carry prompts and file contents. There is no control in the console for it | [`features/logs.md`](./features/logs.md) |
| G89 | An agent is composed with no skills, MCP servers, account or model: `crates/ubiq-host/src/agent.rs` builds a `RunSpec` naming a harness and a folder, because nothing on the wire carries a composition. This is `G31` seen from the half that would consume it | [`tech/agent-manager.md`](./tech/agent-manager.md) |
| G90 | An agent confined in a pane is macOS-only: `agent_manager::isolate::confined_launch` renders the policy and execs `sandbox-exec`, which macOS supports and Landlock cannot. isol8's pty seam replaces it and is in the pinned revision, but `PtyChild` fuses the child with the master — `resize` borrows it and `child` borrows it mutably — so a pane cannot resize on one thread while another waits. The master-only handle that unblocks the switch is requested in `refs/isol8-pty-seam-update.md` §8 | [`tech/agent-manager.md`](./tech/agent-manager.md), [`features/sessions-and-workspaces.md`](./features/sessions-and-workspaces.md) |
| G91 | A confined agent reaches no toolchain outside its project — a `node` under `nvm`, a `cargo` under `rustup` — because the policy grants the project's folder and the run's own directory and nothing else. isol8 has recipes for exactly this, and nothing selects one | [`tech/agent-manager.md`](./tech/agent-manager.md) |
| G7 | The MCP surface Ubiq exposes to hosted agents is a module header. What tools it offers is undecided | [`tech/architecture.md`](./tech/architecture.md) |
| G69 | Ignored paths stay unmarked. git2 cannot collapse an ignored tree the way a `CollapseDirectory` walk would, so the working-tree map does not enumerate `node_modules` and therefore cannot badge it `Ignored` | [`features/workbench.md`](./features/workbench.md) |
| G70 | The git family has no log and no refs list. A history view and a branch picker have nothing to ask for | [`tech/transport-contract.md`](./tech/transport-contract.md) |
| G71 | Nothing watches the git directory, so `HEAD`, `MERGE_HEAD` and the index go stale until a save, a pane exit or a project open asks again | [`features/workbench.md`](./features/workbench.md) |
| G10 | The chat has no transport family, so its composer sends to nothing and its reply is canned | [`features/chat.md`](./features/chat.md) |
| G11 | Two of the six rail modes — Control and KB — render an empty page, and the explorer and the chat leave with IDE mode. The terminals and the console stay, because a panel's visibility is its kind's rule rather than the frame's | [`features/workbench.md`](./features/workbench.md) |
| G12 | Ubiq ships no icon set, so the history and status glyphs borrow the nearest icon from the component library's bundle | [`tech/ui-and-design.md`](./tech/ui-and-design.md) |
| G14 | `just verify` is red on three clippy lints in `crates/agent-manager`, so the project gate cannot pass from a clean checkout | [`tech/operations.md`](./tech/operations.md) |
| G24 | `just test` is red on `codex_bridge_round_trips_events_and_terminates`: under the workspace run its `initialize` handshake times out after 10s, while the test passes on its own | [`tech/operations.md`](./tech/operations.md) |
| G16 | The titlebar's command field accepts text and does nothing with it — no file search, no command palette | [`features/workbench.md`](./features/workbench.md) |
| G17 | The thinking-budget selection is recorded and never sent anywhere, like the rest of the composer's pickers | [`features/chat.md`](./features/chat.md) |
| G18 | No pane shows focus on its edge, because focus across split panes is designed ahead of the code. `selected` and `border_focus` are drawn on the file picker's keyboard cursor and nowhere in a pane | [`tech/ui-and-design.md`](./tech/ui-and-design.md) |
| G19 | Of the session family, only `SpawnWorkspace`, `WorkspaceSpawned` and `CloseWorkspace` are implemented (the project family is complete). `ListSessions`/`SessionList`, `CreateSession`/`SessionCreated`, `AttachToSession`/`SessionAttached`, `DetachFromSession`, `ListAgentTypes`/`AgentTypes`, `Status`, `Error` and the `SessionInfo` and `AgentTypeInfo` records exist in the transport contract document and in no code. The work family's `WorkSession` is a session on the wire, seen from the other side, and the two records merge when the session family lands | [`tech/transport-contract.md`](./tech/transport-contract.md) |
| G22 | Keyboard scrollback navigation is absent — Page Up/Down still go to the harness. The wheel in normal screen moves through scrollback | [`features/panes-and-terminals.md`](./features/panes-and-terminals.md) |
| G27 | Ubiq's four crates pin `serde`, `tracing` and `flume` independently; the workspace has no `[workspace.dependencies]` table to make a skew impossible | [`tech/project-structure.md`](./tech/project-structure.md) |
| G28 | The config root holds an agent's run directories, its preference templates and its isolation state, and not the library's catalogue, accounts or credentials — those keep their own roots, so a development run is self-contained in what a pane writes and not in what it reads | [`tech/agent-manager.md`](./tech/agent-manager.md) |
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
| G68 | The file picker is raised by one screen — the kitchen sink's picker page — over a fixture tree, so no screen chooses a real path through it yet. It takes the forest it draws rather than fetching one, so a caller over a real project needs `ProjectTree`'s listings folded into `PickerNode`s and a second `PickerOwner` to route the answer to | [`features/workbench.md`](./features/workbench.md) |
| G60 | The session family's table documents `ListSessions`, `CreateSession`, `AttachToSession`, `DetachFromSession`, `ListAgentTypes`, `SessionList`, `SessionCreated`, `SessionAttached`, `AgentTypes`, `Status` and `Error`, none of which are variants of `Message` — the table is ahead of `crates/ubiq-proto/src/messages.rs`, which `G19` reads from the other side | [`tech/transport-contract.md`](./tech/transport-contract.md) |
| G61 | A diff compares the blob as it is stored with the file as it is on disk, so a repository using `core.autocrlf` or a clean/smudge filter shows a whole-file change where git shows none. Running the filters means running programs a merely-opened folder configures, which is why the host does not | [`tech/architecture.md`](./tech/architecture.md) |
| G62 | A path in the middle of a merge has no stage-zero entry in the index, so `DiffBase::Index` answers it as wholly added rather than saying it is conflicted. The contract has no conflicted state for a `FileDiff` to carry | [`tech/transport-contract.md`](./tech/transport-contract.md) |
| G63 | A diff is answered only when it is asked for. Nothing tells a window that a pane's agent changed a file it is showing a diff of, which is the same watch `G34` needs for the tree | [`features/workbench.md`](./features/workbench.md) |
| G64 | `AppState::open_diff()` opens a tab on a file's change and asks the host for it, and nothing calls it: the explorer's rows have one click and no modifier reading, so a comparison can only be reached from a saved arrangement that held one | [`features/workbench.md`](./features/workbench.md) |
| G65 | A dirty file panel closed from its tab returns to the first group of the centre to ask, not to the group and the position it was closed from. The dock takes a closed tab out before the window hears about it, so the question is asked by putting the panel back | [`features/workbench.md`](./features/workbench.md) |
| G52 | The inspector's composer sends into a thread nothing answers. `SendToAgent` reaches the host, which appends the line to the mock agent's thread and answers with the agent carrying it, and nothing generates a reply — a reply needs a live harness behind the mock | [`features/workbench.md`](./features/workbench.md) |
| G66 | Nothing bounds the interface's workarea. The host reserves `projects/<ulid>/ui/` and never looks inside, so no size ceiling, no eviction and no age limit exists anywhere — the only thing that empties it is forgetting the project, which removes the directory wholesale | [`tech/architecture.md`](./tech/architecture.md) |
| G67 | The workarea is reserved by a `create_dir_all` inside `Projects::snapshot`, so every listing of the catalogue makes one syscall per project on the coordinator's run loop, beside the health probe on the same line. The same class as `G46` | [`tech/architecture.md`](./tech/architecture.md) |
| G68 | File search on single file (proposal in inbox) |
| G69 | File search on the workspace (proposal in inbox)  |
| G70 | file context menu: duplicate, copy, paste , delete, rename |
| G71 | excalidraw no code view |
| G72 | fix file selection on click no explorer |
| G73 | explorer file wrong spacing and disposition |
| G74 | explorer hide hidden files, and force hidden files and folder on selection |
| G75 | spacing on project selector |
| G77 | manage autorefresh of git/tree on fs events |
| G78 | The settings overlay's Add harness button is present and does nothing. Definitions belong to agent-manager; the catalog messages that would list and add them are not on the wire yet | [`features/workbench.md`](./features/workbench.md), [`tech/agent-manager.md`](./tech/agent-manager.md) |
| G79 | The titlebar's centre holds the command field on every screen, where the design shows the screen's own name — `Agents · parallel columns`. A per-mode title needs somewhere for the field to go, which is `G16`'s question from the other side | [`features/workbench.md`](./features/workbench.md) |
| G80 | A column's footer draws no mode chip, because `WorkAgent` carries no mode. The design shows one beside the harness and the model, so it needs a field on the wire before the footer can report it | [`tech/transport-contract.md`](./tech/transport-contract.md), [`features/workbench.md`](./features/workbench.md) |
| G81 | The agents sidebar's session note is the title of a task in that session, because `WorkSession` has no description on the wire. A session that is for something no task names has no line to draw | [`tech/transport-contract.md`](./tech/transport-contract.md), [`features/workbench.md`](./features/workbench.md) |
| G82 | `COLUMNS_MAX` caps the agents screen at eight columns because each owns a composer out of a fixed pool built before the first frame. An unbounded row needs a text area a column can be given when it opens | [`features/workbench.md`](./features/workbench.md) |
| G83 | The Git screen's refs and history are fixtures: the branch list, the remotes, the tags, the stashes, the submodules and every commit on it are invented, because the git family carries no refs list and no log. This is `G70` seen from the screen that wants it | [`features/workbench.md`](./features/workbench.md), [`tech/transport-contract.md`](./tech/transport-contract.md) |
| G84 | The Git screen's fetch, pull, push, branch, stash, undo and commit are drawn and inert. Version control is read-only, so they wait on a write family and everything it needs — a confirmation surface, an undo, and an answer for the agent in pane two who is mid-rebase | [`features/workbench.md`](./features/workbench.md), [`tech/transport-contract.md`](./tech/transport-contract.md) |
| G85 | A staged path's row is compared against HEAD rather than against the index, because the file family offers `DiffBase::Head` and `DiffBase::Index` and no index-against-HEAD. A staged-only comparison needs a third base on the wire | [`tech/transport-contract.md`](./tech/transport-contract.md), [`features/workbench.md`](./features/workbench.md) |
| G86 | The Git screen has no specimen on the kitchen sink and no wireframe under `design/`; it is built against `inbox/design/git-proposal.png`, which is raw material rather than a captured asset | [`tech/ui-and-design.md`](./tech/ui-and-design.md) |
| G87 | A shell pane's environment starts from Ubiq's own — `portable-pty` captures the process env at spawn — so a variable Ubiq was launched without reaches it only because the login shell's profile sets it. A composed agent carries the environment the library computed, and a confined one carries the whole of it; nothing computes one for a shell | [`features/panes-and-terminals.md`](./features/panes-and-terminals.md) |
| G88 | A mode or a project with no saved arrangement is given that mode's regions but inherits whatever tree was on screen, rather than the default arrangement | [`features/workbench.md`](./features/workbench.md) |
| G89 | mouse scrolling direction in terminal should follow the os (for macos is inverted) |
| G90 | double click on the folder in the explorer opens an empy editor, it should be ignored |
| G91 | the markdown preview does not scroll anymore |



## Open questions — a decision nobody has made

| # | Question | Affects |
|---|---|---|
| Q1 | Do sessions survive a restart? If so, what is persisted — the arrangement, the folders, or the conversations too? The window's own arrangement does, and drops its terminal panels on load; whether a restored layout should offer to respawn the harnesses it dropped is the half this question still owes | [`features/sessions-and-workspaces.md`](./features/sessions-and-workspaces.md) |
| Q3 | Does a crashed harness restart automatically, on request, or never? | [`features/panes-and-terminals.md`](./features/panes-and-terminals.md) |
| Q4 | Which of Ubiq's two session meanings maps onto the library's resumable session, and where does the mapping live? | [`tech/agent-manager.md`](./tech/agent-manager.md) |
| Q5 | The bus is unbounded, so nothing blocks and nothing is dropped. Should the queue be bounded instead, and if it is, what goes: the oldest chunks, a coalesced screen, or the whole pane's backlog? | [`tech/transport-contract.md`](./tech/transport-contract.md) |
| Q6 | How does a subagent's pane show its parentage, and who decides where it opens? | [`features/panes-and-terminals.md`](./features/panes-and-terminals.md) |
| Q7 | Is scrollback owned by the emulator, or does the coordinator keep a buffer so a reattaching UI can be repainted? This one is load-bearing for detach | [`tech/architecture.md`](./tech/architecture.md) |
| Q8 | A detached coordinator cannot write into the window's log ring. Does it carry its records over the transport as a message, keep its own ring the console queries, or write to a file the console reads? | [`features/logs.md`](./features/logs.md) |
| Q11 | Where does "allow always" live? A permission option carries the kind, and something has to remember the answer. Per conversation is the protocol's scope; per agent definition is what a user would expect to survive a restart | [`tech/transport-contract.md`](./tech/transport-contract.md) |
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
