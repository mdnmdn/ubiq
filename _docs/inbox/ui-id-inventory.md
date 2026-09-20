---
id: inbox-ui-id-inventory
title: Proposal — the id inventory, and menus built from it
kind: proposal
status: proposal
summary: The list the two identity designs both assume and neither supplies — every place in the interface a stable name could point at, what consumes the name, and where in the code to find the place. It reports that Ubiq already names places five times over in five disagreeing spellings, that four of those namespaces are exhaustive enums the inventory can be derived from rather than invented beside, and it argues one new use for the result — a menu declared as a list of ids rather than as three parallel lists matched by position, which is what makes submenus, extension rows and a pointable menu possible at all.
read_when: you are naming a screen area, deciding what an extension or a personalisation rule may anchor to, adding a menu or a menu row, or looking for the place a given control lives in the code
updated: 2026-09-20
depends_on: [wip-in-place-help, inbox-ui-id, inbox-ui-id-tooling, wip-help, inbox-plugin-system, inbox-personalisation, tech-ui, tech-components, feat-workbench]
---

# Proposal — the id inventory, and menus built from it

Two designs for UI element identity are in flight. [`wip/in-place-help.md`](../wip/in-place-help.md)
is built — the `UiId` grammar, a `const` catalogue of thirty-four names, a per-frame mark registry
and the targeting overlay over it. [`inbox/ui-id-proposal.md`](./ui-id-proposal.md) argues the
catalogue should be data with aliases, and that repetition should be a typed selector. They agree on
the grammar and disagree on where the catalogue lives.

**Neither supplies the list.** Thirty-four names cover the rail and the titlebar; the window has
nine rail modes, seventeen panel kinds, thirty-odd menus, thirty-odd overlays, fourteen settings
sections and several hundred controls that nothing has named. This document is that enumeration —
what each name would point at, and where in the code the place is drawn — plus one finding and one
proposal that fall out of assembling it.

The finding: **the inventory does not have to be invented.** Ubiq already names places five times
over, and four of those namespaces are closed enums with an `all()` beside them. §2 is the map
between them.

The proposal: **a menu should be a list of ids.** Every menu in the window today is three parallel
lists matched by position, and the code says so in its own comments. §7 is what changes if the row
carries a name instead of an index, and what it costs.

---

## 1. What is being enumerated, and what is not

An entry below is a **place**: something drawn, that a reader can point at, that a preference could
key to, or that an extension could anchor a contribution beside. Four kinds, following
`ui-id-proposal.md` §1:

| Kind | Lifetime | Example |
|---|---|---|
| **area** | the window's | `explorer`, `git.history`, `titlebar` |
| **menu** | one gesture | `explorer.menu`, `titlebar.overflow-menu` |
| **element** | the frame's, one per screen | `git.changes.commit-button` |
| **instance-kind** | the frame's, one per datum | `explorer.tree.row`, `agents.column` |

An instance-kind is named once and drawn many times. The scheme has no way to say *which* one
(`G313`), so every such row is listed as one entry and the count is the user's data, never the
catalogue's.

**Not enumerated**: the 92 bare `.id("…")` literals under `crates/ubiq/src/ui/`, which exist to key
GPUI's hover and scroll bookkeeping and are unique only among siblings. They are the denominator
[`ui-id-tooling-proposal.md`](./ui-id-tooling-proposal.md) §7 proposes measuring coverage against,
not a source of names.

## 2. Ubiq already names places five times over

Five spellings for "which place", all in the tree today, none of them derived from another.

| Namespace | Spelling | Scope | Stability | Where |
|---|---|---|---|---|
| GPUI element id | `"menu-row"`, `"tab-strip"` | one frame, siblings only | none | ~355 kebab strings across `crates/ubiq/src/ui/`, 92 of them bare `.id("…")` literals |
| `UiId` | `rail.mode.tasks` | the product | published contract (`G312`: no alias yet) | `crates/ubiq/src/state/ui_id.rs`, `CATALOGUE` |
| Help context key | `panel.ubiq.explorer`, `view.git`, `rail.ide` | the help ladder | a page's `context:` claims it exactly once | `crates/ubiq/src/app/help.rs`, `context_key()` |
| Saved-layout panel name | `ubiq.explorer`, `ubiq.git.refs` | a persisted layout | **"it never changes"**, stated in the code | `crates/ubiq/src/state/dock.rs`, `PanelKind::name()` |
| Link view slug | `ubiq://./git`, `…/chat` | a `ubiq://` destination | one spelling, three readers | `crates/ubiq/src/state/nav/text.rs`, `View::slug()` |

Two runtime registries sit beside them and are not names but are exhaustive lists of the same
places: `MenuId` — every menu that can be open, one at a time — and `Layer` — every modal, dialog
and overlay, in peel order. Both are in `crates/ubiq/src/state/workbench.rs` and
`crates/ubiq/src/state/overlay.rs`.

**What this buys the inventory.** Three of the five and both registries are closed Rust enums with
an exhaustive `all()` or `match` beside them: `RailMode` (9), `View` (13), `PanelKind` (17),
`MenuId` (~30), `Layer` (~33), `SettingsSection` (14), `ProjectNav` (7), `SinkSection` (12). That is
roughly 135 places already enumerated, in the same tree, with a compile error waiting for anyone who
adds one and forgets. The inventory's spine is a mapping, not a survey — and the mapping is worth
writing down rather than deriving, for the reason `ui_id::rail_mode()` already writes its own out by
hand: deriving a published name from a Rust variant means a refactor silently moves the place a
help page and a user's preference both point at.

**What it costs.** Five namespaces that disagree is four chances to be wrong about the same place.
`PanelKind::Explorer` is `ubiq.explorer` to a saved layout, `panel.ubiq.explorer` to a help page,
`explorer` to a link only indirectly, and nothing at all to `UiId`. A reader asking "what is the
explorer called" gets four answers. The inventory below picks one spelling per place and says which
of the other four it has to agree with.

## 3. The roots

Every name has a root that is itself a name — `ui_id::catalogue_is_well_formed()` checks it, and it
is what lets the overlay widen a highlight from a control to the area around it. Thirty-two roots
cover the window, in four families.

- **The shell**: `titlebar`, `rail`, `dock`, `status-bar` — plus `navigator`, `outline`, `ribbon`,
  `terminal`, `editor` and `viewer`, which are drawn inside the dock but are not any one panel's.
- **The project panels**: `explorer`, `search`, `git`, `kb`, `logs`, `control`, `notifications`,
  `all-projects`.
- **The agent surfaces**: `agents`, `chat`, `conversation`, `acp-capabilities`, `new-agent`,
  `tasks`, `graph`, `teams`, `tools`.
- **The pages and questions**: `settings`, `project-settings`, `help`, `dialog`, `sink`.

`rail.mode.<slug>` names the *button*; the root names the *screen* it switches to. They are two
places and the inventory keeps them apart: a help page about the Git screen is not a sentence about
the button that reaches it. `conversation` is a root of its own rather than a branch of `chat` or
`agents` for the same reason — three surfaces embed it, and it belongs to none of them.

## 4. The inventory

