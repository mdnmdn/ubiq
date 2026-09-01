---
id: feat-workbench
title: The workbench
kind: feature
status: draft
summary: The window's shell — the activity rail and its modes, the three panels around the centre, the file explorer and editor a project owns, the agents screen and the tasks board the rail's other built modes hold, the empty state a window with no project shows, and the status bar that reports on all of it.
read_when: you are changing the window layout, a rail mode, panel visibility or resizing, the explorer, the editor tabs, saving a file, the agents screen's graph, how it arranges itself, its inspector or its tasks, the tasks board's columns, cards or task panel, or the status bar
updated: 2026-09-01
verified: 2026-09-01
code_anchors: [crates/ubiq/src/app.rs, crates/ubiq/src/state/when.rs, crates/ubiq/src/state/prefs.rs, crates/ubiq-host/src/projects.rs, crates/ubiq/src/ui/shell.rs, crates/ubiq/src/ui/logs.rs, crates/ubiq/src/ui/rail.rs, crates/ubiq/src/ui/titlebar.rs, crates/ubiq/src/ui/project_menu.rs, crates/ubiq/src/ui/explorer.rs, crates/ubiq/src/ui/editor.rs, crates/ubiq/src/ui/empty.rs, crates/ubiq/src/ui/status_bar.rs, crates/ubiq/src/state/mod.rs, crates/ubiq/src/state/workbench.rs, crates/ubiq/src/state/windows.rs, crates/ubiq/src/state/explorer.rs, crates/ubiq/src/state/editor.rs, crates/ubiq/src/state/agents.rs, crates/ubiq/src/state/layout.rs, crates/ubiq/src/state/board.rs, crates/ubiq/src/state/sample.rs, crates/ubiq/src/ui/agents/mod.rs, crates/ubiq/src/ui/agents/graph.rs, crates/ubiq/src/ui/agents/inspector.rs, crates/ubiq/src/ui/agents/tasks.rs, crates/ubiq/src/ui/board/mod.rs, crates/ubiq/src/ui/board/detail.rs, crates/ubiq/tests/agents.rs, crates/ubiq/tests/board.rs]
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

**Three modes are built.** IDE fills the centre with the editor over the terminal dock; Agents
fills it with the orchestration graph; Tasks fills it with the board. `Control` and `KB` render one
empty page naming the mode and what it will hold. This is a stated gap, not an error state.

**The window's three panels belong to IDE mode.** Explorer, editor, terminal dock and chat are IDE
furniture and leave together when the mode changes. The chat is written to be reused by the other
screens later, but is not shared furniture today. A screen that wants a panel brings its own: the
agents screen's inspector and tasks drawer, and the board's task panel, are drawn inside the centre
rather than in the window's resizable group, which is why they toggle instead of dragging and go
with the mode.

**The agents screen is one field of state and everything else.** A selection is either a **session**
— a named piece of work — or an **agent**, which is one workspace: one running harness, one
terminal. Which session the graph draws, what the inspector reports and which tasks the drawer lists
are all functions of that one field, so the three cannot disagree about what the user is looking at.

**What a thing is and where it is drawn are separate, and the graph arranges itself.** No position
is authored anywhere. Position is held apart from the definitions and held relative — a task owns an
origin, an agent owns an offset inside the task it serves. Containers flow left to right and wrap;
inside one, cards stack by how far they are from whoever started the work, which draws the three
shapes without naming them: one agent is one card, a chain is a column, a coordinated task is a
coordinator over a row of workers. An agent nobody gave work to sits above the containers. The
toolbar's tidy control asks for the arrangement again, discarding every hand-placed position — the
only thing that undoes a drag.

**The graph draws one session's agents as cards on a dotted ground.** A card carries the agent's
name, role, state, the one line it says, its branch and its token count, in the colour of the
state's bucket. Four filter pills — running, waiting, ended, error — decide which buckets are drawn,
and the last one lit cannot be turned off: a graph emptied by a filter looks like an empty session.
Zoom scales positions, cards and type together, so the graph reads the same at every step.

