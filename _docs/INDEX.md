---
id: index
title: Documentation index
kind: meta
status: current
summary: The map of `_docs/` — how it is organized, the catalogue, which document owns which fact, and which to read for a given task.
read_when: you are starting any task and need to know which two or three documents it needs
updated: 2026-09-08
verified: 2026-09-02
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
| [The chat panel](./features/chat.md) | Editor-like chat tabs — many, movable to any dockable region, each a view onto a host-owned conversation or onto none, drawn by the composer, transcript and tool blocks the whole window shares. | 2026-09-12 |
| [Connectors](./features/connectors.md) | Named authenticated identities at GitHub, GitLab, Gitea, Azure DevOps, Atlassian and Google Workspace — cloud or self-hosted, several per provider — created by completing a flow, with the token in the OS keychain and an untrusted certificate resolved by pinning one confirmed fingerprint to the instance. | 2026-09-10 |
| [Logs](./features/logs.md) | One sink every subsystem writes its diagnostics to, and the console panel that reads it back with a subsystem selector and a level floor. | 2026-09-11 |
| [Notifications](./features/notifications.md) | One bell in the titlebar over a host-owned history — a level, an origin and an optional link per notification, a badge that counts the unread, a flash that carries a click straight to where it points, and mute rules by scope, level and duration that also decide what the desktop hears. | 2026-09-10 |
| [Panes and terminals](./features/panes-and-terminals.md) | What a pane shows, how exactly one of them holds focus, how a resize reaches the harness, and how a pane is moved around the window's dock. | 2026-09-10 |
| [Sessions and workspaces](./features/sessions-and-workspaces.md) | A session is a named piece of work that owns a folder and outlives the agents inside it; a workspace is one running agent within it, and the two have separate lifecycles. | 2026-09-12 |
| [Stats](./features/stats.md) | The Control screen — five readings of the running host on one page, and the usage meter on the other, whose tables exist and whose producer does not. | 2026-09-12 |
| [The workbench](./features/workbench.md) | The window's shell — the activity rail and its modes, the dock of movable panels the user arranges around the centre, the file explorer and editor a project owns, the Git screen of refs, history and uncommitted changes, the agents screen of parallel columns, the orchestration graph and the tasks board the rail's other built modes hold, the kitchen sink the application tests itself against, the file picker any screen raises to choose a path, the empty state a window with no project shows, and the status bar that reports on all of it. | 2026-09-12 |

### Tech

