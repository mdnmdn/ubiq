---
id: feat-workbench
title: The workbench
kind: feature
status: draft
summary: The window's shell — the activity rail and its modes, the three panels around the centre, the file explorer, the editor, and the status bar that reports on all of it.
read_when: you are changing the window layout, a rail mode, panel visibility or resizing, the explorer, the editor tabs or the status bar
updated: 2026-08-31
verified: 2026-08-31
code_anchors: [crates/ubiq/src/app.rs, crates/ubiq/src/state/when.rs, crates/ubiq/src/state/prefs.rs, crates/ubiq-host/src/projects.rs, crates/ubiq/src/ui/shell.rs, crates/ubiq/src/ui/logs.rs, crates/ubiq/src/ui/rail.rs, crates/ubiq/src/ui/titlebar.rs, crates/ubiq/src/ui/project_menu.rs, crates/ubiq/src/ui/explorer.rs, crates/ubiq/src/ui/editor.rs, crates/ubiq/src/ui/status_bar.rs, crates/ubiq/src/state/mod.rs, crates/ubiq/src/state/workbench.rs, crates/ubiq/src/state/windows.rs]
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

**Every window is named by a letter.** Each takes the lowest letter no live window is using — `A`,
`B`, `C`… — printed in the picker's trigger beside the project name, and beside every open project
in the list. A closed window gives its letter back, so the names stay as short as the set of
windows. The letter is in the operating system's window title too, which is what the window
switcher shows.

**A project is open in exactly one window.** Openness is not a flag on the project — it is which
window holds it. Opening a project somewhere therefore takes it from wherever it was, and that is
the only way a project moves between windows.

**A window with no project open closes — unless there are no projects at all.** On a first run the
catalogue is empty, and a window with nothing open still has "Add a project…" to offer, so it stays.
Once a project exists the ordinary rule is back.

**A window with no project open closes.** A window's whole purpose is the projects it holds; with
none it has nothing to show and nothing to be named after. Closing the last project in a window
closes the window, and so does taking it into another one. If it was the last window, the
application quits with it.

**The project picker is a small manager, not a list of values.** It searches on name and path, and
divides into three groups, top to bottom: **open in this window**, **open in another window** with
the letter of the window holding each, and **history** — everything open nowhere, with how long ago
it was. A group with no rows is not drawn. A project's row moves between the groups as the project
moves between windows, in every window's picker at once.

**A project's folder can go away, and the picker says so rather than repairing it.** A row whose
folder is missing, is not a directory, or cannot be read prints its path in the warning colour with
a mark beside it, and offers **Locate** — which re-points the record through the system folder
chooser and keeps the id, the colour and the history. A record is never removed because its folder
went: an unplugged drive and a worktree mid-rebase are both temporary, and forgetting is always the
user's action.

**Every row can be renamed, recoloured and forgotten.** Renaming expands the row into a field;
recolouring expands it into the palette's swatches; forgetting asks first and says what it will do.
Forgetting drops the record and everything Ubiq remembers about the project, and touches nothing
inside the project's own folder — which is why the word is "Forget".

**Choosing a project's folder is the operating system's dialog**, both for Add and for Locate — the
chooser the user already knows, with their bookmarks, their network volumes and a path field. Ubiq
draws no folder browser of its own for this. Opening a file or a folder *inside* a project stays in
the interface, where the explorer is. Adding a folder already in the catalogue points at the project
that is there rather than making a second.

**Each group's rows carry the actions that group needs.** In this window: click to point the window
at it, `Close` to close it, `ExternalLink` to send it to a window of its own. In another window:
click to bring that window to the front — which is how the user moves between windows — or
`ArrowLeft` to take the project into this one. In history: click to open it here, or `ExternalLink`
to open it in a new window. Closing a project that still has terminals running turns the row into a
question rather than taking the click.

**The middle of the titlebar is one field for finding and for doing.** File search and commands go
to the same place, marked `⌘K`. There is no breadcrumb: the titlebar says which project, the tab
strip says which file, and repeating it in the middle bought nothing.

**Three panels, each independently shown and sized.** The titlebar's three toggles control the
explorer, the terminal dock and the chat. Each panel drags to a new size within its own limits, and
hiding a panel does not disturb the sizes of the others. The dock holds one tab that is not a pane —
the log console, which is [`logs.md`](./logs.md).

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
line ending, and the harness and mode the composer is set to. It also says where Ubiq is writing,
whenever that is not the usual `~/.config/ubiq` — a config root you cannot see is a foot-gun.

