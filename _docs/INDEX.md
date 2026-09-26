---
id: index
title: Documentation index
kind: meta
status: current
summary: The map of `_docs/` — how it is organized, the catalogue, which document owns which fact, and which to read for a given task.
read_when: you are starting any task and need to know which two or three documents it needs
updated: 2026-09-25
verified: 2026-09-24
---

# Documentation index

**Read this first, then read the two or three documents it sends you to — not more.** Every rule in
this library exists to make that cheap.

If your change touched code, you owe the documents it touched an update in the same commit;
[`_meta/authoring.md`](./_meta/authoring.md) says which ones and how, and is the only meta document a
contributor needs. Reorganizing the library itself is [`_meta/librarian.md`](./_meta/librarian.md).

`just docs-touched` names the documents your working diff obliges you to check. `just docs-lint`
checks the rules mechanically.

---

## 1. How the library is organized

Folders encode **kind of knowledge**. Frontmatter encodes **stability** — a `status: draft` document
sits beside a `current` one rather than in a separate folder.

| Path | Holds |
|---|---|
| `INDEX.md` | This map |
| `backlog.md` | Every open question, known gap and deferred item, project-wide |
| `product/` | Why Ubiq exists, in user terms. No code references |
| `features/` | One document per user-visible capability: contract on top, implementation below |
| `tech/` | Cross-cutting models, rules, conventions and procedures |
| `references/` | Specifications of protocols Ubiq speaks but does not own. External material, kept verbatim |
| `design/` | Wireframes, prototypes and captured artifacts. Assets, not documents |
| `wip/` | The current task's working notes. Deleted when the task closes |
| `inbox/` | Raw unprocessed input, waiting to be filed |
| `_meta/` | How this library works. The underscore means *not project knowledge* |

The rule that decides where something goes: **if deleting the capability from Ubiq would delete the
document, it belongs in `features/`; if the document would survive, it belongs in `tech/`.**

Three conventions worth knowing before you read anything.

Documents outside `wip/` and `inbox/` are written in the **present tense** and describe only the
state that holds today — history lives in git, in the decision register, and in
`_meta/review-log.md`.

**Every fact has one owning document**; everyone else links to it. Section 3 is that registry.

**`status: draft` means the design is settled and the code is behind it.** Much of Ubiq is designed
ahead of its implementation, and a draft document says so in a field rather than in a banner a
reader has to date. What the tree lacks is listed as gaps in [`backlog.md`](./backlog.md), not
hedged inside the documents.

One boundary sits outside this library entirely: `crates/agent-manager/` is a separate crate with
its own `_docs/` and its own `AGENTS.md`, and it owns every fact about harness configuration. This
library links there rather than restating it — [`tech/agent-manager.md`](./tech/agent-manager.md)
states the boundary once.

## 2. Catalogue

<!-- generated:begin catalogue -->

### Product

| Document | What it is | Verified |
|---|---|---|
| [Glossary](./product/glossary.md) | Plain definitions of the recurring terms — harness, agent type, session, workspace, panel, pane, dock, coordinator, bus, catalog — for anyone reading the rest of this documentation. | 2026-09-01 |
| [Product overview](./product/overview.md) | What Ubiq is, who runs it, why an agent harness needs a real terminal rather than a chat box, and what the product deliberately refuses to be. | 2026-08-31 |

### Features