| Document | What it is | Verified |
|---|---|---|
| [Backlog](./backlog.md) | Every open question, known gap and deferred item across the project, in one register. | 2026-09-12 |
| [The agent-manager boundary](./tech/agent-manager.md) | What the embedded harness-management library owns, what Ubiq owns, how the application consumes it, and the rule that keeps the two from growing into each other. | 2026-09-12 |
| [Architecture](./tech/architecture.md) | The two halves — coordinator and UI — the single bus between them, the rules neither may break, and why the split is drawn before it is needed. | 2026-09-12 |
| [Code map](./tech/code-map.md) | Generated map of the application's source tree, and the inverted index from every file to the documents that anchor it. | 2026-09-08 |
| [Decision register](./tech/decisions.md) | One entry per structural decision — what was chosen, why, and what it costs — cited as `Dnn` across this library. | 2026-09-12 |
| [Diagram format](./tech/diagram-format.md) | The compact YAML authoring format for the wireframes under `_docs/design/`, and the converter that validates and renders it. | 2026-08-31 |
| [Operations](./tech/operations.md) | Prerequisites, the complete command reference, what a first build costs, the checks a change has to pass before it lands, and the runbook for a tool an agent cannot run. | 2026-09-11 |
| [Project structure](./tech/project-structure.md) | Every folder in the workspace, what belongs in it, what must never go in it, and the two crates' division of labour. | 2026-09-11 |
| [Transport contract](./tech/transport-contract.md) | The complete message set the UI and the coordinator exchange — the pane, session, project, file, git, work, conversation, search, account, quota, profile, command-line, host browse, connector, repository, assist, notification and web asset families, the framing rules, and the procedure for adding a variant. | 2026-09-12 |
| [UI and design](./tech/ui-and-design.md) | The GPUI rendering model, the complete theme token set and the rule that no colour escapes it, how a palette is switched, the shape every surface, modal and dialog is drawn in, the page every primitive is looked at on, and the design assets screens are built against. | 2026-09-12 |
| [Version control](./tech/version-control.md) | How the host reads a project's repositories — the rule that Ubiq creates a repository or reads one and never writes into one, where a clone runs, upward discovery and scope, the bounded downward walk that finds the repositories inside a project and merges them into one map, the git worker's two queues and its per-project caches, the three shapes it answers with, the commit-graph lane engine, the refresh discipline that narrows the staleness window, and the ceilings and assumptions the model rests on. | 2026-09-12 |

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
| [Proposal ledger](./_meta/feedback.md) | Append-only ledger of documentation changes the bookkeeper may not make unilaterally, and the resolutions they received. | 2026-09-12 |
| [Librarian rulebook](./_meta/librarian.md) | How `_docs/` is organized, why it is organized that way, and how a bookkeeper agent keeps it that way. | 2026-08-31 |
| [Review log](./_meta/review-log.md) | Append-only record of what each documentation maintenance pass checked, fixed and left alone. | 2026-09-10 |

### Work in progress

