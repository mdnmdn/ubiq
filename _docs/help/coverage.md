---
id: help-coverage
title: Help coverage
kind: help
status: draft
summary: The living table of every interface area, the help page that covers it, its context key and its state.
read_when: you are choosing which help page to write next, or recording that one is written
updated: 2026-10-25
verified: 2026-10-25
code_anchors: [help/manifest.toml, _tools/helpbundle.py, crates/ubiq/src/state/ui_id.rs]
depends_on: [wip-help]
---

# Help coverage

This document owns the living coverage table: every area from the id inventory, the page that covers it, its context key and its state (`missing`, `planned`, `stub` or `done`).

## How to read this table

Each row is one area from `_docs/inbox/ui-id-inventory.md` §4, plus the rail modes from §4.1 and the areas that gained a page after the inventory was cut (marked `added`). The State column reads `missing` when no help page covers the area, `planned` when a page exists in the nav with its frontmatter and a one-paragraph promise but no body yet (`status: planned` or `draft`), `stub` when a page covers part of it, and `done` when a finished page covers it in full. Dialogs and settings sections reach help through rung 1, a `help_page()` call on the dialog or section, so their Context key column reads `— (rung 1)`; a panel or rail row carries the `context:` key in its page's frontmatter, and `—` means the page declares none. The Screenshot column names the guide page whose images (`help/img/guides/*`) show the area, so a UI change can find the page to re-shoot; `—` means no guide carries one.

This table is also the screenshot-rot index: when an interface area changes, its Screenshot column names the guide whose images to re-shoot.

## Coverage