| Document | What it is | Verified |
|---|---|---|
| [The chat panel](./features/chat.md) | Editor-like chat tabs — many, movable to any dockable region, each a view onto a host-owned conversation or onto none, drawn by the composer, transcript and tool blocks the whole window shares. | 2026-09-25 |
| [Connectors](./features/connectors.md) | Named authenticated identities at GitHub, GitLab, Gitea, Azure DevOps, Atlassian and Google Workspace — cloud or self-hosted, several per provider — created by completing a flow, with the token in the OS keychain and an untrusted certificate resolved by pinning one confirmed fingerprint to the instance. | 2026-09-26 |
| [Drones](./features/drone.md) | One small executable Ubiq places on a machine it is not running on, serving that machine's terminal, files, search and machine facts over a single duplex byte stream — attached as an ordinary host over an SSH exec channel, with three lifetimes, a per-project origin, and a hash-pinned binary the interface uploads when the remote `PATH` has none. | 2026-09-26 |
| [Logs](./features/logs.md) | One sink every subsystem writes its diagnostics to, and the console panel that reads it back with a subsystem selector and a level floor. | 2026-09-26 |
| [Notifications](./features/notifications.md) | One bell in the titlebar over a host-owned history — a level, an origin and an optional link per notification, a badge that counts the unread, a flash that carries a click straight to where it points, and mute rules by scope, level and duration that also decide what the desktop hears. | 2026-09-20 |
| [Panes and terminals](./features/panes-and-terminals.md) | What a pane shows, how exactly one of them holds focus, how a resize reaches the harness, and how a pane is moved around the window's dock. | 2026-09-24 |
| [Sessions and workspaces](./features/sessions-and-workspaces.md) | A session is a named piece of work that owns a folder and outlives the agents inside it; a workspace is one running agent within it, and the two have separate lifecycles. | 2026-09-24 |
| [Stats](./features/stats.md) | The Control screen — five readings of the running host on one page, and the usage meter on the other, whose tables exist and whose producer does not. | 2026-09-24 |
| [Agents mode — the columns](./features/workbench-agents.md) | The rail's Agents mode — a row of parallel columns, each a transcript and a composer over one live conversation, tabs that group agents into a column, the bench of agents no column is showing, the sidebar that lists every conversation the window holds, the three-dots menu over a live agent, and the New agent form all three surfaces raise. | 2026-09-26 |
| [Git mode — refs, history and changes](./features/workbench-git.md) | The rail's Git mode — the refs explorer of branches, remotes, tags, stashes and submodules, the paged commit history with its painted lanes, the conflicted, staged and unstaged change lists with the commit box, the diff under them, and the strip that names the repository and what HEAD is doing. | 2026-09-25 |
| [IDE mode — the explorer and the editor](./features/workbench-ide.md) | The rail's IDE mode — the project's file explorer and its right-click menu, the editor tabs each open file is a panel of, the viewer that draws one by kind, Markdown reading width and its minimap, diagrams and Excalidraw scenes, the image editor over any picture, and how a file is saved. | 2026-09-26 |
| [Sink mode — the kitchen sink](./features/workbench-sink.md) | The rail's Sink mode — the application's own test bench, twelve pages of fixtures with nothing behind them — a buffer, one page per special viewer, the style reference, the file picker in each shape a screen can ask for, the two settings layouts, a live conversation beside its bus traffic, the A2UI surface, the script scratchpad and the teamsim testbed. | 2026-09-26 |
| [Tasks mode — the board](./features/workbench-tasks.md) | The rail's Tasks mode — a column per status, a card per task, what a drag means, the labels and the filter that narrow it, missions and the children they spawn, the task panel that reports one task whole and edits it a field at a time, and the plan surface a mission raises over the window. | 2026-09-26 |
| [The Teams graph modes](./features/workbench-teams.md) | The rail's two graph modes — `Teams`, scoped by a window span and drawing its cards' conversations in the dock, and `[Teams]`, the established screen kept beside it with its own inspector and composer — the twelve arrangements the canvas computes for itself, the hexagonal status mark, the filters, the drag model and the tasks drawer under both. | 2026-09-25 |
| [The workbench](./features/workbench.md) | The window's shell — the activity rail and the nine modes it selects between, the dock of movable panels the user arranges around the centre, the titlebar and its navigator, the projects a window holds and the empty state one with none shows, the picker that adds, clones and opens them, project and application settings, the file picker any screen raises, and the status bar that reports on all of it. Each mode's own screen has a document of its own. | 2026-09-26 |

### Tech

