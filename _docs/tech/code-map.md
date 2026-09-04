---
id: tech-code-map
title: Code map
kind: tech
status: current
summary: Generated map of the application's source tree, and the inverted index from every file to the documents that anchor it.
read_when: you changed a file and need to know which documents owe an update, or you are looking for where something lives
updated: 2026-09-04
verified: 2026-09-04
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
crates/ubiq-proto/src/
├── bus.rs
├── ids.rs
├── lib.rs
├── log.rs
├── messages.rs
├── projects.rs
├── files.rs
├── work.rs
├── git.rs
├── settings.rs
├── conversation.rs
└── search.rs

crates/ubiq-host/src/
├── pty/
│   └── mod.rs
├── agent.rs
├── coordinator.rs
├── lib.rs
├── mcp_server.rs
├── store/
│   ├── file.rs
│   ├── memory.rs
│   └── mod.rs
├── atomic.rs
├── config.rs
├── gc.rs
├── health.rs
├── projects.rs
├── files/
│   ├── mod.rs
│   ├── path.rs
│   └── diff.rs
├── work/
│   ├── mock.rs
│   └── mod.rs
├── reply.rs
├── git/
│   ├── mod.rs
│   └── observe.rs
├── settings.rs
├── shells.rs
├── conversation.rs
├── search/
│   ├── ceiling.rs
│   ├── fallback.rs
│   ├── mod.rs
│   ├── walk.rs
│   └── worker.rs
└── watch/
    └── mod.rs

crates/ubiq/src/
├── state/
│   ├── mod.rs
│   ├── chat.rs
│   ├── editor.rs
│   ├── sample.rs
│   ├── workbench.rs
│   ├── windows.rs
│   ├── logs.rs
│   ├── prefs.rs
│   ├── when.rs
│   ├── agents.rs
│   ├── layout.rs
│   ├── board.rs
│   ├── work.rs
│   ├── dock.rs
│   ├── scene.rs
│   ├── diagrams.rs
│   ├── sink.rs
│   ├── viewport.rs
│   ├── file_picker.rs
│   ├── orchestration.rs
│   ├── git.rs
│   ├── settings.rs
│   ├── conversation.rs
│   ├── search.rs
│   └── explorer/
│       ├── filter.rs  the go-to-file substring filter
│       ├── keys.rs    keyboard navigation
│       ├── menu.rs    the row context menu
│       ├── mod.rs     `ExplorerState`, `FileNode`, `Row`, `GitStatus`
│       ├── rows.rs    flattening the tree into drawable rows
│       └── tree.rs    listing, `merge`, expansion
├── ui/
│   ├── mod.rs
│   ├── chat/
│   │   ├── composer.rs
│   │   ├── mod.rs
│   │   ├── sidebar.rs
│   │   └── transcript.rs
│   ├── kit/
│   │   ├── controls.rs
│   │   ├── menu.rs
│   │   ├── mod.rs
│   │   ├── panel.rs
│   │   ├── canvas.rs
│   │   ├── overlay.rs
│   │   ├── files.rs
│   │   └── settings.rs
│   ├── editor.rs
│   ├── empty.rs
│   ├── explorer.rs
│   ├── rail.rs
│   ├── shell.rs
│   ├── status_bar.rs
│   ├── terminal.rs
│   ├── titlebar.rs
│   ├── project_menu.rs
│   ├── logs.rs
│   ├── agents/
│   │   ├── mod.rs
│   │   ├── column.rs
│   │   └── sidebar.rs
│   ├── board/
│   │   ├── detail.rs
│   │   ├── mod.rs
│   │   └── form.rs
│   ├── dock/
│   │   ├── mod.rs
│   │   └── skin.rs
│   ├── viewer/
│   │   ├── diagram.rs
│   │   ├── diff.rs
│   │   ├── image.rs
│   │   ├── markdown.rs
│   │   ├── mod.rs
│   │   ├── scene.rs
│   │   └── viewport.rs
│   ├── sink/
│   │   ├── docs.rs
│   │   ├── mod.rs
│   │   ├── style.rs
│   │   ├── files.rs
│   │   ├── project.rs
│   │   └── settings.rs
│   ├── file_picker.rs
│   ├── file_tab_menu.rs
│   ├── orchestration/
│   │   ├── graph.rs
│   │   ├── inspector.rs
│   │   ├── mod.rs
│   │   └── tasks.rs
│   ├── git/
│   │   ├── changes.rs
│   │   ├── diff.rs
│   │   ├── history.rs
│   │   ├── mod.rs
│   │   └── refs.rs
│   ├── settings.rs
│   ├── work.rs
│   ├── new_pane_menu.rs
│   ├── conversation/
│   │   └── mod.rs
│   ├── languages/
│   │   ├── csharp/
│   │   │   └── highlights.scm
│   │   └── swift/
│   │       └── highlights.scm
│   └── search.rs
├── lib.rs
├── theme.rs
├── web_export/
│   ├── assets.rs
│   ├── mod.rs
│   ├── routes.rs
│   └── server.rs
├── app/
│   ├── agents.rs      agents, conversations and the new-agent menu
│   ├── board.rs       the task board and the jumps out of it
│   ├── boot.rs        `for_project` — the constructor that opens the bus and builds a window
│   ├── chat.rs        chat, diagrams and viewports
│   ├── editor.rs      open files, tabs and saving
│   ├── explorer.rs    explorer rows, menus and file opening
│   ├── git.rs         the git view and project search
│   ├── graph.rs       the orchestration graph and inspector
│   ├── mod.rs         `AppState`, `OpenProject`, `PaneState`, `BusHub`, the free window functions and the key bindings
│   ├── panels.rs      the dock: settling mode, layout, visibility and placement
│   ├── picker.rs      the file picker and the log panel
│   ├── projects.rs    add, edit and close a project; preference persistence
│   ├── settings.rs    the settings overlay and harness login
│   ├── shell.rs       project lifecycle, chrome and menus, rail mode, font and zoom, the `Render` impl
│   ├── sink.rs        the kitchen sink's setters
│   └── wire.rs        `receive()`, the pane and terminal calls, focus drains
└── version.rs