**A task is an outline round the cards serving it, not a container they sit in.** The dashed box is
computed each frame from where its cards are, so dragging a card takes the outline with it, and a
task whose cards are all filtered out has no box. Its shape — direct, chain or coordinated — is
printed on the outline: whether the agents run in order is a fact about the task, not about any one
of them.

**A card is carried, and dropping it inside another task's outline moves it there.** The card itself
follows the pointer rather than a ghost, and the box it would land in lights up while it is in the
air. Which task it landed in is worked out from where it came to rest, not from what took the drop:
an outline takes no clicks. A card that changes task loses its parent but stays where it was let go
of; one put down on open ground keeps both.

**A container is dragged by the ground inside it, and takes everything with it.** The empty space in
a box is the handle, and the cards drawn over it take their own drags, so grabbing a card moves one
agent and grabbing anywhere else moves the task. Only the container's origin moves, so nothing
inside can fall out of step, and a container is never dropped into another one.

**A drag leaves a trail, and it is the only motion on the screen.** Grains fall where the pointer
passed and shrink, drift and fade over the next two-thirds of a second, so a thing that moved reads
as held rather than as a redraw. Cards and containers both shed it; reduced motion skips it.

**The inspector reports whatever is selected, at that selection's scale.** A session gives its
branch and how its agents are spread across the four states; an agent gives its harness, its model,
what is left of its context window, its thread and a composer. Its tabs are that thread and the
drawer's own task list, and the toolbar dismisses the panel and brings it back.

**The composer is real, and nothing answers it.** What is typed lands in the selected agent's
thread, which says in as many words that nothing is listening: a fabricated reply is the one thing a
screen with no transport family must not draw. Enter sends, Shift-Enter inserts a newline, and the
draft is the agents screen's own rather than the chat's.

**The board and the graph are two views of one set of tasks.** The graph answers "who is doing
what"; the board answers "what is there, and where has it got to" — the same tasks, at the scale of
the project rather than of one session. Nothing is copied between them: a task ticked on the board
is ticked in the drawer under the graph, and `Show in graph` is one click because the two screens
are two questions about one set of facts.

**A column is a stage, and a card only ever changes column.** Backlog, ready, in progress, in
review, done: moving a card changes where the work has got to and nothing else about it. Each column
carries its own count and a dot in the token that means what the stage means — nothing yet, queued,
moving, waiting on a person, over.

**A card is filed, not placed.** Unlike the graph's canvas, the column *is* the drop target: a
label follows the pointer while the card stays where it is, the column under the pointer lights up,
and the box that took the drop is the answer. A drag that ends anywhere else changes nothing, and
the card is left in the column it came from.

**A column shuts to a strip and a card shuts to its title.** A board is read by ignoring most of it,
so both fold: a shut column keeps its dot, its count and its name written downwards, and still takes
a drop; a folded card keeps its shape, its title and whose session it is. Neither is a filter —
what is shut is still counted.

**One field finds work and names it.** The filter matches on what a card actually prints, its title
and its session, and `New task` names the next one after whatever is in that field — so typing to
look for a card that turns out not to exist is already most of making it. The new task lands in the
backlog, in the session the pills are on, and the field clears rather than leaving the board
filtered down to the one card just made.

**A card carries the worst thing happening in its task.** Its left edge is the state the user would
want to be told first: a failed sub-task beats one waiting on a person, which beats one moving. The
line under it names the agent the task speaks through — the coordinator of a coordinated task,
whoever is holding it now for any other shape — and clicking that name opens its conversation. A
task nobody has started says so, and counts its sub-tasks instead.

**The task panel reports one task whole, and its checkboxes are the one thing on this screen that
changes the work.** Its session and whether that session is a worktree, its shape in a sentence, who
it speaks through, and every sub-task with the agent that has it and where that has got to. Ticking
one is a change to the task rather than to the view of it; unticking lands on idle, because nothing
here can know what its owner would go back to doing.

**Both ways out of a task lead to the agents screen.** `Show in graph` switches the mode and points
the graph at whoever is doing the task; `Open …'s chat` does the same and puts the inspector on that
agent's thread. A task the user wants to intervene in is a conversation with an agent, and the
conversation lives there.

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