**Paths below are relative to `crates/ubiq/src/`**, and a reference is a file plus the function or
type that draws the place. A row is a *proposal*: the thirty-four names in
`state/ui_id.rs`'s `CATALOGUE` are the only ones that exist today, and every other row is a name
that would have to be written down before anything could point at it.

### 4.1 The shell — titlebar, rail, dock, status bar

The titlebar and the rail are the two areas `CATALOGUE` already covers; they are listed in
`state/ui_id.rs` and not repeated here. What the shell still lacks is everything between them.

| Proposed id | Kind | What it is | Drawn by |
|---|---|---|---|
| `dock` | area | The dock's outer frame | `ui/dock/skin.rs`, `frame` |
| `dock.centre`, `dock.left`, `dock.right`, `dock.bottom` | area | The four regions, one per `Region` | `state/dock.rs`, `Region` |
| `dock.group` | instance-kind | One tabbed group in the dock's tree | `ui/dock/skin.rs`, `frame` |
| `dock.resize-handle` | instance-kind | The strip that resizes a region | `ui/dock/skin.rs`, `resize_strip` |
| `dock.tab-strip` | instance-kind | A group's own tab bar | `ui/dock/skin.rs`, `render_tab_bar` |
| `dock.tab-strip.new-pane`, `.new-pane-menu`, `.new-chat` | element | What a strip starts | `ui/dock/skin.rs`, `render_tab_bar` |
| `dock.tab-strip.scroll-left`, `.scroll-right`, `.zoom` | element | Overflow either way, and fill the region | `ui/dock/skin.rs`, `render_tab_bar` |
| `dock.tab` | instance-kind | One tab | `ui/dock/skin.rs`, `render_tab_bar` |
| `dock.tab.close`, `dock.tab.status-dot` | element | Its ending, and its running-or-dirty mark | `ui/dock/skin.rs`, `render_tab_bar` |
| `dock.tab.menu` | menu | The tab's right-click | `ui/tab_menu.rs`, `rows` |
| `dock.centre.no-project`, `.not-built` | element | The two empty centres | `ui/empty.rs`, `no_project`; `ui/dock/mod.rs`, `not_built` |
| `status-bar` | area | The bottom row | `ui/status_bar.rs`, `render` |
| `status-bar.file-name`, `.git`, `.version`, `.credit` | element | What it reports when a file is open | `ui/status_bar.rs`, `render`, `git_readout`, `version_label`, `made_with_love` |
| `status-bar.agents-summary`, `.sessions-summary`, `.board-summary` | element | The three per-screen readouts | `ui/status_bar.rs`, `render` |
| `status-bar.size` | element | The size popover's trigger | `ui/status_bar.rs`, `size_control` |
| `status-bar.viewer-kind` | menu | The file-kind chip and its viewer override | `ui/status_bar.rs`, `viewer_picker` |
| `status-bar.vim-mode` | element | The Vim chip and its switch | `ui/status_bar.rs`, `vim_chip` |
| `navigator`, `navigator.section`, `navigator.row` | menu, instance-kind | The command field's dropped list, its eight `Group` headings, one offer | `ui/navigator.rs`, `panel`, `line` |
| `outline`, `outline.row` | area, instance-kind | The definitions in the open buffer | `ui/outline.rs`, `render` |
| `ribbon.build`, `ribbon.experimental` | element | The build-channel corner, and Git's experimental band | `ui/ribbon.rs`, `render`, `experimental` |
| `terminal` | instance-kind | One pane's emulator surface | `ui/terminal.rs`, `pane` |
| `editor.file`, `editor.placeholder` | instance-kind, element | An open file's content, and the centre with none | `ui/editor.rs`, `render_file`, `render` |
| `viewer.diagram`, `viewer.diagram-fence`, `viewer.diagram-export` | element, instance-kind | A Mermaid picture, one fenced in Markdown, one a web panel authors | `ui/viewer/diagram.rs`, `render`, `drawn`, `exported` |
| `viewer.diff`, `viewer.diff.hunk`, `viewer.diff.line` | area, instance-kind | A change against its base | `ui/viewer/diff.rs`, `render`, `header`, `draw` |

**The seventeen panel kinds are already named, elsewhere.** `PanelKind::name()` answers
`ubiq.explorer`, `ubiq.git.refs`, `ubiq.kb.doc` and fourteen more, and its own comment says the name
**never changes**, because a saved layout is rebuilt by looking it up. That is a published,
stable-by-contract name for a place — the same thing a `UiId` is — reached by dropping the `ubiq.`
prefix and keeping the rest: `explorer`, `git.refs`, `kb.doc`, `agents.explorer`, `help.panel`,
`tasks.panel`, `chat`, `terminal`. **Wherever this inventory names a panel, it uses that spelling**,
so the two namespaces agree by construction rather than by review. The exceptions are the three
`PanelKind::from_name` refuses to rebuild — terminal, file and chat carry their identity in a
payload beside the name, which is `G313`'s instance problem arriving in the dock.

### 4.2 The project panels

The explorer, search, Git, knowledge base, console, Control and notifications — the panels a project
is read through.

