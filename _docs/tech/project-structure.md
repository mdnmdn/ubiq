---
id: tech-structure
title: Project structure
kind: tech
status: current
summary: Every folder in the workspace, what belongs in it, what must never go in it, and the two crates' division of labour.
read_when: you are adding a file and are not certain where it goes, or you are new to the repository
updated: 2026-09-04
verified: 2026-09-04
code_anchors: [Cargo.toml, crates/ubiq/Cargo.toml, crates/ubiq-proto/Cargo.toml, crates/ubiq-host/Cargo.toml, crates/ubiq-app/Cargo.toml, vendor/gpui-terminal/Cargo.toml, _tools/icns.py]
depends_on: [tech-architecture]
review_cycle: quarterly
---

# Project structure

## The workspace

One Cargo workspace, four crates of Ubiq's own, the harness-management library they embed, one
vendored third-party crate, and everything else is documentation or tooling.

```
ubiq/
├── AGENTS.md            the always-loaded preamble — read first
├── CLAUDE.md            one line: it includes AGENTS.md
├── README.md            the public face of the project
├── Cargo.toml           workspace manifest and the release profile
├── Justfile             every command anyone runs — see operations.md
├── crates/
│   ├── ubiq-proto/      the contract, the bus, the log sink
│   ├── ubiq-host/       the headless host: processes, pseudo-terminals, projects, the work
│   ├── ubiq/            the desktop interface (GPUI)
│   ├── ubiq-app/        the binary, the only thing that names both halves
│   └── agent-manager/   the harness-management library and its `am` CLI
├── vendor/
│   └── gpui-terminal/   the terminal emulator component, vendored
├── assets/              the logos the application icon is built from
├── _docs/               this library
├── _tools/              dev-only scripts, run through `just`
├── refs/                read-only checkouts of other projects, for reference
└── target/              build output
```

Two conventions in that tree are worth stating outright. A leading underscore means **not shipped**:
`_docs/` and `_tools/` exist for the people and agents working on Ubiq and are absent from any
build. And `refs/` is **read-only** — it holds other projects checked out for comparison, and
nothing in it is ever edited, imported, or treated as a claim about this tree.

## The config root

The tree above is the source. The other tree Ubiq owns is the **config root** it writes at runtime —
`~/.config/ubiq` unless a flag, `UBIQ_CONFIG_DIR` or a `ubiq.toml` moves it, and this repository's
`ubiq.toml` points it at `_data/config` so a checkout never touches the catalogue a user works with
([`operations.md`](./operations.md) owns the resolution order). Nothing Ubiq remembers goes inside a
project's own folder — `D30`.

```
<config root>/
├── projects.toml            the catalogue: one record per project
├── preferences.toml         the interface's own view blob, opaque to the host
├── ui-settings.toml         Ui-layer settings, opaque to the host
├── host-settings.toml       Host-layer settings, the host parses
└── projects/
    └── <project ulid>/
        ├── tasks.toml       that project's tasks, the user's data
        ├── view.toml        that project's view blob, opaque to the host
        └── ui/              the interface's workarea — the host makes it and never looks in
```

A `projects/<ulid>/` with no record in the catalogue is collected at the next successful load, which
is what makes forgetting a project complete even after a crash halfway through it. The `ui/`
directory goes with it, and that is the only thing the host ever does to it: everything under `ui/`
is the interface's, is disposable, and is reached by the path on `ProjectSnapshot` rather than over
the bus — rule 6 in [`architecture.md`](./architecture.md).

## `vendor/`

Third-party crates the workspace compiles from source because depending on them as published is not
possible. Each is a workspace member and is consumed by path.

`vendor/gpui-terminal/` is the terminal emulator a pane is drawn by: it parses VT with
`alacritty_terminal` and takes any `Read`/`Write` pair, which is what lets a pane be handed a bus
endpoint rather than a pseudo-terminal. It is a copy of an upstream project, not Ubiq's own code,
and the rules that follow from that are the whole point of the folder:

- **Keep it close to upstream.** A change here is either a rebase onto a newer upstream revision, or
  a minimal patch that upstream's version cannot carry — the crate is vendored because upstream
  builds against the `gpui` published on crates.io while Ubiq builds against Zed's `main`.
- **Record every divergence.** `vendor/gpui-terminal/README.md` names the upstream revision, the
  licence, and each file that differs and why. A rebase reapplies exactly that list, so a patch
  missing from it is a patch the next rebase drops.
- **Never Ubiq's own code.** No `AppState`, no theme token, no message type. Anything Ubiq wants
  from the emulator is passed in as configuration or a callback; anything Ubiq wants to add on top
  belongs in `crates/ubiq/src/ui/`.

## The two crates