**A window with no project open stays open, on a screen of its own.** No panes, an explorer that
says it has nothing to show, no open files, and "Add a project…" in the middle of the window. This
is where a first run starts, where closing a project leaves the window, and where taking a project
into another window leaves the one it came from.

**Ubiq never closes a window.** Only the user does, and the application quits with its last one. A
window holding nothing is a window waiting for a project, not an error to be tidied away.

**The empty state is three screens at once.** The centre says no project is open and offers to add
one; the explorer panel keeps its place and its width, with one muted line in it rather than a tree;
the chat is hidden, because a conversation about nothing is a fiction. The dock stays, holding only
the log console — the one panel a window with no project has a reason to show — and its `+` is gone,
because there is no folder to start a harness in.

**The explorer, the open files and the terminals belong to a project, not to the window.** A window
holds one set of each per project open in it, and switching between them is a lookup: the tree, the
tabs and the dock all change together, and nothing is re-read or rebuilt. The terminals of the
projects behind keep running and keep their scrollback.

**A project leaving the window takes its panes with it.** Closing it, moving it to another window, or
forgetting it kills the harnesses running in it — a pane's working directory is that project's
folder, and no other window can adopt a running emulator. What the project *remembers* is written
down first, so reopening it brings its files back.

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

**The explorer draws the project's folder, one directory at a time.** Opening a project asks the host
for its top level; expanding a folder asks for that folder's children, and only then. A repository's
`node_modules` is therefore one row rather than a walk, and a tree the user never opens costs
nothing. Clicking a folder expands it; clicking a file opens it; typing in the "Go to file…" field
matches on the path.

**A row the host will not follow is drawn and does nothing.** A symlink leading out of the project or
nowhere, a socket, a device, a pipe: the row appears, faint, and takes no click. Drawing it is the
point — a tree with rows missing is a tree that lies about what is in the folder.

**The explorer holds project-relative paths and nothing else.** No absolute path reaches the
interface, for the same reason no file descriptor does: the folder the tree describes is the host's,
and the two need not be on one machine.

**The explorer states git position by colour and by badge — for the states something fills in.**
Modified, untracked, conflicted, staged and ignored each take a colour from the status group and a
single-letter badge, the colour so it reads at a glance and the badge so it does not rely on colour
alone. Nothing reads version control, so every row draws unmarked and the status bar carries no
branch. An unfilled mark is an absence, not a claim that a repository was consulted and answered
clean.

**A tab exists from the click that asked for the file.** It appears at once, says it is reading, and
fills when the bytes arrive — so a click has an effect, a second click cannot ask for the same file
twice, and a read that fails has somewhere to say so. Bytes that arrive for a project the window has
since switched away from are still put in their tab; bytes for a tab that has been closed are
dropped.

**Each open file owns its buffer.** Switching tabs and switching projects both leave a buffer exactly
as it was, with its undo history, its selection and its scroll — nothing is copied in or out. The
active tab is marked on its bottom edge, and the tab's dot reports the *file*: reading, saving, a
failed save, or an unsaved edit.

**`⌘S` writes the active file back, and names the version it read.** A save the host refuses because
the file moved under it is reported on the tab and in the status bar, and the file is left alone —
Ubiq is not the only thing editing these files, and the agents in the panes are the other one. There
is no merge: resolving a conflict is the user's.

**A file that cannot be edited honestly is not offered for saving.** A read the host cut short at its
byte ceiling is readable and unsavable, because writing a prefix back would shorten the file. A file
whose bytes are not text says so instead of drawing them.

**A dirty tab asks before closing.** The first click on its × turns the tab into a question; only a
second one discards the edit. Bringing the tab forward again withdraws the question.

**The status bar reports facts, not intentions, and an absent fact is drawn as absent.** It reports
on whatever is on screen, so the rail mode decides which set of facts it has. In IDE mode with a
project open: the active file's project-relative path, what its save is doing when that is worth a
word, the caret's real one-based line and column, the file's language, encoding and line ending, and
the harness and mode the composer is set to. The caret and the language go with the file, so a window
with no file open reports neither rather than a position in nothing. With no project open it says so
and stops. On the agents screen there is no file and no caret to report, so it counts instead: how
many sessions and agents there are, and how the agents are spread across the four states, each count
in its state's colour. A count of zero is drawn as zero rather than dropped — "no agent is failing"
is a fact, and it is the one the user is checking for. On the board it counts the work instead: how
many cards are in each column, how many sub-tasks are done across the cards on screen, and how many
of them nobody can finish without the user — over the cards the filters leave, because a count that
disagrees with what is drawn is worse than none. Whichever set it is showing, it says where
Ubiq is writing whenever that is not the usual `~/.config/ubiq` — a config root you cannot see is a
foot-gun.