**The window remembers where its furniture was left.** Panel visibility and sizes and the rail mode
belong to a project; the palette belongs to the interface. Both are stored by the host, which keeps
them as an opaque blob it never reads, so the schema stays the interface's own. A blob it cannot
parse is discarded and the window opens on defaults.

**The theme toggle switches both palettes at once.** Ubiq's tokens and the component library's own
theme move together, so the editor and the chat's markdown never sit in a different mode from the
chrome around them.

## Contract

**Projects cross the bus.** The catalogue belongs to the host, and the workbench holds a projection
of it: `ListProjects`, `AddProject`, `ForgetProject`, `UpdateProject`, `LocateProject`,
`OpenedProject` and `RefreshProject` going out, `ProjectList`, `ProjectAdded`, `ProjectChanged`,
`ProjectForgotten` and `ProjectError` coming back, and `GetPreferences`/`SetPreferences` behind
everything the window remembers. A chosen folder reaches the host as a path in `AddProject` or
`LocateProject`; the choosing itself is the platform's, and crosses nothing. The
full family is [`../tech/transport-contract.md`](../tech/transport-contract.md).

The rest of what the workbench shows — the explorer, the editor, the branch and the working-tree
counts — is still UI state seeded from `crates/ubiq/src/state/sample.rs`. The terminal dock has its
own family, in [`panes-and-terminals.md`](./panes-and-terminals.md).

## The window's areas

Ten areas, and every one of them is a module. The table is the map a change starts from: it says
which file draws an area, where it sits, what fixes or bounds its size, and what owns its state.

| Area | Module | Sits | Size | State |
|---|---|---|---|---|
| Titlebar | `ui/titlebar.rs` | Top, full width beside the mark | `TITLEBAR_HEIGHT`, fixed | `WorkbenchState` |
| Project picker | `ui/project_menu.rs` | In the titlebar, leftmost | Its own popup width | `WindowRegistry`, process-wide, projecting the host's catalogue |
| Rail | `ui/rail.rs` | Left, full height | `RAIL_WIDTH`, fixed | `WorkbenchState::rail_mode` |
| Explorer | `ui/explorer.rs` | Left panel | `EXPLORER_WIDTH`/`_MIN`/`_MAX` | `ExplorerState` |
| Editor | `ui/editor.rs` | Centre, above the dock | Grows | `EditorPaneState` + `Entity<EditorState>` |
| Terminal dock | `ui/terminal.rs` | Centre, below the editor | `DOCK_HEIGHT`/`_MIN`/`_MAX` | The panes on `AppState` |
| Log console | `ui/logs.rs` | The dock's last tab | The dock's | `LogState` over the process-wide sink |
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

A window is one `AppState`. Almost everything the workbench shows belongs to it — its panel sizes,
its explorer, its editor buffers, its chat. Which project it points at is the exception: that is the
registry's answer, not the window's.

Four things are process-wide instead: the **palette**, so a second window opens in the mode the
first is in, the **component library's registration**, done once at boot, the **bus's hub**, so
every window reaches the one host, and the **window registry** — the projection of the host's
catalogue, and which window holds which project. The registry has to be shared: no window can
answer "where is this project open?" from a copy of its own.

The catalogue itself is not the window's and not the registry's. It belongs to the host, and
arrives as snapshots the registry replaces by id — which is why every window's picker agrees
without any of them asking twice.

A window's identity is its `WindowId`, which is its key into the registry. Everything else — panel
sizes, explorer, editor buffers, chat — is the window's alone, which is why two windows on the same
project would keep two independent copies of its state. The registry makes that unreachable rather
than merely unlikely: a project is open in one window at a time.

## Implementation

`AppState` in `crates/ubiq/src/app.rs` is the root view and owns everything: the panes, the focused
pane, the layout mode, and the workbench, explorer, editor and chat state, plus the component
library's `EditorState`, `TextareaState` and `InputState` entities and the subscriptions that keep
them mirrored. Every mutator ends in `cx.notify()`.

`open_project_window` in `crates/ubiq/src/app.rs` is the only place a window is created, so the
first window and "open in a new window" reach the same code. It seeds the registry, allocates the
window's letter — before the window exists, because the title carries it — and each window owns its
own `AppState`. `focus_window` brings one to the front; `window_closed`, called from `main.rs`, drops
a closed window's slot so everything it held returns to history.