| Proposed id | Kind | What it is | Drawn by |
|---|---|---|---|
| `explorer` | area | The project's file tree panel | `ui/explorer.rs`, `render` |
| `explorer.toolbar` | area | View, new, follow, collapse and hidden controls | `ui/explorer.rs`, `render` |
| `explorer.view-switch` | element | Tree or list | `ui/explorer.rs`, `render` |
| `explorer.new` | element | New file or folder | `ui/explorer.rs`, `render` |
| `explorer.follow` | element | Tracks the active editor's file | `ui/explorer.rs`, `follow_button` |
| `explorer.collapse-all` | element | Collapse every open folder | `ui/explorer.rs`, `render` |
| `explorer.show-hidden` | element | Dotfiles on or off | `ui/explorer.rs`, `render` |
| `explorer.filter` | element | The path filter above the tree | `ui/explorer.rs`, `render` |
| `explorer.bookmarks` | area | The saved-destination disclosure above the tree | `ui/explorer.rs`, `bookmarks_section` |
| `explorer.bookmarks.row` | instance-kind | One bookmark | `ui/explorer.rs`, `bookmarks_section` |
| `explorer.tree` | area | The scrollable tree or list body | `ui/explorer.rs`, `render` |
| `explorer.tree.row` | instance-kind | One file or folder | `ui/explorer.rs`, `line` |
| `explorer.menu` | menu | The right-click menu on a row or the empty tree | `state/explorer/menu.rs`, `menu_entries` |
| `search` | area | Content search | `ui/search.rs`, `render` |
| `search.query` | element | The query field | `ui/search.rs`, `render` |
| `search.case-sensitive`, `search.whole-word`, `search.regex` | element | The three match toggles | `ui/search.rs`, `render` |
| `search.results` | area | The grouped result list | `ui/search.rs`, `results` |
| `search.results.file-row`, `search.results.hit-row` | instance-kind | A file heading, and one matched line under it | `ui/search.rs`, `results` |
| `search.status` | element | Counts, truncation and the error line | `ui/search.rs`, `status_bar` |
| `git.toolbar` | area | Repository, HEAD, the write actions, refresh | `ui/git/mod.rs`, `toolbar` |
| `git.toolbar.fetch`, `.pull`, `.push`, `.refresh` | element | The four live write actions | `ui/git/mod.rs`, `toolbar` |
| `git.toolbar.branch`, `.stash`, `.undo` | element | The three drawn-and-inert actions | `ui/git/mod.rs`, `toolbar` |
| `git.repo`, `git.repo.menu`, `git.repo.menu.row` | element, menu, instance-kind | Which repository the screen reads | `ui/git/repo_selector.rs`, `render`, `panel`, `repo_row` |
| `git.refs` | area | Branches, remotes, tags, stashes, submodules | `ui/git/refs.rs`, `render` |
| `git.refs.local`, `.remotes`, `.tags`, `.stashes`, `.submodules` | element | The five section headings, one per `RefSection` | `ui/git/refs.rs`, `render` |
| `git.refs.row`, `git.refs.folder-row` | instance-kind | One ref, and a folded path prefix | `ui/git/refs.rs`, `ref_row`, `tree_row` |
| `git.refs.search`, `git.refs.menu` | element, menu | The filter, and the right-click on a ref | `ui/git/refs.rs`, `render` |
| `git.history` | area | The commit log, its graph and its filters | `ui/git/history.rs`, `render` |
| `git.history.branch-picker`, `.mine-only`, `.show-all`, `.search` | element | What the log walks and what it hides | `ui/git/history.rs`, `branch_picker`, `render` |
| `git.history.header`, `.row`, `.uncommitted-row`, `.load-more` | element, instance-kind | The column head, a commit, the pinned working-tree row, the next page | `ui/git/history.rs`, `header_row`, `commit_row`, `uncommitted_row`, `load_more_row` |
| `git.history.menu` | menu | The right-click on a commit | `ui/git/history.rs`, `render` |
| `git.changes` | area | The working tree, one commit, or a range | `ui/git/changes.rs`, `render` |
| `git.changes.commit-box` | area | The message block and its submit | `ui/git/changes.rs`, `commit_box` |
| `git.changes.commit-box.message`, `.amend`, `.submit` | element | What a commit is written with | `ui/git/changes.rs`, `commit_box` |
| `git.changes.section`, `.row` | instance-kind | A Conflicted/Staged/Unstaged heading, and a changed path | `ui/git/changes.rs`, `list_header`, `change_row` |
| `git.changes.stage-all`, `.unstage-all`, `.search` | element | The two bulk actions and the path filter | `ui/git/changes.rs`, `header_glyph`, `change_search_row` |
| `git.changes.commit`, `git.changes.range`, `git.changes.range.row` | area, instance-kind | One commit's summary, and a two-commit comparison | `ui/git/changes.rs`, `commit`, `range_files`, `range_row` |
| `git.changes.menu` | menu | The right-click on a changed path | `ui/git/changes.rs`, `render` |
| `git.diff`, `git.diff.header` | area | The comparison pane and its path row | `ui/git/diff.rs`, `render`, `header` |
| `git.diff.split`, `.unified`, `.collapse` | element | The two layouts and the fold | `ui/git/diff.rs`, `header` |
| `kb` | area | The knowledge base's source explorer | `ui/kb/mod.rs`, `render` |
| `kb.tree`, `kb.tree.source-row`, `.dir-row`, `.file-row` | area, instance-kind | The tree, and its three row kinds per `KbRowKind` | `ui/kb/mod.rs`, `render`, `line` |
| `kb.menu` | menu | The right-click on a source, folder or document | `ui/kb/mod.rs`, `render` |
| `kb.centre`, `kb.document`, `kb.add`, `kb.settings` | area, element | The empty centre, an open document, the first-source offer, the settings link | `ui/kb/mod.rs`, `centre`, `render_doc`, `add_page`, `render` |
| `kb.source-form` | area | The Add a source question | `ui/kb/source_form.rs`, `render` |
| `kb.source-form.kind`, `.name`, `.folder`, `.browse`, `.url`, `.check`, `.branch`, `.store`, `.access` | element | What a source is described by | `ui/kb/source_form.rs`, `body` |
| `kb.source-form.confirm`, `.cancel` | element | The two answers | `ui/kb/source_form.rs`, `render` |
| `logs`, `logs.toolbar`, `logs.body`, `logs.row` | area, instance-kind | The console, its toolbar, its list, one record | `ui/logs.rs`, `render`, `actions`, `body`, `row` |
| `logs.subsystem`, `logs.level`, `logs.follow`, `logs.clear` | element | The selector, the level floor, the tail latch, the clear | `ui/logs.rs`, `actions` |
| `control`, `control.tabs` | area, element | The Control screen and its General/AI strip | `ui/stats.rs`, `render` |
| `control.general`, `control.general.reading` | area, instance-kind | The host readings page, and one label-and-value | `ui/stats.rs`, `general`, `reading` |
| `control.usage`, `control.usage.table`, `control.usage.row` | area, instance-kind | The usage meter, its table, one hourly bucket | `ui/stats.rs`, `ai`, `table`, `row` |
| `notifications` | area | The bell's list | `ui/notifications.rs`, `render` |
| `notifications.list`, `notifications.row` | area, instance-kind | What has arrived, and one of them | `ui/notifications.rs`, `render`, `row` |
| `notifications.read-all`, `.clear-all`, `.mute-all` | element | The three bulk actions | `ui/notifications.rs`, `render` |
| `notifications.picker`, `.picker.level`, `.picker.duration`, `.picker.cancel` | area, instance-kind, element | Scope, level and duration for a new mute | `ui/notifications.rs`, `picker` |
| `notifications.rules`, `notifications.rules.row` | area, instance-kind | The mute rules in force, and one with its lift | `ui/notifications.rs`, `rules`, `rule_row` |
| `all-projects`, `.search`, `.list`, `.row` | area, element, instance-kind | The All projects modal and its filtered list | `ui/all_projects.rs`, `render`, `row` |

Three judgement calls this table makes and could have made the other way. `RefSection`'s five
headings get five names while `ChangeSection`'s three get one `instance-kind`, because the ref
sections are a small closed set a help page would explain one at a time and the change sections are
the same heading three times. `git.history.uncommitted-row` gets its own name although
`commit_row` draws it, because it is singular and pinned. And `git.changes.commit` and
`git.changes.range` nest under `git.changes` although one function draws all three bodies by
selection — they are what the panel *is* in two of its three states, not places beside it.

### 4.3 The agent surfaces

The Agents screen, the chat tab, the conversation view all three of them embed, the New agent form,
the Tasks board and the two graph screens.