**The window remembers where its furniture was left, and which files were open in it.** Panel
visibility and sizes, the rail mode, the files open in the centre with which of them was in front,
the folders the explorer had expanded and the row it had selected all belong to a project; the
palette belongs to the interface. Both are stored by the host, which keeps them as an opaque blob it
never reads, so the schema stays the interface's own. A blob it cannot parse is discarded and the
window opens on defaults.

**A project closed and reopened in one session comes back as it was left**, without asking the host.
The window keeps what a project left behind when it went, so reopening it restores the tabs and the
open folders in the frame it happens, and a restart restores them from the blob the host stored.
Whichever answer arrives first is the one used; the second is ignored, so a stored blob cannot
reopen tabs the user has since closed.

**The tree is restored a level at a time.** A remembered folder cannot be opened before its parent
has been listed, so each listing that arrives opens whatever became reachable and asks for what is
below it. A remembered folder that no longer exists is dropped rather than waited on.

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

**Files cross the bus too.** The explorer and the editor are projections of the host's answers, not
state of their own: `ProjectTree`, `ReadProjectFile` and `WriteProjectFile` going out, and
`ProjectTreeListing`, `ProjectFileContents`, `ProjectFileWritten` and `ProjectFileError` coming back.
Every one of them names a project and a path inside it, and each answers only the window that asked.
The blob behind what a project remembers grows the open files, the active one, the expanded folders
and the selected row, and the host neither parses nor validates any of it.

Two fixtures are left. `crates/ubiq/src/state/sample.rs` holds the chat and the work the agents
screen and the board both draw, because neither has a transport family — [`chat.md`](./chat.md) has
the first, and the graph and the board go the same way when the second gets one, together, since
they read one set of tasks. The terminal dock has its own family, in
[`panes-and-terminals.md`](./panes-and-terminals.md).

## The window's areas

Sixteen areas, and every one of them is a module. The table is the map a change starts from: it
says which file draws an area, where it sits, what fixes or bounds its size, and what owns its
state.

| Area | Module | Sits | Size | State |
|---|---|---|---|---|
| Titlebar | `ui/titlebar.rs` | Top, full width beside the mark | `TITLEBAR_HEIGHT`, fixed | `WorkbenchState` |
| Project picker | `ui/project_menu.rs` | In the titlebar, leftmost | Its own popup width | `WindowRegistry`, process-wide, projecting the host's catalogue |
| Rail | `ui/rail.rs` | Left, full height | `RAIL_WIDTH`, fixed | `WorkbenchState::rail_mode` |
| Explorer | `ui/explorer.rs` | Left panel | `EXPLORER_WIDTH`/`_MIN`/`_MAX` | `ExplorerState`, one per project the window holds |
| Editor | `ui/editor.rs` | Centre, above the dock | Grows | `EditorPaneState` per project, and one `Entity<EditorState>` per open file |
| Terminal dock | `ui/terminal.rs` | Centre, below the editor | `DOCK_HEIGHT`/`_MIN`/`_MAX` | The active project's panes, and the window's emulators |
| Log console | `ui/logs.rs` | The dock's last tab | The dock's | `LogState` over the process-wide sink |
| Chat | `ui/chat/` | Right panel | `CHAT_WIDTH`/`_MIN`/`_MAX` | `ChatState` |
| Agents screen | `ui/agents/mod.rs` | Replaces the centre in Agents mode | Grows; its toolbar takes `TITLEBAR_HEIGHT` | `AgentsState` |
| Orchestration graph | `ui/agents/graph.rs` | The agents screen, beside the inspector | Grows; scrolls to the extent of its cards | `AgentsState` and its `Layout`, and `CARD_WIDTH`/`CARD_HEIGHT` in `state/layout.rs` |
| Inspector | `ui/agents/inspector.rs` | The agents screen, right | `INSPECTOR_WIDTH`, fixed | `AgentsState::selection`, and `agent_input` on `AppState` |
| Tasks drawer | `ui/agents/tasks.rs` | The agents screen, under the graph | `TASKS_HEIGHT` open, its header shut | `AgentsState::tasks_open` |
| Tasks board | `ui/board/mod.rs` | Replaces the centre in Tasks mode | Grows; its columns scroll sideways | `BoardState` over `AgentsState::tasks`, and `COLUMN_WIDTH`/`COLUMN_SHUT` |
| Task panel | `ui/board/detail.rs` | The board, right | `TASK_PANEL_WIDTH`, fixed | `BoardState::selected` and `show_detail` |
| Empty page | `ui/empty.rs` | Replaces the centre in `Control` and `KB` mode, and above the dock with no project | Grows | `RailMode`, or nothing at all |
| Status bar | `ui/status_bar.rs` | Bottom, full width | `STATUS_BAR_HEIGHT`, fixed | Read from everything above |