`WindowRegistry` in `crates/ubiq/src/state/windows.rs` is the process-wide half, held as a GPUI
global. It holds the projection — `replace_all` for a `ProjectList`, `apply` for one snapshot,
`forget` for a `ProjectForgotten` — and one `WindowSlot` per live window — its letter, the projects
open in it, and which of them it is pointed at. `register`, `open_in`, `activate` and `close` are the
four mutations, and each answers with the windows it emptied; `AppState::close_windows` closes those,
deferred, because the caller is usually inside one of them. `groups` computes the picker's three
lists for one window. Every `AppState` subscribes with `observe_global`, so a move in one window
redraws the picker in all of them, and reads go through `WindowRegistry::read` rather than
`default_global`, which would notify the observers on a plain read and spin the frame. The registry
is pure logic and is tested without a frame in `crates/ubiq/tests/windows.rs`, which seeds it the
way the host does.

`reap` is where the empty-catalogue rule lives: it returns no windows at all while the projection is
empty, so a first run keeps the window it opened on nothing. `state/when.rs` renders a row's
relative time at draw time from `last_opened_at`, and `state/prefs.rs` is the schema inside the
opaque blob the host stores.

`crates/ubiq/src/ui/shell.rs` assembles the frame: the mark and the titlebar in one row, then the
rail beside an `h_resizable` group of explorer, centre and chat, then the status bar. The mark is
drawn by `rail::mark` in that first row so it sits in the corner above the rail rather than inside
it. The centre is a `v_resizable` group of
editor and dock in IDE mode, and the empty page otherwise. Panels are hidden with the resizable
panel's own `visible` flag rather than by removing them, which is what keeps their sizes stable
across a toggle.

The rest is one module per area: `rail.rs`, `titlebar.rs`, `project_menu.rs`, `status_bar.rs`,
`explorer.rs`, `editor.rs`, `terminal.rs`, `empty.rs`, and `chat/`. The project picker is its own
module rather than a `Picker`, because a project row carries actions and a confirmation and is not
just a value. Shared primitives are in `ui/kit/`; the
conventions behind that split are in [`../tech/ui-and-design.md`](../tech/ui-and-design.md).

State types live under `crates/ubiq/src/state/`: `workbench.rs` for the rail mode, panel visibility
and the open menu; `explorer.rs` for the tree and its git states; `editor.rs` for the open files;
`logs.rs` for the console's filter.

## Failure

| What happens | Result |
|---|---|
| The last editor tab is closed | The editor keeps the previous file, or shows an empty buffer if none remain |
| A filter matches nothing | The tree renders empty; the filter field keeps what was typed |
| Every panel is hidden | The rail, titlebar and status bar remain; the centre fills the window |
| The dock is hidden while the console is its tab | The console goes with the dock, and comes back to the same tab |
| The last project in a window is closed | The window closes with it. If it was the last window, the application quits |
| A project with terminals is closed | The row asks first, and closes only on a second, explicit click |
| A project open in another window is opened here | It leaves that window, which closes if it held nothing else |
| More than 26 windows are open | The 27th and beyond are named `#`; nothing else changes |
| The last window is closed | The application quits. Closing one of several does not |
| A rail mode has no screen | The empty page names the mode and says it is not built |
| A project's folder is deleted, renamed or unmounted | The next probe marks the row; the record stays and the window keeps its last screen |
| A marked project is located again | The record keeps its id, colour and history; only its path moves |
| A folder already in the catalogue is added again | The picker points at the project that is there; no duplicate appears |
| The catalogue is empty | The window stays open on the picker, which offers to add one |
| The catalogue file is corrupt | It is preserved under a timestamped name, the session starts empty, and one error says so |
| The catalogue cannot be written | Changes hold for the session and one error says they are not durable |
| A window's view state is corrupt or from another schema | It is discarded and the window opens on defaults |

## Related docs

- [`panes-and-terminals.md`](./panes-and-terminals.md) — what the dock's tabs actually are
- [`chat.md`](./chat.md) — the panel that survives every mode switch
- [`../tech/ui-and-design.md`](../tech/ui-and-design.md) — the tokens and the component conventions
- [`../backlog.md`](../backlog.md) — what the shell still lacks

## Next steps

- Drive the explorer, the editor and the status bar from a real folder rather than fixtures.
- Build the Control, Agents, KB and Tasks screens.
- Give each window its own project working tree, rather than one set of fixtures.
- Keyboard navigation for the rail, the tabs and the explorer.
