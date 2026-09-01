---
id: index
title: Documentation index
kind: meta
status: current
summary: The map of `_docs/` — how it is organized, the catalogue, which document owns which fact, and which to read for a given task.
read_when: you are starting any task and need to know which two or three documents it needs
updated: 2026-09-01
verified: 2026-09-01
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
| [Glossary](./product/glossary.md) | Plain definitions of the recurring terms — harness, agent type, session, workspace, pane, coordinator, bus, catalog — for anyone reading the rest of this documentation. | 2026-08-31 |
| [Product overview](./product/overview.md) | What Ubiq is, who runs it, why an agent harness needs a real terminal rather than a chat box, and what the product deliberately refuses to be. | 2026-08-31 |

### Features

| Document | What it is | Verified |
|---|---|---|
| [The chat panel](./features/chat.md) | The conversation beside the work — the chat list, the run and context readout, the transcript with its tool blocks and diffs, and the composer that chooses harness, model and mode. | 2026-09-01 |
| [Logs](./features/logs.md) | One sink every subsystem writes its diagnostics to, and the dock tab that reads it back with a subsystem selector and a level floor. | 2026-09-01 |
| [Panes and terminals](./features/panes-and-terminals.md) | What a pane shows, how exactly one of them holds focus, how a resize reaches the harness, and the layout modes panes are arranged in. | 2026-09-01 |
| [Sessions and workspaces](./features/sessions-and-workspaces.md) | A session is a named piece of work that owns a folder and outlives the agents inside it; a workspace is one running agent within it, and the two have separate lifecycles. | 2026-09-01 |
| [The workbench](./features/workbench.md) | The window's shell — the activity rail and its modes, the three panels around the centre, the file explorer and editor a project owns, the agents screen and the tasks board the rail's other built modes hold, the empty state a window with no project shows, and the status bar that reports on all of it. | 2026-09-01 |

### Tech

| Document | What it is | Verified |
|---|---|---|
| [Backlog](./backlog.md) | Every open question, known gap and deferred item across the project, in one register. | 2026-09-01 |
| [The agent-manager boundary](./tech/agent-manager.md) | What the embedded harness-management library owns, what Ubiq owns, how the application consumes it, and the rule that keeps the two from growing into each other. | 2026-08-31 |
| [Architecture](./tech/architecture.md) | The two halves — coordinator and UI — the single bus between them, the rules neither may break, and why the split is drawn before it is needed. | 2026-09-01 |
| [Code map](./tech/code-map.md) | Generated map of the application's source tree, and the inverted index from every file to the documents that anchor it. | 2026-09-01 |
| [Decision register](./tech/decisions.md) | One entry per structural decision — what was chosen, why, and what it costs — cited as `Dnn` across this library. | 2026-09-01 |
| [Diagram format](./tech/diagram-format.md) | The compact YAML authoring format for the wireframes under `_docs/design/`, and the converter that validates and renders it. | 2026-08-31 |
| [Operations](./tech/operations.md) | Prerequisites, the complete command reference, what a first build costs, and the checks a change has to pass before it lands. | 2026-08-31 |
| [Project structure](./tech/project-structure.md) | Every folder in the workspace, what belongs in it, what must never go in it, and the two crates' division of labour. | 2026-09-01 |
| [Transport contract](./tech/transport-contract.md) | The complete message set the UI and the coordinator exchange — the pane, session, project, file and work families, the framing rules, and the procedure for adding a variant. | 2026-09-01 |
| [UI and design](./tech/ui-and-design.md) | The GPUI rendering model, the complete theme token set and the rule that no colour escapes it, how a palette is switched, the shape every surface is drawn in, and the design assets screens are built against. | 2026-09-01 |

### Meta

| Document | What it is | Verified |
|---|---|---|
| [Writing and updating docs](./_meta/authoring.md) | What every agent and human owes this documentation when they change code — and the small set of edits they may make. | 2026-08-31 |
| [Proposal ledger](./_meta/feedback.md) | Append-only ledger of documentation changes the bookkeeper may not make unilaterally, and the resolutions they received. | 2026-08-31 |
| [Librarian rulebook](./_meta/librarian.md) | How `_docs/` is organized, why it is organized that way, and how a bookkeeper agent keeps it that way. | 2026-08-31 |
| [Review log](./_meta/review-log.md) | Append-only record of what each documentation maintenance pass checked, fixed and left alone. | 2026-08-31 |

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
| The agents screen: the graph, its selection model, the inspector and the tasks drawer | `features/workbench.md` |
| The tasks board: its columns, its cards, what a drag means and the task panel | `features/workbench.md` |
| Commands, prerequisites, environment variables | `tech/operations.md` |
| Structural decisions and their cost (`Dnn`) | `tech/decisions.md` |
| The harness library's boundary and the rules across it | `tech/agent-manager.md` |
| Harness config locations, launch flags, catalog, accounts | `crates/agent-manager/_docs/` |
| The diagram authoring format | `tech/diagram-format.md` |
| Session and workspace lifecycle | `features/sessions-and-workspaces.md` |
| Log subsystems, levels, the ring's capacity and the console | `features/logs.md` |
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
| Changing the agents screen — its graph, inspector or tasks drawer | `features/workbench.md`, then `tech/ui-and-design.md` |
| Changing the tasks board — its columns, cards or task panel | `features/workbench.md`, then `tech/ui-and-design.md` |
| Changing the window layout, or what a window owns | `features/workbench.md`, then `tech/architecture.md` |
| Changing the chat panel or a message renderer | `features/chat.md` |
| Adding a log event, a subsystem, or changing the log console | `features/logs.md` |
| Launching a harness, or touching accounts, skills or MCP servers | `tech/agent-manager.md`, then that crate's own `_docs/` |
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