Two rules hold across all of them. **The chrome does not resize and the panels do** — titlebar, rail
and status bar take fixed constants, while the three panels are resizable panels with a default and
two bounds. And **a hidden panel is hidden, not removed**: visibility is the resizable panel's own
`visible` flag, so the sizes of its neighbours survive a toggle.

Both rules are about the window's own furniture. A screen's furniture is the screen's: the agents
screen's inspector and drawer and the board's task panel take one fixed constant each, are shown and
hidden from the screen they belong to rather than from the titlebar's toggles, and leave with the
mode.

To add an area: give it a module under `ui/`, its state under `state/`, its size constants in
`theme.rs` if it is a panel, a row in this table, and a place in `shell.rs`. To add a rail mode: a
variant on `RailMode`, its label, note and icon, and the branch in `shell.rs` that says what fills
the centre.

## What a window owns

A window is one `AppState`, and inside it one `OpenProject` per project open in that window. The
split between them is the feature's spine. **A project owns what is about that project** — its
explorer tree, its open files and their buffers, its panes, which of them holds the keyboard, and the
furniture it was last left in. **The window owns what is about the window** — its panel sizes, the
palette, the chat, the log console, which dock tab is showing, and one flat map from pane id to
emulator, because an emulator does not care which list draws it.

A project's state lives exactly as long as the window holds the project. That one rule is why
switching projects is a lookup rather than a rebuild, and why the terminals of a project nobody is
looking at keep running.

Which project the window points at is neither's: that is the registry's answer.

Four things are process-wide instead: the **palette**, so a second window opens in the mode the
first is in, the **component library's registration**, done once at boot, the **bus's hub**, so
every window reaches the one host, and the **window registry** — the projection of the host's
catalogue, and which window holds which project. The registry has to be shared: no window can
answer "where is this project open?" from a copy of its own.

The catalogue itself is not the window's and not the registry's. It belongs to the host, and
arrives as snapshots the registry replaces by id — which is why every window's picker agrees
without any of them asking twice.

A window's identity is its `WindowId`, which is its key into the registry. Everything else is that
window's alone, which is why two windows on the same project would keep two independent copies of its
tree, its tabs and its panes. The registry makes that unreachable rather than merely unlikely: a
project is open in one window at a time.

## Implementation

`AppState` in `crates/ubiq/src/app.rs` is the root view. It owns the window's own state — the layout
mode, the chat, the console, the dock's tab, the emulators, the component library's `TextareaState`
and `InputState` entities and the subscriptions that keep them mirrored — and a map of `OpenProject`
keyed by `ProjectId` holding everything that belongs to a project. Every mutator ends in
`cx.notify()`.

`sync_projects()` is the one place the map is reconciled against the registry, and it is idempotent:
it drops the projects the window no longer holds through `drop_project()`, builds an `OpenProject` for
each new one, and calls `enter_project()` when the active project changed. It runs from the
`observe_global` subscription rather than from each call site, so a project taken by *another* window
reaches this one down the same path as a local change. `enter_project()` is where a project gets its
first pane, which is why a window opening on nothing starts no harness.