| Document | What it is | Verified |
|---|---|---|
| [Backlog](./backlog.md) | Every open question, known gap and deferred item across the project, in one register. | 2026-09-25 |
| [The agent-manager boundary](./tech/agent-manager.md) | What the embedded harness-management library owns, what Ubiq owns, how the application consumes it, and the rule that keeps the two from growing into each other. | 2026-09-26 |
| [Architecture](./tech/architecture.md) | The two halves — coordinator and UI — the single bus between them, the rules neither may break, and why the split is drawn before it is needed. | 2026-09-26 |
| [Code map](./tech/code-map.md) | Generated map of the application's source tree, and the inverted index from every file to the documents that anchor it. | 2026-09-23 |
| [Reusable components](./tech/components.md) | The reusable components Ubiq builds out of its own primitives — the floating popover, the multi-select dropdown a filter or a form narrows with, the activity bar a conversation heads with, the file picker any screen raises to choose a path, the viewer that draws one open file whole, the diff renderer two screens reach a change through, and the capabilities and tools panels each asked for by two surfaces — the state that drives them, and the discipline that keeps a compound a component rather than a one-off screen's decoration. | 2026-09-24 |
| [Decision register](./tech/decisions.md) | One entry per structural decision — what was chosen, why, and what it costs — cited as `Dnn` across this library. | 2026-09-26 |
| [Diagram format](./tech/diagram-format.md) | The compact YAML authoring format for the wireframes under `_docs/design/`, and the converter that validates and renders it. | 2026-08-31 |
| [Operations](./tech/operations.md) | Prerequisites, the complete command reference, what a first build costs, the checks a change has to pass before it lands, and the runbook for a tool an agent cannot run. | 2026-09-26 |
| [Project structure](./tech/project-structure.md) | Every folder in the workspace, what belongs in it, what must never go in it, and the two crates' division of labour. | 2026-09-26 |
| [Transport contract](./tech/transport-contract.md) | The complete message set the UI and the coordinator exchange — the pane, session, project, file, git, work, conversation, search, account, quota, agent definition, command-line, host browse, connector, repository, task-source, assist, notification, web asset and carrier families, the framing rules, and the procedure for adding a variant. | 2026-09-26 |
| [UI and design](./tech/ui-and-design.md) | The GPUI rendering model, the complete theme token set and the rule that no colour escapes it, how a palette is switched, the shape every surface, modal and dialog is drawn in, the page every primitive is looked at on, and the design assets screens are built against. | 2026-09-25 |
| [Version control](./tech/version-control.md) | How the host reads a project's repositories and how the Git screen writes them — cloning, upward discovery and scope, the bounded downward walk that finds the repositories inside a project and merges them into one map, the git worker's two queues and its per-project caches, the three shapes it answers with, the commit-graph lane engine, the refresh discipline that narrows the staleness window, and the ceilings and assumptions the model rests on. | 2026-09-22 |

### References

| Document | What it is | Verified |
|---|---|---|
| [A2UI catalogs and components](./references/a2ui-catalog.md) | The vocabulary an A2UI agent draws from — the eighteen components of the basic catalog with their props and enums, the shared envelope properties, the fifteen catalog functions, the layout model and its deliberate refusal of padding and colour, templated lists, and how an application defines a catalog of its own. | 2026-09-12 |
| [A2UI protocol — the wire](./references/a2ui-protocol.md) | The Agent-to-UI protocol as an external contract — the six agent-to-renderer envelopes and the four renderer-to-agent ones, the surface lifecycle, the flat adjacency-list component model, JSON-Pointer data binding and two-way input, actions and function calls, catalog negotiation, the version landscape, and the SDKs that exist. | 2026-09-12 |
| [Agent Client Protocol (ACP)](./references/acp-protocol.md) | The JSON-RPC wire reference for the Agent Client Protocol — initialisation, session setup and loading, the streaming update vocabulary, tool-call reporting, permission prompts, the client-side filesystem and terminal callbacks, and the Rust SDK Ubiq reads. | 2026-09-03 |
| [Servo — the embeddable web engine](./references/servo-engine.md) | Servo as an external dependency Ubiq could embed — the libservo embedding API and its stability, the window and offscreen rendering contexts and what a Metal/GPUI bridge would take, platform and build cost, web-platform completeness measured against WPT, the multiprocess and sandbox model, licensing, and the reference embedders that exist. | 2026-09-10 |

### Meta