crates/ubiq-app/src/
├── main.rs            three lines: `run(Boot::default())`
└── lib.rs             `Stores`, `Boot` and `run(boot)` — the whole boot sequence, and the only crate that names both halves
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
| `_devops/scripts/bundle-version.sh` | [`operations.md`](./operations.md) |
| `_tools/Info.plist` | [`operations.md`](./operations.md) |
| `_tools/docs.py` | [`operations.md`](./operations.md) |
| `_tools/excalidraw.py` | [`diagram-format.md`](./diagram-format.md) |
| `_tools/icns.py` | [`operations.md`](./operations.md), [`project-structure.md`](./project-structure.md) |
| `crates/agent-manager/src/io/jsonl.rs` | [`wip/agent-setup.md`](../wip/agent-setup.md) |
| `crates/agent-manager/src/io/mod.rs` | [`agent-manager.md`](./agent-manager.md) |
| `crates/agent-manager/src/io/model.rs` | [`wip/agent-setup.md`](../wip/agent-setup.md) |
| `crates/agent-manager/src/isolate.rs` | [`agent-manager.md`](./agent-manager.md), [`wip/agent-setup.md`](../wip/agent-setup.md) |
| `crates/agent-manager/src/lib.rs` | [`agent-manager.md`](./agent-manager.md) |
| `crates/agent-manager/src/profile.rs` | [`agent-manager.md`](./agent-manager.md), [`wip/agent-setup.md`](../wip/agent-setup.md) |
| `crates/agent-manager/src/resolve.rs` | [`agent-manager.md`](./agent-manager.md), [`wip/agent-setup.md`](../wip/agent-setup.md) |
| `crates/agent-manager/src/spec.rs` | [`agent-manager.md`](./agent-manager.md) |
| `crates/ubiq-app/Cargo.toml` | [`project-structure.md`](./project-structure.md) |
| `crates/ubiq-app/src/lib.rs` | [`features/logs.md`](../features/logs.md), [`architecture.md`](./architecture.md) |
| `crates/ubiq-app/src/main.rs` | [`architecture.md`](./architecture.md) |
| `crates/ubiq-host/Cargo.toml` | [`agent-manager.md`](./agent-manager.md), [`project-structure.md`](./project-structure.md) |
| `crates/ubiq-host/src/agent.rs` | [`features/sessions-and-workspaces.md`](../features/sessions-and-workspaces.md), [`agent-manager.md`](./agent-manager.md), [`wip/agent-setup.md`](../wip/agent-setup.md) |
| `crates/ubiq-host/src/conversation.rs` | [`agent-manager.md`](./agent-manager.md) |
| `crates/ubiq-host/src/coordinator.rs` | [`features/panes-and-terminals.md`](../features/panes-and-terminals.md), [`features/sessions-and-workspaces.md`](../features/sessions-and-workspaces.md), [`architecture.md`](./architecture.md), [`wip/agent-setup.md`](../wip/agent-setup.md) |
| `crates/ubiq-host/src/files/diff.rs` | [`architecture.md`](./architecture.md) |
| `crates/ubiq-host/src/files/mod.rs` | [`architecture.md`](./architecture.md) |
| `crates/ubiq-host/src/git/mod.rs` | [`architecture.md`](./architecture.md) |
| `crates/ubiq-host/src/git/observe.rs` | [`architecture.md`](./architecture.md) |
| `crates/ubiq-host/src/lib.rs` | [`architecture.md`](./architecture.md) |
| `crates/ubiq-host/src/projects.rs` | [`features/workbench.md`](../features/workbench.md), [`architecture.md`](./architecture.md) |
| `crates/ubiq-host/src/pty/mod.rs` | [`features/panes-and-terminals.md`](../features/panes-and-terminals.md) |
| `crates/ubiq-host/src/settings.rs` | [`architecture.md`](./architecture.md) |
| `crates/ubiq-host/src/shells.rs` | [`features/panes-and-terminals.md`](../features/panes-and-terminals.md) |
| `crates/ubiq-host/src/store/file.rs` | [`architecture.md`](./architecture.md) |
| `crates/ubiq-host/src/store/memory.rs` | [`architecture.md`](./architecture.md) |
| `crates/ubiq-host/src/store/mod.rs` | [`architecture.md`](./architecture.md) |
| `crates/ubiq-host/src/watch/mod.rs` | [`architecture.md`](./architecture.md) |
| `crates/ubiq-host/src/work/mod.rs` | [`architecture.md`](./architecture.md) |
| `crates/ubiq-proto/Cargo.toml` | [`project-structure.md`](./project-structure.md) |
| `crates/ubiq-proto/src/bus.rs` | [`features/panes-and-terminals.md`](../features/panes-and-terminals.md), [`architecture.md`](./architecture.md) |
| `crates/ubiq-proto/src/conversation.rs` | [`transport-contract.md`](./transport-contract.md) |
| `crates/ubiq-proto/src/files.rs` | [`transport-contract.md`](./transport-contract.md) |
| `crates/ubiq-proto/src/git.rs` | [`transport-contract.md`](./transport-contract.md) |
| `crates/ubiq-proto/src/ids.rs` | [`transport-contract.md`](./transport-contract.md) |
| `crates/ubiq-proto/src/lib.rs` | [`architecture.md`](./architecture.md) |
| `crates/ubiq-proto/src/log.rs` | [`features/logs.md`](../features/logs.md), [`architecture.md`](./architecture.md) |
| `crates/ubiq-proto/src/messages.rs` | [`transport-contract.md`](./transport-contract.md) |
| `crates/ubiq-proto/src/projects.rs` | [`transport-contract.md`](./transport-contract.md) |
| `crates/ubiq-proto/src/settings.rs` | [`transport-contract.md`](./transport-contract.md) |
| `crates/ubiq-proto/src/work.rs` | [`transport-contract.md`](./transport-contract.md), [`wip/agent-setup.md`](../wip/agent-setup.md) |
| `crates/ubiq/Cargo.toml` | [`project-structure.md`](./project-structure.md) |
| `crates/ubiq/src/app/boot.rs` | [`architecture.md`](./architecture.md) |
| `crates/ubiq/src/app/mod.rs` | [`features/panes-and-terminals.md`](../features/panes-and-terminals.md), [`features/workbench.md`](../features/workbench.md), [`architecture.md`](./architecture.md), [`ui-and-design.md`](./ui-and-design.md) |
| `crates/ubiq/src/app/panels.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/src/app/shell.rs` | [`features/workbench.md`](../features/workbench.md), [`ui-and-design.md`](./ui-and-design.md) |
| `crates/ubiq/src/app/wire.rs` | [`features/panes-and-terminals.md`](../features/panes-and-terminals.md), [`architecture.md`](./architecture.md) |
| `crates/ubiq/src/lib.rs` | [`architecture.md`](./architecture.md) |
| `crates/ubiq/src/state/agents.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/src/state/board.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/src/state/chat.rs` | [`features/chat.md`](../features/chat.md) |
| `crates/ubiq/src/state/conversation.rs` | [`features/workbench.md`](../features/workbench.md), [`wip/agent-setup.md`](../wip/agent-setup.md) |
| `crates/ubiq/src/state/diagrams.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/src/state/dock.rs` | [`features/logs.md`](../features/logs.md), [`features/panes-and-terminals.md`](../features/panes-and-terminals.md), [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/src/state/editor.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/src/state/explorer/mod.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/src/state/explorer/rows.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/src/state/explorer/tree.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/src/state/file_picker.rs` | [`features/workbench.md`](../features/workbench.md), [`ui-and-design.md`](./ui-and-design.md) |
| `crates/ubiq/src/state/git.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/src/state/layout.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/src/state/logs.rs` | [`features/logs.md`](../features/logs.md) |
| `crates/ubiq/src/state/mod.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/src/state/orchestration.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/src/state/prefs.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/src/state/sample.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/src/state/scene.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/src/state/settings.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/src/state/sink.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/src/state/viewport.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/src/state/when.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/src/state/windows.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/src/state/work.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/src/state/workbench.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/src/theme.rs` | [`ui-and-design.md`](./ui-and-design.md) |
| `crates/ubiq/src/ui/agents/column.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/src/ui/agents/mod.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/src/ui/agents/sidebar.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/src/ui/board/detail.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/src/ui/board/form.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/src/ui/board/mod.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/src/ui/chat/composer.rs` | [`features/chat.md`](../features/chat.md) |
| `crates/ubiq/src/ui/chat/mod.rs` | [`features/chat.md`](../features/chat.md) |
| `crates/ubiq/src/ui/chat/sidebar.rs` | [`features/chat.md`](../features/chat.md) |
| `crates/ubiq/src/ui/chat/transcript.rs` | [`features/chat.md`](../features/chat.md) |
| `crates/ubiq/src/ui/conversation/mod.rs` | [`features/workbench.md`](../features/workbench.md), [`wip/agent-setup.md`](../wip/agent-setup.md) |
| `crates/ubiq/src/ui/dock/mod.rs` | [`features/panes-and-terminals.md`](../features/panes-and-terminals.md), [`features/workbench.md`](../features/workbench.md), [`ui-and-design.md`](./ui-and-design.md) |
| `crates/ubiq/src/ui/dock/skin.rs` | [`features/panes-and-terminals.md`](../features/panes-and-terminals.md), [`features/workbench.md`](../features/workbench.md), [`ui-and-design.md`](./ui-and-design.md) |
| `crates/ubiq/src/ui/editor.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/src/ui/empty.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/src/ui/explorer.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/src/ui/file_picker.rs` | [`features/workbench.md`](../features/workbench.md), [`ui-and-design.md`](./ui-and-design.md) |
| `crates/ubiq/src/ui/git/changes.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/src/ui/git/diff.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/src/ui/git/history.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/src/ui/git/mod.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/src/ui/git/refs.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/src/ui/kit/canvas.rs` | [`ui-and-design.md`](./ui-and-design.md) |
| `crates/ubiq/src/ui/kit/controls.rs` | [`ui-and-design.md`](./ui-and-design.md) |
| `crates/ubiq/src/ui/kit/files.rs` | [`features/workbench.md`](../features/workbench.md), [`ui-and-design.md`](./ui-and-design.md) |
| `crates/ubiq/src/ui/kit/menu.rs` | [`ui-and-design.md`](./ui-and-design.md) |
| `crates/ubiq/src/ui/kit/mod.rs` | [`ui-and-design.md`](./ui-and-design.md) |
| `crates/ubiq/src/ui/kit/overlay.rs` | [`ui-and-design.md`](./ui-and-design.md) |
| `crates/ubiq/src/ui/kit/settings.rs` | [`features/workbench.md`](../features/workbench.md), [`ui-and-design.md`](./ui-and-design.md) |
| `crates/ubiq/src/ui/logs.rs` | [`features/logs.md`](../features/logs.md), [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/src/ui/mod.rs` | [`ui-and-design.md`](./ui-and-design.md) |
| `crates/ubiq/src/ui/new_pane_menu.rs` | [`features/panes-and-terminals.md`](../features/panes-and-terminals.md) |
| `crates/ubiq/src/ui/orchestration/graph.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/src/ui/orchestration/inspector.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/src/ui/orchestration/mod.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/src/ui/orchestration/tasks.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/src/ui/project_menu.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/src/ui/rail.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/src/ui/settings.rs` | [`features/workbench.md`](../features/workbench.md), [`ui-and-design.md`](./ui-and-design.md) |
| `crates/ubiq/src/ui/shell.rs` | [`features/workbench.md`](../features/workbench.md), [`ui-and-design.md`](./ui-and-design.md) |
| `crates/ubiq/src/ui/sink/docs.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/src/ui/sink/files.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/src/ui/sink/mod.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/src/ui/sink/project.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/src/ui/sink/settings.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/src/ui/sink/style.rs` | [`features/workbench.md`](../features/workbench.md), [`ui-and-design.md`](./ui-and-design.md) |
| `crates/ubiq/src/ui/status_bar.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/src/ui/terminal.rs` | [`features/panes-and-terminals.md`](../features/panes-and-terminals.md), [`features/workbench.md`](../features/workbench.md), [`ui-and-design.md`](./ui-and-design.md) |
| `crates/ubiq/src/ui/titlebar.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/src/ui/viewer/diagram.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/src/ui/viewer/diff.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/src/ui/viewer/image.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/src/ui/viewer/markdown.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/src/ui/viewer/mod.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/src/ui/viewer/scene.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/src/ui/viewer/viewport.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/src/ui/work.rs` | [`features/workbench.md`](../features/workbench.md), [`ui-and-design.md`](./ui-and-design.md) |
| `crates/ubiq/src/version.rs` | [`architecture.md`](./architecture.md) |
| `crates/ubiq/src/web_export/mod.rs` | [`architecture.md`](./architecture.md) |
| `crates/ubiq/tests/agents.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/tests/board.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/tests/conversation.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/tests/diagrams.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/tests/dock.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/tests/explorer.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/tests/file_picker.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/tests/files_changed.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/tests/git.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/tests/mode_restore.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/tests/orchestration.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/tests/scene.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/tests/settings.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/tests/sink.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/tests/viewport.rs` | [`features/workbench.md`](../features/workbench.md) |
| `vendor/gpui-terminal/Cargo.toml` | [`project-structure.md`](./project-structure.md) |
| `vendor/gpui-terminal/src/clipboard.rs` | [`features/panes-and-terminals.md`](../features/panes-and-terminals.md) |
| `vendor/gpui-terminal/src/input.rs` | [`features/panes-and-terminals.md`](../features/panes-and-terminals.md) |
| `vendor/gpui-terminal/src/mouse.rs` | [`features/panes-and-terminals.md`](../features/panes-and-terminals.md) |
| `vendor/gpui-terminal/src/render.rs` | [`features/panes-and-terminals.md`](../features/panes-and-terminals.md) |
| `vendor/gpui-terminal/src/view.rs` | [`features/panes-and-terminals.md`](../features/panes-and-terminals.md) |

## Unanchored

No document's `code_anchors` names these. Restricted to Ubiq's own crates.

| File |
|---|
| `crates/ubiq-host/src/atomic.rs` |
| `crates/ubiq-host/src/config.rs` |
| `crates/ubiq-host/src/files/path.rs` |
| `crates/ubiq-host/src/gc.rs` |
| `crates/ubiq-host/src/health.rs` |
| `crates/ubiq-host/src/mcp_server.rs` |
| `crates/ubiq-host/src/reply.rs` |
| `crates/ubiq-host/src/search/ceiling.rs` |
| `crates/ubiq-host/src/search/fallback.rs` |
| `crates/ubiq-host/src/search/mod.rs` |
| `crates/ubiq-host/src/search/walk.rs` |
| `crates/ubiq-host/src/search/worker.rs` |
| `crates/ubiq-host/src/work/mock.rs` |
| `crates/ubiq-proto/src/search.rs` |
| `crates/ubiq/src/app/agents.rs` |
| `crates/ubiq/src/app/board.rs` |
| `crates/ubiq/src/app/chat.rs` |
| `crates/ubiq/src/app/editor.rs` |
| `crates/ubiq/src/app/explorer.rs` |
| `crates/ubiq/src/app/git.rs` |
| `crates/ubiq/src/app/graph.rs` |
| `crates/ubiq/src/app/picker.rs` |
| `crates/ubiq/src/app/projects.rs` |
| `crates/ubiq/src/app/settings.rs` |
| `crates/ubiq/src/app/sink.rs` |
| `crates/ubiq/src/state/explorer/filter.rs` |
| `crates/ubiq/src/state/explorer/keys.rs` |
| `crates/ubiq/src/state/explorer/menu.rs` |
| `crates/ubiq/src/state/search.rs` |
| `crates/ubiq/src/ui/file_tab_menu.rs` |
| `crates/ubiq/src/ui/kit/panel.rs` |
| `crates/ubiq/src/ui/search.rs` |
| `crates/ubiq/src/web_export/assets.rs` |
| `crates/ubiq/src/web_export/routes.rs` |
| `crates/ubiq/src/web_export/server.rs` |

<!-- generated:end anchors -->

## Related docs

- [`project-structure.md`](./project-structure.md) — what belongs in each folder, and what never does
- [`architecture.md`](./architecture.md) — why the modules divide this way
- `_docs/_meta/authoring.md` — the duty the anchor table exists to serve