| Proposed id | Kind | What it is | Drawn by |
|---|---|---|---|
| `agents.screen`, `agents.header`, `agents.columns` | area | The screen, its toolbar, the scrollable row | `ui/agents/mod.rs`, `render`, `header`, `columns` |
| `agents.header.new-agent`, `.close-all` | element | The `+`, and bench everything at once | `ui/agents/mod.rs`, `new_agent`, `close_all` |
| `agents.new-column-strip` | element | The drop target at the row's end | `ui/agents/mod.rs`, `new_column_strip` |
| `agents.column` | instance-kind | One agent's column | `ui/agents/column.rs`, `render` |
| `agents.column.header`, `.footer`, `.thread` | area | Its name and state, its identity pills, a mock agent's history | `ui/agents/column.rs`, `header`, `footer`, `thread` |
| `agents.column.tab`, `.tab.close` | instance-kind | One agent's tab in a grouped column | `ui/agents/column.rs`, `tab` |
| `agents.column.add-tab` | menu | Group a benched agent into this column | `ui/agents/column.rs`, `add_tab` |
| `agents.sidebar` | area | Every session and agent this window holds | `ui/agents/sidebar.rs`, `render` |
| `agents.sidebar.session`, `.session.note`, `.agent` | instance-kind | A session group, its task title, one agent | `ui/agents/sidebar.rs`, `session_row`, `note_row`, `agent_row` |
| `agents.sidebar.collapse-all` | element | Fold or open every session | `ui/agents/sidebar.rs`, `collapse_all` |
| `chat.tab`, `chat.tab.header` | instance-kind, area | One chat tab, and its toolbar row | `ui/chat/mod.rs`, `render`; `ui/chat/sidebar.rs`, `header` |
| `chat.tab.start`, `chat.tab.empty-start` | menu, element | Attach or start the tab's conversation | `ui/chat/sidebar.rs`, `start_control`; `ui/chat/mod.rs`, `body` |
| `conversation.header` | area | The lifecycle strip above the transcript | `ui/conversation/mod.rs`, `lifecycle_header` |
| `conversation.header.menu`, `.state`, `.persistent` | menu, element | Stop/abort/fork/hide/close, the lifecycle glyph, the kept mark | `ui/conversation/mod.rs`, `lifecycle_menu`, `lifecycle_glyph`, `persistence_mark` |
| `conversation.transcript` | area | The virtualized message list | `ui/conversation/mod.rs`, `transcript` |
| `conversation.transcript.block.user`, `.agent`, `.thought`, `.tool`, `.tool-group`, `.compacted` | instance-kind | The six block kinds, five of them one per `ConvBlock` | `ui/conversation/mod.rs`, `one_block`, `user_turn`, `thought_group`, `tool_block`, `tool_group`, `compacted` |
| `conversation.transcript.to-tail`, `.writing` | element | Jump to the last message, and the mid-turn mark | `ui/conversation/mod.rs`, `to_tail_button`, `writing_mark` |
| `conversation.permission` | instance-kind | A permission prompt on a tool call | `ui/conversation/mod.rs`, `permission` |
| `conversation.needs-you`, `conversation.reading-strip` | element | The answerable strip, and the subagent banner | `ui/conversation/mod.rs`, `needs_you_strip`, `reading_strip` |
| `conversation.queue`, `conversation.queue.message` | area, instance-kind | What was typed while a turn ran | `ui/conversation/mod.rs`, `queue_list` |
| `conversation.activity-bar` | area | The subagents-and-todos line (`tech/components.md`) | `ui/conversation/mod.rs`, `activity_bar` |
| `conversation.activity-bar.subagents`, `.subagents.row`, `.todos`, `.todos.item` | element, instance-kind | The two chips and the two panels they open | `ui/conversation/mod.rs`, `activity_bar`, `agent_row` |
| `conversation.composer` | area | The message field and its control row | `ui/conversation/mod.rs`, `composer` |
| `conversation.composer.model`, `.thinking`, `.mode` | menu | The three config chips | `ui/conversation/mod.rs`, `composer` |
| `conversation.composer.identity`, `.attach`, `.attachment`, `.grip` | element, instance-kind | The harness chip, the file picker's trigger, one attached file, the resize handle | `ui/conversation/mod.rs`, `identity_chip`, `composer`, `attachment_tags`, `composer_grip` |
| `conversation.composer.send`, `.stop` | element | Send or enqueue, and stop this turn | `ui/conversation/mod.rs`, `action_button`, `stop_button` |
| `conversation.footer` | area | The token, context and quota readout | `ui/conversation/mod.rs`, `footer` |
| `conversation.footer.context-ring`, `.quota-ring`, `.cache-ring`, `.total-tokens` | element | The three rings and the spend | `ui/conversation/mod.rs`, `footer` |
| `conversation.info` | area | The Conversation info modal | `ui/conversation/info.rs`, `render` |
| `conversation.info.profile`, `.account`, `.context`, `.tokens`, `.session`, `.capabilities`, `.reveal` | element, instance-kind | Its seven sections | `ui/conversation/info.rs`, `profile`, `account`, `context`, `tokens`, `session`, `capabilities_button`, `reveal` |
| `acp-capabilities.panel`, `.capability`, `.auth-method` | area, instance-kind | What a harness said it can do | `ui/acp_capabilities.rs`, `panel`, `capability_row`, `auth_row` |
| `new-agent.modal`, `.naming` | area | The New agent form, and its Save profile prompt | `ui/new_agent.rs`, `render` |
| `new-agent.target`, `.harness`, `.model`, `.thinking`, `.mode`, `.subagents`, `.mcps` | menu | The seven pickers, one per `OpenList` variant | `ui/new_agent.rs`, `body`, `mcps_button` |
| `new-agent.mcps.server` | instance-kind | One MCP server in the checklist | `ui/new_agent.rs`, `mcp_row` |
| `new-agent.prompt`, `.persistent` | element | The opening words, and the inert persistence box | `ui/new_agent.rs`, `body` |
| `new-agent.start`, `.start-terminal`, `.save-profile` | element | The three ways out | `ui/new_agent.rs`, `render` |
| `new-agent.menu`, `new-agent.menu.attach-list` | menu | The `+` menu every conversation host raises, and its second stage | `ui/agents/mod.rs`, `new_agent_menu` |
| `tasks.screen`, `tasks.toolbar`, `tasks.columns` | area | The board, its toolbar, its lanes | `ui/board/mod.rs`, `render`, `toolbar`, `columns` |
| `tasks.toolbar.filter`, `.label`, `.new-task`, `.new-agent`, `.popup-toggle` | element, instance-kind | What narrows the board and what adds to it | `ui/board/mod.rs`, `filter_field`, `toolbar` |
| `tasks.column`, `tasks.column.card`, `tasks.column.card.link` | instance-kind | One lane, one card, its tracker link | `ui/board/mod.rs`, `column`, `task_card`, `link_chip` |
| `tasks.detail`, `.detail.step`, `.detail.comment`, `.detail.open-chat` | area, instance-kind, element | The task report and what is in it | `ui/board/detail.rs`, `render`, `body`, `footer` |
| `tasks.form.title`, `.key`, `.link`, `.assigned-to`, `.description` | element | The five editable facts, one per `Field` variant | `ui/board/form.rs`, same-named symbols |
| `tasks.form.kind`, `.priority`, `.complexity`, `.shape`, `.colour`, `.labels`, `.session` | element, menu | The pill rows and the session picker | `ui/board/form.rs`, `kind_pills`, `priority_pills`, `complexity_pills`, `shape_pills`, `colour`, `labels`, `session` |
| `tasks.form.new-step`, `.new-comment`, `.delete` | element | Append, comment, remove | `ui/board/form.rs`, same-named symbols |
| `graph.screen`, `graph.toolbar`, `graph.canvas` | area | The orchestration screen | `ui/orchestration/mod.rs`, `render`, `toolbar`; `ui/orchestration/graph.rs`, `render` |
| `graph.toolbar.session`, `.bucket`, `.layout` | instance-kind, menu | The two filter pill rows and the arrangement picker | `ui/orchestration/mod.rs`, `session_pill`, `toolbar` |
| `graph.canvas.card`, `.task-box` | instance-kind | One agent card, and the fence round a task's cards | `ui/orchestration/graph.rs`, `agent_card`, `render` |
| `graph.inspector`, `.header`, `.thread`, `.composer` | area | The selection's panel | `ui/orchestration/inspector.rs`, `render`, `header_bar`, `thread`, `composer` |
| `graph.tasks`, `graph.tasks.task`, `graph.tasks.step` | area, instance-kind | The tasks drawer under the graph | `ui/orchestration/tasks.rs`, `render`, `list` |
| `teams.screen`, `teams.toolbar`, `teams.toolbar.hide-done` | area, element | The Teams screen | `ui/teams/mod.rs`, `render`, `toolbar`, `hide_done_check` |
| `teams.canvas`, `.canvas.card`, `.canvas.subagent-card`, `.canvas.task-box` | area, instance-kind | The board, an agent, a delegate inside its parent's fence, a task's fence | `ui/teams/graph.rs`, `render`, `agent_card`, `subagent_card` |
| `teams.inspector`, `.header`, `.tabs` | area, element | The selection's panel and its chat/tasks strip | `ui/teams/inspector.rs`, `render`, `header_bar`, `agent_view` |
| `teams.tasks`, `teams.tasks.task`, `teams.tasks.step` | area, instance-kind | Its tasks drawer | `ui/teams/tasks.rs`, `render`, `list` |
| `tools.panel`, `tools.row`, `tools.editor` | area, instance-kind | The runnable-tools list and its form | `ui/tools.rs`, `panel`, `tool_row`, `editor_form` |