| Document | What it is | Verified |
|---|---|---|
| [Writing and updating docs](./_meta/authoring.md) | What every agent and human owes this documentation when they change code — and the small set of edits they may make. | 2026-08-31 |
| [Proposal ledger](./_meta/feedback.md) | Append-only ledger of documentation changes the bookkeeper may not make unilaterally, and the resolutions they received. | 2026-09-17 |
| [Librarian rulebook](./_meta/librarian.md) | How `_docs/` is organized, why it is organized that way, and how a bookkeeper agent keeps it that way. | 2026-08-31 |
| [Review log](./_meta/review-log.md) | Append-only record of what each documentation maintenance pass checked, fixed and left alone. | 2026-09-16 |

### Work in progress

| Document | What it is | Verified |
|---|---|---|
| [Agent login note](./wip/agent-login-note.md) | Why a conversation fails silently when the run directory has no Claude Code login, and the two ways to wire account selection into the RunSpec. | 2026-09-24 |
| [Wiring a real agent into the agent pane](./wip/agent-setup.md) | The protocol, the library work and the order of packages behind a real conversation with a composed harness — what has landed, and the honest inventory of what today's library cannot yet deliver. | 2026-09-25 |
| [The conversation vocabulary, the chat surface and the login sandbox](./wip/agent-vocabulary.md) | What landed in the round that made a login reach its harness's runtime, gave a conversation its model, thinking level and mode, turned the IDE chat into editor-like tabs, and gave every conversation a lifecycle — and what of it is verified against a running binary rather than only against tests. | 2026-09-24 |
| [Claude credentials vanish from a run directory](./wip/claude-auth-problem.md) | A Claude Code pane loses its OAuth login after some hours. Root cause found: Claude Code stores the login in a macOS keychain item keyed by sha256 of $CLAUDE_CONFIG_DIR whenever the keychain is reachable, migrating the seeded .credentials.json into it and deleting the file, so every refresh is invisible to agent-manager and a per-run config dir makes the key per-run. The fix denies a Claude run the login keychain. This is the full record — evidence, the discarded hypotheses, the fix as landed, and what to check if it is not resolutive. | 2026-09-24 |
| [Cloning a project](./wip/clone-a-project.md) | How a repository becomes a project — a connection's listing or a pasted URL, a branch, a destination, and the throwaway clone that is deleted when it closes. The clone half is built and covered by tests; the named OAuth registrations the connect flow picks from are built and never exercised against a live provider, which is the gap this document exists to record. | 2026-09-23 |
| [The drone as a tool an agent can reach](./wip/drone.md) | The working note for the drone's tenth phase — the drone as a fifth MCP server, with `drone_*` tools over the bus, capability-gated, a new `shell` message pair and a per-root read-only mode. Designed, and none of it written. Carries the gaps the nine built phases left open too, chief among them that nothing has been run against a real `ssh`, `scp` or `sshd`. | 2026-09-25 |
| [Grok ACP — captured from the live binary](./wip/grok-acp-capture.md) | What `grok agent stdio` (grok 1.0.13) actually speaks, captured frame by frame, and the gaps between it and `io/acp_client.rs`. | 2026-09-10 |
| [How in-app help works](./wip/help.md) | The mechanics of Ubiq's help system end to end — the content tree and its page frontmatter, the packer and the bundle it writes, how the app finds and unpacks it, how the panel renders and navigates a page, how a context key becomes a page, what the MCP server exposes, and what happens at every point where something is missing. | 2026-09-19 |
| [In-place help, and the identity layer under it](./wip/in-place-help.md) | An inspector-style help mode — point at any part of the window and read what it is — and the element identity scheme underneath it, which exists to serve personalisation and extensions as much as help. Phases 1 and 2 are built — the `UiId` grammar, a static catalogue of names with a label and a sentence each, a `.ui_id()` element extension feeding a per-frame bounds registry, the deepest-hit lookup, and the targeting mode itself — ⇧F1, a full-window layer, a highlight and a balloon, closed by Escape or its own Done. Phase 3a — marking the rail and the titlebar, every name the catalogue currently holds — is built too, proven by headless render tests against the real elements. The help pages behind the sentences (phase 3b) are still designed here and not written. | 2026-09-26 |
| [Indexing a project](./wip/indexing.md) | What Ubiq keeps about a project so a search need not re-read it — a per-project level defaulting from an application setting, and a full-text index that selects candidate files for the existing content search rather than answering it. The full-text half is built; the symbol half the `full` level names is not, which is the gap this document exists to record. | 2026-09-24 |
| [The knowledge base — sources, the write half, and what is next](./wip/kb.md) | A project's knowledge base as it stands — a per-project list of sources persisted as one TOML file, a folder read where it lies, a git repository cloned and refreshed, an internal wiki, a host-side write half (`kb/ops.rs`) behind six new messages, the `ubiq-kb` MCP server that reaches it, the explorer's right-click menu that reaches it from the interface, and a document as a dock tab — the same `OpenFile` the IDE's editor uses, with its own Save gated on the source's write access. | 2026-09-22 |
| [opencode ACP — captured from a live session](./wip/opencode-acp-capture.md) | What `opencode acp` (opencode 1.18.28) actually speaks over ACP, captured frame by frame from a real session, and the five gaps it exposed in `io/acp_client.rs` — the turn with no user on the wire, the silent wait, the `task` spawn's shape, a todo list with no `plan` update, and the `task` call's own missing origin — and how the bridge reads them. | 2026-09-13 |
| [The planning system — what remains open](./wip/planning-system.md) | What is still undecided about the planning flow — missions, plans and their annotations — after all seven staged slices (T-56 through T-62) and the follow-on provenance work (T-94) shipped. The built design lives in `features/workbench-tasks.md`, `tech/transport-contract.md` and `tech/decisions.md` (`D157` through `D161`); this file is only the remainder. | 2026-09-25 |
| [Pre-editions refactoring plan](./wip/refactor-plan.md) | Phases 0-3 are done and so are phase 4's composition root and preference round-trip; three phase-4 items remain, each blocked or deferred for a recorded reason, and every `just verify` check passes but docs-lint — whose open question is what that lint should apply to, since most of its failures are inbox documents. | — |
| [Teams across projects](./wip/teams-cross-project.md) | How the Teams screen draws every open project's agents at once — a second rail entry in the APP group that the span is read off, one merged projection built from each project's `live_work`, an owner map that answers "whose agent is this" for every write the screen makes, and what the rail, the titlebar and a `ubiq://` link keep meaning when the canvas is about more than one project. | 2026-09-25 |
| [Teams block positioning](./wip/teams-layout-spike.md) | Why the teams graph grows into a tower nobody can read in a rectangular viewport, what the two halves of the spike measured — a Python tool that renders an arrangement and scores it, and a teamsim section of the kitchen sink that drives the production arrangements from the same scenario file — what the measurements say to change, and what five further shapes (organic, multiradial, spider, hex, islands) came out at. | 2026-09-23 |
| [Web panels — the origin and the bridge](./wip/web-panel-phase2.md) | Phase 2 of the web-panel proposal as built — the `_web/<app>/<token>/` routes on the interface's existing loopback server, a per-panel token from the platform's CSPRNG, two frame queues per session with a long-poll that answers on its own thread, the two-transport `bridge.js` shim, and a demo tenant that proves the loop. The container is the external browser; the embedded `wry` webview was not built and the shim carries its half anyway. | 2026-09-12 |
| [Web panels — the shared workarea and the bundle fetch](./wip/web-panel-phase3.md) | Phase 3 of the web-panel proposal as built, Rust side only — a `shared_workarea` on `HostInfo` reserved at `<config root>/ui/`, a four-variant web asset message family, and `crates/ubiq-host/src/web_assets/` fetching a manifest's 554 files six at a time, verifying each against its SHA-256, and writing them into one compressed `.bundle` file whose rename over a `.part` sibling is the only proof of done. No interface wiring, which is a later unit. | 2026-09-24 |
| [Web panels — the Excalidraw chrome, and editing](./wip/web-panel-phase45.md) | Phases 4 and 5 of the web-panel proposal as built — a chrome page that fetches the host's generated import map, injects it under a per-response CSP nonce and mounts Excalidraw off the local mirror; an Edit action on the viewer header that is an axis of its own rather than a fourth ViewLayout; and a debounced `Changed` frame written into the existing `EditorState` buffer, so the dirty dot, `⌘S`, save-as and `D37`'s version check keep working with no new save path and no new message. | 2026-09-12 |
| [Web panels — the embedded browser](./wip/web-panel-phase6.md) | Phase 6 of the web-panel proposal as built — the container moves from an external browser into the window through `gpui-wry` on macOS and Windows, `Edit` becomes a fourth `ViewLayout` (reversing phase 5's call), a mark-and-sweep module keeps the child webview clipped to its own dock tab, sessions are settled every render rather than on a click, saving reaches the file through the existing `⌘S` path with a new `Save` bridge frame, and the explorer gains a `New Excalidraw` row. Every platform without `gpui-wry`'s finished Unix path keeps phase 5's external browser. A later addition, draw.io, is the second tenant on this same axis — it offers the same `[Edit, Preview]` layouts, the explorer gains a matching `New draw.io` row, and a new `Preview { svg }` bridge frame gives its Preview position a picture for a format the interface has no native renderer for. | 2026-09-20 |
| [Windows build FAQ](./wip/windows-build.md) | Answers for the failures a Windows (GNU toolchain) build hits that macOS never does — the one found so far is a stale dlltool.exe on PATH breaking raw-dylib import-lib generation, fixed by an environment change, never by a code change. | — |

<!-- generated:end catalogue -->

## 3. Who owns which fact

**One fact, one owner.** Each class of fact is stated in exactly one document; everyone else links.
If you find the same fact in two places, the copy that is not listed here is the one to replace with
a link.

| Fact | Owner |
|---|---|
| Message variants, payloads, direction and framing | `tech/transport-contract.md` |
| The record types that cross the bus | `tech/transport-contract.md` |
| The rules neither half may break | `tech/architecture.md` |
| Module responsibilities and the dependency direction | `tech/architecture.md` |
| Where a file goes, and what a folder must never hold | `tech/project-structure.md` |
| The source tree, and which document anchors which file | `tech/code-map.md` (generated) |
| Theme tokens, and the rule that no colour escapes them | `tech/ui-and-design.md` |
| Palette switching, and the constants that are not colours | `tech/ui-and-design.md` |
| The shape a surface is drawn in, and screen conventions | `tech/ui-and-design.md` |
| The component conventions: kit, screen areas, gpui-component first | `tech/ui-and-design.md` |
| The reusable compound components, and where each lives | `tech/components.md` |
| Pane chrome, design assets | `tech/ui-and-design.md` |
| The window's areas, their sizes, and what owns each | `features/workbench.md` |
| Rail modes, panel visibility, projects, what a window owns | `features/workbench.md` |
| Project settings, application settings, the navigator and the file picker | `features/workbench.md` |
| The explorer, the editor tabs, the viewers and how a file is saved | `features/workbench-ide.md` |
| The agents screen: its columns, what a tab drag means, and the bench | `features/workbench-agents.md` |
| The Git screen: its refs, history, change lists and diff | `features/workbench-git.md` |
| How a repository is read: discovery, the worker, the log and refs, the lane engine | `tech/version-control.md` |
| The `Teams` and `[Teams]` graphs: the arrangements, the selection model, the inspector and the tasks drawer | `features/workbench-teams.md` |
| The tasks board: its columns, its cards, what a drag means, the task panel and the plan | `features/workbench-tasks.md` |
| The kitchen sink: its pages, its fixtures, the A2UI surface and the script scratchpad | `features/workbench-sink.md` |
| Commands, prerequisites, environment variables | `tech/operations.md` |
| Structural decisions and their cost (`Dnn`) | `tech/decisions.md` |
| The harness library's boundary and the rules across it | `tech/agent-manager.md` |
| Harness config locations, launch flags, catalog, accounts | `crates/agent-manager/_docs/` |
| What the Agent Client Protocol says on the wire | `references/acp-protocol.md` |
| What the A2UI protocol says on the wire, and its widget catalog | `references/a2ui-protocol.md`, `references/a2ui-catalog.md` |
| The diagram authoring format | `tech/diagram-format.md` |
| Session and workspace lifecycle | `features/sessions-and-workspaces.md` |
| Log subsystems, levels, the ring's capacity and the console | `features/logs.md` |
| The host's readings, the usage meter's buckets and its schema | `features/stats.md` |
| Focus, resize, layout modes, pane lifecycle | `features/panes-and-terminals.md` |
| The drone: its handshake, capabilities, lifetimes, deployment and per-project origin | `features/drone.md` |
| SSH connection profiles, where their secrets live, and the Drones settings section | `features/drone.md` |
| Product scope and non-goals | `product/overview.md` |
| Vocabulary | `product/glossary.md` |
| Open questions, known gaps, deferred items | `backlog.md` |
| Documentation rules | `_meta/librarian.md` |
| The contributor's duty | `_meta/authoring.md` |

## 4. Which documents your task needs

Assembled from each document's `read_when`. Read the path, not the library.

| Your task | Read, in order |
|---|---|
| Deciding whether something is in scope | `product/overview.md` |
| Meeting a term you do not know | `product/glossary.md` |
| Finding your way around the repository | `tech/project-structure.md`, then `tech/code-map.md` |
| Setting the project up, or running it | `tech/operations.md` |
| Adding a capability that crosses the UI/coordinator line | `tech/architecture.md`, then `tech/transport-contract.md` |
| Adding, changing or removing a message | `tech/transport-contract.md` |
| Changing session creation, attachment or agent spawning | `features/sessions-and-workspaces.md`, then `tech/transport-contract.md` |
| Changing pane layout, focus, resize or chrome | `features/panes-and-terminals.md`, then `tech/ui-and-design.md` |
| Building or restyling a screen, adding a colour or a size | `tech/ui-and-design.md` |
| Building a control that floats above another, or reshaping the activity bar | `tech/components.md`, then `tech/ui-and-design.md` |
| Adding a screen area, a panel or a rail mode | `features/workbench.md`, then `tech/ui-and-design.md` |
| Changing the Git screen — its refs, history, change lists or diff | `features/workbench-git.md`, then `tech/version-control.md` |
| Changing the explorer, the editor tabs, a viewer or how a file is saved | `features/workbench-ide.md`, then `tech/components.md` |
| Extending version control, or adding a write | `tech/version-control.md`, then `tech/transport-contract.md` |
| Changing the agents screen — its columns, its sidebar or what a tab drag does | `features/workbench-agents.md`, then `tech/ui-and-design.md` |
| Changing the `Teams` or `[Teams]` graph — its arrangements, inspector or tasks drawer | `features/workbench-teams.md`, then `tech/ui-and-design.md` |
| Changing the tasks board — its columns, cards, task panel or the plan surface | `features/workbench-tasks.md`, then `tech/transport-contract.md` |
| Adding a page or a fixture to the kitchen sink | `features/workbench-sink.md`, then `tech/ui-and-design.md` |
| Changing the window layout, or what a window owns | `features/workbench.md`, then `tech/architecture.md` |
| Changing the chat panel or a message renderer | `features/chat.md` |
| Reaching a machine Ubiq is not running on — the drone, its lifetime, its deployment, its SSH profiles | `features/drone.md`, then `tech/transport-contract.md` |
| Adding a log event, a subsystem, or changing the log console | `features/logs.md` |
| Touching the knowledge base — its sources, the git sync worker, its explorer panel or centre | `wip/kb.md`, then `tech/transport-contract.md` |
| Changing the Stats screen, what the host reports, or the usage meter | `features/stats.md`, then `tech/transport-contract.md` |
| Launching a harness, or touching accounts, skills or MCP servers | `tech/agent-manager.md`, then that crate's own `_docs/` |
| Wiring a harness that speaks ACP, or adding a session update | `references/acp-protocol.md`, then `tech/transport-contract.md` |
| Rendering an agent-authored UI tree | `references/a2ui-protocol.md`, then `references/a2ui-catalog.md` |
| Adding a file and not knowing where it goes | `tech/project-structure.md` |
| Adding a command | `tech/operations.md` |
| Editing or rendering a wireframe | `tech/diagram-format.md` |
| Arguing with a rule, or reversing a design choice | `tech/decisions.md` |
| Planning the next piece of work | `backlog.md` |
| Leaving documentation in order after a code change | `_meta/authoring.md` |
| Reorganizing the documentation itself | `_meta/librarian.md` |

## Related docs

- [`_meta/authoring.md`](./_meta/authoring.md) — the duty every contributor owes this library
- [`_meta/librarian.md`](./_meta/librarian.md) — the rulebook behind how this index is built
- [`backlog.md`](./backlog.md) — everything unresolved, project-wide