Accessors read through the active project and tolerate its absence: `open_project()`, `explorer()`,
`editor()`, `panes()`, `focused_pane()` and `dock_tab()` each answer for a window with no project
without a caller having to check. `drop_project()` writes the project's blob, parks a copy against a
reopen in the same session, and kills its panes.

`open_project_window` in `crates/ubiq/src/app.rs` is the only place a window is created, so the
first window and "open in a new window" reach the same code. It seeds the registry, allocates the
window's letter — before the window exists, because the title carries it — and each window owns its
own `AppState`. `focus_window` brings one to the front; `window_closed`, called from `main.rs`, drops
a closed window's slot so everything it held returns to history.

`WindowRegistry` in `crates/ubiq/src/state/windows.rs` is the process-wide half, held as a GPUI
global. It holds the projection — `replace_all` for a `ProjectList`, `apply` for one snapshot,
`forget` for a `ProjectForgotten` — and one `WindowSlot` per live window — its letter, the projects
open in it, and which of them it is pointed at. `register`, `open_in`, `activate` and `close` are the
four mutations. None of them closes anything: `open_in` answers whether the project existed to be
opened, and the others answer nothing, because a window emptied of projects is a window on the empty
state. `groups` computes the picker's three lists for one window. Every `AppState` subscribes with `observe_global`, so a move in one window
redraws the picker in all of them, and reads go through `WindowRegistry::read` rather than
`default_global`, which would notify the observers on a plain read and spin the frame. The registry
is pure logic and is tested without a frame in `crates/ubiq/tests/windows.rs`, which seeds it the
way the host does.

`state/when.rs` renders a row's relative time at draw time from `last_opened_at`, and
`state/prefs.rs` is the schema inside the opaque blob the host stores — including the files and
folders a project reopens with.

`crates/ubiq/src/ui/shell.rs` assembles the frame: the mark and the titlebar in one row, then the
rail beside an `h_resizable` group of explorer, centre and chat, then the status bar. The mark is
drawn by `rail::mark` in that first row so it sits in the corner above the rail rather than inside
it. The centre is a `v_resizable` group of editor and dock in IDE mode, `agents::render` in Agents mode,
`board::render` in Tasks mode, and the empty page otherwise. Panels are hidden with the resizable panel's own `visible` flag
rather than by removing them, which is what keeps their sizes stable across a toggle.

The rest is one module per area: `rail.rs`, `titlebar.rs`, `project_menu.rs`, `status_bar.rs`,
`explorer.rs`, `editor.rs`, `terminal.rs`, `empty.rs`, `chat/`, `agents/` and `board/`. The project picker is
its own module rather than a `Picker`, because a project row carries actions and a confirmation and
is not just a value. Shared primitives are in `ui/kit/`; the conventions behind that split are in
[`../tech/ui-and-design.md`](../tech/ui-and-design.md).

State types live under `crates/ubiq/src/state/`: `workbench.rs` for the rail mode, panel visibility,
the open menu and what was typed into the explorer's filter; `explorer.rs` for the tree; `editor.rs`
for the open files; `logs.rs` for the console's filter; `agents.rs` for the agents screen and the tasks every screen
over the work reads; `board.rs` for the board's view of them.

`state/agents.rs` is to the agents screen what `state/explorer.rs` is to the tree: data and small
mutators, nothing that draws and nothing that names a colour. It owns the sessions, agents, tasks and
steps, the selection, the showing buckets, the zoom, what a drag is carrying and the grains behind it.
It owns what the board reads off a task too — its `Status`, its `Priority`, the `StepState` of each
step and the session it may not have yet — because a task is one record and the two screens are two
views of it. `now()` picks the agent a task speaks through, `pulse()` reduces everything happening in
one to the state its card's edge carries, and `add_task()`, `move_task()` and `toggle_step()` are the
three things the board changes.
`Held` — a card or a container — is what `start_carry()` takes and what `carry_to()` branches on.
`bounds_of()` is the box round a task's cards; `task_at()` is what a drop lands in, and leaves the
carried card out of every box it tests against; `end_carry()` is where a card that changed task loses
its parent and is re-anchored; `settle_sand()` answers whether the trail still owes a frame.