**One view, three hosts, drawn once each per frame.** `ui/conversation/mod.rs`'s `render` is embedded
by the agents column, the chat tab and the Teams inspector — `tech/components.md` names the three.
Every `conversation.*` name therefore describes a kind of place that can be on screen several times
at once, one per open conversation, and never one per host. `acp-capabilities.panel` is the same
shape at a smaller scale. That is not a defect in the naming: it is `G313` again, and it says the
instance form has to carry a conversation's own id, not the surface drawing it.

**`teams` and `graph` are near-duplicates with separate state**, built to the same shape — canvas,
inspector, tasks drawer — so they take parallel namespaces rather than one shared with a
discriminant, and Teams alone carries the delegate ring (`teams.canvas.subagent-card`).

### 4.4 The overlays, the settings pages and the sink

`Layer` in `state/overlay.rs` is the other exhaustive registry: every modal, dialog and overlay in
the window, in the order Escape peels them. **A question a screen owns keeps that screen's root**
(`kb.source-form`, `new-agent.modal`, `conversation.info`, `notifications`, `all-projects`), and
`dialog.` holds the ones that belong to no screen. That rule is what stops the same modal being
`dialog.new-agent` to a help page and `new-agent.modal` to a preference.

| Proposed id | `Layer` | What it asks | Drawn by |
|---|---|---|---|
| `dialog.login`, `.login.harness` | `Login` | Sign a harness in under an identity | `ui/settings.rs`, `login`, `choosing` |
| `dialog.profile-form` | `ProfileForm` | A saved start, named — it reuses `ui/new_agent.rs`'s `body` whole, so its fields inherit `new-agent.*` | `ui/settings.rs`, `profile_form` |
| `dialog.account` | `AccountDialog` | Rename, delete or sign out one account | `ui/settings.rs`, `account_dialog` |
| `dialog.connect`, `.provider`, `.auth`, `.name`, `.application`, `.device-code` | `Connect` | Connect an identity to a code host | `ui/settings.rs`, `connect`, `choosing_connector`, `application_picker`, `device_code` |
| `dialog.app-form`, `.provider`, `.client-id` | `AppForm` | Register an OAuth application | `ui/settings.rs`, `app_form` |
| `dialog.connector` | `Connector` | Rename, disconnect or forget a connection | `ui/settings.rs`, `connector_dialog` |
| `dialog.certificate`, `.trust` | `Cert` | Pin a fingerprint, mid-flow | `ui/settings.rs`, `certificate` |
| `dialog.ai-form`, `.kind`, `.key`, `.model` | `AiForm` | Add or edit an API provider | `ui/settings.rs`, `ai_form`, `model_row` |
| `dialog.ai-test`, `.role`; `dialog.ai-remove` | `AiTest`, `AiRemove` | Try a provider, and drop one | `ui/settings.rs`, `ai_test`, `test_role_pill`, `ai_remove` |
| `dialog.ssh-form`, `.host`, `.method`; `dialog.ssh-remove` | `SshForm`, `SshRemove` | An SSH profile, and its removal | `ui/settings.rs`, `ssh_form`, `ssh_remove` |
| `dialog.drone-stop` | `DroneStop` | Stop a drone | `ui/settings.rs`, `drone_stop` |
| `dialog.clone`, `.identity`, `.repo`, `.url`, `.branch`, `.destination`, `.options` | `Clone` | Clone a project | `ui/clone.rs`, `render`, `connector_picker`, `row`, `url_field`, `branch_picker`, `destination`, `options` |
| `dialog.feedback`, `.kind`, `.title`, `.description`, `.screenshot` | `Feedback` | Send a report with a picture | `ui/feedback.rs`, same-named symbols |
| `dialog.remote-connect`, `.mode`, `.address`, `.token`, `.ssh-profile`, `.folder`, `.lifetime` | `RemoteConnect` | Reach a host or a drone | `ui/remote_connect.rs`, `editing_body`, `socket_body`, `ssh_body` |
| `dialog.remote-manager`, `.host`, `.rename` | `RemoteManager` | Every saved and live remote host | `ui/remote_hosts.rs`, `render`, `saved_row`, `rename_prompt` |
| `dialog.file` | `FileDialog` | One filesystem question — seventeen `FileDialog` variants, one name, because each is a state of the same question | `ui/file_dialog.rs`, `render` |
| `dialog.file-picker`, `.filter`, `.row`, `.tally`, `.grip` | `FilePicker` | Choose a path (`tech/components.md`) | `ui/file_picker.rs`, `render`, `field`, `line`, `footer`, `grip` |
| `dialog.theme-editor`, `.token-list`, `.picker`, `.specimen`; `dialog.theme-naming` | `ThemeEditor`, `ThemeNaming` | Fork a palette, and name it | `ui/themes.rs`, `editor`, `token_list`, `picker`, `specimen`, `name_prompt` |
| `dialog.size-naming` | `SizeNaming` | Name a size preset | `ui/size.rs`, `name_prompt` |
| `dialog.close-pane`, `dialog.end-conversation` | `ClosePane`, `EndConversation` | The two irreversible tab closes | `ui/shell.rs`, `close_pane_confirm`, `end_conversation_confirm` |
| `help`, `help.header`, `help.contents`, `help.contents.item`, `help.body` | — | The documentation panel | `ui/help/mod.rs`, `render`, `header`, `contents`, `body` |
| `help.target`, `.highlight`, `.balloon`, `.done` | `HelpTarget` | The targeting overlay itself | `ui/help_target.rs`, `overlay`, `highlight`, `balloon`, `chrome` |
| `settings`, `settings.nav`, `settings.nav.<section>` | `Settings` | The application page, and one nav row per `SettingsSection` — fourteen names, written out for the reason `rail_mode()` writes its own out | `ui/settings.rs`, `overlay`, `nav` |
| `settings.appearance`, `.size`, `.file-explorer`, `.editor`, `.search`, `.harnesses`, `.isolation`, `.assist`, `.connectors`, `.hosts`, `.ssh`, `.drones`, `.tools`, `.command-line` | — | The fourteen section bodies | `ui/settings.rs`, same-named symbols |
| `settings.harnesses.add`, `.account`, `.harness-row`, `.profile` | — | What the largest section holds | `ui/settings.rs`, `harnesses`, `account_block`, `harness_row`, `profile_row` |
| `settings.isolation.toggle`, `.home`, `.grants` | — | Confinement, the agent's home, the extra roots | `ui/settings.rs`, `isolation`, `home_cards`, `grant_chips` |
| `settings.assist.backend`, `.name-conversations`, `.providers`, `.providers.provider` | — | The prose backend and its providers | `ui/settings.rs`, `assist`, `assist_provider_choice`, `ai_providers`, `ai_provider_row` |
| `settings.connectors.connect`, `.connection`, `.oauth-app`, `.trusted-cert` | — | The three list kinds and the way in | `ui/settings.rs`, `connectors`, `connection_row`, `oauth_row`, `cert_row` |
| `project-settings`, `.nav`, `.nav.<section>` | `ProjectSettings` | The project page, one nav row per `ProjectNav` | `ui/sink/project.rs`, `overlay`, `nav` |
| `project-settings.general` + `.name`, `.colour`, `.initials`, `.path`, `.modes`, `.index`, `.repositories`, `.search-excludes` | — | What a project is | `ui/sink/project.rs`, `general`, `modes_block`, `index_row`, `repos_row`, `search_excludes_row` |
| `project-settings.remote` + `.location`, `.profile`, `.folder`, `.lifetime` | — | Where its folder actually is | `ui/sink/project.rs`, `remote` |
| `project-settings.tasks`, `.tasks.lane`; `.kb`, `.kb.source`, `.kb.add`; `.tools`; `.documentation`; `.integrations` | — | The five remaining sections | `ui/sink/project.rs`, same-named symbols |
| `sink`, `sink.tabs`, `sink.tabs.<page>` | — | The bench, and one tab per `SinkSection` | `ui/sink/mod.rs`, `render` |
| `sink.<page>` (twelve) | — | The twelve pages themselves | `ui/sink/`, one module each |
| `sink.style.<block>` (ten) | — | Tokens, typography, surfaces, controls, fields, files, modals, ribbons, notifications, colour | `ui/sink/style.rs`, same-named symbols |

