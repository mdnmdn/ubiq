---
id: feat-workbench
title: The workbench
kind: feature
status: draft
summary: The window's shell — the activity rail and its modes, the three panels around the centre, the file explorer, the editor, and the status bar that reports on all of it.
read_when: you are changing the window layout, a rail mode, panel visibility or resizing, the explorer, the editor tabs or the status bar
updated: 2026-08-31
verified: 2026-08-31
code_anchors: [crates/ubiq/src/app.rs, crates/ubiq/src/ui/shell.rs, crates/ubiq/src/ui/rail.rs, crates/ubiq/src/ui/titlebar.rs, crates/ubiq/src/ui/project_menu.rs, crates/ubiq/src/ui/explorer.rs, crates/ubiq/src/ui/editor.rs, crates/ubiq/src/ui/status_bar.rs, crates/ubiq/src/state/mod.rs, crates/ubiq/src/state/workbench.rs]
depends_on: [tech-ui]
review_cycle: monthly
---

# The workbench

## Purpose

The workbench is the window a user leaves open all day. It puts the project's files, the file being
read, the terminals the agents run in, and the conversation driving them on one screen, so that
following an agent's work does not mean switching windows. It is the frame every other feature is
seen through, and it is built against [`../design/ubiq-layout.png`](../design/ubiq-layout.png).

## Behaviour

**The rail selects what the middle of the window is for.** Five destinations in two groups: `Control`
under `APP`, and `IDE`, `Agents`, `KB` and `Tasks` under `PROJECT`. Exactly one is active, and the
active one is shown by the accent colour on both its icon and its label.

**Only IDE mode is built.** The other four render one empty page naming the mode and what it will
hold. This is a stated gap, not an error state.

**Every panel belongs to IDE mode.** Explorer, editor, terminal dock and chat are all IDE furniture
and leave together when the mode changes. The chat is written to be reused by the other screens
later, but it is not shared furniture today.

**A project is a colour.** Each project owns one of the theme's swatches, and wears it in four
places at once: its dot in the picker, the fill behind its name in the titlebar, the mark above the
rail, and the window's left edge. Two windows on two projects are told apart without reading
anything.

**The project picker is a small manager, not a list of values.** It searches on name and path, and
divides into the projects that are open and the ones only remembered. An open project can be closed
from its row — and if it still has terminals running, the row turns into a question rather than
taking the click. Any project can be sent to a window of its own, which opens a second window
pointed at it; the two share nothing but the palette.

**The middle of the titlebar is one field for finding and for doing.** File search and commands go
to the same place, marked `⌘K`. There is no breadcrumb: the titlebar says which project, the tab
strip says which file, and repeating it in the middle bought nothing.

**Three panels, each independently shown and sized.** The titlebar's three toggles control the
explorer, the terminal dock and the chat. Each panel drags to a new size within its own limits, and
hiding a panel does not disturb the sizes of the others.

**The explorer states git position by colour and by badge.** Modified, untracked, conflicted, staged
and ignored each take a colour from the status group and a single-letter badge — the colour so it
reads at a glance, the badge so it does not rely on colour alone. Ignored rows are drawn faint.
Clicking a folder expands it; clicking a file selects it and, when it is open, brings its tab
forward. Typing in the "Go to file…" field filters on the path and forces matching folders open.

**Each editor tab keeps its own buffer.** Switching tabs writes the outgoing buffer back first, so
an edit survives the move. The tab's dot carries the file's git state, and the active tab is marked
on its bottom edge.

**The status bar reports facts, not intentions.** Branch with ahead and behind counts, the working
tree's totals, the caret's real one-based line and column, the active file's language, encoding and
line ending, and the harness and mode the composer is set to.

**The theme toggle switches both palettes at once.** Ubiq's tokens and the component library's own
theme move together, so the editor and the chat's markdown never sit in a different mode from the
chrome around them.

## Contract

No transport message. Everything the workbench shows is UI state seeded from
`crates/ubiq/src/state/sample.rs`; the terminal dock is the one part with a message family behind
it, and that belongs to [`panes-and-terminals.md`](./panes-and-terminals.md).

## The window's areas

Nine areas, and every one of them is a module. The table is the map a change starts from: it says
which file draws an area, where it sits, what fixes or bounds its size, and what owns its state.