The division is sharp and worth learning before touching either.

| | `crates/ubiq` | `crates/agent-manager` |
|---|---|---|
| Is | The application | A library, plus a CLI over it |
| Owns | Windows, panes, terminals, layout, focus, process supervision | Composing a harness run: skills, MCP servers, accounts, ephemeral config |
| Knows about | Terminals and the harnesses it hosts | Harnesses and their configuration surfaces |
| Depends on | GPUI, `gpui-terminal`, `portable-pty` | No UI, no terminal emulation |
| Documented in | `_docs/` — this library | `crates/agent-manager/_docs/` — its own |

The dependency runs one way: the application embeds the library. The library has no idea Ubiq
exists. That is what lets the same composition logic serve the terminal and the window, and it is
the reason `agent-manager` builds with `--no-default-features` — the application needs the core, not
the CLI or the TUI. The boundary in detail is in [`agent-manager.md`](./agent-manager.md).

## Inside Ubiq's four crates

Module by module, and what must never appear in each. The generated tree, with every file, is in
[`code-map.md`](./code-map.md). Which crate a module sits in is itself the first rule: the
interface does not depend on the host, so a module in the wrong crate does not compile.

| Path | Holds | Never holds |
|---|---|---|
| `ubiq-proto/src/messages.rs` | The transport contract enum and its payload records | Anything that fails to serialise |
| `ubiq-proto/src/settings.rs` | Which half owns a settings blob, and the host's own record | A Ui-layer field |
| `ubiq-proto/src/ids.rs` | The contract's id newtypes, and the one generator behind them | A second id scheme |
| `ubiq-proto/src/bus.rs` | The hub, a client's end of it, and a pane's `Read`/`Write` byte-stream ends | A pane's contents, a descriptor, or any knowledge of what the bytes mean |
| `ubiq-proto/src/log.rs` | The process-wide sink every subsystem writes to | Anything either half has to be handed |
| `ubiq-proto/src/git.rs` | A project's repository as it crosses the bus: overview, working-tree map, errors | A `git2` type, a path on disk |
| `ubiq-host/src/coordinator.rs` | Spawn, supervise and reap harness processes; answer the bus | Rendering, layout, colour |
| `ubiq-host/src/git/` | A project's repository, observed off the coordinator's thread | A write into the repository, including the index stat cache |
| `ubiq-host/src/pty/` | Pseudo-terminal streams, reading, writing, backpressure | Terminal emulation |
| `ubiq-host/src/config.rs` | Where the config root is, and how it is found | A setting; the bootstrap file names a directory and nothing else |
| `ubiq-host/src/store/` | The catalogue, a project's tasks, the view state and settings, behind four traits | Any opinion about what a Ui-layer blob means |
| `ubiq-host/src/projects.rs` | The catalogue as the host runs it, and the reservation of each project's `ui/` workarea | An opinion about colour or layout, or a read of anything inside a workarea |
| `ubiq-host/src/settings.rs` | Application settings as the host runs them: Ui opaque, Host parsed | An opinion about what a Ui-layer blob means |
| `ubiq-host/src/work/` | A project's tasks as the host keeps them, and the sessions and agents it mocks over them | Where anything is drawn, or an invented reply from an agent |
| `ubiq-host/src/watch/` | One `notify` watch per open project, debounced and coalesced, and the project-relative paths it reports | An absolute path on the wire, an opinion about what a reader should redraw |
| `ubiq-host/src/agent.rs` | Agent-type definitions and the registry over them | Hard-coded harness knowledge that belongs in the library |
| `ubiq-host/src/mcp_server.rs` | The MCP surface Ubiq exposes to the agents it hosts | Anything the hosted agent should not reach |
| `ubiq/src/app/` | `AppState`: the panes, the focused pane, the dock and its panels, the workbench state, and window creation. `mod.rs` holds the struct, the free window functions and the key bindings; `boot.rs` the constructor; `shell.rs` chrome and the `Render` impl; `wire.rs` `receive()` and the pane calls; `panels.rs` the dock; and one file per screen — `explorer`, `editor`, `git`, `agents`, `graph`, `board`, `chat`, `sink`, `picker`, `projects`, `settings` | Process handles, PTY handles, disk |
| `ubiq/src/web_export/` | The on-demand local HTTP server that serves a project's own files read-only, for browsing in a web browser — its own project-root reads, its own `tiny_http` thread, no bus traffic | A proto message, a call into `ubiq-host` |
| `ubiq/src/ui/` | One module per screen area: shell, titlebar, project menu, rail, explorer, editor, terminal, logs, status bar, empty page, settings overlay, `chat/`, `agents/`, `orchestration/`, `board/` | Anything that names the host |
| `ubiq/src/ui/agents/` | The Agents screen: the sidebar of every agent the host reports, and one column per conversation — its tabs, its thread and its composer | Anything that ends an agent; a close that means more than benching one |
| `ubiq/src/ui/orchestration/` | The Orchestration screen: the graph of who spawned whom, its inspector and its tasks drawer | A position a record would have to carry |
| `ubiq/src/ui/work.rs` | What a work record reads as: the token an activity, a bucket or a role takes | A second mapping for a state one screen wants to draw differently |
| `ubiq/src/ui/dock/` | The window's arrangement: the panel adapter over those areas, and Ubiq's skin over the component library's dock | State of its own — a panel holds what identifies it and reads the rest |
| `ubiq/src/ui/kit/` | Reusable primitives, and only what the component library lacks | Application state, sample data, or the name `AppState` |
| `ubiq/src/theme.rs` | The colour palette and its tokens | A literal colour used anywhere else |
| `ubiq/src/state/` | Pane and application state machines, the workbench, explorer, editor and chat state, what a panel is and where it may sit, the projections of the host's catalogue and of a project's work, the three views over that work — `agents.rs` for the columns, `orchestration.rs` for the graph, `board.rs` for the board — and the fixture that still seeds the chat | Rendering, or any component-library type |
| `ubiq-app/src/lib.rs` | The boot as a library: `Stores` (the four boxed store traits), `Boot` (what an edition composes — the stores it hands in) and `run(boot)`, the whole start sequence: config root, host, theme, key bindings, the first window | Any logic, including window construction — that is `app::open_project_window` |
| `ubiq-app/src/main.rs` | Three lines: `run(Boot::default())`. The base edition's binary, entire | A step of the boot — a second binary composing these crates must not be able to skip one |