| Document | What it is | Verified |
|---|---|---|
| [Agent login note](./wip/agent-login-note.md) | Why a conversation fails silently when the run directory has no Claude Code login, and the two ways to wire account selection into the RunSpec. | 2026-09-12 |
| [Wiring a real agent into the agent pane](./wip/agent-setup.md) | The protocol, the library work and the order of packages behind a real conversation with a composed harness — what has landed, and the honest inventory of what today's library cannot yet deliver. | 2026-09-12 |
| [The conversation vocabulary, the chat surface and the login sandbox](./wip/agent-vocabulary.md) | What landed in the round that made a login reach its harness's runtime, gave a conversation its model, thinking level and mode, turned the IDE chat into editor-like tabs, and gave every conversation a lifecycle — and what of it is verified against a running binary rather than only against tests. | 2026-09-10 |
| [Cloning a project](./wip/clone-a-project.md) | How a repository becomes a project — a connection's listing or a pasted URL, a branch, a destination, and the throwaway clone that is deleted when it closes. The clone half is built and covered by tests; the named OAuth registrations the connect flow picks from are built and never exercised against a live provider, which is the gap this document exists to record. | 2026-09-10 |
| [Grok ACP — captured from the live binary](./wip/grok-acp-capture.md) | What `grok agent stdio` (grok 1.0.13) actually speaks, captured frame by frame, and the gaps between it and `io/acp_client.rs`. | 2026-09-10 |
| [Indexing a project](./wip/indexing.md) | What Ubiq keeps about a project so a search need not re-read it — a per-project level defaulting from an application setting, and a full-text index that selects candidate files for the existing content search rather than answering it. The full-text half is built; the symbol half the `full` level names is not, which is the gap this document exists to record. | 2026-09-10 |
| [Pre-editions refactoring plan](./wip/refactor-plan.md) | Phases 0-3 are done and so are phase 4's composition root and preference round-trip; three phase-4 items remain, each blocked or deferred for a recorded reason, and every `just verify` check passes but docs-lint — whose open question is what that lint should apply to, since most of its failures are inbox documents. | — |
| [Web panels — the origin and the bridge](./wip/web-panel-phase2.md) | Phase 2 of the web-panel proposal as built — the `_web/<app>/<token>/` routes on the interface's existing loopback server, a per-panel token from the platform's CSPRNG, two frame queues per session with a long-poll that answers on its own thread, the two-transport `bridge.js` shim, and a demo tenant that proves the loop. The container is the external browser; the embedded `wry` webview was not built and the shim carries its half anyway. | 2026-09-12 |
| [Web panels — the shared workarea and the bundle fetch](./wip/web-panel-phase3.md) | Phase 3 of the web-panel proposal as built, Rust side only — a `shared_workarea` on `HostInfo` reserved at `<config root>/ui/`, a four-variant web asset message family, and `crates/ubiq-host/src/web_assets/` fetching a manifest's 554 files six at a time, verifying each against its SHA-256, and writing them into one compressed `.bundle` file whose rename over a `.part` sibling is the only proof of done. No interface wiring, which is a later unit. | 2026-09-12 |
| [Web panels — the Excalidraw chrome, and editing](./wip/web-panel-phase45.md) | Phases 4 and 5 of the web-panel proposal as built — a chrome page that fetches the host's generated import map, injects it under a per-response CSP nonce and mounts Excalidraw off the local mirror; an Edit action on the viewer header that is an axis of its own rather than a fourth ViewLayout; and a debounced `Changed` frame written into the existing `EditorState` buffer, so the dirty dot, `⌘S`, save-as and `D37`'s version check keep working with no new save path and no new message. | 2026-09-12 |
| [Web panels — the embedded browser](./wip/web-panel-phase6.md) | Phase 6 of the web-panel proposal as built — the container moves from an external browser into the window through `gpui-wry` on macOS and Windows, `Edit` becomes a fourth `ViewLayout` (reversing phase 5's call), a mark-and-sweep module keeps the child webview clipped to its own dock tab, sessions are settled every render rather than on a click, saving reaches the file through the existing `⌘S` path with a new `Save` bridge frame, and the explorer gains a `New Excalidraw` row. Every platform without `gpui-wry`'s finished Unix path keeps phase 5's external browser. A later addition, draw.io, is the second tenant on this same axis — it offers the same `[Edit, Preview]` layouts, the explorer gains a matching `New draw.io` row, and a new `Preview { svg }` bridge frame gives its Preview position a picture for a format the interface has no native renderer for. | 2026-09-11 |
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
| Pane chrome, design assets | `tech/ui-and-design.md` |
| The window's areas, their sizes, and what owns each | `features/workbench.md` |
| Rail modes, panel visibility, projects, what a window owns | `features/workbench.md` |
| The agents screen: its columns, what a tab drag means, and the bench | `features/workbench.md` |
| The Git screen: its refs, history, change lists and diff | `features/workbench.md` |
| How a repository is read: discovery, the worker, the log and refs, the lane engine | `tech/version-control.md` |
| The orchestration screen: the graph, its selection model, the inspector and the tasks drawer | `features/workbench.md` |
| The tasks board: its columns, its cards, what a drag means and the task panel | `features/workbench.md` |
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
| Adding a screen area, a panel or a rail mode | `features/workbench.md`, then `tech/ui-and-design.md` |
| Changing the Git screen — its refs, history, change lists or diff | `features/workbench.md`, then `tech/ui-and-design.md` |
| Extending version control, or adding the write family | `tech/version-control.md`, then `tech/transport-contract.md` |
| Changing the agents screen — its columns, its sidebar or what a tab drag does | `features/workbench.md`, then `tech/ui-and-design.md` |
| Changing the orchestration screen — its graph, inspector or tasks drawer | `features/workbench.md`, then `tech/ui-and-design.md` |
| Changing the tasks board — its columns, cards or task panel | `features/workbench.md`, then `tech/ui-and-design.md` |
| Changing the window layout, or what a window owns | `features/workbench.md`, then `tech/architecture.md` |
| Changing the chat panel or a message renderer | `features/chat.md` |
| Adding a log event, a subsystem, or changing the log console | `features/logs.md` |
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