`state/layout.rs` holds every position, relative: a task's origin and an agent's offset inside it,
absolute only for an agent with no task. `at()` resolves the two, `Layout::auto()` is the arrangement
computed from the definitions alone, `relayout()` replaces one with a fresh one, and `CARD_WIDTH`,
`CARD_HEIGHT`, `GROUP_PAD` and `GROUP_LABEL` live there because the outlines, the connectors and the
hit testing work from them. Both are tested without a frame in `crates/ubiq/tests/agents.rs`, which
asserts no position against the fixture — it has none.

`AppState` carries the screen as `agents` and the composer as `agent_input`, a `TextareaState` of its
own so the two drafts cannot leak into each other. `start_graph_carry()` selects a card and does not
select a container; `move_graph_carry()` honours reduced motion, moving what is held without laying a
grain; `tidy_graph()` is the tidy control; `settle_graph()`, called from `render` beside the other
end-of-frame passes, ages the trail and puts down a carry whose drag ended where the canvas's drop
handler never sees it. `ui/agents/mod.rs` is the frame and its `activity_colour()` is the one place a
state becomes a colour; `graph.rs`, `inspector.rs` and `tasks.rs` are its three areas, painted from
the layers in `ui/kit/canvas.rs`.

`state/board.rs` is the *view* of those tasks, and holds nothing that is a fact about one: the filter
text, which session's pills are on, which task is open, which columns and cards are shut, and the
carry. `column()` is what one column draws, `matches()` is the filter both it and the status bar's
counts go through, and `end_carry()` answers the task and the column it landed in. It is tested
without a frame in `crates/ubiq/tests/board.rs`.

`AppState` carries it as `board` and the filter field as `task_filter`, an `InputState` of its own.
`new_task()` is where the field becomes a title and the task lands in the backlog; `drop_task()` is
the column's own drop handler, because the column is the drop target here; `settle_board()`, beside
`settle_graph()` in `render`, puts down a carry whose drag ended outside every column.
`ui/board/mod.rs` is the toolbar, the columns and the cards, and its `status_colour()` is the one
place a column becomes a colour; `ui/board/detail.rs` is the panel.

`state/explorer.rs` holds every piece of tree logic and no frame, which is what makes it testable in
`crates/ubiq/tests/explorer.rs`. `merge()` puts one directory's listing into the tree, matching
entries by name so that a folder re-listed keeps the children and the expanded flags below it, an
entry that has gone is dropped with its subtree, and a new one arrives shut and unlisted — which also
makes an unsolicited listing harmless. `toggle()` answers whether flipping a folder open means the
host has to be asked. `expanded()` is what gets written down and `reopen()` is what reads it back,
opening the folders a blob named as each of their parents arrives. The order the host sorted a
listing in is kept rather than sorted again, so two windows on one project cannot disagree.

`state/editor.rs` names the component library, unlike its neighbours, because a file's buffer *is*
its state: `FileBody` is either `Loading`, the `Text` of a buffer with the bytes the host sent beside
it, `Binary`, or a `Failed` read. Dirtiness is that comparison against the host's bytes, cached off
the buffer's own change event rather than recomputed per frame. `FileLanguage::of()` picks a
highlighter from the path's extension, and anything it does not recognise opens as plain text, which
is the general case rather than a fallback.

The file path through the two halves: `select_file()` opens a tab and sends `ReadProjectFile`;
`toggle_folder()` sends `ProjectTree` when a folder has never been listed; `save_active_file()` sends
`WriteProjectFile` with the version the read came with. Contents cannot become a buffer where they
arrive, because a buffer needs a window and a message does not come with one, so they queue and
`attach_arrived_files()` drains them in `render` — the same device the panel sizes and the pending
focus already use. `install_key_bindings()` binds `⌘S` in the `Workbench` key context, and the binary
calls it beside its own quit binding.

## Failure

