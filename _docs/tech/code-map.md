---
id: tech-code-map
title: Code map
kind: tech
status: current
summary: Generated map of the application's source tree, and the inverted index from every file to the documents that anchor it.
read_when: you changed a file and need to know which documents owe an update, or you are looking for where something lives
updated: 2026-08-31
verified: 2026-08-31
depends_on: [tech-structure]
review_cycle: monthly
---

# Code map

Two generated blocks: the application's source tree, and the inversion of every document's
`code_anchors` frontmatter. `just docs-index` rewrites both; `just docs-check` fails if they are
stale. The descriptions beside tree entries are written by hand and survive regeneration — add one
by editing the line and running the command again.

The scope is `crates/ubiq/src`. The harness library draws its own tree in
`crates/agent-manager/AGENTS.md`, because it owns its documentation — see
[`agent-manager.md`](./agent-manager.md).

For the rules about what belongs in each of these files, read
[`project-structure.md`](./project-structure.md); this document says only where things are.

## The tree

<!-- generated:begin tree -->

```
crates/ubiq/src/
├── pty/
│   └── mod.rs
├── state/
│   └── mod.rs
├── ui/
│   └── mod.rs
├── agent.rs
├── app.rs
├── lib.rs
├── main.rs
├── mcp_server.rs
├── messages.rs
├── orchestrator.rs
└── theme.rs
```

<!-- generated:end tree -->

<!-- generated:begin anchors -->

## File to document

Every file a document names in its `code_anchors` frontmatter, inverted: change this file, and check
the documents in its row.

| File | Documents |
|---|---|
| `Cargo.toml` | [`project-structure.md`](./project-structure.md) |
| `Justfile` | [`operations.md`](./operations.md) |
| `_tools/docs.py` | [`operations.md`](./operations.md) |
| `_tools/excalidraw.py` | [`diagram-format.md`](./diagram-format.md) |
| `crates/agent-manager/src/lib.rs` | [`agent-manager.md`](./agent-manager.md) |
| `crates/agent-manager/src/spec.rs` | [`agent-manager.md`](./agent-manager.md) |
| `crates/ubiq/Cargo.toml` | [`agent-manager.md`](./agent-manager.md), [`project-structure.md`](./project-structure.md) |
| `crates/ubiq/src/agent.rs` | [`features/sessions-and-workspaces.md`](../features/sessions-and-workspaces.md) |
| `crates/ubiq/src/app.rs` | [`features/panes-and-terminals.md`](../features/panes-and-terminals.md), [`architecture.md`](./architecture.md), [`ui-and-design.md`](./ui-and-design.md) |
| `crates/ubiq/src/lib.rs` | [`architecture.md`](./architecture.md) |
| `crates/ubiq/src/main.rs` | [`architecture.md`](./architecture.md) |
| `crates/ubiq/src/messages.rs` | [`transport-contract.md`](./transport-contract.md) |
| `crates/ubiq/src/orchestrator.rs` | [`features/sessions-and-workspaces.md`](../features/sessions-and-workspaces.md) |
| `crates/ubiq/src/pty/mod.rs` | [`features/panes-and-terminals.md`](../features/panes-and-terminals.md) |
| `crates/ubiq/src/theme.rs` | [`ui-and-design.md`](./ui-and-design.md) |
| `crates/ubiq/src/ui/mod.rs` | [`features/panes-and-terminals.md`](../features/panes-and-terminals.md), [`ui-and-design.md`](./ui-and-design.md) |

## Unanchored

No document's `code_anchors` names these. Restricted to `crates/ubiq/src/`.

| File |
|---|
| `crates/ubiq/src/mcp_server.rs` |
| `crates/ubiq/src/state/mod.rs` |

<!-- generated:end anchors -->

## Related docs

- [`project-structure.md`](./project-structure.md) — what belongs in each folder, and what never does
- [`architecture.md`](./architecture.md) — why the modules divide this way
- `_docs/_meta/authoring.md` — the duty the anchor table exists to serve
