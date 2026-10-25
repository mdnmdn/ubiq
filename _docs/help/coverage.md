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

This document owns the living coverage table: every area from the id inventory, the page that covers it, its context key and its state (`missing`, `planned`, `stub` or `done`).

## How to read this table

Each row is one area from `_docs/inbox/ui-id-inventory.md` §4, plus the rail modes from §4.1. The State column reads `missing` when no help page covers the area, `planned` when a stub page exists in the nav with its frontmatter and a one-paragraph promise but no body yet, `stub` when a page covers part of it, and `done` when a page covers it in full. Dialogs and settings sections reach help through rung 1, a `help_page()` call on the dialog or section, so their Context key column reads `— (rung 1)`. The Screenshot column names the page that carries a screenshot of the area, so a UI change can find the page to re-shoot; `—` means no page carries one.

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
| status-bar | §4.1 | workbench/status-bar | — | planned | — |
| outline | §4.1 | workbench/outline | panel.ubiq.outline | planned | — |
| terminal | §4.1 (instance) | concepts/panes | — | done | — |
| editor.file | §4.1 (instance) | workbench/editor | panel.ubiq.file | stub | — |
| viewer.diff | §4.1 | git/diff | panel.ubiq.git.diff | stub | — |
| explorer | §4.2 | workbench/explorer | panel.ubiq.explorer | done | — |
| explorer.toolbar | §4.2 | workbench/explorer | panel.ubiq.explorer | stub | — |
| explorer.bookmarks | §4.2 | workbench/bookmarks | — | planned | — |
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
| git.changes.range | §4.2 | git/compare-commits | — | planned | — |
| git.diff | §4.2 | git/diff | panel.ubiq.git.diff | stub | — |
| git.diff.header | §4.2 | git/diff | panel.ubiq.git.diff | stub | — |
| kb | §4.2 | kb/overview | rail.kb | done | — |
| kb.tree | §4.2 | kb/overview | panel.ubiq.kb.explorer | done | — |
| kb.centre | §4.2 | kb/documents | panel.ubiq.kb.doc | planned | — |
| kb.document | §4.2 | kb/documents | panel.ubiq.kb.doc | planned | — |
| kb.source-form | §4.2 | kb/sources | — | planned | — |
| logs | §4.2 | workbench/logs | panel.ubiq.logs | planned | — |
| logs.toolbar | §4.2 | workbench/logs | panel.ubiq.logs | planned | — |
| logs.body | §4.2 | workbench/logs | panel.ubiq.logs | planned | — |
| control | §4.2 | workbench/control | rail.control | planned | — |
| control.general | §4.2 | workbench/control | rail.control | planned | — |
| control.usage | §4.2 | workbench/control | rail.control | planned | — |
| notifications | §4.2 | workbench/notifications | — | planned | — |
| notifications.picker | §4.2 | workbench/notifications | — | planned | — |
| notifications.rules | §4.2 | workbench/notifications | — | planned | — |
| all-projects | §4.2 | workbench/all-projects | — | planned | — |
| agents.screen | §4.3 | agents/starting | rail.agents | stub | — |
| agents.header | §4.3 | agents/starting | rail.agents | stub | — |
| agents.columns | §4.3 | agents/starting | rail.agents | stub | — |
| agents.column.header | §4.3 | agents/starting | — | stub | — |
| agents.column.footer | §4.3 | agents/columns | panel.ubiq.agents.explorer | planned | — |
| agents.column.thread | §4.3 | agents/columns | panel.ubiq.agents.explorer | planned | — |
| agents.sidebar | §4.3 | agents/columns | panel.ubiq.agents.explorer | planned | — |
| chat.tab.header | §4.3 | agents/chat | view.chat | stub | — |
| conversation.header | §4.3 | agents/conversation | — | planned | — |
| conversation.transcript | §4.3 | agents/chat | view.chat | stub | — |
| conversation.queue | §4.3 | agents/conversation | — | planned | — |
| conversation.activity-bar | §4.3 | agents/conversation | — | planned | — |
| conversation.composer | §4.3 | agents/conversation | — | planned | — |
| conversation.footer | §4.3 | agents/conversation | — | planned | — |
| conversation.info | §4.3 | agents/conversation | — | planned | — |
| acp-capabilities.panel | §4.3 | agents/conversation | — | planned | — |
| new-agent.modal | §4.3 | agents/new-agent | — (rung 1) | planned | — |
| new-agent.naming | §4.3 | agents/new-agent | — (rung 1) | planned | — |
| tasks.screen | §4.3 | tasks/overview | rail.tasks | planned | — |
| tasks.toolbar | §4.3 | tasks/overview | rail.tasks | planned | — |
| tasks.columns | §4.3 | tasks/overview | rail.tasks | planned | — |
| tasks.detail | §4.3 | tasks/detail | panel.ubiq.task | planned | — |
| graph.screen | §4.3 | teams/overview | rail.teams | planned | — |
| graph.toolbar | §4.3 | teams/overview | rail.teams | planned | — |
| graph.canvas | §4.3 | teams/overview | rail.teams | planned | — |
| graph.inspector | §4.3 | teams/inspector | — | planned | — |
| graph.tasks | §4.3 | teams/inspector | — | planned | — |
| teams.screen | §4.3 | teams/overview | rail.teams | planned | — |
| teams.toolbar | §4.3 | teams/overview | rail.teams | planned | — |
| teams.canvas | §4.3 | teams/overview | rail.teams | planned | — |
| teams.inspector | §4.3 | teams/inspector | — | planned | — |
| teams.tasks | §4.3 | teams/inspector | — | planned | — |
| tools.panel | §4.3 | workbench/tools | — | planned | — |
| help | §4.4 | workbench/help | panel.ubiq.help | planned | — |
| help.target | §4.4 | workbench/help | panel.ubiq.help | planned | — |
| settings | §4.4 | settings/overview | — (rung 1) | planned | — |
| settings.appearance | §4.4 | settings/appearance | — (rung 1) | planned | — |
| settings.size | §4.4 | settings/appearance | — (rung 1) | planned | — |
| settings.file-explorer | §4.4 | settings/files-and-search | — (rung 1) | planned | — |
| settings.editor | §4.4 | settings/files-and-search | — (rung 1) | planned | — |
| settings.search | §4.4 | settings/files-and-search | — (rung 1) | planned | — |
| settings.harnesses | §4.4 | settings/harnesses | — (rung 1) | planned | — |
| settings.isolation | §4.4 | settings/harnesses | — (rung 1) | planned | — |
| settings.assist | §4.4 | settings/assist | — (rung 1) | planned | — |
| settings.connectors | §4.4 | settings/connectors | — (rung 1) | planned | — |
| settings.hosts | §4.4 | settings/hosts | — (rung 1) | planned | — |
| settings.ssh | §4.4 | settings/hosts | — (rung 1) | planned | — |
| settings.drones | §4.4 | settings/hosts | — (rung 1) | planned | — |
| settings.tools | §4.4 | settings/tools | — (rung 1) | planned | — |
| settings.command-line | §4.4 | settings/tools | — (rung 1) | planned | — |
| project-settings | §4.4 | settings/project | — (rung 1) | planned | — |
| project-settings.general | §4.4 | settings/project | — (rung 1) | planned | — |
| project-settings.remote | §4.4 | settings/project | — (rung 1) | planned | — |
| project-settings.tasks | §4.4 | settings/project | — (rung 1) | planned | — |
| project-settings.kb | §4.4 | settings/project | — (rung 1) | planned | — |
| project-settings.tools | §4.4 | settings/project | — (rung 1) | planned | — |
| project-settings.documentation | §4.4 | settings/project | — (rung 1) | planned | — |
| project-settings.integrations | §4.4 | settings/project | — (rung 1) | planned | — |
| dialog.login | §4.4 | agents/accounts | — (rung 1) | stub | — |
| dialog.profile-form | §4.4 | agents/profiles | — (rung 1) | planned | — |
| dialog.account | §4.4 | agents/accounts | — (rung 1) | stub | — |
| dialog.connect | §4.4 | settings/connectors | — (rung 1) | planned | — |
| dialog.app-form | §4.4 | settings/connectors | — (rung 1) | planned | — |
| dialog.connector | §4.4 | settings/connectors | — (rung 1) | planned | — |
| dialog.certificate | §4.4 | settings/connectors | — (rung 1) | planned | — |
| dialog.ai-form | §4.4 | settings/assist | — (rung 1) | planned | — |
| dialog.ai-test | §4.4 | settings/assist | — (rung 1) | planned | — |
| dialog.ai-remove | §4.4 | settings/assist | — (rung 1) | planned | — |
| dialog.ssh-form | §4.4 | settings/hosts | — (rung 1) | planned | — |
| dialog.ssh-remove | §4.4 | settings/hosts | — (rung 1) | planned | — |
| dialog.drone-stop | §4.4 | settings/hosts | — (rung 1) | planned | — |
| dialog.clone | §4.4 | kb/overview | — (rung 1) | stub | — |
| dialog.feedback | §4.4 | workbench/feedback | — (rung 1) | planned | — |
| dialog.remote-connect | §4.4 | remote/connect | — (rung 1) | planned | — |
| dialog.remote-manager | §4.4 | remote/overview | — (rung 1) | planned | — |
| dialog.file | §4.4 | workbench/file-dialogs | — (rung 1) | planned | — |
| dialog.file-picker | §4.4 | workbench/file-dialogs | — (rung 1) | planned | — |
| dialog.theme-editor | §4.4 | settings/appearance | — (rung 1) | planned | — |
| dialog.theme-naming | §4.4 | settings/appearance | — (rung 1) | planned | — |
| dialog.size-naming | §4.4 | settings/appearance | — (rung 1) | planned | — |
| dialog.close-pane | §4.4 | concepts/panes | — (rung 1) | stub | — |
| dialog.end-conversation | §4.4 | how-to/hide-or-close-a-tab | — (rung 1) | planned | — |
| rail.mode.ide | §4.1 (rail; not in §4) | workbench/modes | rail.ide | stub | — |
| rail.mode.db | §4.1 (rail; not in §4) | db/overview | rail.db | done | — |
| rail.mode.git | §4.1 (rail) | git/overview | rail.git | done | — |
| rail.mode.kb | §4.1 (rail) | kb/overview | rail.kb | done | — |
| rail.mode.agents | §4.1 (rail) | agents/starting | rail.agents | stub | — |
| rail.mode.teams | §4.1 (rail) | teams/overview | rail.teams | planned | — |
| rail.mode.teamsall | §4.1 (rail; not in §4) | teams/overview | rail.teamsall | planned | — |
| rail.mode.tasks | §4.1 (rail) | tasks/overview | rail.tasks | planned | — |
| rail.mode.control | §4.1 (rail) | workbench/control | rail.control | planned | — |

The kitchen sink (Sink mode, its pages and style blocks) is out of scope: it is not user-visible, so it has no help page and no rows here. The `graph.*` rows are the earlier orchestration screen, which Teams replaced; they point at the Teams pages.

**Count: 0 missing / 93 planned / 27 stub / 14 done** (134 rows).

## Related docs

- [`wip/help.md`](../wip/help.md) — how the help machinery works
- [`wip/help-content.md`](../wip/help-content.md) — the programme that writes the manual
