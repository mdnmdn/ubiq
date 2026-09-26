---
id: tech-code-map
title: Code map
kind: tech
status: current
summary: Generated map of the application's source tree, and the inverted index from every file to the documents that anchor it.
read_when: you changed a file and need to know which documents owe an update, or you are looking for where something lives
updated: 2026-09-26
verified: 2026-09-23
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
├── search.rs
├── connectors.rs
├── repos.rs
├── stats.rs
├── wire.rs
├── assist.rs
├── notifications.rs
├── tools.rs
├── mcp.rs
├── quota.rs
├── carrier.rs
├── drone/
│   ├── manifest.rs
│   └── mod.rs
├── kb.rs
├── acp.rs
├── feedback.rs
├── help.rs
├── ask.rs
├── plan.rs
├── blocks.rs
├── mission.rs
└── tasksrc.rs

crates/ubiq-host/src/
├── pty/
│   └── mod.rs
├── agent.rs
├── coordinator.rs
├── lib.rs
├── store/
│   ├── file.rs
│   ├── memory.rs
│   ├── mod.rs
│   ├── harness.rs
│   ├── usage.rs
│   ├── plan.rs
│   ├── mission.rs
│   └── project_dir.rs
├── atomic.rs
├── config.rs
├── gc.rs
├── health.rs
├── projects.rs
├── files/
│   ├── mod.rs
│   ├── path.rs
│   ├── diff.rs
│   ├── browse.rs
│   └── related.rs
├── work/
│   ├── mock.rs
│   └── mod.rs
├── reply.rs
├── git/
│   ├── mod.rs
│   ├── graph.rs
│   ├── history.rs
│   ├── observe.rs
│   ├── nested.rs
│   └── write.rs
├── settings.rs
├── shells.rs
├── conversation.rs
├── search/
│   ├── ceiling.rs
│   ├── fallback.rs
│   ├── mod.rs
│   ├── walk.rs
│   ├── worker.rs
│   └── hits.rs
├── watch/
│   └── mod.rs
├── cli_shortcut.rs
├── links.rs
├── connectors/
│   ├── app.rs
│   ├── flow.rs
│   ├── http.rs
│   ├── mod.rs
│   ├── providers.rs
│   ├── store.rs
│   └── tls.rs
├── index/
│   ├── ceiling.rs
│   ├── mod.rs
│   └── text.rs
├── repos/
│   ├── clone.rs
│   ├── list.rs
│   └── mod.rs
├── remote.rs
├── assist/
│   ├── apple.rs
│   ├── mod.rs
│   ├── stub.rs
│   ├── subject.rs
│   ├── api.rs
│   └── providers.rs
├── notifications/
│   ├── mod.rs
│   └── os.rs
├── host_path.rs
├── environment.rs
├── conversation_record.rs
├── host_meta.rs
├── mcp/
│   ├── catalogue.rs
│   ├── mod.rs
│   ├── registry.rs
│   ├── server.rs
│   ├── tools.rs
│   ├── tasks.rs
│   ├── kb.rs
│   ├── help.rs
│   ├── ask.rs
│   ├── plan.rs
│   └── mission.rs
├── web_assets/
│   ├── manifest.rs
│   ├── mod.rs
│   ├── archive.rs
│   └── manifest_drawio.rs
├── quota.rs
├── carrier.rs
├── .DS_Store
├── drone/
│   └── mod.rs
├── kb/
│   ├── mod.rs
│   ├── store.rs
│   ├── sync.rs
│   └── ops.rs
├── feedback/
│   ├── api.rs
│   ├── issues.rs
│   └── mod.rs
├── help/
│   ├── archive.rs
│   └── mod.rs
├── ask.rs
├── plan/
│   ├── mod.rs
│   ├── blocks.rs
│   ├── lines.rs
│   ├── provenance.rs
│   └── service.rs
├── mission/
│   ├── mod.rs
│   └── scheduler.rs
├── armed.rs
└── tasksrc/
    ├── mod.rs
    ├── store.rs
    ├── sync.rs
    ├── trello.rs
    ├── outbound.rs
    └── service.rs

crates/ubiq/src/
├── state/
│   ├── mod.rs
│   ├── chat.rs
│   ├── editor.rs
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
│   ├── explorer/
│   │   ├── filter.rs  the go-to-file substring filter
│   │   ├── keys.rs    keyboard navigation
│   │   ├── menu.rs    the row context menu
│   │   ├── mod.rs     `ExplorerState`, `FileNode`, `Row`, `GitStatus`
│   │   ├── rows.rs    flattening the tree into drawable rows
│   │   └── tree.rs    listing, `merge`, expansion
│   ├── vim/
│   │   ├── mod.rs
│   │   ├── motion.rs
│   │   ├── object.rs
│   │   ├── search.rs
│   │   └── step.rs
│   ├── nav.rs
│   ├── nav/
│   │   └── text.rs
│   ├── navigator.rs
│   ├── clone.rs
│   ├── stats.rs
│   ├── image_edit.rs
│   ├── remote.rs
│   ├── notifications.rs
│   ├── new_agent.rs
│   ├── a2ui/
│   │   ├── checkable-inputs.json
│   │   ├── contact-form.json
│   │   ├── icons-and-text.json
│   │   ├── kitchen-sink.json
│   │   ├── tabs-icons-buttons.json
│   │   ├── action.rs
│   │   ├── bindings.json
│   │   ├── eval.rs
│   │   ├── live.rs
│   │   ├── path.rs
│   │   ├── svg.rs
│   │   ├── ubiq-catalog-demo.json
│   │   ├── ubiq-catalog.json
│   │   └── value.rs
│   ├── a2ui.rs
│   ├── remote_hosts.rs
│   ├── web_panel.rs
│   ├── script.rs
│   ├── kb.rs
│   ├── feedback.rs
│   ├── help.rs
│   ├── overlay.rs
│   ├── teams.rs
│   ├── shapes.rs
│   ├── teamsim.rs
│   ├── ui_id.rs
│   ├── ask.rs
│   ├── plan.rs
│   ├── new_mission.rs
│   ├── document.rs
│   ├── status.rs
│   ├── mission.rs
│   ├── wbs.rs
│   ├── zoom.rs
│   └── tasksrc.rs
├── ui/
│   ├── mod.rs
│   ├── chat/
│   │   ├── mod.rs
│   │   └── sidebar.rs
│   ├── kit/
│   │   ├── controls.rs
│   │   ├── menu.rs
│   │   ├── mod.rs
│   │   ├── panel.rs
│   │   ├── canvas.rs
│   │   ├── overlay.rs
│   │   ├── files.rs
│   │   ├── settings.rs
│   │   ├── icons.rs
│   │   ├── ribbon.rs
│   │   ├── popover.rs
│   │   ├── blocks.rs
│   │   ├── colour.rs
│   │   ├── slider.rs
│   │   ├── md_navigator.rs
│   │   └── minimap.rs
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
│   │   ├── viewport.rs
│   │   ├── image_edit.rs
│   │   ├── web.rs
│   │   ├── md_options.rs
│   │   └── zoom_modal.rs
│   ├── sink/
│   │   ├── docs.rs
│   │   ├── mod.rs
│   │   ├── style.rs
│   │   ├── files.rs
│   │   ├── project.rs
│   │   ├── settings.rs
│   │   ├── messages.rs
│   │   ├── a2ui.rs
│   │   ├── script.rs
│   │   ├── teamsim.rs
│   │   └── ext_demo.rs
│   ├── file_picker.rs
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
│   │   ├── refs.rs
│   │   └── repo_selector.rs
│   ├── settings.rs
│   ├── work.rs
│   ├── new_pane_menu.rs
│   ├── conversation/
│   │   ├── mod.rs
│   │   └── info.rs
│   ├── languages/
│   │   ├── csharp/
│   │   │   └── highlights.scm
│   │   ├── swift/
│   │   │   └── highlights.scm
│   │   └── markdown/
│   │       └── tags.scm
│   ├── search.rs
│   ├── file_dialog.rs
│   ├── ribbon.rs
│   ├── navigator.rs
│   ├── clone.rs
│   ├── outline.rs
│   ├── stats.rs
│   ├── remote_connect.rs
│   ├── notifications.rs
│   ├── new_agent.rs
│   ├── a2ui.rs
│   ├── overflow_menu.rs
│   ├── tab_menu.rs
│   ├── remote_hosts.rs
│   ├── all_projects.rs
│   ├── tools.rs
│   ├── web_view.rs
│   ├── a2ui/
│   │   ├── path.rs
│   │   ├── registry.rs
│   │   └── svg.rs
│   ├── kb/
│   │   ├── mod.rs
│   │   └── source_form.rs
│   ├── help/
│   │   ├── mod.rs
│   │   └── unavailable.md
│   ├── teams/
│   │   ├── graph.rs
│   │   ├── mod.rs
│   │   ├── status.rs
│   │   └── tasks.rs
│   ├── acp_capabilities.rs
│   ├── feedback.rs
│   ├── new_project_menu.rs
│   ├── run_tool_menu.rs
│   ├── mark.rs
│   ├── size.rs
│   ├── themes.rs
│   ├── help_target.rs
│   ├── ident.rs
│   ├── project_face.rs
│   ├── ask.rs
│   ├── plan.rs
│   ├── new_mission.rs
│   ├── document.rs
│   ├── mission/
│   │   ├── full.rs
│   │   ├── menu.rs
│   │   ├── mod.rs
│   │   ├── panel.rs
│   │   ├── settings.rs
│   │   └── wbs.rs
│   └── tasksrc.rs
├── lib.rs
├── theme.rs
├── web_export/
│   ├── assets.rs
│   ├── mod.rs
│   ├── routes.rs
│   ├── server.rs
│   ├── bridge.rs
│   └── archive.rs
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
│   ├── wire.rs        `receive()`, the pane and terminal calls, focus drains
│   ├── vim.rs
│   ├── nav.rs
│   ├── clone.rs
│   ├── stats.rs
│   ├── capture.rs
│   ├── clipboard.rs
│   ├── host_browse.rs
│   ├── hosts.rs
│   ├── image_edit.rs
│   ├── remote_connect.rs
│   ├── notifications.rs
│   ├── new_agent.rs
│   ├── host_secrets.rs
│   ├── remote_hosts.rs
│   ├── web_panel.rs
│   ├── ssh_connect.rs
│   ├── kb.rs
│   ├── feedback.rs
│   ├── help.rs
│   ├── teams.rs
│   ├── mark.rs
│   ├── size.rs
│   ├── themes.rs
│   ├── teams_span.rs
│   ├── ask.rs
│   ├── plan.rs
│   ├── new_mission.rs
│   ├── mission.rs
│   └── tasksrc.rs
├── version.rs
├── .DS_Store
└── ext/
    ├── id.rs
    ├── ids.rs
    ├── mod.rs
    ├── registry.rs
    ├── settings.rs
    └── rail.rs