Two of `Layer`'s variants name no place: `Menu` and `Dropdown` say *a* menu or *a* form picker is
open, not which. §4.5 names the menus individually, and those two stay out of the catalogue — a
target nothing can point at is a row a lint would report forever.

**A settings section gets two names, not one.** `settings.nav.harnesses` is the row a reader points
at and `settings.harnesses` is the pane it opens; they are different places with different content,
and the precedent is already in `CATALOGUE`, where `rail.modes` and each mode's own screen are both
named. The same holds for `project-settings`.

**One implementation, two named places.** `ui/sink/project.rs` draws the project settings body for
both the kitchen sink's fixture page and the live overlay, keyed by `Form::Sink`/`Form::Live`. The
two get different roots — `sink.project` and `project-settings` — because the naming rule is about
what a place *is* in the product, and a fixture and a real dialog are not the same place even when
one function draws both.

### 4.5 The menus

`MenuId` in `state/workbench.rs` is an exhaustive list of every menu that can be open — one at a
time, window-wide — so the menu half of the inventory is a rename rather than a survey. Each row
below is the menu itself; §7 is what it would take to name the rows inside it.

| Proposed id | `MenuId` | Where it hangs from | Rows built by |
|---|---|---|---|
| `titlebar.project-menu` | `Project` | The project cluster | `ui/project_menu.rs`, `render` |
| `titlebar.new-project-menu` | `NewProject` | The `+` beside the picker | `state/workbench.rs`, `NewProjectRow` |
| `titlebar.new-terminal-menu` | `NewPane` | The new-pane chevron | `state/workbench.rs`, `NewPaneRow` |
| `titlebar.run-tool-menu` | `RunTool` | The play control's chevron | `ui/run_tool_menu.rs`, `overlay` |
| `titlebar.overflow-menu` | `Overflow` | The overflow chevron | `state/workbench.rs`, `OverflowRow` |
| `dock.tab.menu` | `Tab` | A right-click on any tab | `ui/tab_menu.rs`, `rows` |
| `explorer.menu` | `Explorer` | A right-click on a row or the empty tree | `state/explorer/menu.rs`, `menu_entries` |
| `kb.menu` | `Kb` | A right-click in the KB tree | `state/kb.rs`, `KbAction` |
| `git.menu` | `Git` | A right-click on a change, a commit or a ref | `state/git.rs`, `GitMenuEntry` |
| `git.repo.menu`, `git.history.branch-menu` | `GitRepo`, `GitBranch` | The two Git pickers | `ui/git/repo_selector.rs`, `ui/git/history.rs` |
| `new-agent.menu` | `NewAgent` | Every conversation host's `+` | `ui/agents/mod.rs`, `new_agent_menu` |
| `conversation.header.menu` | `ConversationLifecycle(AgentId)` | One conversation's three dots | `ui/conversation/mod.rs`, `lifecycle_menu` |
| `agents.column.add-tab` | `AgentBench(usize)` | One column's `+` | `ui/agents/column.rs`, `add_tab` |
| `tasks.form.session-menu`, `tasks.form.labels-menu` | `TaskSession`, `TaskLabels` | The task panel's two pickers | `ui/board/form.rs` |
| `graph.toolbar.layout-menu` | `GraphLayout` | The arrangement trigger | `ui/orchestration/mod.rs`, `toolbar` |
| `status-bar.viewer-kind` | `ViewerKind` | The file-kind chip | `ui/status_bar.rs`, `viewer_picker` |
| `status-bar.size` | `Size` | The size chip | `ui/size.rs` |
| `logs.subsystem-menu`, `logs.level-menu` | `LogSubsystem`, `LogLevel` | The console's two selectors | `ui/logs.rs`, `actions` |
| `dialog.clone.connection-menu`, `.branch-menu` | `CloneConnection`, `CloneBranch` | The clone modal's two dropdowns | `ui/clone.rs` |
| `dialog.feedback.kind-menu` | `FeedbackKind` | What the report is about | `ui/feedback.rs` |
| `sink.*-menu` (five) | `SinkPicker`, `SinkA2ui`, `SinkScript`, `SinkTeamsimPreset`, `SinkTeamsimAlgo`, `SinkSettings` | The kitchen sink's own pickers | `ui/sink/` |

