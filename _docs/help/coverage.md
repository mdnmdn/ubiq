---
id: help-coverage
title: Help coverage
kind: help
status: draft
summary: The living table of every interface area, the help page that covers it, its context key and its state.
read_when: you are choosing which help page to write next, or recording that one is written
updated: 2026-10-09
verified: 2026-10-09
code_anchors: [help/manifest.toml, _tools/helpbundle.py, crates/ubiq/src/state/ui_id.rs]
depends_on: [wip-help, wip-help-content]
---

# Help coverage

This document owns the living coverage table: every area from the id inventory, the page that covers it, its context key and its state (`missing`, `stub` or `done`).

## How to read this table

Each row is one area from `_docs/inbox/ui-id-inventory.md` §4, plus the rail modes from §4.1. The State column reads `missing` when no help page covers the area, `stub` when a page covers part of it, and `done` when a page covers it in full. Dialogs and settings sections reach help through rung 1, a `help_page()` call on the dialog or section, so their Context key column reads `— (rung 1)`. The Screenshot column names the page that carries a screenshot of the area, so a UI change can find the page to re-shoot; `—` means no page carries one.

## Coverage

| Area | Inventory § | Page | Context key | State | Screenshot |
|---|---|---|---|---|---|
| titlebar | §4.1 (CATALOGUE) | workbench/titlebar | — | done | — |
| rail | §4.1 (CATALOGUE) | workbench/rail | — | done | — |
| dock | §4.1 | workbench/panels | — | stub | — |
| dock.centre | §4.1 | workbench/panels | — | stub | — |
| dock.left | §4.1 | workbench/panels | — | stub | — |
| dock.right | §4.1 | workbench/panels | — | stub | — |
| dock.bottom | §4.1 | workbench/panels | — | stub | — |
| status-bar | §4.1 | — | — | missing | — |
| outline | §4.1 | — | panel.ubiq.outline | missing | — |
| terminal | §4.1 (instance) | concepts/panes | — | done | — |
| editor.file | §4.1 (instance) | workbench/editor | panel.ubiq.file | stub | — |
| viewer.diff | §4.1 | git/diff | panel.ubiq.git.diff | stub | — |
| explorer | §4.2 | workbench/explorer | panel.ubiq.explorer | done | — |
| explorer.toolbar | §4.2 | workbench/explorer | panel.ubiq.explorer | stub | — |
| explorer.bookmarks | §4.2 | — | — | missing | — |
| explorer.tree | §4.2 | workbench/explorer | panel.ubiq.explorer | done | — |
| search | §4.2 | workbench/search | panel.ubiq.search | stub | — |
| search.results | §4.2 | workbench/search | panel.ubiq.search | stub | — |
| git.toolbar | §4.2 | git/overview | rail.git | stub | — |
| git.refs.repositories | §4.2 | git/refs | panel.ubiq.git.refs | stub | — |
| git.refs | §4.2 | git/refs | panel.ubiq.git.refs | done | — |
| git.history | §4.2 | git/history | panel.ubiq.git.history | done | — |
| git.changes | §4.2 | git/changes | panel.ubiq.git.changes | done | — |
| git.changes.commit-box | §4.2 | git/changes | panel.ubiq.git.changes | done | — |
| git.changes.commit | §4.2 | git/changes | panel.ubiq.git.changes | stub | — |
| git.changes.range | §4.2 | — | — | missing | — |
| git.diff | §4.2 | git/diff | panel.ubiq.git.diff | stub | — |
| git.diff.header | §4.2 | git/diff | panel.ubiq.git.diff | stub | — |
| kb | §4.2 | kb/overview | rail.kb | done | — |
| kb.tree | §4.2 | kb/overview | panel.ubiq.kb.explorer | done | — |
| kb.centre | §4.2 | kb/overview | — | stub | — |
| kb.document | §4.2 | kb/overview | — | stub | — |
| kb.source-form | §4.2 | kb/overview | — | stub | — |
| logs | §4.2 | workbench/panels | panel.ubiq.logs | stub | — |
| logs.toolbar | §4.2 | workbench/panels | panel.ubiq.logs | stub | — |
| logs.body | §4.2 | workbench/panels | panel.ubiq.logs | stub | — |
| control | §4.2 | workbench/modes | rail.control | stub | — |
| control.general | §4.2 | — | — | missing | — |
| control.usage | §4.2 | — | — | missing | — |
| notifications | §4.2 | workbench/titlebar | — | stub | — |
| notifications.picker | §4.2 | — | — | missing | — |
| notifications.rules | §4.2 | — | — | missing | — |
| all-projects | §4.2 | — | — | missing | — |
| agents.screen | §4.3 | agents/starting | rail.agents | stub | — |
| agents.header | §4.3 | agents/starting | rail.agents | stub | — |
| agents.columns | §4.3 | agents/starting | rail.agents | stub | — |
| agents.column.header | §4.3 | agents/starting | — | stub | — |
| agents.column.footer | §4.3 | — | — | missing | — |
| agents.column.thread | §4.3 | — | — | missing | — |
| agents.sidebar | §4.3 | concepts/sessions | — | stub | — |
| chat.tab.header | §4.3 | agents/chat | view.chat | stub | — |
| conversation.header | §4.3 | — | — | missing | — |
| conversation.transcript | §4.3 | agents/chat | view.chat | stub | — |
| conversation.queue | §4.3 | — | — | missing | — |
| conversation.activity-bar | §4.3 | — | — | missing | — |
| conversation.composer | §4.3 | — | — | missing | — |
| conversation.footer | §4.3 | — | — | missing | — |
| conversation.info | §4.3 | — | — | missing | — |
| acp-capabilities.panel | §4.3 | — | — | missing | — |
| new-agent.modal | §4.3 | agents/starting | — (rung 1) | stub | — |
| new-agent.naming | §4.3 | — | — | missing | — |
| tasks.screen | §4.3 | workbench/modes | rail.tasks | stub | — |
| tasks.toolbar | §4.3 | — | — | missing | — |
| tasks.columns | §4.3 | workbench/modes | rail.tasks | stub | — |
| tasks.detail | §4.3 | — | panel.ubiq.task | missing | — |
| graph.screen | §4.3 | — | — | missing | — |
| graph.toolbar | §4.3 | — | — | missing | — |
| graph.canvas | §4.3 | — | — | missing | — |
| graph.inspector | §4.3 | — | — | missing | — |
| graph.tasks | §4.3 | — | — | missing | — |
| teams.screen | §4.3 | workbench/rail | rail.teams | stub | — |
| teams.toolbar | §4.3 | — | — | missing | — |
| teams.canvas | §4.3 | — | — | missing | — |
| teams.inspector | §4.3 | — | — | missing | — |
| teams.tasks | §4.3 | — | — | missing | — |
| tools.panel | §4.3 | — | — | missing | — |
| help | §4.4 | troubleshooting | panel.ubiq.help | stub | — |
| help.target | §4.4 | — | — | missing | — |
| settings | §4.4 | workbench/titlebar | — (rung 1) | stub | — |
| settings.appearance | §4.4 | workbench/titlebar | — (rung 1) | stub | — |
| settings.size | §4.4 | — | — (rung 1) | missing | — |
| settings.file-explorer | §4.4 | — | — (rung 1) | missing | — |
| settings.editor | §4.4 | — | — (rung 1) | missing | — |
| settings.search | §4.4 | — | — (rung 1) | missing | — |
| settings.harnesses | §4.4 | agents/accounts | — (rung 1) | stub | — |
| settings.isolation | §4.4 | — | — (rung 1) | missing | — |
| settings.assist | §4.4 | — | — (rung 1) | missing | — |
| settings.connectors | §4.4 | — | — (rung 1) | missing | — |
| settings.hosts | §4.4 | workbench/titlebar | — (rung 1) | stub | — |
| settings.ssh | §4.4 | — | — (rung 1) | missing | — |
| settings.drones | §4.4 | — | — (rung 1) | missing | — |
| settings.tools | §4.4 | — | — (rung 1) | missing | — |
| settings.command-line | §4.4 | — | — (rung 1) | missing | — |
| project-settings | §4.4 | workbench/modes | — (rung 1) | stub | — |
| project-settings.general | §4.4 | concepts/projects | — (rung 1) | stub | — |
| project-settings.remote | §4.4 | — | — (rung 1) | missing | — |
| project-settings.tasks | §4.4 | — | — (rung 1) | missing | — |
| project-settings.kb | §4.4 | — | — (rung 1) | missing | — |
| project-settings.tools | §4.4 | — | — (rung 1) | missing | — |
| project-settings.documentation | §4.4 | — | — (rung 1) | missing | — |
| project-settings.integrations | §4.4 | — | — (rung 1) | missing | — |
| sink | §4.4 | workbench/modes | rail.sink | stub | — |
| sink.<page> (twelve) | §4.4 | — | — | missing | — |
| sink.style.<block> (ten) | §4.4 | — | — | missing | — |
| dialog.login | §4.4 | agents/accounts | — (rung 1) | stub | — |
| dialog.profile-form | §4.4 | — | — (rung 1) | missing | — |
| dialog.account | §4.4 | agents/accounts | — (rung 1) | stub | — |
| dialog.connect | §4.4 | — | — (rung 1) | missing | — |
| dialog.app-form | §4.4 | — | — (rung 1) | missing | — |
| dialog.connector | §4.4 | — | — (rung 1) | missing | — |
| dialog.certificate | §4.4 | — | — (rung 1) | missing | — |
| dialog.ai-form | §4.4 | — | — (rung 1) | missing | — |
| dialog.ai-test | §4.4 | — | — (rung 1) | missing | — |
| dialog.ai-remove | §4.4 | — | — (rung 1) | missing | — |
| dialog.ssh-form | §4.4 | — | — (rung 1) | missing | — |
| dialog.ssh-remove | §4.4 | — | — (rung 1) | missing | — |
| dialog.drone-stop | §4.4 | — | — (rung 1) | missing | — |
| dialog.clone | §4.4 | kb/overview | — (rung 1) | stub | — |
| dialog.feedback | §4.4 | workbench/titlebar | — (rung 1) | stub | — |
| dialog.remote-connect | §4.4 | — | — (rung 1) | missing | — |
| dialog.remote-manager | §4.4 | workbench/titlebar | — (rung 1) | stub | — |
| dialog.file | §4.4 | — | — (rung 1) | missing | — |
| dialog.file-picker | §4.4 | — | — (rung 1) | missing | — |
| dialog.theme-editor | §4.4 | — | — (rung 1) | missing | — |
| dialog.theme-naming | §4.4 | — | — (rung 1) | missing | — |
| dialog.size-naming | §4.4 | — | — (rung 1) | missing | — |
| dialog.close-pane | §4.4 | concepts/panes | — (rung 1) | stub | — |
| dialog.end-conversation | §4.4 | — | — (rung 1) | missing | — |
| rail.mode.ide | §4.1 (rail; not in §4) | workbench/modes | rail.ide | stub | — |
| rail.mode.db | §4.1 (rail; not in §4) | db/overview | rail.db | done | — |
| rail.mode.git | §4.1 (rail) | git/overview | rail.git | done | — |
| rail.mode.kb | §4.1 (rail) | kb/overview | rail.kb | done | — |
| rail.mode.agents | §4.1 (rail) | agents/starting | rail.agents | stub | — |
| rail.mode.teams | §4.1 (rail) | — | rail.teams | missing | — |
| rail.mode.teamsall | §4.1 (rail; not in §4) | — | rail.teamsall | missing | — |
| rail.mode.tasks | §4.1 (rail) | workbench/modes | rail.tasks | stub | — |
| rail.mode.control | §4.1 (rail) | workbench/modes | rail.control | stub | — |
| rail.mode.sink | §4.1 (rail) | workbench/modes | rail.sink | stub | — |

**Count: 71 missing / 53 stub / 14 done** (138 rows).

## Related docs

- [`wip/help.md`](../wip/help.md) — how the help machinery works
- [`wip/help-content.md`](../wip/help-content.md) — the programme that writes the manual