| What happens | Result |
|---|---|
| The last editor tab is closed | The centre says no file is open, and the status bar reports no caret and no language |
| A filter matches nothing | The tree renders empty; the filter field keeps what was typed |
| Every panel is hidden | The rail, titlebar and status bar remain; the centre fills the window |
| The dock is hidden while the console is its tab | The console goes with the dock, and comes back to the same tab |
| The last project in a window is closed | The window stays, on the empty state. Its harnesses are killed with the project, and what it remembered is written down |
| A project with terminals is closed | The row asks first, and closes only on a second, explicit click |
| A project open in another window is opened here | It leaves that window, which stays open on the empty state if it held nothing else. Its panes are killed rather than moved |
| More than 26 windows are open | The 27th and beyond are named `#`; nothing else changes |
| The last window is closed | The application quits. Closing one of several does not |
| A rail mode has no screen | The empty page names the mode and says it is not built |
| Nothing is selected on the agents screen | The inspector says so and points at the toolbar and the graph. The graph and the drawer fall back to the first session, so neither goes blank |
| Every agent in a session is filtered out | The graph says no agent matches the filters. The last pill lit cannot be turned off, so an empty graph is always an empty session |
| A task's cards are all hidden | No outline is drawn for it. The task keeps its place in the drawer's list |
| A card is dropped outside the graph | The next frame puts it down where the drag left it, so it cannot stay stuck to the pointer |
| A card is dropped on open ground | It keeps its task and its parent, and stays where it was put |
| A container is dragged onto another | Nothing is filed anywhere. The outlines overlap until one is moved or the graph is tidied |
| The composer sends with a session selected, or with nothing | Nothing is sent, and Send reads as disabled while the draft is empty |
| A message is sent to an agent | It is appended to that agent's thread. Nothing answers, and the thread says so rather than inventing a reply |
| A project's folder is deleted, renamed or unmounted | The next probe marks the row; the record stays and the window keeps its last screen |
| A marked project is located again | The record keeps its id, colour and history; only its path moves |
| A folder already in the catalogue is added again | The picker points at the project that is there; no duplicate appears |
| The catalogue is empty | The window stays open on the empty state, which offers to add a project |
| The catalogue file is corrupt | It is preserved under a timestamped name, the session starts empty, and one error says so |
| The catalogue cannot be written | Changes hold for the session and one error says they are not durable |
| A window's view state is corrupt or from another schema | It is discarded and the window opens on defaults |
| A folder's listing cannot be read | The tab or the row says so; the rest of the tree is untouched. A missing or refused path is the interface's cue to ask the host to probe the project again |
| A file's bytes never arrive | Its tab says it is reading, and keeps saying so. Nothing else waits on it |
| A file's read fails | The tab shows the reason in the danger colour instead of a buffer |
| A file is too large for one read | What arrived is drawn and can be read; saving it is refused, because writing a prefix back would shorten the file |
| A file is not text | The tab says so rather than drawing bytes as characters |
| A file changed on disk since it was read | The save is refused, the file is left alone, and the tab and the status bar say so. Ubiq offers no merge |
| A save fails for any other reason | The same report, cleared by the next edit |
| A dirty tab is closed | The tab becomes a question and takes a second click. Bringing it forward withdraws it |
| Contents arrive for a project the window no longer holds | Dropped. For one it holds but is not showing, they are put in their tab, which is there on the next switch |
| Contents arrive for a tab that has been closed | Dropped, so nothing reopens under the user |
| A remembered folder no longer exists | It is dropped from the restore rather than waited on; the rest of the tree opens |
| A remembered file no longer exists | Its tab opens and reports the failed read, so the loss is visible rather than silent |

## Related docs

- [`panes-and-terminals.md`](./panes-and-terminals.md) — what the dock's tabs actually are
- [`chat.md`](./chat.md) — the panel that survives every mode switch
- [`../tech/ui-and-design.md`](../tech/ui-and-design.md) — the tokens and the component conventions
- [`../backlog.md`](../backlog.md) — what the shell still lacks

## Next steps

- Build the Control, KB and Tasks screens.
- Give the orchestration graph a transport family, so its sessions, agents and tasks are the host's.
- Keyboard navigation for the rail, the tabs and the explorer.
- Give the explorer's git marks and the status bar a branch something to read.
- A viewer per kind of file, so Markdown, a diagram and a diff are not drawn as source.
- Make the titlebar's command field find a file in the project.