Two of these carry an instance in the variant already — `ConversationLifecycle(AgentId)` and
`AgentBench(usize)` — because several of each can be on screen and only one may be open. They are
the same `G313` problem, solved once, locally, in the one place the code could not avoid it.

## 5. The repeated places, and what their identity rests on

About fifty of the entries above are instance-kinds: one name, drawn once per datum. `G313`
says the scheme cannot address one of them, and [`ui-id-proposal.md`](./ui-id-proposal.md) §2
proposes a typed selector. Sorting the inventory by **what the selector's type would have to be**
answers a question neither document asks: which repeated places could ever carry a durable
preference, and which can only ever be pointed at.

| Selector | Survives | The places | What it enables |
|---|---|---|---|
| `path:` | until the file moves | `explorer.tree.row`, `search.results.file-row`, `git.changes.row`, `git.changes.range.row`, `kb.tree.*-row`, `explorer.bookmarks.row` | A rule on a folder — exclude it, collapse it, colour it |
| `ulid:` | a restart, and the config root | `dock.tab`, `dock.group`, `terminal`, `chat.tab`, `editor.file`, `agents.column`, `agents.sidebar.session`, `agents.sidebar.agent`, `tasks.column.card`, `graph.canvas.card`, `teams.canvas.card`, `notifications.row`, `all-projects.row` | Everything durable: a pinned tab, a hidden column, a per-agent preference |
| `action:` | the release | every menu row (§7), `notifications.picker.level`, `notifications.picker.duration`, `tasks.toolbar.label` | A hidden row, an extension's anchor, a per-row help sentence |
| `index:` only | nothing | `conversation.transcript.block.*`, `viewer.diff.line`, `viewer.diff.hunk`, `logs.row`, `control.usage.row`, `git.history.row`, `conversation.queue.message` | **Nothing durable.** Pointing at one, and nothing else |

The last row is the useful finding. A transcript block, a diff line and a log record have no
identity beyond their position in a list that is still growing, so they can be inspected and can
never be personalised or anchored to — and `ui-id-proposal.md` §2's rule that an `index:` selector
is *forbidden anywhere it would be persisted* turns out to name a whole class of place rather than a
mistake an author might make. Those entries earn a sentence in the catalogue and nothing else, which
is worth knowing before anyone builds a per-instance override for them.

`git.history.row` sits in that row although a commit has a hash, because the hash identifies the
*commit* and not the *row*: a filter changes which rows exist without changing any commit.

## 6. What consumes a name — help first

Three consumers were named before any of this was built: help binds a sentence or a page to a place,
personalisation keys a preference to one, an extension anchors a contribution beside one. Help is
the only one with code behind it today, and the enumeration says something about the shape of the
content it implies.

**Two questions, two sizes of answer, and the inventory sorts itself by which.** F1 asks *where am
I* and resolves through the five-rung ladder in `app/help.rs`, `context_key()`, to exactly one page.
⇧F1 asks *what is that* and resolves through `ui_id::describe()` to one sentence. The rule that
falls out of §4 is mechanical:

- **An area gets a page**, and it already has a key for one — a panel through `panel.<PanelKind::name()>`,
  a screen through `view.<View::slug()>`, a rail mode through `rail.<RailMode::slug()>`. §4 proposes
  about a hundred — panels, screens, dialogs and settings sections — and the ladder already reaches
  most of them.
- **An element gets a sentence**, in the catalogue beside its name, written by whoever named it.
  There are about a hundred and ten, and the reason they are not pages is the one
  `ui-id-proposal.md` §5 gives: every `context:` key must be claimed by a page
  reachable from `nav.order`, so binding them that way would grow the manual by a hundred and ten
  stubs whose only reader is a balloon.
- **An instance-kind gets a sentence about the kind**, never about the instance — "a file in the
  project", not "src/main.rs".

**The page half is built on one side of the wire only.** `_tools/helpbundle.py` reads a `targets:`
list from a page's frontmatter, checks its shape and resolves each entry against the real catalogue
scraped out of `state/ui_id.rs`, inverts it many-to-many, and writes `"targets"` into
`catalog.json` beside `"context"` and `"redirects"`. `HelpCatalog` in `crates/ubiq-proto/src/help.rs`
has fields for the other two and none for that one, so serde drops the map on the way in and
nothing in the interface can read it. A page can therefore claim a target today, be validated for
it, and never be reachable from it — which is why a click inside the targeting overlay still opens
nothing (`G319`). Closing it is a `crates/ubiq-proto` change, which is precisely the cost
[`wip/in-place-help.md`](../wip/in-place-help.md) §2.5 hoped the binding would avoid.

**What the other two consumers need that help does not** is stated in
`ui-id-proposal.md` §6 and is not restated here. What §4 adds to it is the
denominator: thirty-four names exist, §4 proposes close to three hundred more, and about a hundred
and thirty-five of those are already enumerated by an exhaustive enum somewhere in `state/`.


## 7. Declarative menus — a menu as a list of ids

This is the proposal the enumeration produced, and it is the one place where a name would stop being
documentation and start being load-bearing.

### 7.1 What a menu is today

Three parallel lists, matched by position.

1. **A row enum in `state/`** — `OverflowRow`, `NewPaneRow`, `NewProjectRow`, `HarnessChoice`,
   `ExplorerAction`, `KbAction`, `GitAction`.
2. **A builder in `ui/`** that turns those rows into `kit::ContextItem`s.
3. **A `pick_*` in `app/`** that rebuilds the same list and indexes into it —
   `pick_tab_menu`, `pick_overflow_menu`, `pick_new_pane_menu`, `pick_explorer_action`,
   `pick_git_menu_action`, `pick_kb_action`, `pick_conversation_menu` and eight more.

`kit::context_menu(id, at, items, on_pick: Fn(usize), on_dismiss)` in
`crates/ubiq/src/ui/kit/menu.rs` is the mechanism, and `usize` is the whole contract between the
three. The tree knows the hazard and works around it in five separate places, each with a comment
saying so:

- A **separator occupies a slot**, because a hairline drawn but not counted shifts every row under
  it — `ExplorerAction::Separator` and `KbAction::Separator` are actions for that reason alone.
- A **heading is a row** — `NewPaneRow::DetachedHeading` is drawn disabled so that the pick after it
  still lands.
- **A suppressed row shifts the rest**, which is why `ui::tab_menu::rows` pushes `"Close"` only when
  the tab is unpinned and why its module header says "the pick is matched by position, so the two
  have to read the same list".
- **Whole groups are added or dropped together** in `state/explorer/menu.rs`, "so one that has
  nothing in it takes its separator with it".
- **There are no submenus anywhere in the window.** `HarnessChoice`'s own doc comment gives the
  reason outright: *"A flat list rather than a submenu, because the kit has no submenu and the pick
  is an index."*

None of those is a bug. They are five correct workarounds for one missing thing.

### 7.2 What changes if a row carries a name

A row is identified by a `UiId`, and the index disappears from the contract.

- `kit::ContextItem` gains an `id: UiId`; `context_menu`'s `on_pick` takes that id instead of a
  `usize`. `context_panel` marks each row with `.ui_id(...)` as it draws it, which the mark registry
  already supports.