crates/ubiq-app/src/
├── main.rs            three lines: `run(Boot::default())`
├── lib.rs             `Stores`, `Boot` and `run(boot)` — the whole boot sequence, and the only crate that names both halves
└── handoff.rs
```

<!-- generated:end tree -->

<!-- generated:begin anchors -->

## File to document

Every file a document names in its `code_anchors` frontmatter, inverted: change this file, and check
the documents in its row.

| File | Documents |
|---|---|
| `.claude/excalidraw.py` | [`diagram-format.md`](./diagram-format.md) |
| `.github/workflows/create-release.yml` | [`operations.md`](./operations.md) |
| `.github/workflows/release-macos.yml` | [`operations.md`](./operations.md) |
| `.github/workflows/release-windows.yml` | [`operations.md`](./operations.md) |
| `Cargo.toml` | [`project-structure.md`](./project-structure.md) |
| `Justfile` | [`operations.md`](./operations.md) |
| `_devops/scripts/bundle-version.sh` | [`operations.md`](./operations.md) |
| `_tools/Info.plist` | [`operations.md`](./operations.md) |
| `_tools/docs.py` | [`operations.md`](./operations.md) |
| `_tools/drone.py` | [`features/drone.md`](../features/drone.md), [`operations.md`](./operations.md) |
| `_tools/dump.py` | [`operations.md`](./operations.md) |
| `_tools/helpbundle.py` | [`operations.md`](./operations.md), [`wip/help.md`](../wip/help.md) |
| `_tools/icns.py` | [`operations.md`](./operations.md), [`project-structure.md`](./project-structure.md) |
| `_tools/icons.py` | [`ui-and-design.md`](./ui-and-design.md) |
| `_tools/teamsim/FORMAT.md` | [`features/workbench-sink.md`](../features/workbench-sink.md), [`wip/teams-layout-spike.md`](../wip/teams-layout-spike.md) |
| `_tools/teamsim/algos.py` | [`wip/teams-layout-spike.md`](../wip/teams-layout-spike.md) |
| `_tools/teamsim/shapes.py` | [`wip/teams-layout-spike.md`](../wip/teams-layout-spike.md) |
| `_tools/webassets.py` | [`operations.md`](./operations.md) |
| `assets/icons/icons.yaml` | [`ui-and-design.md`](./ui-and-design.md) |
| `crates/agent-manager/examples/confined_shell_probe.rs` | [`agent-manager.md`](./agent-manager.md) |
| `crates/agent-manager/src/account.rs` | [`wip/claude-auth-problem.md`](../wip/claude-auth-problem.md) |
| `crates/agent-manager/src/credentials/mod.rs` | [`agent-manager.md`](./agent-manager.md), [`wip/claude-auth-problem.md`](../wip/claude-auth-problem.md) |
| `crates/agent-manager/src/harness/claude.rs` | [`wip/agent-setup.md`](../wip/agent-setup.md), [`wip/claude-auth-problem.md`](../wip/claude-auth-problem.md) |
| `crates/agent-manager/src/harness/mod.rs` | [`agent-manager.md`](./agent-manager.md), [`wip/agent-setup.md`](../wip/agent-setup.md), [`wip/agent-vocabulary.md`](../wip/agent-vocabulary.md), [`wip/claude-auth-problem.md`](../wip/claude-auth-problem.md) |
| `crates/agent-manager/src/io/acp.rs` | [`agent-manager.md`](./agent-manager.md) |
| `crates/agent-manager/src/io/acp_caps.rs` | [`agent-manager.md`](./agent-manager.md) |
| `crates/agent-manager/src/io/acp_client.rs` | [`inbox/hitl-registered-dialogs.md`](../inbox/hitl-registered-dialogs.md), [`agent-manager.md`](./agent-manager.md) |
| `crates/agent-manager/src/io/jsonl.rs` | [`inbox/hitl-registered-dialogs.md`](../inbox/hitl-registered-dialogs.md), [`inbox/subagent-status-dictionaries-and-hexagon.md`](../inbox/subagent-status-dictionaries-and-hexagon.md), [`wip/agent-setup.md`](../wip/agent-setup.md) |
| `crates/agent-manager/src/io/mod.rs` | [`agent-manager.md`](./agent-manager.md) |
| `crates/agent-manager/src/io/model.rs` | [`wip/agent-setup.md`](../wip/agent-setup.md) |
| `crates/agent-manager/src/io/structured.rs` | [`agent-manager.md`](./agent-manager.md), [`operations.md`](./operations.md) |
| `crates/agent-manager/src/isolate.rs` | [`agent-manager.md`](./agent-manager.md), [`operations.md`](./operations.md), [`wip/agent-setup.md`](../wip/agent-setup.md), [`wip/agent-vocabulary.md`](../wip/agent-vocabulary.md), [`wip/claude-auth-problem.md`](../wip/claude-auth-problem.md) |
| `crates/agent-manager/src/lib.rs` | [`agent-manager.md`](./agent-manager.md) |
| `crates/agent-manager/src/main.rs` | [`agent-manager.md`](./agent-manager.md) |
| `crates/agent-manager/src/overlay.rs` | [`wip/claude-auth-problem.md`](../wip/claude-auth-problem.md) |
| `crates/agent-manager/src/profile.rs` | [`agent-manager.md`](./agent-manager.md), [`wip/agent-setup.md`](../wip/agent-setup.md) |
| `crates/agent-manager/src/provision.rs` | [`agent-manager.md`](./agent-manager.md), [`wip/claude-auth-problem.md`](../wip/claude-auth-problem.md) |
| `crates/agent-manager/src/quota.rs` | [`agent-manager.md`](./agent-manager.md) |
| `crates/agent-manager/src/resolve.rs` | [`agent-manager.md`](./agent-manager.md), [`wip/agent-setup.md`](../wip/agent-setup.md) |
| `crates/agent-manager/src/run.rs` | [`wip/claude-auth-problem.md`](../wip/claude-auth-problem.md) |
| `crates/agent-manager/src/session.rs` | [`features/sessions-and-workspaces.md`](../features/sessions-and-workspaces.md), [`agent-manager.md`](./agent-manager.md), [`wip/agent-setup.md`](../wip/agent-setup.md) |
| `crates/agent-manager/src/spec.rs` | [`agent-manager.md`](./agent-manager.md) |
| `crates/ubiq-app/Cargo.toml` | [`project-structure.md`](./project-structure.md), [`wip/web-panel-phase6.md`](../wip/web-panel-phase6.md) |
| `crates/ubiq-app/build.rs` | [`operations.md`](./operations.md) |
| `crates/ubiq-app/res/ubiq-app.rc` | [`operations.md`](./operations.md) |
| `crates/ubiq-app/src/lib.rs` | [`features/drone.md`](../features/drone.md), [`features/logs.md`](../features/logs.md), [`agent-manager.md`](./agent-manager.md), [`architecture.md`](./architecture.md), [`operations.md`](./operations.md) |
| `crates/ubiq-app/src/main.rs` | [`architecture.md`](./architecture.md) |
| `crates/ubiq-drone/Cargo.toml` | [`project-structure.md`](./project-structure.md) |
| `crates/ubiq-drone/src/lib.rs` | [`features/drone.md`](../features/drone.md), [`architecture.md`](./architecture.md), [`wip/drone.md`](../wip/drone.md) |
| `crates/ubiq-drone/src/linger.rs` | [`features/drone.md`](../features/drone.md) |
| `crates/ubiq-drone/src/main.rs` | [`features/drone.md`](../features/drone.md) |
| `crates/ubiq-drone/src/relay.rs` | [`features/drone.md`](../features/drone.md) |
| `crates/ubiq-drone/src/scrollback.rs` | [`features/drone.md`](../features/drone.md) |
| `crates/ubiq-drone/src/search.rs` | [`features/drone.md`](../features/drone.md), [`transport-contract.md`](./transport-contract.md) |
| `crates/ubiq-drone/src/socket.rs` | [`features/drone.md`](../features/drone.md) |
| `crates/ubiq-drone/src/state.rs` | [`features/drone.md`](../features/drone.md) |
| `crates/ubiq-host/Cargo.toml` | [`agent-manager.md`](./agent-manager.md), [`architecture.md`](./architecture.md), [`operations.md`](./operations.md), [`project-structure.md`](./project-structure.md), [`wip/drone.md`](../wip/drone.md) |
| `crates/ubiq-host/src/agent.rs` | [`features/sessions-and-workspaces.md`](../features/sessions-and-workspaces.md), [`agent-manager.md`](./agent-manager.md), [`wip/agent-login-note.md`](../wip/agent-login-note.md), [`wip/agent-setup.md`](../wip/agent-setup.md), [`wip/claude-auth-problem.md`](../wip/claude-auth-problem.md) |
| `crates/ubiq-host/src/armed.rs` | [`transport-contract.md`](./transport-contract.md) |
| `crates/ubiq-host/src/ask.rs` | [`inbox/hitl-registered-dialogs.md`](../inbox/hitl-registered-dialogs.md), [`agent-manager.md`](./agent-manager.md), [`transport-contract.md`](./transport-contract.md) |
| `crates/ubiq-host/src/assist/api.rs` | [`transport-contract.md`](./transport-contract.md) |
| `crates/ubiq-host/src/assist/mod.rs` | [`transport-contract.md`](./transport-contract.md) |
| `crates/ubiq-host/src/assist/providers.rs` | [`transport-contract.md`](./transport-contract.md) |
| `crates/ubiq-host/src/assist/stub.rs` | [`transport-contract.md`](./transport-contract.md) |
| `crates/ubiq-host/src/assist/subject.rs` | [`transport-contract.md`](./transport-contract.md) |
| `crates/ubiq-host/src/carrier.rs` | [`features/drone.md`](../features/drone.md), [`transport-contract.md`](./transport-contract.md) |
| `crates/ubiq-host/src/cli_shortcut.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq-host/src/connectors/app.rs` | [`features/connectors.md`](../features/connectors.md), [`wip/clone-a-project.md`](../wip/clone-a-project.md) |
| `crates/ubiq-host/src/connectors/flow.rs` | [`features/connectors.md`](../features/connectors.md) |
| `crates/ubiq-host/src/connectors/mod.rs` | [`features/connectors.md`](../features/connectors.md) |
| `crates/ubiq-host/src/connectors/providers.rs` | [`features/connectors.md`](../features/connectors.md) |
| `crates/ubiq-host/src/connectors/store.rs` | [`features/connectors.md`](../features/connectors.md) |
| `crates/ubiq-host/src/connectors/tls.rs` | [`features/connectors.md`](../features/connectors.md) |
| `crates/ubiq-host/src/conversation.rs` | [`features/sessions-and-workspaces.md`](../features/sessions-and-workspaces.md), [`inbox/subagent-status-dictionaries-and-hexagon.md`](../inbox/subagent-status-dictionaries-and-hexagon.md), [`agent-manager.md`](./agent-manager.md), [`transport-contract.md`](./transport-contract.md) |
| `crates/ubiq-host/src/conversation_record.rs` | [`features/sessions-and-workspaces.md`](../features/sessions-and-workspaces.md), [`transport-contract.md`](./transport-contract.md) |
| `crates/ubiq-host/src/coordinator.rs` | [`features/panes-and-terminals.md`](../features/panes-and-terminals.md), [`features/sessions-and-workspaces.md`](../features/sessions-and-workspaces.md), [`features/stats.md`](../features/stats.md), [`features/workbench-tasks.md`](../features/workbench-tasks.md), [`agent-manager.md`](./agent-manager.md), [`architecture.md`](./architecture.md), [`transport-contract.md`](./transport-contract.md), [`wip/agent-setup.md`](../wip/agent-setup.md), [`wip/agent-vocabulary.md`](../wip/agent-vocabulary.md), [`wip/claude-auth-problem.md`](../wip/claude-auth-problem.md), [`wip/indexing.md`](../wip/indexing.md), [`wip/web-panel-phase3.md`](../wip/web-panel-phase3.md) |
| `crates/ubiq-host/src/environment.rs` | [`agent-manager.md`](./agent-manager.md), [`operations.md`](./operations.md) |
| `crates/ubiq-host/src/feedback/api.rs` | [`transport-contract.md`](./transport-contract.md) |
| `crates/ubiq-host/src/feedback/issues.rs` | [`transport-contract.md`](./transport-contract.md) |
| `crates/ubiq-host/src/feedback/mod.rs` | [`transport-contract.md`](./transport-contract.md) |
| `crates/ubiq-host/src/files/diff.rs` | [`architecture.md`](./architecture.md), [`version-control.md`](./version-control.md) |
| `crates/ubiq-host/src/files/mod.rs` | [`architecture.md`](./architecture.md), [`wip/web-panel-phase6.md`](../wip/web-panel-phase6.md) |
| `crates/ubiq-host/src/git/graph.rs` | [`version-control.md`](./version-control.md) |
| `crates/ubiq-host/src/git/history.rs` | [`version-control.md`](./version-control.md) |
| `crates/ubiq-host/src/git/mod.rs` | [`architecture.md`](./architecture.md), [`version-control.md`](./version-control.md) |
| `crates/ubiq-host/src/git/nested.rs` | [`version-control.md`](./version-control.md) |
| `crates/ubiq-host/src/git/observe.rs` | [`architecture.md`](./architecture.md), [`version-control.md`](./version-control.md) |
| `crates/ubiq-host/src/git/write.rs` | [`version-control.md`](./version-control.md) |
| `crates/ubiq-host/src/index/ceiling.rs` | [`wip/indexing.md`](../wip/indexing.md) |
| `crates/ubiq-host/src/index/mod.rs` | [`wip/indexing.md`](../wip/indexing.md) |
| `crates/ubiq-host/src/index/text.rs` | [`wip/indexing.md`](../wip/indexing.md) |
| `crates/ubiq-host/src/kb/mod.rs` | [`architecture.md`](./architecture.md), [`project-structure.md`](./project-structure.md), [`wip/kb.md`](../wip/kb.md) |
| `crates/ubiq-host/src/kb/ops.rs` | [`wip/kb.md`](../wip/kb.md) |
| `crates/ubiq-host/src/kb/store.rs` | [`wip/kb.md`](../wip/kb.md) |
| `crates/ubiq-host/src/kb/sync.rs` | [`wip/kb.md`](../wip/kb.md) |
| `crates/ubiq-host/src/lib.rs` | [`architecture.md`](./architecture.md), [`project-structure.md`](./project-structure.md) |
| `crates/ubiq-host/src/links.rs` | [`architecture.md`](./architecture.md) |
| `crates/ubiq-host/src/mcp/ask.rs` | [`inbox/hitl-registered-dialogs.md`](../inbox/hitl-registered-dialogs.md), [`agent-manager.md`](./agent-manager.md), [`transport-contract.md`](./transport-contract.md) |
| `crates/ubiq-host/src/mcp/catalogue.rs` | [`features/workbench-tasks.md`](../features/workbench-tasks.md), [`transport-contract.md`](./transport-contract.md), [`wip/drone.md`](../wip/drone.md) |
| `crates/ubiq-host/src/mcp/help.rs` | [`wip/help.md`](../wip/help.md) |
| `crates/ubiq-host/src/mcp/kb.rs` | [`wip/kb.md`](../wip/kb.md) |
| `crates/ubiq-host/src/mcp/mission.rs` | [`features/workbench-tasks.md`](../features/workbench-tasks.md), [`transport-contract.md`](./transport-contract.md) |
| `crates/ubiq-host/src/mcp/mod.rs` | [`agent-manager.md`](./agent-manager.md), [`architecture.md`](./architecture.md) |
| `crates/ubiq-host/src/mcp/plan.rs` | [`features/workbench-tasks.md`](../features/workbench-tasks.md), [`architecture.md`](./architecture.md), [`transport-contract.md`](./transport-contract.md) |
| `crates/ubiq-host/src/mcp/registry.rs` | [`features/workbench-tasks.md`](../features/workbench-tasks.md), [`transport-contract.md`](./transport-contract.md), [`wip/drone.md`](../wip/drone.md) |
| `crates/ubiq-host/src/mcp/server.rs` | [`wip/drone.md`](../wip/drone.md) |
| `crates/ubiq-host/src/mcp/tasks.rs` | [`features/workbench-tasks.md`](../features/workbench-tasks.md), [`architecture.md`](./architecture.md), [`transport-contract.md`](./transport-contract.md) |
| `crates/ubiq-host/src/mcp/tools.rs` | [`wip/drone.md`](../wip/drone.md) |
| `crates/ubiq-host/src/mission/mod.rs` | [`features/workbench-tasks.md`](../features/workbench-tasks.md), [`transport-contract.md`](./transport-contract.md) |
| `crates/ubiq-host/src/mission/scheduler.rs` | [`features/workbench-tasks.md`](../features/workbench-tasks.md), [`transport-contract.md`](./transport-contract.md) |
| `crates/ubiq-host/src/notifications/mod.rs` | [`features/notifications.md`](../features/notifications.md), [`transport-contract.md`](./transport-contract.md) |
| `crates/ubiq-host/src/notifications/os.rs` | [`features/notifications.md`](../features/notifications.md) |
| `crates/ubiq-host/src/plan/blocks.rs` | [`features/workbench-tasks.md`](../features/workbench-tasks.md) |
| `crates/ubiq-host/src/plan/mod.rs` | [`features/workbench-tasks.md`](../features/workbench-tasks.md), [`architecture.md`](./architecture.md), [`transport-contract.md`](./transport-contract.md), [`wip/planning-system.md`](../wip/planning-system.md) |
| `crates/ubiq-host/src/plan/service.rs` | [`features/workbench-tasks.md`](../features/workbench-tasks.md), [`wip/planning-system.md`](../wip/planning-system.md) |
| `crates/ubiq-host/src/projects.rs` | [`features/stats.md`](../features/stats.md), [`features/workbench.md`](../features/workbench.md), [`architecture.md`](./architecture.md), [`wip/clone-a-project.md`](../wip/clone-a-project.md), [`wip/web-panel-phase3.md`](../wip/web-panel-phase3.md) |
| `crates/ubiq-host/src/pty/mod.rs` | [`features/panes-and-terminals.md`](../features/panes-and-terminals.md) |
| `crates/ubiq-host/src/quota.rs` | [`transport-contract.md`](./transport-contract.md) |
| `crates/ubiq-host/src/remote.rs` | [`architecture.md`](./architecture.md), [`operations.md`](./operations.md) |
| `crates/ubiq-host/src/repos/clone.rs` | [`features/workbench.md`](../features/workbench.md), [`version-control.md`](./version-control.md), [`wip/clone-a-project.md`](../wip/clone-a-project.md) |
| `crates/ubiq-host/src/repos/list.rs` | [`features/workbench.md`](../features/workbench.md), [`version-control.md`](./version-control.md), [`wip/clone-a-project.md`](../wip/clone-a-project.md) |
| `crates/ubiq-host/src/repos/mod.rs` | [`features/workbench.md`](../features/workbench.md), [`architecture.md`](./architecture.md), [`version-control.md`](./version-control.md), [`wip/clone-a-project.md`](../wip/clone-a-project.md) |
| `crates/ubiq-host/src/search/hits.rs` | [`wip/indexing.md`](../wip/indexing.md) |
| `crates/ubiq-host/src/search/worker.rs` | [`wip/indexing.md`](../wip/indexing.md) |
| `crates/ubiq-host/src/settings.rs` | [`features/connectors.md`](../features/connectors.md), [`features/drone.md`](../features/drone.md), [`architecture.md`](./architecture.md) |
| `crates/ubiq-host/src/shells.rs` | [`features/panes-and-terminals.md`](../features/panes-and-terminals.md), [`wip/agent-vocabulary.md`](../wip/agent-vocabulary.md) |
| `crates/ubiq-host/src/store/file.rs` | [`features/workbench-tasks.md`](../features/workbench-tasks.md), [`architecture.md`](./architecture.md) |
| `crates/ubiq-host/src/store/harness.rs` | [`wip/agent-vocabulary.md`](../wip/agent-vocabulary.md) |
| `crates/ubiq-host/src/store/memory.rs` | [`architecture.md`](./architecture.md) |
| `crates/ubiq-host/src/store/mission.rs` | [`features/workbench-tasks.md`](../features/workbench-tasks.md), [`transport-contract.md`](./transport-contract.md) |
| `crates/ubiq-host/src/store/mod.rs` | [`architecture.md`](./architecture.md) |
| `crates/ubiq-host/src/store/plan.rs` | [`features/workbench-tasks.md`](../features/workbench-tasks.md), [`architecture.md`](./architecture.md), [`transport-contract.md`](./transport-contract.md), [`wip/planning-system.md`](../wip/planning-system.md) |
| `crates/ubiq-host/src/store/project_dir.rs` | [`project-structure.md`](./project-structure.md) |
| `crates/ubiq-host/src/store/usage.rs` | [`features/stats.md`](../features/stats.md), [`project-structure.md`](./project-structure.md) |
| `crates/ubiq-host/src/tasksrc/mod.rs` | [`inbox/task-sources-proposal.md`](../inbox/task-sources-proposal.md) |
| `crates/ubiq-host/src/tasksrc/outbound.rs` | [`inbox/task-sources-proposal.md`](../inbox/task-sources-proposal.md) |
| `crates/ubiq-host/src/tasksrc/service.rs` | [`inbox/task-sources-proposal.md`](../inbox/task-sources-proposal.md) |
| `crates/ubiq-host/src/tasksrc/store.rs` | [`inbox/task-sources-proposal.md`](../inbox/task-sources-proposal.md) |
| `crates/ubiq-host/src/tasksrc/sync.rs` | [`inbox/task-sources-proposal.md`](../inbox/task-sources-proposal.md) |
| `crates/ubiq-host/src/tasksrc/trello.rs` | [`inbox/task-sources-proposal.md`](../inbox/task-sources-proposal.md) |
| `crates/ubiq-host/src/watch/mod.rs` | [`architecture.md`](./architecture.md), [`version-control.md`](./version-control.md), [`wip/indexing.md`](../wip/indexing.md) |
| `crates/ubiq-host/src/web_assets/archive.rs` | [`wip/web-panel-phase3.md`](../wip/web-panel-phase3.md) |
| `crates/ubiq-host/src/web_assets/manifest.rs` | [`wip/web-panel-phase3.md`](../wip/web-panel-phase3.md) |
| `crates/ubiq-host/src/web_assets/mod.rs` | [`architecture.md`](./architecture.md), [`transport-contract.md`](./transport-contract.md), [`wip/web-panel-phase3.md`](../wip/web-panel-phase3.md) |
| `crates/ubiq-host/src/work/mod.rs` | [`features/workbench-tasks.md`](../features/workbench-tasks.md), [`architecture.md`](./architecture.md), [`transport-contract.md`](./transport-contract.md) |
| `crates/ubiq-host/tests/coordinator.rs` | [`features/panes-and-terminals.md`](../features/panes-and-terminals.md) |
| `crates/ubiq-host/tests/files.rs` | [`wip/web-panel-phase6.md`](../wip/web-panel-phase6.md) |
| `crates/ubiq-host/tests/usage.rs` | [`features/stats.md`](../features/stats.md) |
| `crates/ubiq-proto/Cargo.toml` | [`project-structure.md`](./project-structure.md) |
| `crates/ubiq-proto/src/ask.rs` | [`features/chat.md`](../features/chat.md), [`agent-manager.md`](./agent-manager.md), [`transport-contract.md`](./transport-contract.md) |
| `crates/ubiq-proto/src/assist.rs` | [`transport-contract.md`](./transport-contract.md) |
| `crates/ubiq-proto/src/blocks.rs` | [`features/workbench-tasks.md`](../features/workbench-tasks.md) |
| `crates/ubiq-proto/src/bus.rs` | [`features/panes-and-terminals.md`](../features/panes-and-terminals.md), [`architecture.md`](./architecture.md), [`transport-contract.md`](./transport-contract.md) |
| `crates/ubiq-proto/src/carrier.rs` | [`features/drone.md`](../features/drone.md), [`transport-contract.md`](./transport-contract.md), [`wip/drone.md`](../wip/drone.md) |
| `crates/ubiq-proto/src/connectors.rs` | [`features/connectors.md`](../features/connectors.md), [`transport-contract.md`](./transport-contract.md), [`wip/clone-a-project.md`](../wip/clone-a-project.md) |
| `crates/ubiq-proto/src/conversation.rs` | [`transport-contract.md`](./transport-contract.md) |
| `crates/ubiq-proto/src/drone/manifest.rs` | [`features/drone.md`](../features/drone.md) |
| `crates/ubiq-proto/src/drone/mod.rs` | [`features/drone.md`](../features/drone.md) |
| `crates/ubiq-proto/src/feedback.rs` | [`transport-contract.md`](./transport-contract.md) |
| `crates/ubiq-proto/src/files.rs` | [`transport-contract.md`](./transport-contract.md) |
| `crates/ubiq-proto/src/git.rs` | [`transport-contract.md`](./transport-contract.md), [`version-control.md`](./version-control.md) |
| `crates/ubiq-proto/src/ids.rs` | [`features/connectors.md`](../features/connectors.md), [`transport-contract.md`](./transport-contract.md) |
| `crates/ubiq-proto/src/kb.rs` | [`wip/kb.md`](../wip/kb.md) |
| `crates/ubiq-proto/src/lib.rs` | [`architecture.md`](./architecture.md) |
| `crates/ubiq-proto/src/log.rs` | [`features/logs.md`](../features/logs.md), [`architecture.md`](./architecture.md), [`wip/claude-auth-problem.md`](../wip/claude-auth-problem.md) |
| `crates/ubiq-proto/src/mcp.rs` | [`transport-contract.md`](./transport-contract.md) |
| `crates/ubiq-proto/src/messages.rs` | [`features/connectors.md`](../features/connectors.md), [`features/stats.md`](../features/stats.md), [`transport-contract.md`](./transport-contract.md), [`wip/drone.md`](../wip/drone.md), [`wip/kb.md`](../wip/kb.md), [`wip/web-panel-phase3.md`](../wip/web-panel-phase3.md) |
| `crates/ubiq-proto/src/mission.rs` | [`transport-contract.md`](./transport-contract.md) |
| `crates/ubiq-proto/src/notifications.rs` | [`features/notifications.md`](../features/notifications.md), [`transport-contract.md`](./transport-contract.md) |
| `crates/ubiq-proto/src/plan.rs` | [`features/workbench-tasks.md`](../features/workbench-tasks.md), [`transport-contract.md`](./transport-contract.md), [`wip/planning-system.md`](../wip/planning-system.md) |
| `crates/ubiq-proto/src/projects.rs` | [`features/drone.md`](../features/drone.md), [`transport-contract.md`](./transport-contract.md), [`wip/clone-a-project.md`](../wip/clone-a-project.md), [`wip/indexing.md`](../wip/indexing.md) |
| `crates/ubiq-proto/src/quota.rs` | [`transport-contract.md`](./transport-contract.md) |
| `crates/ubiq-proto/src/repos.rs` | [`features/workbench.md`](../features/workbench.md), [`transport-contract.md`](./transport-contract.md), [`wip/clone-a-project.md`](../wip/clone-a-project.md) |
| `crates/ubiq-proto/src/settings.rs` | [`features/connectors.md`](../features/connectors.md), [`features/drone.md`](../features/drone.md), [`transport-contract.md`](./transport-contract.md), [`wip/indexing.md`](../wip/indexing.md) |
| `crates/ubiq-proto/src/stats.rs` | [`features/stats.md`](../features/stats.md), [`transport-contract.md`](./transport-contract.md) |
| `crates/ubiq-proto/src/tasksrc.rs` | [`inbox/task-sources-proposal.md`](../inbox/task-sources-proposal.md), [`transport-contract.md`](./transport-contract.md) |
| `crates/ubiq-proto/src/tools.rs` | [`transport-contract.md`](./transport-contract.md) |
| `crates/ubiq-proto/src/wire.rs` | [`architecture.md`](./architecture.md), [`transport-contract.md`](./transport-contract.md) |
| `crates/ubiq-proto/src/work.rs` | [`transport-contract.md`](./transport-contract.md), [`wip/agent-setup.md`](../wip/agent-setup.md) |
| `crates/ubiq/Cargo.toml` | [`project-structure.md`](./project-structure.md), [`wip/web-panel-phase6.md`](../wip/web-panel-phase6.md) |
| `crates/ubiq/assets/web/bridge.js` | [`wip/web-panel-phase2.md`](../wip/web-panel-phase2.md) |
| `crates/ubiq/assets/web/drawio/app.js` | [`wip/web-panel-phase45.md`](../wip/web-panel-phase45.md) |
| `crates/ubiq/assets/web/drawio/index.html` | [`wip/web-panel-phase45.md`](../wip/web-panel-phase45.md) |
| `crates/ubiq/assets/web/excalidraw/app.js` | [`wip/web-panel-phase45.md`](../wip/web-panel-phase45.md) |
| `crates/ubiq/assets/web/excalidraw/index.html` | [`wip/web-panel-phase45.md`](../wip/web-panel-phase45.md) |
| `crates/ubiq/build.rs` | [`wip/web-panel-phase2.md`](../wip/web-panel-phase2.md) |
| `crates/ubiq/src/app/agents.rs` | [`features/chat.md`](../features/chat.md), [`features/workbench-agents.md`](../features/workbench-agents.md), [`wip/agent-vocabulary.md`](../wip/agent-vocabulary.md) |
| `crates/ubiq/src/app/ask.rs` | [`features/chat.md`](../features/chat.md), [`features/notifications.md`](../features/notifications.md) |
| `crates/ubiq/src/app/board.rs` | [`features/workbench-tasks.md`](../features/workbench-tasks.md) |
| `crates/ubiq/src/app/boot.rs` | [`features/chat.md`](../features/chat.md), [`architecture.md`](./architecture.md) |
| `crates/ubiq/src/app/capture.rs` | [`features/workbench-ide.md`](../features/workbench-ide.md) |
| `crates/ubiq/src/app/chat.rs` | [`features/chat.md`](../features/chat.md) |
| `crates/ubiq/src/app/clipboard.rs` | [`features/chat.md`](../features/chat.md), [`features/panes-and-terminals.md`](../features/panes-and-terminals.md), [`features/workbench-ide.md`](../features/workbench-ide.md) |
| `crates/ubiq/src/app/clone.rs` | [`features/workbench.md`](../features/workbench.md), [`wip/clone-a-project.md`](../wip/clone-a-project.md) |
| `crates/ubiq/src/app/editor.rs` | [`features/panes-and-terminals.md`](../features/panes-and-terminals.md), [`features/workbench-tasks.md`](../features/workbench-tasks.md), [`wip/web-panel-phase6.md`](../wip/web-panel-phase6.md) |
| `crates/ubiq/src/app/explorer.rs` | [`features/workbench-ide.md`](../features/workbench-ide.md), [`wip/web-panel-phase6.md`](../wip/web-panel-phase6.md) |
| `crates/ubiq/src/app/feedback.rs` | [`features/workbench-ide.md`](../features/workbench-ide.md) |
| `crates/ubiq/src/app/git.rs` | [`features/workbench-git.md`](../features/workbench-git.md), [`version-control.md`](./version-control.md) |
| `crates/ubiq/src/app/graph.rs` | [`features/workbench-teams.md`](../features/workbench-teams.md) |
| `crates/ubiq/src/app/help.rs` | [`wip/help.md`](../wip/help.md) |
| `crates/ubiq/src/app/host_browse.rs` | [`features/workbench.md`](../features/workbench.md), [`components.md`](./components.md) |
| `crates/ubiq/src/app/hosts.rs` | [`features/drone.md`](../features/drone.md), [`features/workbench.md`](../features/workbench.md), [`architecture.md`](./architecture.md) |
| `crates/ubiq/src/app/image_edit.rs` | [`features/workbench-ide.md`](../features/workbench-ide.md) |
| `crates/ubiq/src/app/kb.rs` | [`wip/kb.md`](../wip/kb.md) |
| `crates/ubiq/src/app/mark.rs` | [`features/workbench-ide.md`](../features/workbench-ide.md) |
| `crates/ubiq/src/app/mission.rs` | [`features/workbench-agents.md`](../features/workbench-agents.md), [`features/workbench-tasks.md`](../features/workbench-tasks.md) |
| `crates/ubiq/src/app/mod.rs` | [`features/panes-and-terminals.md`](../features/panes-and-terminals.md), [`features/workbench.md`](../features/workbench.md), [`architecture.md`](./architecture.md), [`ui-and-design.md`](./ui-and-design.md), [`wip/teams-cross-project.md`](../wip/teams-cross-project.md) |
| `crates/ubiq/src/app/nav.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/src/app/new_agent.rs` | [`features/workbench-agents.md`](../features/workbench-agents.md) |
| `crates/ubiq/src/app/new_mission.rs` | [`features/workbench-tasks.md`](../features/workbench-tasks.md) |
| `crates/ubiq/src/app/notifications.rs` | [`features/notifications.md`](../features/notifications.md) |
| `crates/ubiq/src/app/panels.rs` | [`features/chat.md`](../features/chat.md), [`features/panes-and-terminals.md`](../features/panes-and-terminals.md), [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/src/app/picker.rs` | [`features/chat.md`](../features/chat.md), [`features/workbench.md`](../features/workbench.md), [`components.md`](./components.md) |
| `crates/ubiq/src/app/plan.rs` | [`features/workbench-tasks.md`](../features/workbench-tasks.md) |
| `crates/ubiq/src/app/projects.rs` | [`features/chat.md`](../features/chat.md), [`features/drone.md`](../features/drone.md), [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/src/app/remote_connect.rs` | [`features/drone.md`](../features/drone.md), [`features/workbench.md`](../features/workbench.md), [`architecture.md`](./architecture.md), [`transport-contract.md`](./transport-contract.md) |
| `crates/ubiq/src/app/settings.rs` | [`features/connectors.md`](../features/connectors.md), [`features/panes-and-terminals.md`](../features/panes-and-terminals.md), [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/src/app/shell.rs` | [`features/chat.md`](../features/chat.md), [`features/stats.md`](../features/stats.md), [`features/workbench.md`](../features/workbench.md), [`ui-and-design.md`](./ui-and-design.md), [`wip/teams-cross-project.md`](../wip/teams-cross-project.md), [`wip/web-panel-phase6.md`](../wip/web-panel-phase6.md) |
| `crates/ubiq/src/app/sink.rs` | [`features/workbench-sink.md`](../features/workbench-sink.md) |
| `crates/ubiq/src/app/size.rs` | [`features/workbench.md`](../features/workbench.md), [`ui-and-design.md`](./ui-and-design.md) |
| `crates/ubiq/src/app/ssh_connect.rs` | [`features/drone.md`](../features/drone.md), [`features/workbench.md`](../features/workbench.md), [`architecture.md`](./architecture.md) |
| `crates/ubiq/src/app/stats.rs` | [`features/stats.md`](../features/stats.md) |
| `crates/ubiq/src/app/tasksrc.rs` | [`features/workbench-tasks.md`](../features/workbench-tasks.md), [`inbox/task-sources-proposal.md`](../inbox/task-sources-proposal.md) |
| `crates/ubiq/src/app/teams.rs` | [`features/workbench-teams.md`](../features/workbench-teams.md), [`components.md`](./components.md), [`wip/teams-cross-project.md`](../wip/teams-cross-project.md) |
| `crates/ubiq/src/app/teams_span.rs` | [`features/workbench-teams.md`](../features/workbench-teams.md) |
| `crates/ubiq/src/app/vim.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/src/app/web_panel.rs` | [`features/workbench-ide.md`](../features/workbench-ide.md), [`wip/kb.md`](../wip/kb.md), [`wip/web-panel-phase45.md`](../wip/web-panel-phase45.md), [`wip/web-panel-phase6.md`](../wip/web-panel-phase6.md) |
| `crates/ubiq/src/app/wire.rs` | [`features/chat.md`](../features/chat.md), [`features/notifications.md`](../features/notifications.md), [`features/panes-and-terminals.md`](../features/panes-and-terminals.md), [`features/stats.md`](../features/stats.md), [`features/workbench-tasks.md`](../features/workbench-tasks.md), [`architecture.md`](./architecture.md), [`ui-and-design.md`](./ui-and-design.md) |
| `crates/ubiq/src/ext/id.rs` | [`architecture.md`](./architecture.md) |
| `crates/ubiq/src/ext/ids.rs` | [`architecture.md`](./architecture.md) |
| `crates/ubiq/src/ext/mod.rs` | [`architecture.md`](./architecture.md) |
| `crates/ubiq/src/ext/rail.rs` | [`features/workbench.md`](../features/workbench.md), [`architecture.md`](./architecture.md) |
| `crates/ubiq/src/ext/registry.rs` | [`architecture.md`](./architecture.md) |
| `crates/ubiq/src/ext/settings.rs` | [`features/workbench.md`](../features/workbench.md), [`architecture.md`](./architecture.md) |
| `crates/ubiq/src/lib.rs` | [`architecture.md`](./architecture.md) |
| `crates/ubiq/src/state/a2ui.rs` | [`features/workbench-sink.md`](../features/workbench-sink.md) |
| `crates/ubiq/src/state/a2ui/action.rs` | [`features/workbench-sink.md`](../features/workbench-sink.md) |
| `crates/ubiq/src/state/a2ui/eval.rs` | [`features/workbench-sink.md`](../features/workbench-sink.md) |
| `crates/ubiq/src/state/a2ui/live.rs` | [`features/workbench-sink.md`](../features/workbench-sink.md) |
| `crates/ubiq/src/state/a2ui/path.rs` | [`features/workbench-sink.md`](../features/workbench-sink.md) |
| `crates/ubiq/src/state/a2ui/svg.rs` | [`features/workbench-sink.md`](../features/workbench-sink.md) |
| `crates/ubiq/src/state/a2ui/ubiq-catalog.json` | [`features/workbench-sink.md`](../features/workbench-sink.md) |
| `crates/ubiq/src/state/a2ui/value.rs` | [`features/workbench-sink.md`](../features/workbench-sink.md) |
| `crates/ubiq/src/state/agents.rs` | [`features/workbench-agents.md`](../features/workbench-agents.md), [`components.md`](./components.md) |
| `crates/ubiq/src/state/ask.rs` | [`features/chat.md`](../features/chat.md), [`inbox/hitl-registered-dialogs.md`](../inbox/hitl-registered-dialogs.md) |
| `crates/ubiq/src/state/board.rs` | [`features/workbench-tasks.md`](../features/workbench-tasks.md), [`ui-and-design.md`](./ui-and-design.md) |
| `crates/ubiq/src/state/chat.rs` | [`features/chat.md`](../features/chat.md) |
| `crates/ubiq/src/state/clone.rs` | [`features/workbench.md`](../features/workbench.md), [`wip/clone-a-project.md`](../wip/clone-a-project.md) |
| `crates/ubiq/src/state/conversation.rs` | [`features/chat.md`](../features/chat.md), [`features/workbench-agents.md`](../features/workbench-agents.md), [`inbox/subagent-status-dictionaries-and-hexagon.md`](../inbox/subagent-status-dictionaries-and-hexagon.md), [`components.md`](./components.md), [`wip/agent-setup.md`](../wip/agent-setup.md) |
| `crates/ubiq/src/state/diagrams.rs` | [`features/workbench-ide.md`](../features/workbench-ide.md), [`wip/web-panel-phase6.md`](../wip/web-panel-phase6.md) |
| `crates/ubiq/src/state/dock.rs` | [`features/chat.md`](../features/chat.md), [`features/logs.md`](../features/logs.md), [`features/panes-and-terminals.md`](../features/panes-and-terminals.md), [`features/workbench.md`](../features/workbench.md), [`wip/agent-vocabulary.md`](../wip/agent-vocabulary.md), [`wip/kb.md`](../wip/kb.md) |
| `crates/ubiq/src/state/document.rs` | [`features/workbench-tasks.md`](../features/workbench-tasks.md), [`wip/planning-system.md`](../wip/planning-system.md) |
| `crates/ubiq/src/state/editor.rs` | [`features/workbench-ide.md`](../features/workbench-ide.md), [`components.md`](./components.md), [`wip/kb.md`](../wip/kb.md), [`wip/web-panel-phase6.md`](../wip/web-panel-phase6.md) |
| `crates/ubiq/src/state/explorer/menu.rs` | [`features/workbench-ide.md`](../features/workbench-ide.md), [`wip/web-panel-phase6.md`](../wip/web-panel-phase6.md) |
| `crates/ubiq/src/state/explorer/mod.rs` | [`features/workbench-ide.md`](../features/workbench-ide.md), [`wip/web-panel-phase6.md`](../wip/web-panel-phase6.md) |
| `crates/ubiq/src/state/explorer/rows.rs` | [`features/workbench-ide.md`](../features/workbench-ide.md) |
| `crates/ubiq/src/state/explorer/tree.rs` | [`features/workbench-ide.md`](../features/workbench-ide.md) |
| `crates/ubiq/src/state/feedback.rs` | [`features/workbench-ide.md`](../features/workbench-ide.md) |
| `crates/ubiq/src/state/file_picker.rs` | [`features/workbench.md`](../features/workbench.md), [`components.md`](./components.md), [`ui-and-design.md`](./ui-and-design.md) |
| `crates/ubiq/src/state/git.rs` | [`features/workbench-git.md`](../features/workbench-git.md), [`version-control.md`](./version-control.md) |
| `crates/ubiq/src/state/help.rs` | [`wip/help.md`](../wip/help.md) |
| `crates/ubiq/src/state/image_edit.rs` | [`features/workbench-ide.md`](../features/workbench-ide.md) |
| `crates/ubiq/src/state/kb.rs` | [`wip/kb.md`](../wip/kb.md) |
| `crates/ubiq/src/state/layout.rs` | [`features/workbench-teams.md`](../features/workbench-teams.md), [`wip/teams-layout-spike.md`](../wip/teams-layout-spike.md) |
| `crates/ubiq/src/state/logs.rs` | [`features/logs.md`](../features/logs.md) |
| `crates/ubiq/src/state/mission.rs` | [`features/workbench-tasks.md`](../features/workbench-tasks.md) |
| `crates/ubiq/src/state/mod.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/src/state/nav.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/src/state/nav/text.rs` | [`features/workbench.md`](../features/workbench.md), [`wip/teams-cross-project.md`](../wip/teams-cross-project.md) |
| `crates/ubiq/src/state/navigator.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/src/state/new_agent.rs` | [`features/workbench-agents.md`](../features/workbench-agents.md) |
| `crates/ubiq/src/state/new_mission.rs` | [`features/workbench-tasks.md`](../features/workbench-tasks.md) |
| `crates/ubiq/src/state/notifications.rs` | [`features/notifications.md`](../features/notifications.md) |
| `crates/ubiq/src/state/orchestration.rs` | [`features/workbench-teams.md`](../features/workbench-teams.md) |
| `crates/ubiq/src/state/overlay.rs` | [`ui-and-design.md`](./ui-and-design.md) |
| `crates/ubiq/src/state/plan.rs` | [`features/workbench-tasks.md`](../features/workbench-tasks.md) |
| `crates/ubiq/src/state/prefs.rs` | [`features/chat.md`](../features/chat.md), [`features/workbench.md`](../features/workbench.md), [`ui-and-design.md`](./ui-and-design.md) |
| `crates/ubiq/src/state/remote.rs` | [`features/workbench.md`](../features/workbench.md), [`architecture.md`](./architecture.md) |
| `crates/ubiq/src/state/scene.rs` | [`features/workbench-ide.md`](../features/workbench-ide.md) |
| `crates/ubiq/src/state/script.rs` | [`features/workbench-sink.md`](../features/workbench-sink.md) |
| `crates/ubiq/src/state/settings.rs` | [`features/connectors.md`](../features/connectors.md), [`features/panes-and-terminals.md`](../features/panes-and-terminals.md), [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/src/state/shapes.rs` | [`features/workbench-teams.md`](../features/workbench-teams.md), [`wip/teams-layout-spike.md`](../wip/teams-layout-spike.md) |
| `crates/ubiq/src/state/sink.rs` | [`features/workbench-sink.md`](../features/workbench-sink.md) |
| `crates/ubiq/src/state/stats.rs` | [`features/stats.md`](../features/stats.md) |
| `crates/ubiq/src/state/status.rs` | [`features/workbench-agents.md`](../features/workbench-agents.md) |
| `crates/ubiq/src/state/tasksrc.rs` | [`features/workbench-tasks.md`](../features/workbench-tasks.md), [`inbox/task-sources-proposal.md`](../inbox/task-sources-proposal.md) |
| `crates/ubiq/src/state/teams.rs` | [`features/workbench-teams.md`](../features/workbench-teams.md), [`inbox/subagent-status-dictionaries-and-hexagon.md`](../inbox/subagent-status-dictionaries-and-hexagon.md), [`wip/teams-cross-project.md`](../wip/teams-cross-project.md) |
| `crates/ubiq/src/state/teamsim.rs` | [`features/workbench-sink.md`](../features/workbench-sink.md), [`features/workbench-teams.md`](../features/workbench-teams.md), [`wip/teams-layout-spike.md`](../wip/teams-layout-spike.md) |
| `crates/ubiq/src/state/ui_id.rs` | [`wip/in-place-help.md`](../wip/in-place-help.md) |
| `crates/ubiq/src/state/viewport.rs` | [`features/workbench-ide.md`](../features/workbench-ide.md) |
| `crates/ubiq/src/state/vim/mod.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/src/state/vim/motion.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/src/state/vim/object.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/src/state/vim/search.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/src/state/vim/step.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/src/state/wbs.rs` | [`features/workbench-tasks.md`](../features/workbench-tasks.md) |
| `crates/ubiq/src/state/web_panel.rs` | [`features/workbench-ide.md`](../features/workbench-ide.md), [`wip/web-panel-phase45.md`](../wip/web-panel-phase45.md) |
| `crates/ubiq/src/state/when.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/src/state/windows.rs` | [`features/workbench.md`](../features/workbench.md), [`architecture.md`](./architecture.md) |
| `crates/ubiq/src/state/work.rs` | [`features/chat.md`](../features/chat.md), [`features/workbench-tasks.md`](../features/workbench-tasks.md), [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/src/state/workbench.rs` | [`features/panes-and-terminals.md`](../features/panes-and-terminals.md), [`features/workbench.md`](../features/workbench.md), [`wip/agent-vocabulary.md`](../wip/agent-vocabulary.md), [`wip/web-panel-phase6.md`](../wip/web-panel-phase6.md) |
| `crates/ubiq/src/theme.rs` | [`ui-and-design.md`](./ui-and-design.md) |
| `crates/ubiq/src/ui/a2ui.rs` | [`features/workbench-sink.md`](../features/workbench-sink.md) |
| `crates/ubiq/src/ui/a2ui/path.rs` | [`features/workbench-sink.md`](../features/workbench-sink.md) |
| `crates/ubiq/src/ui/a2ui/registry.rs` | [`features/workbench-sink.md`](../features/workbench-sink.md) |
| `crates/ubiq/src/ui/a2ui/svg.rs` | [`features/workbench-sink.md`](../features/workbench-sink.md) |
| `crates/ubiq/src/ui/acp_capabilities.rs` | [`features/chat.md`](../features/chat.md), [`features/workbench.md`](../features/workbench.md), [`components.md`](./components.md) |
| `crates/ubiq/src/ui/agents/column.rs` | [`features/workbench-agents.md`](../features/workbench-agents.md), [`inbox/subagent-status-dictionaries-and-hexagon.md`](../inbox/subagent-status-dictionaries-and-hexagon.md), [`wip/agent-vocabulary.md`](../wip/agent-vocabulary.md) |
| `crates/ubiq/src/ui/agents/mod.rs` | [`features/chat.md`](../features/chat.md), [`features/workbench-agents.md`](../features/workbench-agents.md) |
| `crates/ubiq/src/ui/agents/sidebar.rs` | [`features/workbench-agents.md`](../features/workbench-agents.md) |
| `crates/ubiq/src/ui/all_projects.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/src/ui/ask.rs` | [`features/chat.md`](../features/chat.md) |
| `crates/ubiq/src/ui/board/detail.rs` | [`features/workbench-tasks.md`](../features/workbench-tasks.md) |
| `crates/ubiq/src/ui/board/form.rs` | [`features/workbench-tasks.md`](../features/workbench-tasks.md) |
| `crates/ubiq/src/ui/board/mod.rs` | [`features/workbench-tasks.md`](../features/workbench-tasks.md), [`ui-and-design.md`](./ui-and-design.md) |
| `crates/ubiq/src/ui/chat/mod.rs` | [`features/chat.md`](../features/chat.md) |
| `crates/ubiq/src/ui/chat/sidebar.rs` | [`features/chat.md`](../features/chat.md), [`wip/agent-vocabulary.md`](../wip/agent-vocabulary.md) |
| `crates/ubiq/src/ui/clone.rs` | [`features/workbench.md`](../features/workbench.md), [`wip/clone-a-project.md`](../wip/clone-a-project.md) |
| `crates/ubiq/src/ui/conversation/info.rs` | [`features/chat.md`](../features/chat.md) |
| `crates/ubiq/src/ui/conversation/mod.rs` | [`features/chat.md`](../features/chat.md), [`features/workbench-agents.md`](../features/workbench-agents.md), [`inbox/subagent-status-dictionaries-and-hexagon.md`](../inbox/subagent-status-dictionaries-and-hexagon.md), [`components.md`](./components.md), [`ui-and-design.md`](./ui-and-design.md), [`wip/agent-setup.md`](../wip/agent-setup.md), [`wip/agent-vocabulary.md`](../wip/agent-vocabulary.md) |
| `crates/ubiq/src/ui/dock/mod.rs` | [`features/panes-and-terminals.md`](../features/panes-and-terminals.md), [`features/stats.md`](../features/stats.md), [`features/workbench.md`](../features/workbench.md), [`ui-and-design.md`](./ui-and-design.md) |
| `crates/ubiq/src/ui/dock/skin.rs` | [`features/chat.md`](../features/chat.md), [`features/panes-and-terminals.md`](../features/panes-and-terminals.md), [`features/workbench.md`](../features/workbench.md), [`ui-and-design.md`](./ui-and-design.md) |
| `crates/ubiq/src/ui/document.rs` | [`features/workbench-tasks.md`](../features/workbench-tasks.md) |
| `crates/ubiq/src/ui/editor.rs` | [`features/workbench-ide.md`](../features/workbench-ide.md), [`components.md`](./components.md) |
| `crates/ubiq/src/ui/empty.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/src/ui/explorer.rs` | [`features/workbench-ide.md`](../features/workbench-ide.md), [`ui-and-design.md`](./ui-and-design.md) |
| `crates/ubiq/src/ui/feedback.rs` | [`features/workbench-ide.md`](../features/workbench-ide.md) |
| `crates/ubiq/src/ui/file_dialog.rs` | [`features/workbench-ide.md`](../features/workbench-ide.md), [`components.md`](./components.md), [`wip/kb.md`](../wip/kb.md), [`wip/web-panel-phase6.md`](../wip/web-panel-phase6.md) |
| `crates/ubiq/src/ui/file_picker.rs` | [`features/workbench.md`](../features/workbench.md), [`components.md`](./components.md), [`ui-and-design.md`](./ui-and-design.md) |
| `crates/ubiq/src/ui/git/changes.rs` | [`features/workbench-git.md`](../features/workbench-git.md) |
| `crates/ubiq/src/ui/git/diff.rs` | [`features/workbench-git.md`](../features/workbench-git.md), [`components.md`](./components.md) |
| `crates/ubiq/src/ui/git/history.rs` | [`features/workbench-git.md`](../features/workbench-git.md) |
| `crates/ubiq/src/ui/git/mod.rs` | [`features/workbench-git.md`](../features/workbench-git.md) |
| `crates/ubiq/src/ui/git/refs.rs` | [`features/workbench-git.md`](../features/workbench-git.md) |
| `crates/ubiq/src/ui/git/repo_selector.rs` | [`features/workbench-git.md`](../features/workbench-git.md) |
| `crates/ubiq/src/ui/help/mod.rs` | [`wip/help.md`](../wip/help.md) |
| `crates/ubiq/src/ui/help_target.rs` | [`wip/in-place-help.md`](../wip/in-place-help.md) |
| `crates/ubiq/src/ui/ident.rs` | [`wip/in-place-help.md`](../wip/in-place-help.md) |
| `crates/ubiq/src/ui/kb/mod.rs` | [`components.md`](./components.md), [`wip/kb.md`](../wip/kb.md) |
| `crates/ubiq/src/ui/kb/source_form.rs` | [`wip/kb.md`](../wip/kb.md) |
| `crates/ubiq/src/ui/kit/blocks.rs` | [`features/workbench-sink.md`](../features/workbench-sink.md), [`ui-and-design.md`](./ui-and-design.md), [`wip/teams-layout-spike.md`](../wip/teams-layout-spike.md) |
| `crates/ubiq/src/ui/kit/canvas.rs` | [`ui-and-design.md`](./ui-and-design.md) |
| `crates/ubiq/src/ui/kit/colour.rs` | [`ui-and-design.md`](./ui-and-design.md) |
| `crates/ubiq/src/ui/kit/controls.rs` | [`features/workbench.md`](../features/workbench.md), [`ui-and-design.md`](./ui-and-design.md) |
| `crates/ubiq/src/ui/kit/files.rs` | [`features/workbench-ide.md`](../features/workbench-ide.md), [`ui-and-design.md`](./ui-and-design.md) |
| `crates/ubiq/src/ui/kit/md_navigator.rs` | [`features/workbench-ide.md`](../features/workbench-ide.md), [`ui-and-design.md`](./ui-and-design.md) |
| `crates/ubiq/src/ui/kit/menu.rs` | [`features/workbench-sink.md`](../features/workbench-sink.md), [`features/workbench-teams.md`](../features/workbench-teams.md), [`components.md`](./components.md), [`ui-and-design.md`](./ui-and-design.md), [`wip/clone-a-project.md`](../wip/clone-a-project.md), [`wip/teams-cross-project.md`](../wip/teams-cross-project.md) |
| `crates/ubiq/src/ui/kit/minimap.rs` | [`features/workbench-ide.md`](../features/workbench-ide.md), [`ui-and-design.md`](./ui-and-design.md) |
| `crates/ubiq/src/ui/kit/mod.rs` | [`components.md`](./components.md), [`ui-and-design.md`](./ui-and-design.md) |
| `crates/ubiq/src/ui/kit/overlay.rs` | [`ui-and-design.md`](./ui-and-design.md) |
| `crates/ubiq/src/ui/kit/popover.rs` | [`components.md`](./components.md), [`ui-and-design.md`](./ui-and-design.md) |
| `crates/ubiq/src/ui/kit/ribbon.rs` | [`ui-and-design.md`](./ui-and-design.md) |
| `crates/ubiq/src/ui/kit/settings.rs` | [`features/workbench.md`](../features/workbench.md), [`ui-and-design.md`](./ui-and-design.md) |
| `crates/ubiq/src/ui/logs.rs` | [`features/logs.md`](../features/logs.md), [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/src/ui/mark.rs` | [`features/workbench-ide.md`](../features/workbench-ide.md), [`ui-and-design.md`](./ui-and-design.md) |
| `crates/ubiq/src/ui/mission/full.rs` | [`features/workbench-tasks.md`](../features/workbench-tasks.md) |
| `crates/ubiq/src/ui/mission/menu.rs` | [`features/workbench-tasks.md`](../features/workbench-tasks.md) |
| `crates/ubiq/src/ui/mission/mod.rs` | [`features/workbench-tasks.md`](../features/workbench-tasks.md) |
| `crates/ubiq/src/ui/mission/panel.rs` | [`features/workbench-tasks.md`](../features/workbench-tasks.md) |
| `crates/ubiq/src/ui/mission/settings.rs` | [`features/workbench-tasks.md`](../features/workbench-tasks.md) |
| `crates/ubiq/src/ui/mission/wbs.rs` | [`features/workbench-tasks.md`](../features/workbench-tasks.md) |
| `crates/ubiq/src/ui/mod.rs` | [`ui-and-design.md`](./ui-and-design.md) |
| `crates/ubiq/src/ui/navigator.rs` | [`features/workbench.md`](../features/workbench.md), [`ui-and-design.md`](./ui-and-design.md) |
| `crates/ubiq/src/ui/new_agent.rs` | [`features/workbench-agents.md`](../features/workbench-agents.md) |
| `crates/ubiq/src/ui/new_mission.rs` | [`features/workbench-tasks.md`](../features/workbench-tasks.md) |
| `crates/ubiq/src/ui/new_pane_menu.rs` | [`features/panes-and-terminals.md`](../features/panes-and-terminals.md) |
| `crates/ubiq/src/ui/new_project_menu.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/src/ui/notifications.rs` | [`features/notifications.md`](../features/notifications.md) |
| `crates/ubiq/src/ui/orchestration/graph.rs` | [`features/workbench-teams.md`](../features/workbench-teams.md) |
| `crates/ubiq/src/ui/orchestration/inspector.rs` | [`features/workbench-teams.md`](../features/workbench-teams.md) |
| `crates/ubiq/src/ui/orchestration/mod.rs` | [`features/workbench-teams.md`](../features/workbench-teams.md) |
| `crates/ubiq/src/ui/orchestration/tasks.rs` | [`features/workbench-teams.md`](../features/workbench-teams.md) |
| `crates/ubiq/src/ui/outline.rs` | [`features/workbench.md`](../features/workbench.md), [`ui-and-design.md`](./ui-and-design.md) |
| `crates/ubiq/src/ui/overflow_menu.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/src/ui/plan.rs` | [`features/workbench-tasks.md`](../features/workbench-tasks.md) |
| `crates/ubiq/src/ui/project_face.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/src/ui/project_menu.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/src/ui/rail.rs` | [`features/workbench.md`](../features/workbench.md), [`wip/in-place-help.md`](../wip/in-place-help.md) |
| `crates/ubiq/src/ui/remote_connect.rs` | [`features/drone.md`](../features/drone.md), [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/src/ui/remote_hosts.rs` | [`ui-and-design.md`](./ui-and-design.md) |
| `crates/ubiq/src/ui/ribbon.rs` | [`features/workbench-git.md`](../features/workbench-git.md), [`features/workbench.md`](../features/workbench.md), [`ui-and-design.md`](./ui-and-design.md) |
| `crates/ubiq/src/ui/settings.rs` | [`features/connectors.md`](../features/connectors.md), [`features/drone.md`](../features/drone.md), [`features/workbench.md`](../features/workbench.md), [`components.md`](./components.md), [`ui-and-design.md`](./ui-and-design.md), [`wip/indexing.md`](../wip/indexing.md) |
| `crates/ubiq/src/ui/shell.rs` | [`features/panes-and-terminals.md`](../features/panes-and-terminals.md), [`features/workbench.md`](../features/workbench.md), [`ui-and-design.md`](./ui-and-design.md), [`wip/web-panel-phase6.md`](../wip/web-panel-phase6.md) |
| `crates/ubiq/src/ui/sink/a2ui.rs` | [`features/workbench-sink.md`](../features/workbench-sink.md) |
| `crates/ubiq/src/ui/sink/docs.rs` | [`features/workbench-sink.md`](../features/workbench-sink.md) |
| `crates/ubiq/src/ui/sink/ext_demo.rs` | [`features/workbench-sink.md`](../features/workbench-sink.md), [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/src/ui/sink/files.rs` | [`features/workbench-sink.md`](../features/workbench-sink.md) |
| `crates/ubiq/src/ui/sink/mod.rs` | [`features/workbench-sink.md`](../features/workbench-sink.md) |
| `crates/ubiq/src/ui/sink/project.rs` | [`features/drone.md`](../features/drone.md), [`features/workbench-sink.md`](../features/workbench-sink.md), [`features/workbench.md`](../features/workbench.md), [`wip/indexing.md`](../wip/indexing.md), [`wip/kb.md`](../wip/kb.md) |
| `crates/ubiq/src/ui/sink/script.rs` | [`features/workbench-sink.md`](../features/workbench-sink.md) |
| `crates/ubiq/src/ui/sink/settings.rs` | [`features/workbench-sink.md`](../features/workbench-sink.md) |
| `crates/ubiq/src/ui/sink/style.rs` | [`features/workbench-sink.md`](../features/workbench-sink.md), [`ui-and-design.md`](./ui-and-design.md) |
| `crates/ubiq/src/ui/sink/teamsim.rs` | [`features/workbench-sink.md`](../features/workbench-sink.md), [`features/workbench-teams.md`](../features/workbench-teams.md), [`wip/teams-layout-spike.md`](../wip/teams-layout-spike.md) |
| `crates/ubiq/src/ui/size.rs` | [`features/workbench.md`](../features/workbench.md), [`ui-and-design.md`](./ui-and-design.md) |
| `crates/ubiq/src/ui/stats.rs` | [`features/stats.md`](../features/stats.md) |
| `crates/ubiq/src/ui/status_bar.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/src/ui/tab_menu.rs` | [`features/panes-and-terminals.md`](../features/panes-and-terminals.md), [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/src/ui/tasksrc.rs` | [`features/workbench-tasks.md`](../features/workbench-tasks.md), [`inbox/task-sources-proposal.md`](../inbox/task-sources-proposal.md) |
| `crates/ubiq/src/ui/teams/graph.rs` | [`features/workbench-teams.md`](../features/workbench-teams.md), [`inbox/subagent-status-dictionaries-and-hexagon.md`](../inbox/subagent-status-dictionaries-and-hexagon.md), [`components.md`](./components.md), [`wip/teams-cross-project.md`](../wip/teams-cross-project.md), [`wip/teams-layout-spike.md`](../wip/teams-layout-spike.md) |
| `crates/ubiq/src/ui/teams/mod.rs` | [`features/workbench-teams.md`](../features/workbench-teams.md), [`wip/teams-cross-project.md`](../wip/teams-cross-project.md) |
| `crates/ubiq/src/ui/teams/status.rs` | [`features/workbench-agents.md`](../features/workbench-agents.md), [`features/workbench-teams.md`](../features/workbench-teams.md), [`ui-and-design.md`](./ui-and-design.md) |
| `crates/ubiq/src/ui/teams/tasks.rs` | [`features/workbench-teams.md`](../features/workbench-teams.md) |
| `crates/ubiq/src/ui/terminal.rs` | [`features/panes-and-terminals.md`](../features/panes-and-terminals.md), [`features/workbench.md`](../features/workbench.md), [`ui-and-design.md`](./ui-and-design.md) |
| `crates/ubiq/src/ui/titlebar.rs` | [`features/notifications.md`](../features/notifications.md), [`features/workbench.md`](../features/workbench.md), [`ui-and-design.md`](./ui-and-design.md), [`wip/in-place-help.md`](../wip/in-place-help.md) |
| `crates/ubiq/src/ui/tools.rs` | [`features/panes-and-terminals.md`](../features/panes-and-terminals.md), [`components.md`](./components.md) |
| `crates/ubiq/src/ui/viewer/diagram.rs` | [`features/workbench-ide.md`](../features/workbench-ide.md), [`wip/web-panel-phase6.md`](../wip/web-panel-phase6.md) |
| `crates/ubiq/src/ui/viewer/diff.rs` | [`features/workbench-git.md`](../features/workbench-git.md), [`features/workbench-ide.md`](../features/workbench-ide.md), [`components.md`](./components.md), [`ui-and-design.md`](./ui-and-design.md) |
| `crates/ubiq/src/ui/viewer/image.rs` | [`features/workbench-ide.md`](../features/workbench-ide.md) |
| `crates/ubiq/src/ui/viewer/image_edit.rs` | [`features/workbench-ide.md`](../features/workbench-ide.md) |
| `crates/ubiq/src/ui/viewer/markdown.rs` | [`features/workbench-ide.md`](../features/workbench-ide.md) |
| `crates/ubiq/src/ui/viewer/md_options.rs` | [`features/workbench-ide.md`](../features/workbench-ide.md), [`components.md`](./components.md), [`ui-and-design.md`](./ui-and-design.md) |
| `crates/ubiq/src/ui/viewer/mod.rs` | [`features/workbench-ide.md`](../features/workbench-ide.md), [`components.md`](./components.md), [`ui-and-design.md`](./ui-and-design.md), [`wip/web-panel-phase45.md`](../wip/web-panel-phase45.md), [`wip/web-panel-phase6.md`](../wip/web-panel-phase6.md) |
| `crates/ubiq/src/ui/viewer/scene.rs` | [`features/workbench-ide.md`](../features/workbench-ide.md), [`ui-and-design.md`](./ui-and-design.md) |
| `crates/ubiq/src/ui/viewer/viewport.rs` | [`features/workbench-ide.md`](../features/workbench-ide.md) |
| `crates/ubiq/src/ui/viewer/web.rs` | [`features/workbench-ide.md`](../features/workbench-ide.md), [`wip/web-panel-phase6.md`](../wip/web-panel-phase6.md) |
| `crates/ubiq/src/ui/web_view.rs` | [`features/workbench-ide.md`](../features/workbench-ide.md), [`wip/web-panel-phase6.md`](../wip/web-panel-phase6.md) |
| `crates/ubiq/src/ui/work.rs` | [`features/workbench-agents.md`](../features/workbench-agents.md), [`ui-and-design.md`](./ui-and-design.md) |
| `crates/ubiq/src/version.rs` | [`architecture.md`](./architecture.md) |
| `crates/ubiq/src/web_export/archive.rs` | [`wip/web-panel-phase2.md`](../wip/web-panel-phase2.md), [`wip/web-panel-phase45.md`](../wip/web-panel-phase45.md) |
| `crates/ubiq/src/web_export/assets.rs` | [`wip/web-panel-phase2.md`](../wip/web-panel-phase2.md), [`wip/web-panel-phase45.md`](../wip/web-panel-phase45.md) |
| `crates/ubiq/src/web_export/bridge.rs` | [`wip/web-panel-phase2.md`](../wip/web-panel-phase2.md), [`wip/web-panel-phase6.md`](../wip/web-panel-phase6.md) |
| `crates/ubiq/src/web_export/mod.rs` | [`architecture.md`](./architecture.md) |
| `crates/ubiq/src/web_export/routes.rs` | [`wip/web-panel-phase2.md`](../wip/web-panel-phase2.md), [`wip/web-panel-phase45.md`](../wip/web-panel-phase45.md) |
| `crates/ubiq/src/web_export/server.rs` | [`wip/web-panel-phase2.md`](../wip/web-panel-phase2.md) |
| `crates/ubiq/tests/a2ui.rs` | [`features/workbench-sink.md`](../features/workbench-sink.md) |
| `crates/ubiq/tests/agents.rs` | [`features/workbench-agents.md`](../features/workbench-agents.md) |
| `crates/ubiq/tests/board.rs` | [`features/workbench-tasks.md`](../features/workbench-tasks.md) |
| `crates/ubiq/tests/bookmarks.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/tests/conversation.rs` | [`features/workbench-agents.md`](../features/workbench-agents.md) |
| `crates/ubiq/tests/diagrams.rs` | [`features/workbench-ide.md`](../features/workbench-ide.md) |
| `crates/ubiq/tests/dismiss.rs` | [`ui-and-design.md`](./ui-and-design.md) |
| `crates/ubiq/tests/dock.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/tests/explorer.rs` | [`features/workbench-ide.md`](../features/workbench-ide.md) |
| `crates/ubiq/tests/file_picker.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/tests/files_changed.rs` | [`features/workbench-ide.md`](../features/workbench-ide.md) |
| `crates/ubiq/tests/git.rs` | [`features/workbench-git.md`](../features/workbench-git.md) |
| `crates/ubiq/tests/image_gestures.rs` | [`features/workbench-ide.md`](../features/workbench-ide.md) |
| `crates/ubiq/tests/kb.rs` | [`wip/kb.md`](../wip/kb.md) |
| `crates/ubiq/tests/mission.rs` | [`features/workbench-tasks.md`](../features/workbench-tasks.md) |
| `crates/ubiq/tests/mode_restore.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/tests/nav.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/tests/nav_text.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/tests/navigator.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/tests/new_mission.rs` | [`features/workbench-tasks.md`](../features/workbench-tasks.md) |
| `crates/ubiq/tests/new_project.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/tests/orchestration.rs` | [`features/workbench-teams.md`](../features/workbench-teams.md) |
| `crates/ubiq/tests/plan.rs` | [`features/workbench-tasks.md`](../features/workbench-tasks.md) |
| `crates/ubiq/tests/prefs.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/tests/rail_container.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/tests/scene.rs` | [`features/workbench-ide.md`](../features/workbench-ide.md) |
| `crates/ubiq/tests/script.rs` | [`features/workbench-sink.md`](../features/workbench-sink.md) |
| `crates/ubiq/tests/settings.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/tests/settings_container.rs` | [`features/workbench.md`](../features/workbench.md) |
| `crates/ubiq/tests/sink.rs` | [`features/workbench-sink.md`](../features/workbench-sink.md) |
| `crates/ubiq/tests/stats.rs` | [`features/stats.md`](../features/stats.md) |
| `crates/ubiq/tests/tasksrc.rs` | [`features/workbench-tasks.md`](../features/workbench-tasks.md), [`inbox/task-sources-proposal.md`](../inbox/task-sources-proposal.md) |
| `crates/ubiq/tests/teams.rs` | [`features/workbench-teams.md`](../features/workbench-teams.md), [`wip/teams-cross-project.md`](../wip/teams-cross-project.md) |
| `crates/ubiq/tests/ui_id.rs` | [`wip/in-place-help.md`](../wip/in-place-help.md) |
| `crates/ubiq/tests/viewer_kind.rs` | [`features/workbench-ide.md`](../features/workbench-ide.md) |
| `crates/ubiq/tests/viewport.rs` | [`features/workbench-ide.md`](../features/workbench-ide.md) |
| `crates/ubiq/tests/vim.rs` | [`features/workbench.md`](../features/workbench.md) |
| `vendor/gpui-terminal/Cargo.toml` | [`project-structure.md`](./project-structure.md) |
| `vendor/gpui-terminal/src/clipboard.rs` | [`features/panes-and-terminals.md`](../features/panes-and-terminals.md) |
| `vendor/gpui-terminal/src/event.rs` | [`features/panes-and-terminals.md`](../features/panes-and-terminals.md) |
| `vendor/gpui-terminal/src/input.rs` | [`features/panes-and-terminals.md`](../features/panes-and-terminals.md) |
| `vendor/gpui-terminal/src/mouse.rs` | [`features/panes-and-terminals.md`](../features/panes-and-terminals.md) |
| `vendor/gpui-terminal/src/render.rs` | [`features/panes-and-terminals.md`](../features/panes-and-terminals.md) |
| `vendor/gpui-terminal/src/terminal.rs` | [`features/panes-and-terminals.md`](../features/panes-and-terminals.md) |
| `vendor/gpui-terminal/src/view.rs` | [`features/panes-and-terminals.md`](../features/panes-and-terminals.md) |

## Unanchored

No document's `code_anchors` names these. Restricted to Ubiq's own crates.

| File |
|---|
| `crates/ubiq-app/src/handoff.rs` |
| `crates/ubiq-host/src/assist/apple.rs` |
| `crates/ubiq-host/src/atomic.rs` |
| `crates/ubiq-host/src/config.rs` |
| `crates/ubiq-host/src/connectors/http.rs` |
| `crates/ubiq-host/src/drone/mod.rs` |
| `crates/ubiq-host/src/files/browse.rs` |
| `crates/ubiq-host/src/files/path.rs` |
| `crates/ubiq-host/src/files/related.rs` |
| `crates/ubiq-host/src/gc.rs` |
| `crates/ubiq-host/src/health.rs` |
| `crates/ubiq-host/src/help/archive.rs` |
| `crates/ubiq-host/src/help/mod.rs` |
| `crates/ubiq-host/src/host_meta.rs` |
| `crates/ubiq-host/src/host_path.rs` |
| `crates/ubiq-host/src/plan/lines.rs` |
| `crates/ubiq-host/src/plan/provenance.rs` |
| `crates/ubiq-host/src/reply.rs` |
| `crates/ubiq-host/src/search/ceiling.rs` |
| `crates/ubiq-host/src/search/fallback.rs` |
| `crates/ubiq-host/src/search/mod.rs` |
| `crates/ubiq-host/src/search/walk.rs` |
| `crates/ubiq-host/src/web_assets/manifest_drawio.rs` |
| `crates/ubiq-host/src/work/mock.rs` |
| `crates/ubiq-proto/src/acp.rs` |
| `crates/ubiq-proto/src/help.rs` |
| `crates/ubiq-proto/src/search.rs` |
| `crates/ubiq/src/app/host_secrets.rs` |
| `crates/ubiq/src/app/remote_hosts.rs` |
| `crates/ubiq/src/app/themes.rs` |
| `crates/ubiq/src/state/explorer/filter.rs` |
| `crates/ubiq/src/state/explorer/keys.rs` |
| `crates/ubiq/src/state/remote_hosts.rs` |
| `crates/ubiq/src/state/search.rs` |
| `crates/ubiq/src/state/zoom.rs` |
| `crates/ubiq/src/ui/kit/icons.rs` |
| `crates/ubiq/src/ui/kit/panel.rs` |
| `crates/ubiq/src/ui/kit/slider.rs` |
| `crates/ubiq/src/ui/run_tool_menu.rs` |
| `crates/ubiq/src/ui/search.rs` |
| `crates/ubiq/src/ui/sink/messages.rs` |
| `crates/ubiq/src/ui/themes.rs` |
| `crates/ubiq/src/ui/viewer/zoom_modal.rs` |

<!-- generated:end anchors -->

## Related docs

- [`project-structure.md`](./project-structure.md) — what belongs in each folder, and what never does
- [`architecture.md`](./architecture.md) — why the modules divide this way
- `_docs/_meta/authoring.md` — the duty the anchor table exists to serve