| Area | Inventory § | Page | Context key | State | Screenshot |
|---|---|---|---|---|---|
| titlebar | §4.1 (CATALOGUE) | workbench/titlebar | — | done | guides/first-project-and-agent, index |
| rail | §4.1 (CATALOGUE) | workbench/rail | — | done | guides/first-project-and-agent, index |
| dock | §4.1 | workbench/panels | — | done | — |
| dock.centre | §4.1 | workbench/panels | — | done | — |
| dock.left | §4.1 | workbench/panels | — | done | — |
| dock.right | §4.1 | workbench/panels | — | done | — |
| dock.bottom | §4.1 | workbench/panels | — | done | — |
| status-bar | §4.1 | workbench/status-bar | — | done | — |
| outline | §4.1 | workbench/outline | panel.ubiq.outline | done | — |
| terminal | §4.1 (instance) | concepts/panes | — | done | — |
| editor.file | §4.1 (instance) | workbench/editor | panel.ubiq.file | done | — |
| viewer.diff | §4.1 | git/diff | panel.ubiq.git.diff | done | guides/review-and-commit |
| explorer | §4.2 | workbench/explorer | panel.ubiq.explorer | done | guides/first-project-and-agent |
| explorer.toolbar | §4.2 | workbench/explorer | panel.ubiq.explorer | done | — |
| explorer.bookmarks | §4.2 | workbench/bookmarks | — | done | — |
| explorer.tree | §4.2 | workbench/explorer | panel.ubiq.explorer | done | guides/first-project-and-agent |
| search | §4.2 | workbench/search | panel.ubiq.search | done | — |
| search.results | §4.2 | workbench/search | panel.ubiq.search | done | — |
| git.toolbar | §4.2 | git/overview | rail.git | done | — |
| git.refs.repositories | §4.2 | git/refs | panel.ubiq.git.refs | done | — |
| git.refs | §4.2 | git/refs | panel.ubiq.git.refs | done | — |
| git.history | §4.2 | git/history | panel.ubiq.git.history | done | guides/review-and-commit |
| git.changes | §4.2 | git/changes | panel.ubiq.git.changes | done | guides/review-and-commit |
| git.changes.commit-box | §4.2 | git/changes | panel.ubiq.git.changes | done | guides/review-and-commit |
| git.changes.commit | §4.2 | git/changes | panel.ubiq.git.changes | done | guides/review-and-commit |
| git.changes.range | §4.2 | git/compare-commits | — | done | — |
| git.diff | §4.2 | git/diff | panel.ubiq.git.diff | done | guides/review-and-commit |
| git.diff.header | §4.2 | git/diff | panel.ubiq.git.diff | done | guides/review-and-commit |
| kb | §4.2 | kb/overview | rail.kb, panel.ubiq.kb.explorer | done | — |
| kb.tree | §4.2 | kb/overview | rail.kb, panel.ubiq.kb.explorer | done | guides/knowledge-base |
| kb.centre | §4.2 | kb/documents | panel.ubiq.kb.doc | done | guides/knowledge-base |
| kb.document | §4.2 | kb/documents | panel.ubiq.kb.doc | done | guides/knowledge-base |
| kb.source-form | §4.2 | kb/sources | — | done | guides/knowledge-base |
| db | §4.1 (rail; not in §4), added | db/overview | rail.db, panel.ubiq.db.explorer, panel.ubiq.db.table, panel.ubiq.db.sql | done | — |
| db.connection-form | added | db/overview | rail.db, panel.ubiq.db.explorer, panel.ubiq.db.table, panel.ubiq.db.sql | done | guides/query-a-database |
| db.sql | added | db/overview | rail.db, panel.ubiq.db.explorer, panel.ubiq.db.table, panel.ubiq.db.sql | done | guides/query-a-database |
| logs | §4.2 | workbench/logs | panel.ubiq.logs | done | — |
| logs.toolbar | §4.2 | workbench/logs | panel.ubiq.logs | done | — |
| logs.body | §4.2 | workbench/logs | panel.ubiq.logs | done | — |
| control | §4.2 | workbench/control | rail.control | done | — |
| control.general | §4.2 | workbench/control | rail.control | done | — |
| control.usage | §4.2 | workbench/control | rail.control | done | — |
| notifications | §4.2 | workbench/notifications | — | done | — |
| notifications.picker | §4.2 | workbench/notifications | — | done | — |
| notifications.rules | §4.2 | workbench/notifications | — | done | — |
| all-projects | §4.2 | workbench/all-projects | — | done | — |
| agents.screen | §4.3 | agents/starting | rail.agents | done | — |
| agents.header | §4.3 | agents/starting | rail.agents | done | — |
| agents.columns | §4.3 | agents/starting | rail.agents | done | guides/two-agents-side-by-side |
| agents.column.header | §4.3 | agents/starting | rail.agents | done | guides/two-agents-side-by-side |
| agents.column.footer | §4.3 | agents/columns | panel.ubiq.agents.explorer | done | — |
| agents.column.thread | §4.3 | agents/columns | panel.ubiq.agents.explorer | done | — |
| agents.sidebar | §4.3 | agents/columns | panel.ubiq.agents.explorer | done | — |
| chat.tab.header | §4.3 | agents/chat | view.chat | done | guides/first-project-and-agent |
| conversation.header | §4.3 | agents/conversation | — | done | — |
| conversation.transcript | §4.3 | agents/chat | view.chat | done | guides/first-project-and-agent |
| conversation.queue | §4.3 | agents/conversation | — | done | — |
| conversation.activity-bar | §4.3 | agents/conversation | — | done | — |
| conversation.composer | §4.3 | agents/conversation | — | done | — |
| conversation.footer | §4.3 | agents/conversation | — | done | — |
| conversation.info | §4.3 | agents/conversation | — | done | — |
| acp-capabilities.panel | §4.3 | agents/conversation | — | done | — |
| new-agent.modal | §4.3 | agents/new-agent | — (rung 1) | done | guides/first-project-and-agent, guides/tasks-board, guides/accounts-and-definitions |
| new-agent.naming | §4.3 | agents/new-agent | — (rung 1) | done | — |
| tasks.screen | §4.3 | tasks/overview | rail.tasks | done | guides/tasks-board |
| tasks.toolbar | §4.3 | tasks/overview | rail.tasks | done | — |
| tasks.columns | §4.3 | tasks/overview | rail.tasks | done | guides/tasks-board |
| tasks.detail | §4.3 | tasks/detail | panel.ubiq.task | done | guides/tasks-board |
| teams.screen | §4.3 | teams/overview | rail.teams, rail.teamsall | done | — |
| teams.toolbar | §4.3 | teams/overview | rail.teams, rail.teamsall | done | — |
| teams.canvas | §4.3 | teams/overview | rail.teams, rail.teamsall | done | — |
| teams.card | §4.3 (was teams.inspector) | teams/inspector | — | done | — |
| teams.tasks-drawer | §4.3 (was teams.tasks) | teams/inspector | — | done | — |
| tools.panel | §4.3 | workbench/tools | — | done | — |
| help | §4.4 | workbench/help | panel.ubiq.help | done | — |
| help.target | §4.4 | workbench/help | panel.ubiq.help | done | — |
| settings | §4.4 | settings/overview | — (rung 1) | done | — |
| settings.appearance | §4.4 | settings/appearance | — (rung 1) | done | — |
| settings.size | §4.4 | settings/appearance | — (rung 1) | done | — |
| settings.file-explorer | §4.4 | settings/files-and-search | — (rung 1) | done | — |
| settings.editor | §4.4 | settings/files-and-search | — (rung 1) | done | — |
| settings.search | §4.4 | settings/files-and-search | — (rung 1) | done | — |
| settings.harnesses | §4.4 | settings/harnesses | — (rung 1) | done | — |
| settings.isolation | §4.4 | settings/harnesses | — (rung 1) | done | — |
| settings.assist | §4.4 | settings/assist | — (rung 1) | done | — |
| settings.connectors | §4.4 | settings/connectors | — (rung 1) | done | — |
| settings.hosts | §4.4 | settings/hosts | — (rung 1) | done | — |
| settings.ssh | §4.4 | settings/hosts | — (rung 1) | done | — |
| settings.drones | §4.4 | settings/hosts | — (rung 1) | done | — |
| settings.tools | §4.4 | settings/tools | — (rung 1) | done | — |
| settings.command-line | §4.4 | settings/tools | — (rung 1) | done | — |
| settings.agent-definitions | added | agents/definitions | — (rung 1) | done | guides/accounts-and-definitions |
| settings.skills | added | settings/skills | — (rung 1) | done | — |
| settings.mcp-servers | added | settings/mcp-servers | — (rung 1) | done | — |
| settings.updates | added | settings/updates | — (rung 1) | done | — |
| settings.diagrams | added | settings/diagrams | — (rung 1) | done | — |
| project-settings | §4.4 | settings/project | — (rung 1) | done | — |
| project-settings.general | §4.4 | settings/project | — (rung 1) | done | guides/query-a-database |
| project-settings.remote | §4.4 | settings/project | — (rung 1) | done | — |
| project-settings.tasks | §4.4 | settings/project | — (rung 1) | done | — |
| project-settings.kb | §4.4 | settings/project | — (rung 1) | done | — |
| project-settings.tools | §4.4 | settings/project | — (rung 1) | done | — |
| project-settings.documentation | §4.4 | settings/project | — (rung 1) | done | — |
| project-settings.integrations | §4.4 | settings/project | — (rung 1) | done | — |
| project-settings.databases | added | settings/project-databases | — (rung 1) | done | — |
| project-settings.task-sync | added | settings/project-task-sync | — (rung 1) | done | — |
| project-settings.skills | added | settings/skills | — (rung 1) | done | — |
| project-settings.mcp-servers | added | settings/mcp-servers | — (rung 1) | done | — |
| dialog.login | §4.4 | agents/accounts | — (rung 1) | done | guides/accounts-and-definitions |
| dialog.profile-form | §4.4 | agents/definitions | — (rung 1) | done | guides/accounts-and-definitions |
| dialog.account | §4.4 | agents/accounts | — (rung 1) | done | — |
| dialog.connect | §4.4 | settings/connectors | — (rung 1) | done | — |
| dialog.app-form | §4.4 | settings/connectors | — (rung 1) | done | — |
| dialog.connector | §4.4 | settings/connectors | — (rung 1) | done | — |
| dialog.certificate | §4.4 | settings/connectors | — (rung 1) | done | — |
| dialog.ai-form | §4.4 | settings/assist | — (rung 1) | done | — |
| dialog.ai-test | §4.4 | settings/assist | — (rung 1) | done | — |
| dialog.ai-remove | §4.4 | settings/assist | — (rung 1) | done | — |
| dialog.ssh-form | §4.4 | settings/hosts | — (rung 1) | done | guides/remote-machine |
| dialog.ssh-remove | §4.4 | settings/hosts | — (rung 1) | done | — |
| dialog.drone-stop | §4.4 | settings/hosts | — (rung 1) | done | — |
| dialog.clone | §4.4 | how-to/clone-a-project | — (rung 1) | done | — |
| dialog.feedback | §4.4 | workbench/feedback | — (rung 1) | done | — |
| dialog.remote-connect | §4.4 | remote/connect | — (rung 1) | done | guides/remote-machine |
| dialog.remote-manager | §4.4 | remote/overview | — (rung 1) | done | — |
| dialog.file | §4.4 | workbench/file-dialogs | — (rung 1) | done | — |
| dialog.file-picker | §4.4 | workbench/file-dialogs | — (rung 1) | done | — |
| dialog.theme-editor | §4.4 | settings/appearance | — (rung 1) | done | — |
| dialog.theme-naming | §4.4 | settings/appearance | — (rung 1) | done | — |
| dialog.size-naming | §4.4 | settings/appearance | — (rung 1) | done | — |
| dialog.close-pane | §4.4 | concepts/panes | — (rung 1) | done | — |
| dialog.end-conversation | §4.4 | how-to/hide-or-close-a-tab | — (rung 1) | done | — |
| rail.mode.ide | §4.1 (rail; not in §4) | workbench/modes | rail.ide | done | — |
| rail.mode.db | §4.1 (rail; not in §4) | db/overview | rail.db | done | guides/query-a-database |
| rail.mode.git | §4.1 (rail) | git/overview | rail.git | done | — |
| rail.mode.kb | §4.1 (rail) | kb/overview | rail.kb | done | — |
| rail.mode.agents | §4.1 (rail) | agents/starting | rail.agents | done | — |
| rail.mode.teams | §4.1 (rail) | teams/overview | rail.teams | done | — |
| rail.mode.teamsall | §4.1 (rail; not in §4) | teams/overview | rail.teamsall | done | — |
| rail.mode.tasks | §4.1 (rail) | tasks/overview | rail.tasks | done | — |
| rail.mode.control | §4.1 (rail) | workbench/control | rail.control | done | — |

The kitchen sink (Sink mode, its pages and style blocks) is out of scope: it is not user-visible, so it has no help page and no rows here. Teams has no inspector: `teams/inspector` covers selecting a card and the tasks drawer, so the earlier `graph.*` rows (the orchestration screen Teams replaced) are gone and the two inspector rows are named for what the user sees. A page's rows read `done` when the page's frontmatter carries no `status: planned` or `draft` and the page names the area; The capabilities dialog is a section of `agents/conversation`, and the clone dialog has its own how-to. The `index` screenshot (`help/img/start/index-1.png`) is the one full-window capture outside a guide.

**Count: 0 missing / 0 planned / 0 stub / 141 done** (141 rows).

## Related docs

- [`wip/help.md`](../wip/help.md) — how the help machinery works