- The row enum stays — it is the typed action, and it is what the `app/` layer wants to match on.
  It gains `fn id(self) -> UiId`, an **exhaustive hand-written match**, the same shape and for the
  same reason as `ui_id::rail_mode()`.
- The `pick_*` handler matches an id rather than an index, so it no longer rebuilds the list to
  interpret the answer. Suppressing a row, reordering two, inserting a separator, adding a heading:
  all free, and none of them can silently fire the wrong action.

Four things that are impossible today become expressible, and each is asked for elsewhere in this
library:

| Becomes possible | Wanted by |
|---|---|
| **A submenu** — a row whose id has children. `explorer.menu.new` over `explorer.menu.new.file`, `.folder`, `.excalidraw`, `.drawio`. The dots are already the hierarchy, so the nesting needs no second model | The explorer's four `New …` rows and the new-pane menu's flattened shells |
| **An extension row** — a manifest naming an anchor and a side: `after: explorer.menu.copy-path`. Resolution is late and tolerant, an unresolved anchor inert | `plugin-system-proposal.md`, "declarative contributions and never HTML or native drawing" |
| **A pointable menu** — ⇧F1 over a row says what the row does, because the row is a marked element with a blurb beside its name | `wip/in-place-help.md` §3, phase 3b |
| **A hidden row** — a preference naming a row it never wants to see, read at startup by a build that has never seen the file | [`personalisation-proposal.md`](./personalisation-proposal.md) |

### 7.3 What it costs, honestly

**Fifteen pick handlers and every menu builder change.** That is the bulk of the work and it is
mechanical: the row enums already exist, and adding `fn id()` to each is the same exhaustive match
`rail_mode()` is.

**The id becomes runtime-load-bearing.** Today a wrong `UiId` costs a missing help sentence; after
this it costs a dead menu row. That raises `G315` — nothing checks that a `.ui_id(...)` names a
catalogued place — from a lint that would be nice to a lint that is required, and it is the strongest
argument yet for [`ui-id-tooling-proposal.md`](./ui-id-tooling-proposal.md)'s place in `just verify`.

**Roughly 150 new names.** Every menu row needs one, which stretches the "elements on demand" rule
in `wip/in-place-help.md` §2.2. The defence is that a menu row is the one
element class where the name is *also* the dispatch key, so it is not overhead added for help's
sake — it replaces the index that was doing the job badly.

**It turns the instance form from a nicety into a blocker.** This is the real finding.
`NewPaneRow::Shell(usize)`, `NewPaneRow::Detached(usize)`, `HarnessChoice::Pair { account }`, the
task panel's label rows and the agents column's bench rows are all rows whose identity is a *datum*,
not a place. An id alone cannot carry them. So the pick has to answer with an id **plus an instance
token** — which is exactly the typed selector `ui-id-proposal.md` §2 argues
for as an inspector convenience, arriving instead as a hard runtime requirement of the menu.
`G313` stops being the explorer tree's problem and becomes the first thing this work needs.

### 7.4 The cheaper half, if the whole is too much

The change splits cleanly, and the first half is worth having alone: **mark the rows without
changing the pick.** `ContextItem` carries an id used only for `.ui_id()` and `describe()`; `on_pick`
keeps its `usize`. That buys the pointable menu and a personalisation anchor, costs no `pick_*`
rewrite, and needs no instance form — because the id is being read, not dispatched on. It does not
buy submenus or extension rows, which are the two that need the index gone.

## 8. Extension points — what an anchor would attach to

[`plugin-system-proposal.md`](./plugin-system-proposal.md) settles that plugin UI is declarative
contributions rather than drawing, and asks the register for a row saying so — under the number
`D100`, which is already taken by a decision about deleted conversations, so the row it wants is one
`tech/decisions.md` has yet to give it. A contribution names a place; this is what those places
are, and whether a seam exists for one today.

| Seam | A contribution would be | Seam today |
|---|---|---|
| Rail mode | A whole screen behind a new rail button | `RailMode`, a closed enum matched exhaustively in a dozen places. No seam |
| Dock panel | A panel in any region, restored with the layout | **The closest thing to a seam that exists.** `PanelKind::name()`/`from_name()` is already a stable-string registry a saved layout is rebuilt through — a plugin panel arrives as a name `from_name` does not recognise, which the code already handles by dropping it |
| Viewer | A renderer for a file kind | `ViewerKind`, picked from the status bar's file-kind readout. A closed enum |
| Titlebar action | A button in the right cluster | Hand-built row in `ui::titlebar`. `titlebar.actions` is already marked |
| Overflow menu row | A command behind the chevron | `OverflowRow`, closed enum. §7's change is what opens it |
| Tab menu row | A verb on a file, terminal or chat tab | `ui::tab_menu::rows` returns `Vec<&'static str>` — the least declarative menu in the window |
| Explorer / KB / Git row menu | A verb on a path, a commit or a ref | `ExplorerAction`, `KbAction`, `GitAction`. Groups already exist, which is half of an anchor model |
| Conversation lifecycle menu | A verb on a running conversation | `MenuId::ConversationLifecycle(AgentId)`, already instance-keyed |
| Settings section | A page of the settings nav | `SettingsSection::all()` — an ordered list, which is the easiest of all of these to open |
| Project settings section | A page of the project nav | `ProjectNav::all()`, the same shape |
| Status bar readout | A reading beside the others | Hand-built in `ui::status_bar` |
| Composer control | A chip beside the model and mode pickers | Hand-built in `ui::conversation` |
| Command palette entry | A row in `titlebar.command` | The one seam that is already list-driven by data |

**The pattern across the table**: everywhere a place is a closed enum, an extension needs the enum
opened; everywhere a place is already keyed by a string a registry looks up — `PanelKind::name()`,
the command field — the seam is nearly there. That is the same conclusion §7 reaches from the menu
side, which is why the two are one proposal rather than two.

## 9. What this owes the library when any of it lands

- The inventory itself belongs in `tech/`, beside the grammar, once the grammar's own durable home
  exists — `wip/in-place-help.md` already owes
  [`tech/ui-and-design.md`](../tech/ui-and-design.md) that amendment. It is a reference table, not a
  capability, so it is filed rather than created by the implementer.
- A decision row if §7 is taken: a menu row is identified by name rather than by position, and what
  that costs.
- Three rows are already filed: `G322` for the `targets:` map the interface cannot read, `G323` for
  the position-matched menu, and `G324` for the five spellings §2 maps. `G313` and `G315` gain a
  second reason each from §7.3 and need no new row.
- `tech/components.md` gains the menu as a compound once its rows are declared rather than built.

---

## Related docs

- [`ui-id-proposal.md`](./ui-id-proposal.md) — the catalogue, the grammar and the stability contract
- [`ui-id-tooling-proposal.md`](./ui-id-tooling-proposal.md) — the lint the enumeration needs
- [`../wip/in-place-help.md`](../wip/in-place-help.md) — the scheme as built, and the thirty-four names it holds
- [`../features/workbench.md`](../features/workbench.md) — what the areas in §4 actually do
- [`plugin-system-proposal.md`](./plugin-system-proposal.md) — the third consumer, and the
  declarative-contributions rule it asks the register for