The "never holds" column is the enforcement of the architecture's rules in file terms. A
`portable-pty` type under `ui/`, or a GPUI type in `messages.rs`, is a violation you can grep for —
and the crate split makes most of them a violation you cannot compile.

## Where a new file goes

1. Is it a colour, a font, or a size the layout depends on? → `theme.rs`, referenced everywhere else.
2. Does it draw, and does it know about the workbench? → a module under `ui/`.
3. Does it draw, and would a second caller want it? → `ui/kit/`, naming no application type.
4. Does it own a process, a file descriptor or a path on disk? → `crates/ubiq-host/`.
5. Does it cross the bus? → its type goes in `crates/ubiq-proto/`, its handling on both sides.
6. Does it know how a *harness* is configured or launched? → `crates/agent-manager`, not here.
7. Is it a patch to somebody else's crate? → `vendor/`, with a line in that crate's `README.md`.
8. Is it a script a person runs? → `_tools/`, with a `just` recipe in front of it.

Anything that fits none of these is worth a question before it is worth a file.

## `_tools/`

Dev-only Python, each script self-contained with inline dependency metadata so `uv run` needs no
environment set up.

| Script | Does |
|---|---|
| `_tools/docs.py` | Lints, indexes and drift-checks this library. Fronted by the `docs-*` recipes |
| `_tools/excalidraw.py` | Converts, validates and renders the diagram format described in [`diagram-format.md`](./diagram-format.md) |
| `_tools/icns.py` | Builds the macOS application icon from `assets/`. Fronted by `just icns`, consumed by `just bundle` |

Nothing in `_tools/` is imported by the crates, and nothing in the crates is imported by it.

## `_docs/` and `refs/`

`_docs/` is described by its own map, [`../INDEX.md`](../INDEX.md), and governed by
`_docs/_meta/librarian.md`. Its one structural oddity: `_docs/design/` holds wireframes, prototypes
and captured artifacts rather than documents, and is excluded from documentation checks.

`refs/` holds other repositories checked out for reference. Its contents are never edited and never
cited as if they were part of this tree — the documentation linter treats a path under `refs/` as
external precisely so nobody mistakes one for a claim about Ubiq.

## Rationale

**Why a workspace rather than one crate?** Because `agent-manager` has a life outside Ubiq: it ships
its own `am` CLI and is meant to be embedded by other tools. Folding it into the application would
make every harness-configuration fix an application release, and would let application types leak
into a library that must stay front-end agnostic.

**Why does the library keep its own `_docs/`?** Documentation lives with the thing it documents, so
that a crate extracted from this workspace leaves with its own manual. The cost is one boundary to
state — which is [`agent-manager.md`](./agent-manager.md), and it is stated once.

## Related docs

- [`architecture.md`](./architecture.md) — why the modules divide this way
- [`code-map.md`](./code-map.md) — the generated file-by-file tree
- [`operations.md`](./operations.md) — the commands that build and run all of this
- [`agent-manager.md`](./agent-manager.md) — the embedded library's boundary
