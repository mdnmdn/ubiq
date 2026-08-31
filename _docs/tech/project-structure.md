---
id: tech-structure
title: Project structure
kind: tech
status: current
summary: Every folder in the workspace, what belongs in it, what must never go in it, and the two crates' division of labour.
read_when: you are adding a file and are not certain where it goes, or you are new to the repository
updated: 2026-08-31
verified: 2026-08-31
code_anchors: [Cargo.toml, crates/ubiq/Cargo.toml]
depends_on: [tech-architecture]
review_cycle: quarterly
---

# Project structure

## The workspace

One Cargo workspace, two crates, and everything else is documentation or tooling.

```
ubiq/
├── AGENTS.md            the always-loaded preamble — read first
├── CLAUDE.md            one line: it includes AGENTS.md
├── README.md            the public face of the project
├── Cargo.toml           workspace manifest and the release profile
├── Justfile             every command anyone runs — see operations.md
├── crates/
│   ├── ubiq/            the desktop application (GPUI)
│   └── agent-manager/   the harness-management library and its `am` CLI
├── _docs/               this library
├── _tools/              dev-only scripts, run through `just`
├── refs/                read-only checkouts of other projects, for reference
└── target/              build output
```

Two conventions in that tree are worth stating outright. A leading underscore means **not shipped**:
`_docs/` and `_tools/` exist for the people and agents working on Ubiq and are absent from any
build. And `refs/` is **read-only** — it holds other projects checked out for comparison, and
nothing in it is ever edited, imported, or treated as a claim about this tree.

## The two crates

The division is sharp and worth learning before touching either.

| | `crates/ubiq` | `crates/agent-manager` |
|---|---|---|
| Is | The application | A library, plus a CLI over it |
| Owns | Windows, panes, terminals, layout, focus, process supervision | Composing a harness run: skills, MCP servers, accounts, ephemeral config |
| Knows about | Terminals and the harnesses it hosts | Harnesses and their configuration surfaces |
| Depends on | GPUI, `portable-pty`, `termwiz` | No UI, no terminal emulation |
| Documented in | `_docs/` — this library | `crates/agent-manager/_docs/` — its own |

The dependency runs one way: the application embeds the library. The library has no idea Ubiq
exists. That is what lets the same composition logic serve the terminal and the window, and it is
the reason `agent-manager` builds with `--no-default-features` — the application needs the core, not
the CLI or the TUI. The boundary in detail is in [`agent-manager.md`](./agent-manager.md).

## Inside `crates/ubiq/src`

Module by module, and what must never appear in each. The generated tree, with every file, is in
[`code-map.md`](./code-map.md).

| Path | Holds | Never holds |
|---|---|---|
| `main.rs` | Window creation, theme install, key bindings. Nothing else | Any logic. It stays short enough to read at a glance |
| `lib.rs` | The module list and the crate's public surface | Implementation |
| `app.rs` | `AppState`: the pane map, the focused pane, the layout mode, and the root render | Process handles, PTY handles |
| `ui/` | Pane components, chrome, toolbars, status | Anything that names the coordinator |
| `theme.rs` | The colour palette and its tokens | A literal colour used anywhere else |
| `state/` | Pane and application state machines, event handling | Rendering |
| `messages.rs` | The transport contract enum and its payload records | Anything that fails to serialise |
| `orchestrator.rs` | Spawn, supervise and reap harness processes | Rendering, layout, colour |
| `pty/` | Pseudo-terminal streams, reading, writing, backpressure | Terminal emulation |
| `agent.rs` | Agent-type definitions and the registry over them | Hard-coded harness knowledge that belongs in the library |
| `mcp_server.rs` | The MCP surface Ubiq exposes to the agents it hosts | Anything the hosted agent should not reach |

The "never holds" column is the enforcement of the architecture's rules in file terms. A
`portable-pty` type under `ui/`, or a GPUI type in `messages.rs`, is a violation you can grep for.

## Where a new file goes

1. Is it a colour? → a token in `theme.rs`, referenced everywhere else.
2. Does it draw? → `ui/`.
3. Does it own a process or a file descriptor? → `orchestrator.rs` or `pty/`.
4. Does it cross the bus? → its type goes in `messages.rs`, its handling on both sides.
5. Does it know how a *harness* is configured or launched? → `crates/agent-manager`, not here.
6. Is it a script a person runs? → `_tools/`, with a `just` recipe in front of it.

Anything that fits none of these is worth a question before it is worth a file.

## `_tools/`

Dev-only Python, each script self-contained with inline dependency metadata so `uv run` needs no
environment set up.

| Script | Does |
|---|---|
| `_tools/docs.py` | Lints, indexes and drift-checks this library. Fronted by the `docs-*` recipes |
| `_tools/excalidraw.py` | Converts, validates and renders the diagram format described in [`diagram-format.md`](./diagram-format.md) |

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