| Area | Module | Sits | Size | State |
|---|---|---|---|---|
| Titlebar | `ui/titlebar.rs` | Top, full width beside the mark | `TITLEBAR_HEIGHT`, fixed | `WorkbenchState` |
| Project picker | `ui/project_menu.rs` | In the titlebar, leftmost | Its own popup width | `WorkbenchState::projects` |
| Rail | `ui/rail.rs` | Left, full height | `RAIL_WIDTH`, fixed | `WorkbenchState::rail_mode` |
| Explorer | `ui/explorer.rs` | Left panel | `EXPLORER_WIDTH`/`_MIN`/`_MAX` | `ExplorerState` |
| Editor | `ui/editor.rs` | Centre, above the dock | Grows | `EditorPaneState` + `Entity<EditorState>` |
| Terminal dock | `ui/terminal.rs` | Centre, below the editor | `DOCK_HEIGHT`/`_MIN`/`_MAX` | The panes on `AppState` |
| Chat | `ui/chat/` | Right panel | `CHAT_WIDTH`/`_MIN`/`_MAX` | `ChatState` |
| Empty page | `ui/empty.rs` | Replaces the centre outside IDE mode | Grows | `RailMode` |
| Status bar | `ui/status_bar.rs` | Bottom, full width | `STATUS_BAR_HEIGHT`, fixed | Read from everything above |

Two rules hold across all of them. **The chrome does not resize and the panels do** — titlebar, rail
and status bar take fixed constants, while the three panels are resizable panels with a default and
two bounds. And **a hidden panel is hidden, not removed**: visibility is the resizable panel's own
`visible` flag, so the sizes of its neighbours survive a toggle.

To add an area: give it a module under `ui/`, its state under `state/`, its size constants in
`theme.rs` if it is a panel, a row in this table, and a place in `shell.rs`. To add a rail mode: a
variant on `RailMode`, its label, note and icon, and the branch in `shell.rs` that says what fills
the centre.

## What a window owns

A window is one `AppState`. Everything the workbench shows belongs to it — the project it points at,
its panel sizes, its explorer, its editor buffers, its chat.

Two things are process-wide instead: the **palette**, so a second window opens in the mode the first
is in, and the **component library's registration**, done once at boot. Nothing else is shared, which
is why two windows on the same project would keep two independent copies of its state. The open
project set is one of those copies today, and whether it should be is an open question in
[`../backlog.md`](../backlog.md).

## Implementation

`AppState` in `crates/ubiq/src/app.rs` is the root view and owns everything: the panes, the focused
pane, the layout mode, and the workbench, explorer, editor and chat state, plus the component
library's `EditorState`, `TextareaState` and `InputState` entities and the subscriptions that keep
them mirrored. Every mutator ends in `cx.notify()`.

`open_project_window` in `crates/ubiq/src/app.rs` is the only place a window is created, so the
first window and "open in a new window" reach the same code. Each window owns its own `AppState`.

`crates/ubiq/src/ui/shell.rs` assembles the frame: titlebar, then the rail beside an `h_resizable`
group of explorer, centre and chat, then the status bar. The centre is a `v_resizable` group of
editor and dock in IDE mode, and the empty page otherwise. Panels are hidden with the resizable
panel's own `visible` flag rather than by removing them, which is what keeps their sizes stable
across a toggle.

The rest is one module per area: `rail.rs`, `titlebar.rs`, `project_menu.rs`, `status_bar.rs`,
`explorer.rs`, `editor.rs`, `terminal.rs`, `empty.rs`, and `chat/`. The project picker is its own
module rather than a `Picker`, because a project row carries actions and a confirmation and is not
just a value. Shared primitives are in `ui/kit/`; the
conventions behind that split are in [`../tech/ui-and-design.md`](../tech/ui-and-design.md).

State types live under `crates/ubiq/src/state/`: `workbench.rs` for the rail mode, panel visibility
and the open menu; `explorer.rs` for the tree and its git states; `editor.rs` for the open files.

## Failure

| What happens | Result |
|---|---|
| The last editor tab is closed | The editor keeps the previous file, or shows an empty buffer if none remain |
| A filter matches nothing | The tree renders empty; the filter field keeps what was typed |
| Every panel is hidden | The rail, titlebar and status bar remain; the centre fills the window |
| The last open project is closed | It stays open. The window is never pointed at nothing |
| A project with terminals is closed | The row asks first, and closes only on a second, explicit click |
| The last window is closed | The application quits. Closing one of several does not |
| A rail mode has no screen | The empty page names the mode and says it is not built |

## Related docs

- [`panes-and-terminals.md`](./panes-and-terminals.md) — what the dock's tabs actually are
- [`chat.md`](./chat.md) — the panel that survives every mode switch
- [`../tech/ui-and-design.md`](../tech/ui-and-design.md) — the tokens and the component conventions
- [`../backlog.md`](../backlog.md) — what the shell still lacks

## Next steps

- Drive the explorer, the editor and the status bar from a real folder rather than fixtures.
- Build the Control, Agents, KB and Tasks screens.
- Persist panel sizes, visibility and the open project set across restarts.
- Give each window its own project working tree, rather than one set of fixtures.
- Keyboard navigation for the rail, the tabs and the explorer.
