---
id: feat-workbench
title: The workbench
kind: feature
status: draft
summary: The window's shell — the activity rail and the nine modes it selects between, the dock of movable panels the user arranges around the centre, the titlebar and its navigator, the projects a window holds and the empty state one with none shows, the picker that adds, clones and opens them, project and application settings, the file picker any screen raises, and the status bar that reports on all of it. Each mode's own screen has a document of its own.
read_when: you are changing the window layout, the rail, the dock, where a panel may sit or when it is drawn, the titlebar, the navigator, the project picker, cloning a project, project or application settings, remote hosts, the file picker, vim mode, or the status bar
updated: 2026-09-26
verified: 2026-09-26
code_anchors: [crates/ubiq/src/app/mod.rs, crates/ubiq/src/app/shell.rs, crates/ubiq/src/app/panels.rs, crates/ubiq/src/state/mod.rs, crates/ubiq/src/state/workbench.rs, crates/ubiq/src/state/windows.rs, crates/ubiq/src/state/when.rs, crates/ubiq/src/state/prefs.rs, crates/ubiq/tests/prefs.rs, crates/ubiq-host/src/projects.rs, crates/ubiq/src/ui/shell.rs, crates/ubiq/src/ui/ribbon.rs, crates/ubiq/src/state/dock.rs, crates/ubiq/src/ui/dock/mod.rs, crates/ubiq/src/ui/dock/skin.rs, crates/ubiq/src/ui/tab_menu.rs, crates/ubiq/src/ui/overflow_menu.rs, crates/ubiq/src/ui/new_project_menu.rs, crates/ubiq/tests/new_project.rs, crates/ubiq/tests/dock.rs, crates/ubiq/tests/mode_restore.rs, crates/ubiq/src/ui/terminal.rs, crates/ubiq/src/ui/logs.rs, crates/ubiq/src/ui/outline.rs, crates/ubiq/src/ui/rail.rs, crates/ubiq/src/ext/rail.rs, crates/ubiq/tests/rail_container.rs, crates/ubiq/src/ui/project_face.rs, crates/ubiq/src/ui/titlebar.rs, crates/ubiq/src/ui/project_menu.rs, crates/ubiq/src/ui/all_projects.rs, crates/ubiq/src/ui/empty.rs, crates/ubiq/src/ui/status_bar.rs, crates/ubiq/src/ui/size.rs, crates/ubiq/src/app/size.rs, crates/ubiq/src/ui/kit/controls.rs, crates/ubiq/src/state/work.rs, crates/ubiq/src/ui/settings.rs, crates/ubiq/src/ext/settings.rs, crates/ubiq/src/ui/sink/ext_demo.rs, crates/ubiq/src/ui/acp_capabilities.rs, crates/ubiq/src/app/settings.rs, crates/ubiq/src/state/settings.rs, crates/ubiq/src/ui/kit/settings.rs, crates/ubiq/tests/settings.rs, crates/ubiq/tests/settings_container.rs, crates/ubiq/src/ui/sink/project.rs, crates/ubiq-host/src/cli_shortcut.rs, crates/ubiq/src/state/file_picker.rs, crates/ubiq/src/ui/file_picker.rs, crates/ubiq/tests/file_picker.rs, crates/ubiq/src/state/vim/mod.rs, crates/ubiq/src/state/vim/step.rs, crates/ubiq/src/state/vim/motion.rs, crates/ubiq/src/state/vim/object.rs, crates/ubiq/src/state/vim/search.rs, crates/ubiq/src/app/vim.rs, crates/ubiq/tests/vim.rs, crates/ubiq/src/state/nav.rs, crates/ubiq/src/state/nav/text.rs, crates/ubiq/src/state/navigator.rs, crates/ubiq/src/app/nav.rs, crates/ubiq/src/ui/navigator.rs, crates/ubiq/tests/nav.rs, crates/ubiq/tests/nav_text.rs, crates/ubiq/tests/bookmarks.rs, crates/ubiq/tests/navigator.rs, crates/ubiq/src/app/projects.rs, crates/ubiq/src/state/clone.rs, crates/ubiq/src/app/clone.rs, crates/ubiq/src/ui/clone.rs, crates/ubiq-proto/src/repos.rs, crates/ubiq-host/src/repos/mod.rs, crates/ubiq-host/src/repos/list.rs, crates/ubiq-host/src/repos/clone.rs, crates/ubiq/src/state/remote.rs, crates/ubiq/src/app/remote_connect.rs, crates/ubiq/src/app/ssh_connect.rs, crates/ubiq/src/ui/remote_connect.rs, crates/ubiq/src/app/host_browse.rs, crates/ubiq/src/app/hosts.rs, crates/ubiq/src/app/picker.rs]
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

**The rail selects what the middle of the window is for.** Nine destinations in two groups, less
whichever ones the project has hidden in project settings:
`Control` and `Sink` under `APP`, and `IDE`, `Git`, `Agents`, `Teams`, `[Teams]`, `KB` and `Tasks`
under `PROJECT`. `Teams` and `[Teams]` sit side by side, sharing an icon: `Teams` is the mode
being built, `[Teams]` the established graph screen kept beside it, unrenamed and undisturbed,
until the newer one replaces it. Exactly one
is active, and the active one is shown by the accent colour on both its icon and its label. The
group is not decoration: a `PROJECT` mode with no folder open draws the page saying so, and an `APP`
mode answers whether or not one is open.

**Under the modes, the projects this window holds, once it holds more than one.** Bottom-justified
badges, each in the project's own colour with its initial — or, where project settings set an
override, up to two letters of its own choosing — and its full name as the tooltip; a click
points the window at it. The active project is the filled square, edge to edge; the rest are the
same square drawn as a thick ring, inset. One project is nothing to pick between, so the rail shows
no badge at all. The modes have precedence: the rail fits as many whole badges as the space left
under them allows and drops the least recently opened — the active project is the one that survives
when there is room for a single badge, and a window too short for even one shows none. **The order
never moves**: badges stay in the order the window holds them, whichever of them the rail had to
leave out. The switch is Appearance settings, on by default. **`cmd-1`..`cmd-9` jump straight to the
Nth badge**, in that same order, including the least-recently-opened trim — a badge the rail had no
room for is a digit that does nothing. **`ctrl-1`..`ctrl-9` jump to the Nth mode in the `PROJECT`
group** — `IDE`, `Git`, `Agents`, `Teams`, `[Teams]`, `KB`, `Tasks`, counting only the ones the
project has not hidden — never the `APP` group above it. Either chord is a no-op past the last
badge or the last enabled mode.

**The rail is a container, and a mode is a registration in it** (`D184`). Its icon, its label, its
note, its `UiId`, the screen it draws, the arrangement it opens on and the panels it calls
furniture are all fields on one `RailModeSpec`, and the two groups below are the container's
declared order rather than a split written into the enum. A mode is offered `Always` — on the rail
unless the project hid it, which all ten of the base's own are — or by `OptIn`, where the project
has to ask for it first, or by a `When` predicate the contribution owns. The predicate is answered
from interface state, so a contributed mode costs no message. The kitchen sink's own demo mode
(`ubiq.rail.ext-demo`, M4, `X11`) is the eleventh registration and the first that is not `Always` —
`When`, gated by a switch its own demo settings section draws — proving invariant 9's rule that a
container with no contributor is silently dead. Finding that first `When` in the wild also found a
real bug: `AppState::toggle_mode`'s "the last visible mode survives" guard counted *every*
registered mode as potentially on screen, which only the base's own `Always` ten made true by
accident; it counts how many are actually enabled instead (`D186`).

**Every mode is built.** What the rail selects between is the centre. Git fills it with the
repository, Agents with the parallel columns, Teams and `[Teams]` each with a graph, Tasks with the
board, Sink with the kitchen sink, Control with the stats screen, and the centre panel's tab is
named for the mode. IDE fills it with the open files, one panel each, and the centre panel steps
aside for as long as any is open. KB fills it with the one document its explorer selected.

**KB is the documents counterpart of IDE, and its explorer has several sources.** A source is a
folder the user pointed at, a repository the host clones for them, or a wiki Ubiq keeps itself, each
with its own name, its own read-only/read-write access and its own name-glob filter —
`*.md *.excalidraw` — and none of them need be inside the project. A project nobody has configured
draws one page in the panel with an **Add KB** button on it, which opens project settings on its own
section; everything else about a source is edited there. The sources, their listings and their
documents are the transport contract's knowledge-base family, and a source that has to be fetched
reports its clone on its own row rather than in a modal. The centre renders markdown through the
same viewer the IDE uses, and now draws a diagram or an image through it too — a Mermaid,
Excalidraw or draw.io document opens the same web-panel bridge a project file's does, and an image
draws through `ui/viewer/image.rs` (`T-32`); it does not offer that image the annotation toolbar a
project picture gets, which is `G311`.

**Every row of the KB explorer has a right-click menu**, drawn with the same `kit::context_menu` the
project explorer uses and answered by the same prompt and confirm modals. A **source row** offers
Open in Finder, Copy path, *Get latest version* for a git source only, New file and New folder for a
writable source, and *Rename source* always — a source's name is Ubiq's label for it, so it is
renameable even where the material behind it is not, and it commits through the whole-list
`SetKbSources` write rather than a rename on disk. A **folder row** offers Open in Finder and Copy
path, plus New file, New folder, Rename and Delete when the source is writable; a **file row** the
same two, plus Rename and Delete. Nothing is drawn greyed: an entry the row cannot do is left out,
because unlike the explorer's Paste there is no state a user could reach to earn it. Open in Finder
opens the *host's* file manager (`RevealKbPath`), and Copy path asks for the absolute path
(`AskKbPath`) and puts the answer on the clipboard — the one place the interface learns one, and it
is asked for rather than volunteered. Delete confirms first, and a folder's confirmation says what is
inside it goes too. Details in `_docs/wip/kb.md`.

**The KB settings section lists the sources and raises one modal to add another.** Each row carries
the source's name, where it comes from, its filter field, a read-only/read-write marker and a remove
control; under them is a single **Add source** button. The modal it raises is the New agent modal's
shape — `ui/kb/source_form.rs`, `kit::modal` with a footer, a first full-width `Picker` and every
row below it drawn inert until that first answer lands: a name seeded from whatever the form points
at until the user types their own, the kind (folder, git repository or wiki), and then the one thing
that kind needs. A folder is chosen through **Ubiq's own host-backed file picker**, never the
platform dialog, because the path has to be one on the machine the *host* runs on; a repository
takes a URL with a **Check** button beside it that sends `ListRepoBranches` — the clone modal's own
ask, answered in the same `receive_repo` arm — and fills the branch picker from the answer, plus a
store picker over `KbStore`. Access is read-only by default and is fixed on read-write for a wiki.
Confirm appends the source and commits the whole list as one `SetKbSources`; Cancel discards.

**The explorer belongs to IDE mode; the chat belongs to the project.** The explorer is IDE furniture
and leaves with the mode — it is mode-owned (`PanelKind::is_mode_owned`), so it travels in the IDE's
own blob and is never put back into another mode's tree. A conversation is furniture on every screen
about a project instead: the right region holds it in every mode but Agents, where the columns *are*
the conversation, and Control and the sink, which are not about a project at all. The console, the
terminals and the centre panel itself outlive a mode switch too. **What no switch does is place a
chat** (`D156`): a tab is opened by the user, on one screen, and comes back from that mode's own
blob — a switch that put one beside Git's changes panel would be the window starting a conversation
nobody asked for and then writing it down as Git's.

**A side panel defaults to the dock, and only a panel meaningful in exactly one mode stays out of
it.** `PanelKind` (`state/dock.rs`) is the shared, draggable, per-window arrangement — `Terminal`,
`Logs`, `Explorer`, `Chat`, `File`, `Search`, `Outline`, the Git family, `KbExplorer`, `Kb` (one open
knowledge-base document, the same `OpenFile` a `File` panel draws — `wip/kb.md`), `AgentsExplorer`
and the board's own `Task` panel all live there, so any of them can be dragged
wherever the window is arranged that day; the board's popup flag only swaps that panel's *shape*,
docked or modal, never whether the dock owns it. A screen that wants a panel with no meaning outside
its own mode brings its own instead of asking for a `PanelKind`: `[Teams]`'s inspector and tasks
drawer are a bespoke state toggle (`GraphView::show_inspector`) and the drawer's own flag, drawn
inline by `orchestration::render`, never in the dock, which is why they toggle instead of dragging
and vanish with the rail rather than persisting across a mode switch. `Teams` keeps the tasks
drawer the same way, but has no inspector of its own: selecting a card opens that agent's
conversation in a `Chat` panel in the right dock instead
(`AppState::select_in_teams`/`open_teams_agent_panel`) — the first chat tab the project already
holds is reused and re-aimed, or a fresh one is minted, and the dock is revealed if it was put
away.

**Two kinds of screen stand over the same records, and the split is the point** — `D47`. Agents is
where the user *talks to* the agents; `[Teams]` and `Teams` are where the user *arranges* them. A
graph is a map of who spawned whom, a column is a transcript and a composer, and neither kind of
screen draws the other's view. All three read one projection of what the host reports about a
project.

**Each built mode's screen is its own document, and this one is the frame round them.** What the
rail selects between, in rail order:

| Mode | What the centre becomes | Document |
|---|---|---|
| `Control` | Five readings of the running host on one page, and the usage meter on the other | [Stats](./stats.md) |
| `Sink` | The application's own test bench — twelve pages of fixtures with nothing behind them | [Sink mode](./workbench-sink.md) |
| `IDE` | The project's files: the explorer, one editor tab per open file, and a viewer per kind | [IDE mode](./workbench-ide.md) |
| `Git` | What version control knows, whole — refs, history, change lists and a diff | [Git mode](./workbench-git.md) |
| `Agents` | A row of columns, each a transcript and a composer over one live conversation | [Agents mode](./workbench-agents.md) |
| `Teams` | The graph being built, scoped by a window span, drawing its cards' conversations in the dock | [The Teams graph modes](./workbench-teams.md) |
| `[Teams]` | The established graph screen, with its own inspector and composer | [The Teams graph modes](./workbench-teams.md) |
| `KB` | A knowledge-base document, over the sources explorer described above | [`../wip/kb.md`](../wip/kb.md) |
| `Tasks` | A column per status, a card per task, and the panel that reports one whole | [Tasks mode](./workbench-tasks.md) |

**IDE** draws the project's folder one directory at a time down the left and each open file as its
own centre panel, chosen a viewer by kind — Markdown, a diagram, a picture, a scene or a plain
buffer — with the image editor and the capture behind the same tabs.

**Git** puts the refs explorer in the left region, the conflicted, staged and unstaged lists with
the commit box in the right, and the paged commit history with its painted lanes in the centre,
under a strip naming the repository and what HEAD is doing.

**Agents** is a row of up to eight columns. A column holds tabs, more than one tab is a group, and
every agent the host reports that no column is showing is on the bench. The sidebar lists every
conversation the window holds rather than what is on screen.

**`Teams` and `[Teams]`** draw a project's agents as cards on a dotted ground, fenced into the task
each serves, arranged by one of twelve algorithms the canvas computes for itself, with a tasks
drawer underneath. `Teams` is scoped by a window span and opens a card's conversation in the dock;
`[Teams]` keeps an inspector and a composer of its own.

**Tasks** is a column per status and a card per task, filtered by labels and one find field, with
the task panel reporting the selected task whole and editing it one field at a time; a mission
carries a plan, raised as a surface over the window.

**Sink** is the kitchen sink: twelve pages of fixtures — a buffer, one per special viewer, the
style reference, the file picker in each shape a screen can ask for, the two settings layouts, a
live conversation beside its bus traffic, the A2UI surface, the script scratchpad and the teamsim
testbed.

**Control** is the Stats screen, and **KB** is the documents counterpart of IDE described above.

**A project is a colour.** Each project owns one of the theme's swatches, and wears it in four
places at once: its dot in the picker, the fill behind its name in the titlebar, the mark above the
rail, and the window's left edge. Two windows on two projects are told apart without reading
anything. The mark is Ubiq's logo on that fill — the white file on a dark swatch and the blue one
on a light swatch, chosen by the swatch's luminance — so the logo reads on whatever the project is
tinted.

**Every window is named by a letter.** Each takes the lowest letter no live window is using — `A`,
`B`, `C`… — printed in its own box at the head of the titlebar, in the project's tint, and beside
every open project in the picker's list. The box sits *before* the picker rather than inside it:
one says which window, the other which project, and a letter inside the chip reads as part of the
project's name. A closed window gives its letter back, so the names stay as short as the set of
windows. The letter is in the operating system's window title too, which is what the window
switcher shows.

**A letter is drawn only where there is a second window to tell it from.** With one window open the
titlebar has no letter box, the picker's first heading is just "This window", and its rows carry no
mark; a second window brings all three back. The letter's whole job is to distinguish, and there is
nothing to distinguish in a single-window session — `AppState::several_windows()` is the one
question the chrome asks, and the operating system's window title keeps the letter either way.

**A project is open in exactly one window.** Openness is not a flag on the project — it is which
window holds it. Opening a project somewhere therefore takes it from wherever it was, and that is
the only way a project moves between windows.

**A window with no project open stays open, on a screen of its own.** No panes, an explorer that
says it has nothing to show, no open files, and "Add a project…" in the middle of the window. This
is where a first run starts, where closing a project leaves the window, and where taking a project
into another window leaves the one it came from.

**Ubiq never closes a window.** Only the user does, and the application quits with its last one. A
window holding nothing is a window waiting for a project, not an error to be tidied away.

**The empty state is two panels and a strip.** The centre panel says no project is open and offers
to add one; the explorer keeps its place in the arrangement and its size, with one muted line in it
rather than a tree; every chat tab is hidden, because a conversation about nothing is a fiction. The `+`
at the end of the pane region's tab strip is not offered, because there is no folder to start a
harness in — but the chevron beside it is, and its menu offers the console: the one panel a window
with no project has a reason to show.

**The explorer, the open files and the terminals belong to a project, not to the window.** A window
holds one set of each per project open in it, and switching between them is a lookup: the tree, the
tabs and which terminal panels are drawn all change together, and nothing is re-read or rebuilt. The
terminals of the projects behind keep running and keep their scrollback.

**A project leaving the window takes its panes and its running agents with it — moved, or ended.**
Moving it to another window moves them: the harnesses keep running and answer to the window that
took it, which is what `Message::AdoptProject` re-homes on the host's side, and only the scrollback
is lost, because the emulator is rebuilt on the taking window's bus. Closing it or forgetting it
ends them: the terminal harnesses are killed and every conversation whose process is up is unloaded,
because a pane nobody is left looking at is a leak. Closing asks first when any of those are open,
or when a buffer is unsaved, in the same modal the window's own close raises. An unloaded conversation
is stopped rather than deleted: a conversation the user marked to keep comes back, and the window's
own close is what retires the rest. What the project *remembers* is written down first, so reopening
it brings its files back.

**The project picker is a small manager, not a list of values.** It searches on name and path, and
divides into three groups, top to bottom: **open in this window**, **open in another window** with
the letter of the window holding each, and **history** — everything open nowhere, with how long ago
it was. A group with no rows is not drawn. A project's row moves between the groups as the project
moves between windows, in every window's picker at once. The three ways in — Add, Clone, Open
remote project — sit between "open in another window" and History: what is already open answers
"where is everything", how to add to that list is one topic read together with it, and History is
what has piled up, drawn last. **History shows at most nine rows.** Past that it hands off to an
"Others…" row — styled like Add, Clone and the rest, opening the **All projects** modal: every
project in the catalogue, filtered the same way, with the same open-here, open-in-a-new-window and
Forget actions on each row, in a list that scrolls rather than a menu that keeps growing.

**The same three ways in are also reached without opening the picker.** A `+` sits in the titlebar
beside the project picker and runs Add directly — the same folder chooser the picker's own Add row
raises. A chevron beside it opens a small menu offering the identical three rows, Add, Clone and
Open remote project, worded exactly as the picker's foot words them; picking one runs the same
`AppState` method the picker's row would. `WorkbenchState::new_project_rows` is the one list both
read, the rule every position-matched menu in this window follows.

**Every row is one line, and the name has the right of way.** The name takes the space it can; the
path, when it has a parent, is the last component with a leading `.../`, and it is printed **only
when what is left over holds it** — a folder is the answer to "which of these two is it", which is
worth nothing once the name it is meant to distinguish has been truncated to fit it. Hovering the
name shows the full path as a tooltip whether or not the row had room to print it. A row
whose folder is missing, is not a directory, or cannot be read prints that path in the warning
colour with a mark beside it, and offers **Locate** — which re-points the record through the system
folder chooser and keeps the id, the colour and the history. A record is never removed because its
folder went: an unplugged drive and a worktree mid-rebase are both temporary, and forgetting is
always the user's action.

**Forgetting asks first and says what it will do.** It drops the record and everything Ubiq
remembers about the project, and touches nothing inside the project's own folder — which is why the
word is "Forget". Rename and recolour are not on the row: they live in project settings.

**Choosing a project's folder is the operating system's dialog**, both for Add and for Locate — the
chooser the user already knows, with their bookmarks, their network volumes and a path field. Ubiq
draws no folder browser of its own for this. Opening a file or a folder *inside* a project stays in
the interface, where the explorer is. Adding a folder already in the catalogue points at the project
that is there rather than making a second.

**Add does not create the project on the spot.** The chooser returns a folder; project settings
opens over the window with only General enabled, the path filled and immutable, and the name
prefilled from the folder's last component. Create sends `AddProject`. Cancel leaves the catalogue
untouched.

**Where the project keeps its data is chosen in that panel, and only there.** A "Project data" row
offers two answers: Ubiq's config folder, which is the default and writes nothing inside the
project, and "In the project (.ubiq/)", which writes the tasks, the project metadata and the
configuration to a `.ubiq/` folder the project commits — with a `.gitignore` that leaves this
machine's caches and view state out. The edit panel draws the same row for an existing project and
does not take a click: it says where the data is. The host can move an existing project either way
— `SetProjectStorage`, T-204 — and nothing in the interface asks it to yet, which is `G354`. `D173`
and [`../tech/project-structure.md`](../tech/project-structure.md)
are the shape of the folder.

**Cloning is the picker's second way in.** A "Clone a project…" row sits beside Add and opens a
modal of its own. With a connection to GitHub, GitLab or Gitea the modal lists that identity's
repositories and filters them as the user types; a repository URL pasted into the modal's own field
needs no connection at all, and is what reaches a public repository. Either way the branch is picked
from what the remote actually has — the provider's branch API for a connection, an anonymous ref
listing for a bare URL — the destination folder is chosen in the platform's dialog, and shallow is a
checkbox. **https only:** an `ssh` remote is recognised and named as unsupported rather than
attempted. The connection last used is remembered, so the modal reopens where it was left.

**"Open remote project…" is the picker's third way in**, sitting below Add and Clone. With no
remote host attached it opens the "Connect to a remote host" modal instead of a dead end, since the
row is the only place a first-time user learns the feature exists at all. With one remote it raises
the file picker walking that host's own filesystem, rooted at whatever `BrowseHostDir { path: None
}` answers with; choosing a folder there sends `AddProject` to that host rather than to the local
one. **With more than one remote attached it opens the one application settings' Hosts section has
active** — `AppState::preferred_remote_host()` reads `Bus::active` and falls back to the first
remote attached when `active` is Local (the common case, since nothing points it anywhere else
until the Hosts dropdown, below, is used), so the row never refuses to act just because a second
remote showed up.

**The clone registers the project itself.** There is no Add afterwards and no success dialog: the
project arriving in every window's picker is what says the clone finished. Until then the modal
names the stage it is on — resolving, counting, receiving, checking out, registering — rather than a
percentage, and Cancel is live throughout. A cancelled or failed clone leaves nothing behind, its
partial destination removed, and the modal says which of network, authentication, a repository that
is not there, a folder already there, an unsupported provider or a refusal it was.

**A pasted repository URL is a clone, wherever it is pasted.** Typing one into the `⌘K` field and
pressing Enter opens the same modal on that URL — see the navigator below.

**A clone can be ephemeral.** The checkbox puts it under Ubiq's own ephemeral folder instead of the
default project folder and marks the record `temporary`: the case is a repository read once and
thrown away — a pull request looked at, a dependency checked before it is trusted. Closing an
ephemeral project turns its row into the same inline confirmation a project with terminals raises,
saying that the clone will be discarded, and confirming sends `ForgetProject` the way closing any
temporary project does. The host is what deletes the folder, and only when the record is
`temporary` **and** the folder lies inside the ephemeral root — a folder dragged in from anywhere on
the disk is `temporary` too, and the root is a setting a user could point anywhere (`D74`). Folders
left under that root by a crash are swept at startup, after the catalogue has loaded and can say
which of them no record names.

**A 3-dot next to the title chip opens project settings for the project this window is showing.**
The path stays as it is: "Project path" is a read-only field rather than a plain label, so a long
path can be scrolled and selected instead of overflowing, and the home directory is abbreviated to
`~` for display (`ui/sink/project.rs::home_abbreviated`; nothing stored or sent is ever the
abbreviated form). Documentation and Integrations are not drawn at all here: they are
`SectionGate::SinkOnly`, which is fixture copy, and a row the live dialog can only ever grey out is
a row the live dialog does not have (`T-231`). Save writes the name and colour through
`UpdateProject`.

**Task sync is the dialog's ninth row**, between Remote and the knowledge base, and it is the
container's second genuine contribution rather than a converted enum variant (`D187`, after M4's
demo): it registers from `ui/tasksrc.rs` through `ext::settings::register` with nothing in
`ext/settings.rs` or `ui/sink/project.rs` touched to add it. What it draws is **one renderer over a
schema the bound provider declared** — a text box, a tick, a single-choice picker or a row of
toggles per field, with a **Test** that runs the filter and reports what came back — plus the lane,
kind and priority maps, the direction, the authority switch and the interval, which are the same
fields for every provider. **Nothing in it names a tracker**, and what a provider cannot do is
greyed with the reason beside it rather than removed, so two providers' pages are the same shape.
[`workbench-tasks.md`](./workbench-tasks.md) has what the board does with the result.

**Agent definitions is the project's own nav item, and it opens on `Use the global agents`.**
Ticked is the answer a project gives until it has written a setup of its own: the globals are what
a start in any project is offered, so a project with nothing of its own has nothing to say here.
Unticking it enables the list below — this project's own setups, each with `Clone` and `Edit`,
`Add agent` above them under the same no-harness guard the application section's carries, and then
the globals, listed ticked and with no action on them, because a global is edited where it was
written and a project adds to what it inherits rather than taking from it. A global a project
setup shadows by name says so on its row.

**The tick is seeded, not stored.** What outlives the dialog is whether the project has
definitions of its own — `ProjectSettings::definitions_use_global` is read off
`project_definitions` when the dialog opens and is the section's own state thereafter. There is
deliberately no record of it on the wire: a second record of "does this project use the globals"
could disagree with the definitions themselves. A project that wants *only* its own, with no
global on offer at all, is `G358` in the backlog.

An agent definition saved here belongs to the
project, which is a fact about where it is stored and not a field it carries (`D158`): the host
keeps it under that project's own directory, offers it when an agent starts in that project, and
deletes it with the project — forgetting the project takes the whole directory, agent definitions included,
and nothing asks first. `Add agent` raises the same agent definition form the settings page raises, aimed
at this project; `Edit` on a row reopens it in the scope it was found in, because Edit is not a
way to move an agent definition between roots. **The project's own rows appear nowhere else.** The application's own
Agent definitions section lists the global ones only — "visible only in the project they were created
in" is the ask, and a second listing under Settings would contradict it. The cost of reading it
that way is that a surface holding no project offers none of them: the new-mission dialog's
assistant picker is the one that bites, so a project-scoped agent definition cannot be a mission assistant
yet — a row in the backlog rather than a hedge here.

**Tasks says which lanes this project's board draws.** Every one of the seven statuses is listed —
hidden ones included, since a page that dropped them would be a page with no way back — with two
independent switches each: whether the lane is drawn at all, and whether it shuts itself to a strip
while it holds nothing. Both are the project's, written onto the record through `UpdateProject` on
the click, the way the index pill and the search excludes are. A hidden lane still holds whatever
work is in it: hiding is about the board, not about the tasks, and nothing is moved or deleted.
A lane that shuts itself when empty still opens on a click — the board keeps that answer for the
window in `BoardState::opened`, the counterpart of `shut`, and neither list is written down.

**Remote says where the project's folder actually is** — here, or behind a drone on another machine.
It is the dialog's fourth nav item, enabled on the same terms as Tools and Tasks: all three attach
to a record, and a folder not yet in the catalogue has nothing to pin. On-a-drone asks for a saved SSH profile, the
folder on *that* machine, and the drone's lifetime — the same three presets the connect modal
offers. Saving writes the record and stops: **nothing dials from here.** The drone is launched, or
an already-attached one adopted, when the project is next opened, which is the moment there is
anything for it to serve. A project pinned this way keeps its row in the catalogue whether or not
its drone is reachable; with the drone away the row reads as unreadable and names it, so a laptop
that sleeps loses the drone and never the project.

**General carries the project's rail modes, as the rail's own icons.** One tile per mode, lit when
the mode is on screen and flat when it is not, flipped by a click — it takes effect at once and is
written into the project's view blob (`ViewPrefs::hidden_modes`), not through `UpdateProject`, so
it survives a restart with the rest of what the project remembers. **The last lit mode cannot be
turned off**, and hiding the mode the window is in moves it to the first one still visible. A rail
group whose every mode is hidden takes its heading with it.

**A folder dropped on the editor centre or a file tab opens at once, as a temporary project.** The
host mints an ordinary record with a `temporary` flag, never writes it to the catalogue, and keeps
it only in memory — gone at the next launch. It is named after its folder, and it wears the
theme's grey tint in every place a project's colour shows rather than a swatch, because it is not
in the catalogue and should not look like it is. Every file, git, work and pane operation works
against it exactly as it would any other project. Its titlebar button is a `+` rather than the
3-dot, and opens the same project-settings dialog. **Naming it there is what keeps it**: Save on a
temporary project sends `UpdateProject`, and the host's answer clears the flag and writes the
record down — there is deliberately no separate "promote" message, so the button reads "Keep this
project" rather than "Project settings" and the dialog's own button reads "Create" rather than "Save
changes". Adding the same folder again through the picker's chooser promotes it the same way.
Closing a temporary project sends `ForgetProject`, but only once no other window still holds it —
the project message is a broadcast, so a second window's picker can still be pointed at it.

**Each group's rows carry the actions that group needs.** In this window: click to point the window
at it, `Close` to close it, `ExternalLink` to send it to a window of its own. In another window:
click to bring that window to the front — which is how the user moves between windows — or
`ArrowLeft` to take the project into this one. In history: click to open it here, or `ExternalLink`
to open it in a new window. Closing a project that still has terminals running turns the row into a
question rather than taking the click.

**The middle of the titlebar is one field, marked `⌘K`.** The chord raises the navigator over the
field and puts the caret in it, and Enter then goes to the row under the cursor. **Enter with the
navigator shut is the same chord**: it raises the list rather than searching, and since the list
opens on its own Search row, Enter twice is a project content search. **`⌘⏎` is the way past the
list**: it runs that search straight from what is typed, whether the navigator is up or not. The
search itself switches the window to the IDE, hands the text to the search panel's own query
field, reveals the panel (`reveal_search`) and runs a project content search — then clears the
field, the way a command palette clears once its command has fired. A no-op with no project open. The field draws itself with the kit's shared text-entry container — a surface
with a coloured left edge, and a bottom underline while it holds the keyboard. There is no
breadcrumb: the titlebar says which project, the tab strip says which file, and repeating it in the
middle bought nothing.

**Every place the window can show is one value, and one call goes there.** A destination names a
project, a view — a file by its tab key, a folder, a pane, the graph with its selection and its
inspector tab, an agent, a task, a chat, or one of Control, Knowledge, Git and Logs — and
optionally a spot inside that view: a line, a span, a heading slug, a viewport or a graph node.
**The locus is not part of the place**, so arriving at a file already open moves the caret and
nothing else. A link, a card's button, a bookmark, a navigator row and a back press all end in the
same `navigate` call, which is what keeps one record of where the user has been. A screen pointed
at nothing — the graph with no selection, the board with no task — is not a place, and the kitchen
sink is not addressable at all: it is the test bench and has no project behind it. **A destination
names a project, never a window**: one whose project another window holds sends *that* window there
and raises it, so nothing moves between windows and the follower's own place is untouched.

**A repository's remote earns a globe beside the 3-dot**, which opens the project's page on its
provider and says which URL in its hint. It is drawn only when `ubiq_proto::git::web_url` recognises
the default remote's URL, so a project that is not a repository, has no remote, or fetches from a
local path shows nothing rather than a dead button.

**The project's runnable tools are a play triangle and a chevron, next along from the globe.** They
sit with the project's settings and its web link because that is what they are — a property of the
project rather than of a terminal — and both are drawn only with a project open, for the same
reason the new-pane control is: a tool runs in a project's folder. The triangle runs the **first**
tool the project offers and its tooltip names it; with no tool to run it does nothing and says so.
The chevron opens the list of every applicable tool, machine-wide rows and the project's own
together in the order the host listed them, each behind the same play glyph; a host with none to
offer draws one disabled row saying so, so the control never opens onto nothing. The list is asked
for every time the chevron opens, which is what makes a tool added in the settings runnable without
a restart. **Starting one brings the pane region on screen**: the bottom region opens if it was put
away, and a window in a mode with no pane region at all — Control or the kitchen sink, neither of
which is a view onto a project's folder — is moved to the IDE first. The rows themselves, what a
pick sends and what a tool is are the panes-and-terminals document's.

**Back and forward are one stack per window, spanning every project it has shown.** `⌃-` and
`⌃⇧-`, and two controls at the left of the titlebar, past a rule from the project menu, its own
`+`/chevron pair, project settings and the run controls: they walk that project's places, so they sit beside it
rather than beside the field — each drawn
faint and taking neither pointer nor click when its end of the stack is empty, and each carrying
the name of where it would
go, with the project's name in front of it when that is not the project on screen. An arrival is
recorded once a frame from what the window is actually drawing, so there is no call site to keep in
step: a rail switch, a file opened, a tab brought forward and a project change are new places and
push; a caret move, a pan and a zoom are the same place at a new locus and refresh the entry where
it stands; a filter, a panel toggle and a palette change are not places. Going back and then
scrolling means coming forward lands where the user left rather than where the link pointed. The
stack holds 64, drops its oldest, and is not persisted. An entry whose project another window now
holds is stepped over and kept; one whose project has been forgotten is dropped.

**A destination survives being written down, as `ubiq://<project-id>/<view>[/<item>][#<locus>]`.**
The view slug decides how much follows it rather than the number of segments, so a file path needs
no escaping: `ide` and `explorer` take everything up to the fragment, `terminal`, `tasks`, `chat`
and `agents` take exactly one id, `graph` and `teams` each take `s:<session-id>` or `a:<agent-id>`
— the prefix is forced, because both are 26-character ULIDs and the text alone cannot tell them
apart — followed optionally by `chat` or `tasks`, and `control`, `kb`, `git` and `logs` take
nothing, a trailing segment on one of them being a different string rather than a refinement.
`teams` alone may carry a third part after the tab, naming a delegate: the id of the `Task` call
that spawned it, unsplit and taken to the end of the item, so a call id holding its own `/`
survives; a `teams` link built on `s:` never carries one — a session has no delegate, and one named
on it parses as no link at all. **A `teams` link names the project of the agent it points at**, not
the window's active one; a `teams` link built on `s:` names no agent, so it names the project
holding the session — the first agent under it answers, the same way the session pills resolve the
chip they wear.
The span is not in the address, because the span is not part of the place: the same agent read on a
canvas showing one project and on one showing six is the same agent, so a link built under the
window span is one a window in the project span can follow. The fragment is `L42`,
`L42-58`, `v=x,y,scale`, `n=key` or a heading slug. `%`, `#`, space and the control bytes are
escaped and UTF-8 passes through raw, so a link stays readable. **The project is written as its id
and drawn as its name**: a name is not stable and a bookmark from three weeks ago needs one that
is. **Parsing is total** — a string that does not parse is not a link, and nothing at all happens.
`Copy link` is on the explorer's row menu and on a file tab's menu.

**A bookmark is a destination the user wrote down, with a name.** `⌘⌥K` writes down where the user
is, and takes it away again if it is already down. Bookmarks ride the project's own preference
blob, so forgetting a project forgets them, and a chat is never offered one because its id is
minted by this window and means nothing tomorrow. **A line number rots, so a bookmark on a line
keeps that line's own text**, trimmed and capped at 120 characters. When the file is read again the
number is trusted first — so a line that appears twice does not have its bookmark stolen by the
other copy — and then the text is looked for outward, 200 lines either way: found elsewhere, the
bookmark re-stamps its number; found nowhere, it is **adrift**, drawn in the warning colour and
never quietly repaired. A bookmarked line is lit by a highlight over the line itself, and a file's
tab wears the count of the bookmarks it holds so it says so while their lines are off screen. The
project's bookmarks are a collapsible section at the head of the explorer, above the filter and
outside the tree's focus, so neither the tree's keymap nor its cursor knows it is there; the rows
are plain, and the only thing to do with one is go where it points.

**The navigator is the list the `⌘K` field offers.** Flat and single-select — up and down walk it,
Enter goes, Escape takes the list away and leaves the typing alone. **Search is the first row
whenever anything is typed** — it is what was typed, offered as a project content search, never
filtered and always under the cursor as the list opens. Then five groups, drawn in this order:
Recent, Bookmarks, Files, Tasks and Agents, eight rows each, matched by subsequence over what the
row itself says. An empty query is Recent then Bookmarks and **no files**: a file list
with nothing typed is the explorer. Files are the explorer's own filtered rows, under the same
three-character floor and deepening as its background walk lands. A pasted `ubiq://` is an answer
rather than a filter and collapses the list to one row naming where it goes; one naming a project
this catalogue does not hold says so and takes no click. **A pasted repository URL collapses it the
same way**, to a single Clone row whose Enter opens the clone modal on that URL rather than going
anywhere — one of the two rows in the list that carry an action instead of a destination, the
Search row being the other. There are no
commands in it: a command is not a place.

**A link in rendered content is a destination.** A Markdown document's own relative links resolve
against the document, so `../src/app.rs#L200` opens that file at that line, and a target that pops
past the project root is not a link. `http`, `https` and `mailto` are handed to the operating
system; anything else does nothing at all. The same handler is on the chat transcript's rendered
replies and on a task's rendered description, both resolved from the project root. **Terminal
output is never scanned for links** — reading a harness's screen is the thing Ubiq does not do.

**The window's body is a dock, and the user arranges it.** Everything between the chrome is a tree
of tabbed groups: dropping a tab in the middle of a group tabs it there, dropping it on a group's
edge makes a row or a column, a divider drags, and a group's zoom control fills the region with
whatever it is displaying and gives it back. The window fixes no arrangement — it draws the
titlebar, the rail, the dock and the status bar, and what is inside the dock is the user's answer.

**The window says which build it is.** A diagonal ribbon — the same `kit::ribbon` Git mode uses
for `experimental` — lies across the bottom-left corner, over everything, reading `beta` when the
bundle's version is a released one — a `UBIQ_VERSION` starting with `v` — and `alpha` otherwise, a
bare `cargo build`'s `dev` included. **Clicking the band swaps
the dock icon** for the yellow mark, and clicking it again puts the bundle's own icon back — which
is how two builds running side by side are told apart in the dock. macOS only, since nothing else
lets a running process change its icon, and the mark is compiled into the binary rather than read
from the bundle, so `just dev` has it too. Only the corner the band actually crosses takes the
click: the box around the diagonal takes none, or the ribbon would swallow every press in the
bottom-left of the window.

**Every area in the dock is a panel.** One per pane for the terminals, one per open file in IDE
mode, one per open document in KB mode (the same `OpenFile` a file panel draws, over a
knowledge-base source rather than the project — `wip/kb.md`), one per open chat tab — see the chat
document — and one each for the explorer, the log console — which is [`logs.md`](./logs.md) — the
outline, and the centre. A mode with side furniture of its own adds to that list: Git's four, the
knowledge base's documents explorer, the board's task (`PanelKind::Task`, the report or the form for
whatever the columns select) and the agents list (`PanelKind::AgentsExplorer`).

**Placement is a property of the kind of panel, not a special case.** The explorer sits in the left
or the right region and nowhere else, because an explorer squeezed into the bottom is a sixty-pixel
tree — and the board's task and the agents list answer the same class for the same reason. A terminal, the console, a chat tab and the outline may sit in any region at all — a chat
tab moves to any dockable region the same way a terminal already could. The centre panel takes the
centre. A panel
dropped where its class forbids is moved back to its home region on the same edit, so the drop reads
as refused rather than half-taken. A file takes the centre, like the centre panel: the open files
*are* the centre in IDE mode, so a file dragged to a border would leave nothing behind it — an open
KB document takes it the same way in KB mode.

**There is no top region.** The dock has a centre and three edges — left, right and bottom — so
"docked above the editor" is a split at the top of the centre rather than a region of its own.

**A panel with nothing to show is hidden, not removed.** It keeps its place in the tree and its tab
slot, and comes back where it was left. The explorer leaves with IDE mode; a chat tab is furniture on
every screen about a project except Agents — it is drawn with a project open in every mode but
Agents, Control and the sink — because the agents are how work gets done everywhere else and a
conversation about nothing is a fiction, and Agents draws no chat panel because the columns *are*
the conversation there; the board's task wants Tasks mode and a project, and the agents list wants
Agents mode and a project, the way the documents explorer wants KB's; a terminal is hidden while its project is not the one on screen, so its
harness goes on running and keeps its scrollback; a file panel is hidden while its tab is not one the
project on screen holds. The console is always drawn, and **the centre panel steps aside in IDE mode
for as long as a file is open** — the same machinery, which is what brings it back where it was left
when the last tab closes rather than rebuilding it somewhere else.

**A panel's own kind is the whole of whether its tab closes — the last one in a region included.**
A terminal, a file, a chat tab, the console, the search and the outline each close wherever they
sit, and emptying a region that way is a legal arrangement rather than a refused gesture: the region
collapses, and the switch brings it back with a fresh panel in it. Nothing is kept on screen for
being the only thing left.

**The titlebar's switches open and close the dock's edge regions**, and read the dock rather than a
flag beside it. A region the user empties — closing its last panel, or dragging it out to another
region — is closed the same way the switch would: the switch reads it as closed because it *is*
closed, not merely because it looks it. **All three switches are drawn in every rail mode**: every
mode has side furniture of its own now — the IDE's explorer and chat, Git's refs and changes, KB's
documents, the board's task, the agents list, Teams' chat — so a switch that was hidden would be a
region the user could not ask for. **The pane region never opens by default, in any mode** — `D94`. A mode or a project never arranged
before opens on the centre and its own explorer side alone: the IDE's, Git's, the knowledge base's
and the agents screen's left region, and Git's and Tasks' right region, come up with the mode
because they *are* how that screen is read — `mode_side_furniture` is what each one opens onto —
and every other edge, the bottom always and the right everywhere but Git and Tasks, stays shut until
it is asked for.
**A mode or a project switch is not that ask** (`D156`): it opens no region and adds no panel
beyond what the mode's own defaults or its saved blob already name, and a region a restore or a
switch leaves open with nothing in it is closed rather than filled — the same
`collapse_empty_regions` a user emptying a region by hand triggers. The one gesture that fills an
empty edge is **a click on that region's own titlebar switch**: `toggle_region` is the only place
this happens, and it answers with the mode's own side furniture where the mode has one — the bottom
starts a pane; the right in IDE, Git, Tasks, the knowledge base and Teams opens onto that mode's
chat, the changes panel, the task, or a fresh chat tab in turn — Teams names no furniture of its own
for the right (`mode_side_furniture`), so it falls into the same fresh-chat-tab default as the
knowledge base; the left opens onto that mode's explorer, Git's refs, the knowledge base's documents
or the agents list. Teams and Tasks have no left-hand furniture, so an empty left there is left as
the user made it. A
switch that gave the user a bar of nothing would not have answered what was asked. The left is
rarely genuinely empty in the modes that have furniture for it: the explorer panel is already in the
tree the moment IDE mode is entered, so opening a closed left reveals it rather than spawning
anything. **A switch fills nothing that is already on screen** — where the user has dragged that
furniture to the other edge there is nothing this side can show, and the switch leaves the
arrangement as it found it rather than opening a bar of nothing the dock would write down as the
user's own. A mode with no furniture for a side promised nothing there and keeps the empty edge the
user asked for, which is why Control's left stays open on a click and the IDE's does not.

**Tasks' right region opens onto the task alone, and the chat strip's `+` is what the user starting
or attaching an agent from there reaches for.** `mode_side_furniture` names no chat for Tasks — the
task is that region's answer — so the idle, unattached chat tab every project seeds never lands
beside it (`AppState::is_idle_chat` filters an idle chat out of a region that already holds
something and is not asking for one). The `+` still draws, on `is_chat_region`'s strength rather
than a chat panel's presence: see [chat's own document](chat.md) for the control's two-part rule.
Picking *New agent* or *Attach existing agent* there reveals a `Chat` tab beside the task the same
way `AppState::assign_task_to_agent` does — see the board section above.

**Window furniture travels only when it can be brought back.** Search, the console and help are the
window's rather than any one mode's, and a switch sweeps all three out so the incoming arrangement
is the only thing that puts them on screen (`D156`). Three narrowings make that rule honest. A
**window pointed at no project** sweeps nothing: nothing is written down for it, so nothing could
restore what a sweep took, and the console it lost would be lost for good. A switch the **window
made rather than the user** — a runner sending its pane to the IDE, the mode the window is in being
hidden — sweeps nothing either. And **help in follow mode is revealed, not merely left in the
tree**: the panel's whole contract is to keep up with the reader, so a mode whose arrangement has
that region shut has it opened for the page, which is the one thing a switch is allowed to open.
The `+` that opens another one sits at the right end of that region's tab strip, drawn in a
group that holds a terminal or the console, in the pane region even when it holds neither, and only
while a project is open — because a pane runs in a project's folder. Beside it is a chevron, drawn
whether or not a project is, which opens the menu of what else can be reached here: the shells this
machine has, and a row that puts the console on screen. A runnable tool is not one of its rows —
the titlebar's own play triangle and chevron are where a project's tools are reached. What the rows are and what a click does is
`feat-panes`'s. Past a divider, the titlebar offers search (a stub), then two shortcuts that need a
project the same reason the pane region's own `+` does: **New agent** (`IconName::Bot`) raises the
New agent form directly, picking the chat strip as its surface in IDE mode and the agents screen
everywhere else — the same aim the `+` menu's own first row makes, with that menu's first stage
skipped; **New terminal** (`IconName::SquareTerminal`) opens the bottom region if it is shut and
spawns exactly one pane, never two, whether the region was already open or had to be opened onto
panes still in it. Then the notification bell, then a chevron gathering four controls reached
occasionally rather than every session — Connect to a remote host, Explore the project in browser
(needing a project), Capture this window (needing a project and captures offered on this platform),
and Settings — each row left off whole rather than drawn disabled when its condition fails, in that
order. Finally the theme switch. The overflow chevron lights while its own menu is open, the rule
every menu-raising control in this strip follows; the bell lights while its list is up, carries a
badge of what has not been read, and flashes when something arrives —
[`notifications.md`](./notifications.md) owns all of it.
Settings are interface-wide, so the overlay opens with no project.

**The network button's modal opens on a pick between two carriers**, a pair of pills reading `crate::state::remote::ConnectMode` — **Ubiq host**, a socket dial taking a pasted connection string
or an address and token typed separately, and **Drone over ssh**, a saved SSH profile plus a folder
on the far machine. Both share the same four steps — editing, connecting, connected and
failed-with-reason, `crate::state::remote::RemoteConnectStep` names them — and the same footer and
Connect button; only the body beneath the pills and what "ready" means differ. Pasting a whole
`http://host:port?token=…` into the socket carrier's address field splits it across both fields at
once, so the one string a running `ubiq --serve` printed is the only thing a user has to paste. The
ssh carrier lists the SSH profiles from the settings page (below) as pills of their own — connecting
is disabled until one is picked — a folder field the drone serves, empty meaning the login
directory, and a third pill row for the drone's own lifetime: **Attached** (dies with this `ssh`),
**Session** (detaches, survives a dropped link, not Ubiq quitting) or **Managed** (detaches for
good, until stopped from the Drones settings section, below). `Session` and `Managed` both launch
the drone with a `--listen … && exec … --attach …` line that *adopts* one already running on those
roots rather than starting a rival — reconnecting after a dropped link is a fresh instance of that
same line, on the same terms. `Connected` attaches the dialled host to the window's `Bus` as a `HostRef::Remote`,
labelled — for a fresh dial, by the address or the picked agent definition's name, or the saved host's own
name for a reconnect started from the Hosts settings section, below — what `Bus::remotes()` hands
back to the project picker's "Open remote project…" row (above) so a project can be opened on it. A
dial that succeeds is saved automatically, its carrier included, so it can be offered again after a
restart — the Hosts section, below, is where that record is managed. A dial that fails, ssh included,
lands on the same `Failed` step and shows the reason verbatim — an unreachable host, a refused
handshake, or a drone that exited on the far machine all read here rather than the modal being stuck
on "Connecting" forever. **A first connect to a machine with no drone deploys one and says so as it
goes**: the far `PATH` is tried first, and only a shell answering "not found" sends Ubiq round the
four steps the `Connecting` line names in turn — checking what was uploaded before, asking what
machine it is, verifying the drone built for it, uploading it. A machine this build has no
cross-built binary for stops there, naming the triple and `just drone-build`, which is what a tree
with an empty manifest says every time. The transport underneath the modal — `crates/ubiq/src/app/remote_connect.rs`'s
socket dial, and `crates/ubiq/src/app/ssh_connect.rs`'s `ssh` child process, each ending in the same
pump threads — is [`../tech/architecture.md`](../tech/architecture.md)'s to describe, not this
document's.

**A Check control sits under the ssh carrier's profile picker**, answering a question neither of
the modal's own moves can: is this host reachable at all, and does it already have a drone on its
`PATH` — without listing what that drone is running (which needs one already dialled) or dialling
and deploying one (which the Connect button already does, at the cost of a real attempt). Three
outcomes land, not two: unreachable or unauthenticated shows the connect failure's own sentence in
`theme::danger`; reachable with nothing installed shows in `theme::warning` — a useful middle
answer, not an error, since the Connect button can still deploy one from here; reachable with a
drone shows its version, its triple and the round-trip time in `theme::success`.
`crate::ui::host_check_line` renders that one line wherever this document names Check — here, and
in the Drones settings section below.

**Application settings' Hosts section is where a window says which host an unaddressed message
reaches, and where a saved host is managed.** A dropdown lists `Local` first, always attached; then
every remote this window has dialled, each read as attached; then every saved host with no live
connection of its own, read as not attached or, if the last reconnect from this list failed to
reach it, failed — a saved host reached over a live connection is folded into that attached row
rather than drawn a second time. Picking `Local` or an attached remote calls `Bus::set_active` and
nothing else: **switching the active host does not move a pane or a project the window has open** —
each stays routed to the host that owns it, exactly as `D81` has every unaddressed message resolve
against whichever host `active` names. Picking a saved-but-unattached or failed row instead raises
the same connect modal described above, address prefilled and the token left blank — a token is
never kept, so a reconnect always asks for it again. Below the dropdown, a "Saved hosts" list shows
every remembered host with a per-entry **Forget**, which drops the record and nothing else — a live
connection under that address, if this window still has one, is untouched. Each row carries a chip
saying which of attached, saved and last-attempt-failed it is, and an attached row gains a
**Disconnect** beside Forget: it closes every pane that host was running and drops the socket while
leaving the record, where Forget drops the record and leaves the connection — a host can be either
without being the other. A socket that fails on its own runs the same teardown and marks the address
as a failed attempt; a Disconnect does not, because nothing about the host went wrong.
**A saved host is named
after the address it was first reached at, and renaming is not offered** — only forgetting;
[`../backlog.md`](../backlog.md) (`G169`) names both that and the always-blank token as the gaps
this section leaves open.

**A remote's projects sit in the same list as the local machine's.** Attaching a host asks it for
its catalogue, and its answer adds its projects to the picker beside the ones already there rather
than replacing them — each row is opened, closed and worked in exactly as a local one is, and
nothing on it says which machine it lives on. Losing that host takes its projects out of the list
again, along with its panes. What a new pane's menus offer stays the local machine's whichever host
is active (`G188`).

**How much of a project Ubiq indexes is a setting, per project, with an application-wide
default.** Application settings' Search section offers three levels — Off, Full text, and Full text
+ symbols — and a project's own settings dialog offers the same three plus a Default row that
returns it to whichever the application says. The levels are cumulative: Full text + symbols is
Full text plus a record of what each file defines, not an alternative to it. Off walks every file
on every query, which is what content search did before an index existed; the other two read only
the files that could match. The default is Full text.

A change takes effect at once rather than at the next open: turning indexing off frees the disk
while the dialog is still up, and turning it on starts a build in the background. **The filesystem
watch runs at every level, Off included** — what a level decides is what Ubiq keeps, not what it
notices — so a project that is not indexed still refreshes its explorer and its Git state. Nothing
is written into the project's own folder at any level.

**What the board calls a mission is a word, not a fixed label, on the same application-wide-default-
with-a-per-project-override shape the indexing level takes.** Application settings' new Board
section offers one field, seeded "Mission" — *mission*, *epic*, *user story*, whatever the team
calls a task allowed to have children and to carry a plan — and a project's own Tasks tab offers a
Default pill naming that word and a Custom pill that reveals a field of the project's own. A change
takes effect at once: the board's mission chips, its task panel and its settings row all read the
resolved word the moment it is typed, with no restart and no per-window drift.

**The browser button starts the web-export server and opens the project in it.** Drawn only with a
project open, it starts the local, on-demand `tiny_http` server on first use (or reuses it if
already running), registers the active project under a slug, and opens the returned URL in the
system's default browser. The server stays up for the process's life once started; a failure to
bind or spawn is logged and the next click tries a fresh start — there is no stop control and no
retry loop. It serves markdown rendered to HTML and other source files as syntax-highlighted code
(highlighted client-side, via a CDN-loaded highlight.js — there is no server-side highlighter here),
read-only, over `127.0.0.1` only. See `D55` for why this lives in the interface rather than the host.
Its palette is Ubiq's own — `style.css`'s custom properties are transcribed from
`crates/ubiq/src/theme.rs`'s `light()`/`dark()`, not a separate design.

**The page itself has two search affordances, both browser-side.** The sidebar's filter matches
names already in the DOM, live as it is typed, and its "docs only" toggle beside it (same row as
the title/filename toggle, both persisted per-browser) hides everything but `.md`/`.markdown` rows
— the same rule the server uses to classify a file. A second, separate box in the header searches
file *contents*: Enter hits a `GET /<slug>/_search?q=` route that walks the project the nav tree
already walks (so `.gitignore` still applies), does a plain case-insensitive substring scan over
doc and known source files, and returns matches as a dropdown of file/line hits. Clicking one opens
that file with its in-page find already run — see below. The scan is capped — 200 files, 20 lines
per file — and says so (`truncated`) rather than pretending a stopped scan was a complete one.
Deliberately no regex and no ranking: this module keeps zero dependency on `ubiq-host`, so it
doesn't reach for the ripgrep machinery `crates/ubiq-host/src/search/` already has for the in-app
project search.

**Every file the server can classify gets one of four views**, decided in `routes.rs`'s `serve_path`
by extension first and, only when that's not enough, by a bounded read of the file itself. A `.md`
or a known source extension renders as text — markdown to HTML, source as highlighted code — full
width, with a toolbar: Copy (fetches the file's own `?raw=1` and writes it to the clipboard),
Download (a plain link; the `?raw=1&dl=1` response carries a `Content-Disposition: attachment`
header, so this is a real save, not a re-render), and Find, an in-page search over the rendered
content that wraps matches in `<mark>` with a count and next/prev — the same box a content-search
result opens pre-filled with its query, via `?q=` on the destination URL. A recognised image, video
or PDF embeds inline (`<img>`/`<video>`/`<iframe>`, still pointed at its own `?raw=1`) with only a
Download button — nothing to copy or find in a picture. Everything else is sized first: past 256
KiB an unrecognised extension is never read at all, only its name and size are shown beside a
Download button; at or under that ceiling the file is read once and sniffed (valid UTF-8, no NUL
byte) — text-shaped, it renders exactly like a known source file; not sure, it stays a size-and-
Download card with an added "Show text" button that force-renders it client-side, cheap because the
sniff ceiling already bounded the fetch. No file is ever downloaded by just visiting its page.

**Navigating around the page is a single-page app over a server that still answers every URL on
its own.** Clicking a same-site content link — the sidebar, a directory listing, a search result,
the project link — is intercepted, fetched, and swapped into `.app-main`/`.app-toc` via
`history.pushState`, so the theme and scroll position never flash between pages; the browser's
back/forward buttons do the same swap on `popstate`. A same-page anchor (a bullet-point table of
contents, a footnote back-reference) is deliberately left alone to scroll natively rather than
being swapped for a copy of the page it's already on. None of this is load-bearing: a pasted link,
a hard refresh, or the router simply failing to fetch all fall through to a normal request, which
the server renders in full exactly as it always did — the SPA layer is progressive enhancement, not
the only way to reach a page. A theme flash on a full load is separately prevented by a small
blocking script at the top of `<head>` that sets `data-theme` from `localStorage` before the
stylesheet paints, ahead of `script.js`'s own `DOMContentLoaded` handler.

**The capture button photographs this window into a dirty untitled tab.** It wears
`IconName::Frame`, carries the tooltip every icon-only control in the cluster does, and is drawn
only with a project open and where the capture is offered — the Appearance switch is on, the
platform is one whose frames this build decodes, and the runtime backend answers.
`⌘⇧2` / `⌃⇧2` is the same act from the keyboard, and the switch removes the button, the tooltip and
the keystroke together. The route is the operating system's screen capture, so a source is a
display and never a window: one frame off the window's own display is streamed, cropped to the
window's bounds at the window's scale, encoded PNG, and opened as `capture-{n}.png` — the same tab
an image on the clipboard opens, dirty from the start. The stream ends on its first frame; a
screenshot is start, take one, stop. Where the platform asks for consent the first press is what
asks it, and where it does not the Appearance switch is the only thing between the application and
a screenshot. Nothing crosses the bus.

**The balloon beside the bell photographs the window before it raises anything over it.** Pressing
it runs the same capture the capture button does — `capture_offered()`'s three gates apply, the
platform's own permission prompt on macOS and Wayland included — and only once the picture is back
does the feedback modal open, so the modal is never in its own screenshot. Where no capture is
possible the modal still opens, with no picture, because a report is worth sending without one.
It carries a title, a Type dropdown (Bug, Feature, Eco feature), a description, the screenshot with
a Remove button, and Send. Send is dimmed while the title is blank, while a report is already in
flight, and in a build compiled with no feedback destination at all — the sentence under the button
says which of the three. A report that lands turns the footer into a receipt, with an Open link
when the destination returned one. The balloon is drawn always, project or none, because it says
something about Ubiq itself rather than about a project.

**Application settings is a page overlay, not a one-question modal.** It is `SETTINGS_WIDTH` by
`SETTINGS_HEIGHT`, clamped to the viewport, with a left nav and a body that both scroll —
`kit::settings_split`, the one component the project dialog draws too; switching
sections does not resize the panel. Toggles persist as they are flipped — there is no Save. Opening
it dismisses project settings, and the reverse.

**The nav is a container, and a section is a registration in it** (`D180`). Its label, its icon, its
body and whatever it asks the host for on arrival are fields on one `SettingsSectionSpec`, shared
with the project settings dialog's own nav; the container owns the panel, the nav and the body's
scroll, and a section owns only what is inside it. The order is by declared group — interface,
agents, connectivity, system — and never by an integer a section carries.

Sixteen sections ship: **Appearance** (the palette
family as one pill per family — labelled by the member whose ground is in use, so picking one keeps
the ground; the ground itself, the same flip the titlebar offers, within the family; the accent as a
row of swatches, the palette's own first and then the six the build ships, each named on its
hover; then **Themes**, the themes the user authored as pills beside the built-ins, each with an
edit affordance, and **New theme…** — followed by
whether the rail carries the open-project badges, whether
the titlebar's capture control and its keystroke are offered at all, and whether a conversation
footer draws a second ring comparing cached tokens to the total — off by default), **Size** (the
same preset pills, `ui_scale` slider and `text_ratio` slider the status bar's size popover draws,
built from the one shared `crates/ubiq/src/ui/size.rs`; the saved-preset list, with Rename and
Delete beside each user-saved preset and neither offered on a built-in nobody has saved over; a
trim pill ladder each for **Chrome text**, **Conversation text** and **Content text**; and Reset,
which returns both sliders to 1.0 and leaves the three trims alone), **File
explorer** (whether a
single click opens a preview tab, and the two folders a clone lands in — the default project folder
and the ephemeral folder, each with a chooser and a clear button, and each showing the host's own
default as a placeholder rather than a path the interface invented), **Editor** (whether a new markdown file opens in preview or
source), **Harnesses** (the accounts
registered here, and an `Add harness` above them), **Agent definitions** (the saved setups, with
`Add agent` above them), **Isolation** (whether an
agent is confined, whose home it runs in, and the directories it may reach beyond its policy),
**Search** (what every project's search skips, and what a project is indexed to), **Connectors**
(the named identities at each provider, and the registrations a flow picks from), **Hosts** (the
remote hosts a window may attach to), **SSH profiles** (the named SSH targets a host connection can
dial, each with its address, port and user and one of four ways to authenticate — an agent, a key
file, a password or a `~/.ssh/config` alias — and a key's passphrase or a password filed at this
machine's own credential store rather than in the settings file), **Drones** (per saved ssh-carrier
host, what `ubiq-drone --list` reports is running there — uptime, pane count, version, linger and
the roots served — with a Check beside Refresh that asks only whether the host is reachable and a
drone already on its `PATH`, drawn the same one line the connect modal's own Check does, above; a
Refresh that shells to `ssh` again and a Stop, danger-confirmed because it
kills every pane the drone holds; a version differing from this build's own is flagged with the
one remedy the design names: stop, redeploy, reattach), **Assistance** (which backend
writes the short lines Ubiq
would otherwise invent mechanically, what that backend reports about itself, whether a conversation
names itself from its opening exchange, and the API providers
configured here — each with its key typed once, its models picked from what the provider lists, and
a test that streams a real answer back), **Tools** (the runnable tools
every project inherits, edited at machine scope),
**Command line** (the `ubiq`
command on the shell's `PATH`), and **Extensions demo** — the kitchen sink's own contribution
(`X11`, `D186`), registered through `ext::settings::register` with nothing in this list's own
module touched to add it, and the switch that gates the rail's own demo mode below. The kitchen
sink still draws the larger
fixture nav; that page is how the furniture is looked at, not how the application is configured.

**The theme editor is a modal over that page, not more rows in it.** Appearance is a fixed panel
with a nav down its left, so a token list beside a colour picker above a live specimen does not fit
in what is left; the editor is a `kit::modal_sized` at `THEME_EDITOR_WIDTH` by
`THEME_EDITOR_HEIGHT`, raised by **New theme…** and by a theme pill's edit affordance, painted at
the window root, and peeled by Escape one rung above the settings page. Its left column is the
editable set grouped as `theme.rs` groups it — Grounds, Ink, Borders, Accent — with a dot on every
token the author has written and no dot on the ones inherited from the fork; its right is
`kit::colour_picker`, the same control project settings uses, with **Inherit from the base** under
it for a token being given back. Under both is the style reference's own token strip, so what an
author is shown is what the window is painting. **Editing a theme wears it**: there is no draft and
no Save, the change is in force and written down as it is made, which is also why a theme is edited
in the palette it *is* rather than against somebody else's ground. Naming is `kit::prompt_modal`,
raised for **New theme…** and for Rename alike. Delete sits in the editor's footer, and a theme
deleted while it is worn falls back to the palette it forked rather than leaving the window naming
something that no longer exists. Nothing here imports or exports a theme file, follows the system
appearance, or is per project — a theme is a property of the person, like every other appearance
value (`D151`, `D152`).

**An account is a home, drawn as one block per identity, with a harness line under it for each
one it can start.** That is the whole of what the interface knows about an identity: no
credential and no path reaches it, because neither crosses the bus. A block with an empty
harness list says "not signed in" rather than nothing at all — an account can reference an
environment variable instead of a captured session, and the two are different answers. The list
is asked for on every open, so an account signed in from elsewhere appears without a restart.

A block's header carries the account id and two icon actions — **Rename** and **Delete** — that
act on the identity as a whole, because renaming or deleting an account renames or deletes every
harness signed in there with it: an account is a home, not a per-harness reference. Each harness
line under it carries the harness's own display name (resolved through what the host offers for
that harness, falling back to the raw id when it no longer lists one), its last-checked status,
and three actions that act on that one login alone: **Check** asks the host whether the stored
credential still works and updates the status line in place, with no modal; **Re-authenticate**
runs an ordinary login against the harness's own flow, exactly as a first sign-in does, and does
not pre-empt what the harness says — it may well answer that the identity is already signed in,
and the user reads that in the terminal and decides; **Sign out** removes just that harness's
credential, leaving the account and its other harnesses untouched. Deleting the account is the one
action with nothing left behind afterward, which is why the confirmation says so and why the word
is "Delete" rather than "Forget".

A status line reads `valid · expires in 3 days`, `expired 2 days ago`, `signed in · no expiry
recorded` for a credential with no embedded expiry, or `no credential stored` for one the host
found nothing for — an expired credential's line is drawn in the danger colour, everything else
muted. No entry is drawn until a check has answered, rather than guessing. A refusal from any of
these actions — a rename to a name already taken, a delete or sign-out the host would not do — is
not a dialog of its own: it surfaces as a dismissible banner over the harnesses section, in the
same warning shape a project's own row confirmations use, and clears itself the next time the user
opens a dialog, starts a login, or dismisses it by hand.

**Under each harness line sits how much of that plan is left.** One row per window the provider
states — its own label, a meter, the reading, and when it resets — then the plan, how old the
reading is, and a Refresh. This is the surface that answers when nothing is running, which is the
moment the question is actually asked, so it is filled by asking the host for every login on the
page when settings opens; Refresh is the one control that makes the provider be asked again, because
the endpoint behind Claude's is unofficial and rate-limits.

**Every negative answer is drawn in place rather than hidden**, because an absent readout reads as a
missing feature and for most harnesses this is a permanent fact about the provider instead. A
harness whose provider publishes no queryable limit says so in one sentence. A provider that
answered and named no limit says that, which is a different sentence. A failure is drawn as the
sentence the host sent, since it is written to be read — "Claude is rate-limiting the usage
endpoint" is not the same news as "this harness does not report usage limits". A reading with no
denominator anybody stated — a count against no ceiling, a credit balance — draws the figure and
**no meter**, on the rule the Control screen already keeps: a meter needs a denominator, and a
fraction of an invented total is drawn with more confidence than the number behind it deserves.
Meters take their colour from the same `theme::usage_tone` thresholds the conversation footer's ring
does, so the two surfaces cannot disagree about where "nearly out" begins.

**Signing in is a modal with a real terminal in it, because the harness runs its own login.** Add
harness asks two things — which harness, and what to call the identity — and then the harness's own
flow runs in a pane inside the modal, browser round-trip included. That running step alone draws
in a wider, taller surface than the rest of the flow — some harnesses' logins are full-screen TUIs,
not a line-based prompt, and need real room to redraw correctly; the picker and the brief
Signing-in step in between stay at the ordinary modal size, because they are just a question. The
shape itself is `tech/ui-and-design.md`'s to state. Between asking and the harness
actually answering, the modal shows a **Signing in** step with nothing to interact with but Cancel
— the same step a re-authentication starts on directly, skipping the picker because both the
harness and the identity are already known. A modal rather than a tab on purpose: an OAuth flow
wants the whole of the user's attention for the half-minute it takes, and a login that scrolled
away behind a pane is a login nobody finishes. Abort is always available and always safe — a flow
that wrote no credential captured nothing, and the host says so rather than recording a half-made
account, so starting again is free.

**The picker lists every harness, installed or not, because an override is what makes an absent one
startable.** A row for a harness whose binary this machine lacks draws faint but stays pickable,
the way an absent harness reads elsewhere in this settings page, rather than being withheld the way
the new-agent menu withholds one. Picking a harness reveals a **Custom command** ghost toggle; open,
it shows a text field pre-filled with whatever override this machine already holds for that harness
(placeholder-only when there is none, showing what the library would otherwise run) and a **Check**
button beside it that asks the host to try the typed command and answers with one line, shown under
the field, without saving anything. The value is committed — folded into the settings the interface
holds, and `ListAgentTypes` re-asked so every row's availability reflects it — when the field is
closed and again when **Sign in** or **Shell** is pressed; an emptied field removes the override.

**A `Shell` button beside `Sign in` runs a plain shell under the login's own sandbox, and signs
nobody in.** Same harness and identity picker, but the pane it opens runs the user's shell rather
than the harness — under byte-for-byte the same policy the harness's login would get, computed the
same way, from the harness's own program rather than the shell. This exists so a human can
empirically check what that sandbox actually permits (`which node`, `ls ~/.local/share/mise`, …)
instead of only reasoning about it — a real Codex login failure inside the sandbox is what this
diagnostic was built to let someone see for themselves. Its button wears the ghost treatment
rather than the primary one, with a tooltip saying so, and the running/starting/done steps say
"shell" rather than "signing in" while a probe is up. A probe never writes a credential, never
records an account, and its pane's close is read entirely from the ordinary `PaneExited` a real
login also gets — the host answers a probe with nothing else at all.

The host scans the login pane's own output for a URL and offers each one as a row below the
terminal — a button carrying the URL itself (truncated so a long one cannot widen the modal, the
full string still on hover) that opens it in the system browser, and a small icon beside it that
copies it instead. The terminal's bytes are never touched by this: the harness's real output is
still what is drawn, the links are only ever an offer to click what a terminal cannot make
clickable.

The login modal is painted from the window root rather than from the settings page that raised it,
for the reason every overlay there is: a login outlives the page. Closing settings mid-flow must
not take the harness's sign-in with it. Its pane belongs to no project and gets no dock panel — the
modal is the only thing that draws it, which is also what keeps one emulator from being rendered in
two places at once. The rename, delete and sign-out questions are painted the same way, over
whatever raised them, for the same reason.

**Agent definitions is its own section, and it is where a conversation's setup gets a name.** A
harness is a tool this machine has and a definition is a recipe written against one; the two lists
grow at different rates, and reading the recipes under the heading of the tools made them read as
one list. One row per saved agent definition, read as `reviewer — Codex · syn · gpt-5 · Plan · 2
MCPs`: the id, then
whichever of the harness, the account, the model, the mode and a count of the MCP servers it
launches with that agent definition pins, the mode drawn by its label rather than its harness-native id, and
the MCP count left off an agent definition that pins none. Badges before the summary say what the
record says about itself — `Coordinator`, `Worker`, `Off` — because a status is shown by colour
from the status group and never by wording alone. A row whose harness is not installed on this machine is drawn
faint, the same way a harness type is in the New-agent menu, because a definition survives a machine
that cannot run it; a switched-off one reads faint for the same reason, being listed and not on
offer. A definition carrying `AgentDefinition::description` draws it on a second line under the
name, elided to one rather than wrapped — the field is written for another agent to read through
the mission tools (`tech/transport-contract.md`), and the row is a summary, not the form it can be
read in full from. Each row carries `Clone` and `Edit`, and `+ Add agent` opens the same form empty.
Nothing here deletes or renames — see [`../backlog.md`](../backlog.md) — so correcting an agent definition
means saving over its id, and typing a different name saves a second agent definition beside the first.

**`Clone` names the copy itself.** The host refuses a clone onto a name already taken — a clone
never overwrites a saved setup — so the interface sends `CloneAgentDefinition` with the first free
`<id> copy`, `<id> copy 2` in that definition's own scope, rather than a name it can already see
will be rejected. What is copied is the record, fields no screen shows included, which is why it
is one message rather than a read and a save.

**`Add agent` is drawn unavailable where the host would refuse it.** A definition names a harness,
so a machine with none has nothing one could run and `Agents::save_definition` rejects a *create*
(never an edit: a machine that lost its harness must still be able to repair what it wrote). The
interface asks the same question of `AgentTypeInfo::available` and draws the button faint, taking
no click, with the reason on its hover. A refusal the interface can see coming is drawn, not
waited for.

**The agent definition form is the New agent form with a name field above it.** An agent definition is a saved answer
to the questions a start asks, so it asks them with the same rows, drawn from the same
`NewAgentForm` — a second form asking one set of questions differently is how the two drift apart.
What `Purpose::AgentDefinition` changes is only what an agent definition does not have: no agent definition row in the target
picker (an agent definition built out of an agent definition is the `extends` chain, and that is written by hand), no
`Start`, and a `Save` under a name that is typed rather than asked for afterwards. The model and
the level are drawn from `HarnessCatalogue`, asked for as the form opens, so a model is chosen off
the harness's real list here as it is at a start. Save is dimmed until the agent definition is named and a
target is chosen; everything else may be left unset, because "no answer" is a real answer an agent definition
can hold.

**A `Mission assistant` checkbox sits under `Purpose::AgentDefinition` only**, beside `persistent`, and
answers one question a start never asks: whether this agent definition should be offered by the new-mission
dialog's assistant picker. Ticking it writes `AgentDefinition::mission_assistant`, read forward from the
opened agent definition so a save never silently clears a flag already set. `None` and untucked both read as
"not an assistant" to every filter over the field — the distinction exists only so an agent definition
extending one that is ticked can un-mention it.

**Three more checkboxes sit under it, and they are the record's own answers**: `Mission/task
coordinator`, `Mission/task worker` and `Disabled`. The two roles are not labels — the host holds
the MCP set each implies and re-adds it on every save, so a definition carrying a role cannot be
left without the servers the role needs, however the checklist was edited. The MCPs panel draws
that: a server a role implies is ticked, says `Required by this agent's role` under its name, and
takes no click, because a tick the next save would put straight back is not the checklist's to
remove. The slugs are mirrored in `state::new_agent::{COORDINATOR_MCPS, WORKER_MCPS}` the way
`TASK_ASSIGN_MCPS` already is — the interface names no host type, and the slug is the contract —
and the form still writes down only what was ticked by hand, leaving the merge to the host.
`Disabled` is the off switch: the definition is kept, still listed and still editable, and offered
nowhere a run is started from.

**The Isolation section is the one the host acts on**, so its three rows are the only ones that write
the Host layer rather than the interface's own — an agent runs under a policy, and the half that
spawns the pane is the half that has to know. Every other row in settings is a `UiSettings` field.
Which layers a policy is built from is not here at all: that belongs to the harness library, which
already has the shape for it. See [`../tech/agent-manager.md`](../tech/agent-manager.md).

**The home is a choice with a cost, and the section says so rather than offering three equal
options.** The agent's home is the user's own by default, because a policy grants a toolchain by
naming paths inside a home — and a replaced home aims every one of them at a directory nothing
populated, so the agent holds `cargo` on its `PATH` and cannot build. A fresh home per run and a
named home are the two answers for someone who wants an agent kept away from their own dotfiles and
will populate a home to get there; each card carries what it costs. Extra grants are the escape
hatch for a toolchain installed somewhere the policy does not expect: a path per chip, read-only
until the chip is clicked, because read-only is the safer half of the choice and what a shared cache
usually wants.

**Pane confinement works on macOS and Windows, and the section reports what does not rather than
failing at the spawn.** A build with no enforcing backend — Linux today — draws the state chip in
the warning colour and dims the toggle, status by colour and not by wording alone, the same as
everywhere else. A row that promised protection it could not deliver would be worse than the plain
run it silently became.

**The Editor section's "Closing a tab" heading holds three settings, one per pane kind, each a
`Hide`/`Close` choice for what that kind's × means.** `terminal_close` (a plain shell pane) and
`agent_terminal_close` (a pane running an agent harness) both default to `Close` — a terminal is
cheap to start again, and a pane hidden by accident is one the user has to go looking for in the
new-pane menu's Detached group. `agent_chat_close` defaults to `Hide` — the conversation behind a
chat tab is the host's and outlives every view of it, so its × does not delete a transcript
unasked. The two kinds of terminal pane are told apart the same way `pane_is_agent()` tells them
apart everywhere else: a pane's resolved `agent_type` matched against the harness catalogue the
host sent, so a plain shell — whose type is a program name — always reads as the plain-shell
setting. Whatever a row says, the tab's right-click menu still offers both Hide and Close, so
either ending is reachable regardless of the standing setting.

**Vim mode is one switch over every text surface a document is written in.** Off by default, and
turned on either from the Editor section of settings or by clicking the status bar's mode chip. On,
the file editor and every multi-line box — the chat composer, an agent's input, a task description,
a commit message — read keys as vim commands: Normal, Insert and both visual modes, operators over
motions and text objects, counts, the unnamed register, and `/` search. `*` and `#` take the word under the cursor — the next
one along the line when the cursor sits on a blank — and match it whole, so `*` on `foo` never stops
on `foobar`; a pattern typed at `/` means exactly what was typed and still matches inside a word. Single-line fields are
untouched, because a filter is a question and not a document, and so are terminal panes, which are
not inputs at all. A buffer is focused in Normal mode and a composer in Insert, so that clicking
into the chat and typing still works the way it always has; the mode is the window's, not each
input's, because exactly one input holds focus at a time.

The caret does not change shape between modes. The component paints one fixed caret and offers no
way to ask for another, so the status bar is where the mode is reported instead — and, since a
readout the user cannot act on and a switch that does not say what it did are the same slot asked
twice, that chip is also the switch.

**The `ubiq` command is a small script Ubiq writes onto the shell's `PATH`, and the Command line
section is where it is put there and taken away.** `ubiq .` opens a folder as a project and
`ubiq README.md` opens a file, in the window that already holds it when there is one. It is a
script rather than a symbolic link because that is the only shape that reaches the running
application: on a macOS bundle the script runs `open -a Ubiq.app`, so LaunchServices hands the path
to the window that is already up. Off a bundle it execs the binary, and a second application starts
with a catalogue of its own — the section says so rather than pretending otherwise, and `D56`
records why. What the window then does with the path is what a drop on the window does — a folder
opens as a temporary project, a file outside every project opens as a guest tab (`D54`).

**Every script Ubiq writes carries a `# ubiq-target:` marker naming the build it launches, and that
line does both of the section's jobs.** Its presence is the proof Ubiq wrote the file, so **Remove**
deletes only a shortcut of Ubiq's own and never a `ubiq` the user put on their `PATH` themselves.
The path on it, compared against the running executable, is how a shortcut left behind by a moved or
replaced build is told apart from a current one — that is the `stale` case, and the button reads
**Update** in the danger tint instead of **Remove**.

**The section is one row over the list of directories the host considered.** The row is a button —
**Install**, **Remove** or **Update** — and a status line saying where the shortcut is or where one
would go, and whether that directory is on `PATH`; a directory that is not draws the line asking the
user to add it to their shell profile. Under it every candidate is listed in the order the host
considered them, each marked *missing*, *exists · on PATH* or *exists · not on PATH*, with the
chosen one ticked, so a machine none of them fit says why rather than failing quietly. The button is
disabled when no directory can be used at all. Which directory is used is never the interface's
choice: the candidates, the choice among them and every path drawn here are the host's answer, and
the interface sends one of three actions and draws what it is told. The candidates, best first, are
`~/.local/bin`, `/usr/local/bin`, `~/bin` and then any `PATH` entry under the home directory — on
Windows, `%USERPROFILE%\AppData\Local\Microsoft\WindowsApps` and then `PATH` entries under the home
directory. The one written to is the directory already holding a shortcut, else the first directory
**in the user's own tree** that both exists and is on `PATH`, else `~/.local/bin`, which install
creates. `/usr/local/bin` is listed so that a shortcut already sitting there is found and can be
removed, and is never chosen: it is root-owned on a stock machine, and choosing it would answer a
button press with a permission error while a writable directory sat one line above it. A write or a delete the
host refused surfaces as a banner over the section, in the same shape the harnesses section uses.

**The section asks every time it is opened**, for the same reason the harnesses section re-lists its
accounts: a shortcut can be moved, deleted or left behind by another build while the window is open,
and the answer costs a directory listing. Until the host answers, the section says it is looking —
which is a different thing from saying no shortcut is installed.

**The status bar's right holds the size popover, an icon-only trigger whose tooltip names the
active preset — `Size — <preset>` or `Size — Custom`.** Clicking it opens `ui::size::panel`, an
anchored menu rather than a modal — no scrim, dismissed by an outside click or by Escape through
`MenuId::Size` — holding a row of preset pills, the interface slider over `ui_scale`, the text
slider over `text_ratio`, and a `Save preset…` / `Reset` pair of `kit::ghost_button`s; no number and
no unit appears anywhere in it. It works with no project open. `cmd-=` and `cmd-shift-=` nudge the
content family's own trim up and `cmd--` down, by ±0.05 within the range the chrome admits,
independently of the popover — the content trim pill ladder is one of the three the Size settings
section draws instead.

**An explorer row is sized from its text, not from a constant.** The row's height and the tree's
per-level indent are both derived from the size the row draws at — `kit::row_height()` and
`kit::row_indent()`, floored so the twisty and the kind icon never touch the edges and capped so the
tree does not become a column of buttons at the top of the range. A zoom therefore makes the tree
taller as well as larger, and a small size gives a genuinely denser list rather than small text in
the old box. The file picker and the ref list draw the same chrome at `kit::row_font()`, because they
are dialogs rather than a project's workspace and no project's zoom reaches them.

**The status bar reports facts, not intentions, and an absent fact is drawn as absent.** It reports
on whatever is on screen, so the rail mode decides which set of facts it has. In IDE mode with a
project open: the active file's project-relative path, what its save is doing when that is worth a
word, the project's branch (or detached short id, or unborn name) with ahead/behind and working-tree
totals when those exist, the vim chip, the caret's real one-based line and column, the file's
language, encoding and line ending, the harness and mode the composer is set to, and the active
file's text size. The vim chip is the one thing in the strip that is not only a readout: it reads
`VIM` faint when modal editing is off and the mode — or the command being half-typed over it, or
the open `/` line — when it is on, and a click toggles the same setting the checkbox does. **The
language is the second**: it is a chip, and clicking it opens a searchable picker of the viewers a
file can be drawn with — `ViewerKind::all()`, filtered by what is typed — so a file the extension
sent to the wrong viewer can be redirected without renaming it. Forcing a kind that names a
language of its own — Markdown is the one today — settles `language` the same way, so the source
underneath is highlighted as what was picked rather than as what the extension named; a kind with
no language of its own, the plain Editor included, leaves whatever is there. What is picked lasts as
long as the tab does and is written nowhere: a viewer forced on for one sitting is a way of looking
at the file now, not a fact about the file, so a tab closed and reopened starts from
`ViewerKind::of` again. A project that is not a repository
prints nothing git-related, and a branch with no upstream draws no `0/0`. The caret and the language
go with the file, so a window with no file open reports neither rather than a position in nothing.
With no project open it says so and stops. On the screens over the agents there is no file and
no caret to report, so it counts instead — on `[Teams]` alone today: how many sessions and agents
there are, and how the agents are spread across the four states, each count in its state's colour. A
count of zero is drawn as zero rather than dropped — "no agent is failing" is a fact, and it is the
one the user is checking for. `Teams` draws none of it. On the agents screen it reports on the field rather than on the
project: how many columns there are, how many agents they hold, how many of them are grouped, how
many are on the bench, and the same four states over the agents in those columns. The strip reports
on what is on screen, and the bench is exactly the difference between the field and what the host
reports. At the right it names each harness behind the columns once, which is the one fact about
them the columns' own footers say only one at a time. On the board it counts the work instead: how
many cards are in each column, how many sub-tasks are done across the cards on screen, and how many
of them nobody can finish without the user — over the cards the filters leave, because a count that
disagrees with what is drawn is worse than none. Where Ubiq is writing is not among the facts it
reports: a non-default config root is named once in the log at startup and nowhere in the chrome,
because the strip is read while working on a file and a path that never changes is not news.

**Two entries at the strip's right are the one deliberate exception to "facts, not intentions."**
The build version (`version_label()`, `crate::version::short()` with the full string as its tooltip)
and the "Made with ♥ in Turin" attribution (`made_with_love()`) sit at the leftmost position of the
right-justified half, in every rail mode. Neither is a fact about what is on screen — a build never
changes while the window is open, and the attribution reports nothing at all — so the rule above is
about state, and this is static branding drawn alongside it, not a fact the strip forgot to gate.

**The window remembers the arrangement it was left in, and which files were open in it.** The whole
dock as it serialises itself — the tree, the axes, the sizes and which tab of each group was
displayed — the rail mode, the files open in the centre with which of them was in front, the folders
the explorer had expanded and the row it had selected all belong to a project; the palette belongs
to the interface. Both are stored by the host, which keeps them as an opaque blob it never reads, so
the schema stays the interface's own. The arrangement is remembered **per rail mode**: each mode
keeps its own record of which regions were on screen and its own dock blob, so a visit to the sink
does not undo the IDE's side panels and a visit back does not summon them where the sink was
arranging. A mode with no record of its own opens on its defaults — the centre alone, no region
open, except Git, whose refs and changes open with the mode (`D119`) — and **so does a project that
has never been arranged in the mode it is entered in**: entering a mode forces its regions to those
defaults rather than leaving whatever was on screen, so a project reached from another one never
inherits that one's edges and has them written down as its own. A blob it cannot parse is discarded and the window opens on defaults.

**A mode is keyed in that blob by its id**, `ubiq.rail.ide`, since `RailMode` became a container
(`D184`). The ten names it used to be written under are read by the decoder, so nothing was
discarded at the `5 → 6` schema step — and **an id this build has no registration for is kept, not
dropped**: a mode a second edition contributes keeps its saved arrangement across a base launch,
which is the whole reason the serialization step exists. Which
mode the window is in is written down when the mode is chosen rather than when the arrangement next
changes, so two modes that arrange nothing between them still reopen in the right one.

**Three panels belong to the window, not to any project or any mode, and a project or a mode switch
takes them out of the tree** (`D156`). The log console and the search panel can be opened from any
project, but neither names one, so both are swept out on a project switch *and* on a rail-mode
switch, before the incoming arrangement installs — carrying one project's results, or one mode's
open console, into a screen that has never seen either would be inheriting, not remembering. Help is
the third, and the one exception: it is swept the same way on both switches **unless follow mode is
on**, because follow's whole contract is to keep the reader's page up with wherever they are
standing, and a panel that is a means of staying with the reader cannot also be swept out from under
them. A saved arrangement that names any of the three puts it straight back — that is the difference
between *remembered* and *inherited*. When the incoming mode or project has no saved arrangement at
all, the region a swept panel just vacated is collapsed rather than left as an empty bar. The search
*results* are reset only on a project switch, because they are one project's hits and a mode switch
inside one project has no reason to lose them.

**None of the three is ever put back by a state flag.** The search panel reaches the dock through
`reveal_search()` and the console through `reveal_console()` — the titlebar's icon, the ⌘⇧F binding,
`search_for()` and the new-pane menu's *Console* row — help through its own gesture, and each
through a saved arrangement that already named it. A search still in flight is not a reason to add
the panel: entering the IDE with an unfinished search behind it used to re-add the search panel over
whatever arrangement the user had left, which is the whole reason the presence of these three is the
arrangement's business alone, and a switch is never one of the gestures that adds them.

**Layout persists; harnesses do not.** The arrangement carries a version of its own, and one written
for another version is discarded whole for the default arrangement rather than half-applied. A saved
terminal panel names its pane, so a pane the window still holds keeps its place — its group, its
split and its tab position — across the two things that rebuild the tree under it, a rail-mode
switch and a project switch. One naming a pane the window does not hold is dropped and the tree
normalises around the gap: a project reopens after a restart with its side panels and its open files
where they were, and no terminals. Restoring a mode's arrangement forces every region back to the
state that mode left it in — a region another mode shut in between is reopened, one that was shut is
closed, and a region whose every panel was dropped is installed empty rather than left holding the
last mode's — so a visit away does not leave the mode's edges silently gone. A panel the window
holds that the restored arrangement does not name is put back in its home region rather than lost —
**added there, never revealed** (`D156`): a reveal would open the region it lands in, which is a
switch rearranging the window rather than restoring what the user left, and the very next
`LayoutChanged` would write that rearrangement down as though the user had asked for it. Which
regions the restore actually settled on is read first and forced back afterwards, since adding a
leftover into a region no blob named is what creates that region, and a created region is open.

**A panel writes down what it is looking at, not what it drew.** A file panel carries its tab and
the layout its viewer was left in, a terminal panel carries its pane's id, and nothing else — never
a parsed scene, a computed diff or a rendered diagram. Each of those is a function of bytes: the host will send the file again, and the
scene, the diff and the picture are made from it again, the last of them off the workarea's cache. Every other panel is
its name and nothing more.

**An untitled buffer with real text in it survives a project close; a real file's unsaved edits do
not.** A tab has no path to reread, so an untitled buffer is kept by its display name and the
buffer's own text rather than by the tab key every other file uses — an untitled *image* capture has
no buffer and is dropped, since there is no text to carry. Reopening the project replays each one
through the same machinery `⌘N` uses, no host round trip, before the saved `active_file` is
reapplied; a tab on a real project file is unchanged, its unsaved edits still gone the moment it
closes. **A pinned file persists the same way its tab key does**, because a tab key is the
one tab identity that survives a restart at all; a pinned terminal or a pinned chat tab does not,
and neither does a typed-over tab name, because a pane's id dies with its process and a chat's is
reminted every run — a saved name or pin would only ever point at nothing.

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
chrome around them. The toggle flips ground **inside the palette family the window is in** — a warm
dark reaches the warm light — because a family's two grounds are a pair the registry names, not an
inverse computed from a mode. Which family, which accent and how tight the grid is belong to the
interface rather than to any one project: all three are written into `InterfacePrefs` as they change
and are what a second window opens in, and so are the chrome and conversation text bases
(`chrome_font_size`, `conversation_font_size`). The project keeps one size of its own, the content
family's, nudged by `cmd-=` / `cmd-shift-=` / `cmd--` and set from the Size settings section's
content trim pills. The token set, the palette registry and the four axes
belong to the UI and design document, linked below.

## Contract

**Projects cross the bus.** The catalogue belongs to the host, and the workbench holds a projection
of it: `ListProjects`, `AddProject`, `ForgetProject`, `UpdateProject`, `LocateProject`,
`OpenedProject` and `RefreshProject` going out, `ProjectList`, `ProjectAdded`, `ProjectChanged`,
`ProjectForgotten` and `ProjectError` coming back, `GetPreferences`/`SetPreferences` behind
everything the window remembers, and `GetSettings`/`SetSettings` behind how it behaves. A chosen folder reaches the host as a path in `AddProject` or
`LocateProject`; Add also carries the name and colour from project settings. The choosing itself is
the platform's, and crosses nothing. A folder dropped on the window sets `AddProject.temporary`, and
project settings reads that flag off the project it is editing to choose its own button's label,
`Create` for a temporary project's first real save or `Save changes` otherwise. The full family is
[`../tech/transport-contract.md`](../tech/transport-contract.md).

**A clone is its own family, and it ends in the project family.** `ListRepos` and `ListRepoBranches`
fill the modal's two pickers, `CloneRepo` starts the work and `CancelClone` stops it; `Repos`,
`RepoBranches` and `RepoError` come back to the pickers and `ClonePending` and `CloneFailed` to the
modal. There is deliberately no clone-success message: the host registers the project, so the
broadcast `ProjectAdded` is the signal, which is why every window's picker learns about the clone
rather than only the one that asked. The two folders the settings section writes ride the Host
settings blob as `projects_root` and `ephemeral_root`, through the same `SetSettings` every other
host setting uses. The repository family is in the transport contract with the rest.

**The `ubiq` command crosses the bus as a single pair.** `CliShortcut` goes out carrying nothing but
`Query`, `Install` or `Remove`, and `CliShortcutState` comes back to every one of them — where a
shortcut is, whether it launches this build, where one would go, and the directories considered. The
interface names no directory in either direction, because which one the shell would find is a fact
about the machine's `PATH` and the host is the half allowed to look.

Every chat tab speaks the conversation family, the same as a column does, which is
[`chat.md`](./chat.md)'s. The terminals have a family of their own, in
[`panes-and-terminals.md`](./panes-and-terminals.md). The file, work, plan, git and conversation
families are each named by the mode document that speaks them, and all of them in full by
[`../tech/transport-contract.md`](../tech/transport-contract.md).

## The window's areas

Every area is a module, and the window is two kinds of thing: the **chrome**, which the dock is drawn
inside and which the user cannot move, and the **panels**, which are the arrangement. The tables are
the map a change starts from.

The chrome:

| Area | Module | Sits | Size | State |
|---|---|---|---|---|
| Titlebar | `ui/titlebar.rs` | Top, full width beside the mark | `TITLEBAR_HEIGHT`, fixed | `WorkbenchState`, and the dock for the three switches |
| Project picker | `ui/project_menu.rs` | In the titlebar, leftmost | Its own popup width | `WindowRegistry`, process-wide, projecting the host's catalogue |
| Rail | `ui/rail.rs` | Left, full height | `RAIL_WIDTH`, fixed | `WorkbenchState::rail_mode` |
| Status bar | `ui/status_bar.rs` | Bottom, full width | `STATUS_BAR_HEIGHT`, fixed | Read from everything else |

The panels, each one a `PanelKind` in `state/dock.rs`:

| Panel | Module | Class | Opens in | State |
|---|---|---|---|---|
| Explorer | `ui/explorer.rs` | Edge | Left, at `EXPLORER_WIDTH` | `ExplorerState`, one per project the window holds |
| KB explorer | `ui/kb/` | Edge | Left, at `EXPLORER_WIDTH`, in `KB` mode only | `KbState`, one per project the window holds |
| Outline | `ui/outline.rs` | Free | Left, beside the explorer | One `Vec<Def>` cache on `AppState` (`outline`, `outline_key`, `outline_gen`), rebuilt from the buffer on screen and drawn as a `uniform_list` |
| Chat | `ui/chat/` | Free | Right, at `CHAT_WIDTH`, or wherever it is dragged | One `ChatTab` per open instance, in `OpenProject::chats` — see the chat document |
| Centre | `ui/dock/mod.rs`, `centre()` | Centre | The centre | `WorkbenchState::rail_mode`, and whatever the screen it draws owns |
| File | `ui/editor.rs` | Centre | The centre, one per open tab | The `OpenFile` its tab key names, and that file's own `Entity<EditorState>` |
| Terminal | `ui/terminal.rs` | Free | Bottom, at `DOCK_HEIGHT` | The pane it names, and the window's emulator for it |
| Log console | `ui/logs.rs` | Free | Bottom, beside the terminals | `LogState` over the process-wide sink |

What the centre panel draws, and what each of those brings with it. In IDE mode it is only the page
saying no file is open, because the files are panels of their own:

| Area | Module | Sits | Size | State |
|---|---|---|---|---|
| Agents screen | `ui/agents/mod.rs` | The centre panel in Agents mode | Fills it; its header strip takes `TITLEBAR_HEIGHT` | `AgentsView`, over the project's `WorkProjection` |
| Agents sidebar | `ui/agents/sidebar.rs` | The agents screen, left | `AGENT_SIDEBAR_WIDTH`, fixed | The same projection, and `AgentsView::collapsed`, with `AppState::agents_scroll` |
| A column | `ui/agents/column.rs` | The agents screen, one per column in the row | Shares the row and is floored at `COLUMN_MIN_WIDTH` in `state/agents.rs`; the row scrolls sideways | The `Column` it draws, and the window's composer for that column's slot |
| New-column strip | `ui/agents/mod.rs`, `new_column_strip()` | The agents screen, past the last column | `NEW_COLUMN_STRIP`, fixed | `AgentsView::dragging` |
| A conversation | `ui/conversation/mod.rs` | Inside whichever surface hosts one — a column today | Fills what its host gives it | The `Conversation` in `AppState::conversations`, and the `ConversationView` its host passes |
| `[Teams]` screen | `ui/orchestration/mod.rs` | The centre panel in `[Teams]` mode (`RailMode::TeamsOld`) | Fills it; its toolbar takes `TITLEBAR_HEIGHT` | `GraphView`, over the project's `WorkProjection` |
| `[Teams]` graph | `ui/orchestration/graph.rs` | The `[Teams]` screen, beside the inspector | Grows; scrolls to the extent of its cards | `GraphView` and its `Layout` over the same projection, and `CARD_WIDTH`/`CARD_HEIGHT` in `state/layout.rs` |
| `[Teams]` inspector | `ui/orchestration/inspector.rs` | The `[Teams]` screen, right | `INSPECTOR_WIDTH`, fixed | `GraphView::selection`, and `agent_input` on `AppState` |
| `[Teams]` tasks drawer | `ui/orchestration/tasks.rs` | The `[Teams]` screen, under the graph | `TASKS_HEIGHT` open, its header shut | `GraphView::tasks_open` |
| `Teams` screen | `ui/teams/mod.rs` | The centre panel in `Teams` mode (`RailMode::Teams`) | Fills it; its toolbar takes `TITLEBAR_HEIGHT` | `TeamsView`, over `AppState::teams_work`'s narrowed `WorkProjection`, as wide as `AppState::teams_span` says — the active project, or every project the window holds |
| `Teams` graph | `ui/teams/graph.rs` | The whole of the `Teams` screen below its toolbar — there is no inspector beside it | Fills it; scrolls to the extent of its cards | `TeamsView` and its `Layout` over the narrowed projection, whose width `AppState::teams_span` decides |
| `Teams` tasks drawer | `ui/teams/tasks.rs` | The `Teams` screen, under the graph | `TASKS_HEIGHT` open, its header shut | `TeamsView::tasks_open` |
| Tasks board | `ui/board/mod.rs` | The centre panel in Tasks mode | Fills it; the row of columns scrolls sideways and each column scrolls vertically | `BoardState` over the project's `WorkProjection`, and `COLUMN_WIDTH`/`COLUMN_SHUT` |
| Task panel | `ui/board/detail.rs` | The board, right, or a centred modal when `BoardState::popup` is on | `TASK_PANEL_WIDTH`, fixed | `BoardState::selected`, `show_detail`, `editing` and `popup`, and the window's four form entities |
| Kitchen sink | `ui/sink/mod.rs` | The centre panel in Sink mode, project or no project | Fills it; its page strip takes the tab strip's own height | `SinkState`, on the window rather than on a project |
| Sink documents | `ui/sink/docs.rs` | The kitchen sink, on four of its eleven pages | Fills it | The fixture in `state/sink.rs` its page names, and the window's buffer for it |
| A2UI surface | `ui/sink/a2ui.rs`, drawn by `ui/a2ui/` | The kitchen sink, on its tenth page | Fills it; the preview half scrolls, and the model-and-actions pane sits under the editor | The live surface in `state/a2ui/live.rs` — its data model, its per-field entities and its action log — reached through `SinkState::a2ui`, over the window's `a2ui_buffer` |
| Style reference | `ui/sink/style.rs` | The kitchen sink, on its fifth page | Fills it; scrolls | `SinkState`, and the theme itself |
| Picker page | `ui/sink/files.rs` | The kitchen sink, on its sixth page | Fills it; scrolls | `SinkState::picker`, and the fixture tree in `state/sink.rs` |
| Settings | `ui/sink/settings.rs` | The kitchen sink, on its seventh page | Fills it; nav plus a scrolling body | `SinkState::settings`, and the window's settings fields |
| Project settings | `ui/sink/project.rs` | The kitchen sink, on its eighth page | A dialog-shaped panel in the page | `SinkState::project`, and the window's project-name fields |
| Script page | `ui/sink/script.rs` | The kitchen sink, on its last page | Fills it; the settings panel discloses under the chrome, the console scrolls under the two editors, and the right half switches between the reference and the declared panel | `script_buffer` and `script_prelude` on `AppState`, and `SinkState::script` — the last `ScriptOutcome`, the `OxcOptions`, and the `Live` a declared panel draws into |
| New agent form | `ui/new_agent.rs` | A modal over the whole window, above the settings overlay | `MODAL_WIDTH`; its body scrolls inside it | `WorkbenchState::new_agent`, or the settings page's `definition_form` — one `NewAgentForm` either way |
| Add KB source form | `ui/kb/source_form.rs` | A modal over the whole window, above the project settings overlay that raises it | `MODAL_WIDTH`; its body scrolls inside it | `WorkbenchState::kb_source`, one `KbSourceForm` |
| Plan editor | `ui/plan.rs` over `ui/document.rs` | A surface over the whole window, raised from a mission's task panel — **a dialog by decision**, and the one frame of the shared annotation surface that is one | `MINIMAP_WIDTH` + `DOC_WIDTH` + `RAIL_WIDTH` by `DOC_HEIGHT`; the minimap, the document and the thread rail each scroll (or, for the minimap, position marks) inside it, arranged left-to-right by `UiSettings::md_minimap_side` | `WorkbenchState::plan`, one `DocumentEditor`, over the window's `plan_editor` buffer; the minimap's own show/hide is `UiSettings::md_minimap` |
| File picker | `ui/file_picker.rs` | Over the whole window, wherever it was raised | `DEFAULT_WIDTH` by `DEFAULT_HEIGHT`, resized from its corner grip and floored at `MIN_WIDTH`/`MIN_HEIGHT` | `AppState::file_picker`, and the window's `picker_filter` |
| Stats screen | `ui/stats.rs` | The centre panel in Control mode, project or no project | Fills it; its page strip takes the tab strip's own height, and its table scrolls both ways | `StatsState`, on the window rather than on a project |
| KB document | `ui/kb/` | The centre panel in `KB` mode | A flush header naming the document, then the body fills the rest and scrolls | `KbState::doc`, the one document the explorer selected |
| Empty page | `ui/empty.rs` | The centre panel with no project open, and every rail mode with no screen | Fills it | `RailMode`, or nothing at all |

Two rules hold across the three tables. **The chrome does not move and the panels do** — the
titlebar, the rail and the status bar each take one fixed constant and are the frame the dock is
drawn inside, while a region opens at `EXPLORER_WIDTH`, `CHAT_WIDTH` or `DOCK_HEIGHT` and keeps
whatever the user drags it to from then on. And **a screen's furniture is the screen's**: `[Teams]`'s
inspector, and both its and `Teams`'s tasks drawer, the agents screen's sidebar and the board's
task panel take one fixed constant each, are shown and
hidden from the screen they belong to rather than from the titlebar's switches, and leave with the
mode.

To add a panel: a `PanelKind` variant with its class, its home, its permanent name and its rule in
`is_drawn()`, an arm in `ui::dock::body`, and the area module under `ui/`. One that a saved
arrangement cannot rebuild from its name alone — as a file cannot, every file panel answering
`ubiq.file` — also writes a payload in `dump()` and is read back out of it in `rebuild()`. To add a
rail mode: **one registration**, not a variant (`D184`). `RailMode` is a `SlotId` newtype over the
rail container, and a mode is a `RailModeSpec` handed to `ext::rail::register` — its group, label,
note, slug, icon and `UiId`, its `Availability`, whether it needs a project, which side regions a
first visit opens, and `fn` pointers for the centre screen, the default layout, the destination,
the furniture and the on-arrival ask. What used to be "an arm in `ui::dock`'s `centre()`, and for a
mode under `APP` an arm that answers before the no-project case" is now two fields, `centre` and
`needs_project`. The base's own ten are registered in `ui::rail::modes`; a second edition's are
registered on `Boot.contributions.rail_modes`, which arrives seeded with the base's.

## What a window owns

A window is one `AppState`, and inside it one `OpenProject` per project open in that window. The
split between them is the feature's spine. **A project owns what is about that project** — its
explorer tree, its open files and their buffers, its panes, which of them holds the keyboard, the
furniture it was last left in, the work the host last described to it, the repository as the host
last described it, its chat tabs, and the graph's, the columns', the board's and the Git screen's own
views of those. **The window owns what is about the window** — the dock and one
panel per kind in it, the palette, the log console, and one flat map from pane id to
emulator, because an emulator does not care which list draws it.

The task panel's four fields, the board's filter, the Git screen's search and commit box and the
explorer's filter sit across that line: the entities are the
**window's**, because a text field is a component the window builds and there is one of each, while
the text in them is the **project's**, because it is about the task that project has open. So a
project switch refills them from whichever project came forward — the explorer's filter included,
which is how a window brought back to a project finds that project's own query in it rather than
the last one typed anywhere.

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

`AppState` in `crates/ubiq/src/app/mod.rs` is the root view. It owns the window's own state — the dock
and the panels in it, a typed-over name and a pin for any non-file tab (`tab_names`, `pinned_tabs`
— a file's own name and pin live on the `OpenFile` instead, since they are the tab keys that
persist), the console, the emulators, the component library's `TextareaState`
and `InputState` entities and the subscriptions that keep them mirrored — and a map of `OpenProject`
keyed by `ProjectId` holding everything that belongs to a project, chat tabs among them. Every
mutator ends in
`cx.notify()`, with one gate on the way: a streamed conversation chunk asks for a frame only when
some surface shows that conversation, and only once per frame — the rule *UI and design* owns.

`sync_projects()` is the one place the map is reconciled against the registry, and it is idempotent:
it drops the projects the window no longer holds through `drop_project()`, builds an `OpenProject` for
each new one, and calls `enter_project()` when the active project changed. It runs from the
`observe_global` subscription rather than from each call site, so a project taken by *another* window
reaches this one down the same path as a local change. `enter_project()` is where a project's
furniture reaches the window — the rail mode it was left in and that mode's arrangement — and it
starts no pane, which is why a window opening on nothing starts no harness. It also sets
`reset_furniture`, consumed at the top of `settle_layout()`: the two panels that are the window's
own rather than any project's — `Logs` and `Search` — are taken out of the tree and `SearchState`
is reset before the incoming arrangement installs, and `collapse_empty_regions()` puts away the
region they just vacated when the entering project has no saved layout to fill it back in.

Accessors read through the active project and tolerate its absence: `open_project()`, `explorer()`,
`editor()`, `work()`, `agents()`, `graph()`, `board()`, `panes()` and `focused_pane()` each answer
for a window with no project without a caller having to check, and `work_mut()`, `agents_mut()`,
`graph_mut()` and `board_mut()` are the writing twins of the four over the work. `drop_project()` writes the project's blob, parks a copy against a
reopen in the same session, kills its panes and unloads every conversation whose harness is up. The content family's live text
size is `theme::content_base()`, a trim over `theme::TEXT_BASE` held in `InterfacePrefs`, not the
project; `nudge_content_trim()` is how it changes, reconfiguring every already-open emulator in
every project the window holds through `redress_terminals()`, debounced behind `settle_metrics`, so
a zoom reaches panes that are on screen with no project open required. `toggle_editor_wrap()` flips a project's wrap and brings every open buffer into line, and
`remember()` writes the explorer's filter down with the rest of the view prefs, alongside a
project's untitled buffers as `prefs::Scratch` entries — by display name and the buffer's own text,
an untitled image capture dropped rather than carried — and its pinned files' tab keys;
`restore_files()` replays each scratch entry through `push_untitled()`, the same machinery `⌘N`
uses, before reapplying `active_file`. `remember()` also writes the chat tabs, as what each was
attached to, and **only where the attachment is a persistent conversation** — the host keeps no
other, so remembering one would revive a tab over an agent that is gone — the rule is
`features/chat.md`'s. It reads that mark off the project's work projection, so while
`work.agents` is empty it leaves `prefs.chats` untouched rather than writing an empty list: a
project is opened by asking for its preferences and its work in two independent round trips, and a
`remember()` landing between them would read the missing projection as "nothing was persistent" and
throw away a tab the blob was right about — the same guard `settle_persistent_chat` makes. The tab menu's handlers are `open_tab_menu()`,
`pick_tab_menu()`, the four `close_editor_tabs*` helpers — each of which skips a pinned file
outright, at the one place every bulk close routes through — `save_file()`, `open_rename_tab()`,
`toggle_tab_pin()` and `dismiss_tab_menu()`. `ui::tab_menu::rows(kind, pinned)` is read by both the
frame that draws the menu and the pick that resolves it, so Pin/Unpin and a suppressed Close row
never disagree about what row an index means.

`open_project_window` in `crates/ubiq/src/app/mod.rs` is the only place a window is created, so the
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
state. `groups` computes the picker's three lists for one window, and `window_count` is what
`AppState::several_windows()` asks before drawing a letter anywhere. Every `AppState` subscribes with `observe_global`, so a move in one window
redraws the picker in all of them, and reads go through `WindowRegistry::read` rather than
`default_global`, which would notify the observers on a plain read and spin the frame. The registry
is pure logic and is tested without a frame in `crates/ubiq/tests/windows.rs`, which seeds it the
way the host does.

`state/when.rs` renders a row's relative time at draw time from `last_opened_at`, and
`state/prefs.rs` is the schema inside the opaque blob the host stores — one `ModeLayout` per rail
mode in the `ViewPrefs::modes` map, each carrying that mode's region flags and a dock blob of its
own, beside the files and folders a project reopens with, its untitled buffers (`scratch:
Vec<Scratch>`, a display name and the buffer's text) and pinned files (`pinned_files`), the point
size field kept for a downgrade's sake (`content_font_size`, parsed and ignored — it is where the
`4 -> 5` migration takes `InterfacePrefs.content_trim` from, the value of the most recently opened
project), whether its editors wrap (`editor_wrap`) and the text in
its explorer's filter (`file_filter`) — each new field `#[serde(default)]`, so a field costs the
schema nothing, `scratch` and `pinned_files` included. `ModeLayout::default_for` is what a mode with
no entry opens on: every region flag `false`, in every mode, because no region is furniture — `D94`.
**Nothing overrides that.** A right region the user left closed stays closed until the user's own
click reopens it. `AppState::settle_persistent_chat` (see the chat document) runs after a project's
layout settles and, if that project holds a persistent agent, attaches its seed chat tab to that
agent — but it queues the panel as `PanelEdit::Open`, which joins the group in the right region
without touching whether that region is on screen, so the tab is waiting there whenever the user
does open it rather than opening it for them.
The number is `3`, because one value in the blob carries a meaning that moves with the build: a
`rail_mode` of `Agents` names the column screen, and an older blob wrote it for the graph. That is
the one case a default cannot rescue — nothing is missing, and the value means something else — so
the blob is discarded whole, and `ui::dock::LAYOUT_VERSION` follows the number so the arrangement
inside it goes with it.

`crates/ubiq/src/ui/shell.rs` assembles the frame and nothing more: the mark and the titlebar in one
row, then the rail beside `AppState::dock()`, then the status bar, and — when one is up — the
project-settings overlay, the application-settings overlay, the login modal, or one of the window's
own menus over all of it: the tab menu, the new-pane menu, the new-agent menu and the titlebar's
overflow menu. Those are painted here rather than by the surface that opened them, because more than one surface
opens them and what there is to offer is the window's answer, not a page's — the new-agent menu is
`ui::agents::new_agent_menu`, raised by the agents screen's `+`, the IDE chat strip's and the
kitchen sink's bench alike. The New agent modal is painted here too, on the same terms and above
the settings overlay: it is opened from the workbench, and a question left under an open page is a
question the user cannot see. The mark is drawn by `rail::mark`
in that first row so it sits in the corner above the rail rather than inside it. It fixes no
arrangement — everything between the chrome is the dock's.

`ui/dock/` is the adapter. `state/dock.rs` is the policy over the tree and draws nothing:
`PanelKind` names a panel — a pane id for a terminal, a tab key for a file, a knowledge-base
document's own tab key for a `Kb`, nothing at all for the rest — `class()` says which regions it may
sit in, `home()` where it opens and where one put back goes, `name()` is the permanent key a saved
layout is rebuilt from, `tab_key()` is the tab a file panel draws and `kb_key()` the tab a `Kb`
document panel draws — kept apart because every caller of `tab_key()` means "one of the project's
open files" and would find nothing among the documents — `is_drawn()` is the hidden-not-removed
rule, and `closable()` says whose tab offers a close. `is_drawn()` is asked against one `Visibility` — everything the window knows about itself
that a panel could care about — so a new rule is a field on that struct rather than another argument
threaded through the dock. All of it is pure logic and is tested without a frame in
`crates/ubiq/tests/dock.rs`, which is also what makes the cost of renaming a panel visible at the
moment somebody edits the string. `ui/dock/mod.rs` holds `WorkbenchPanel` — a `PanelKind`, a weak
`AppState` handle and a focus handle, and nothing else — whose render delegates through `body()` to
the area functions that already exist, so adding a panel is an arm of a `match` rather than a new
owner of state. `default_layout()` is the arrangement a window opens in — the IDE tree, or Git's refs / history /
changes / diff when that mode is what is being rebuilt — `add()` and `remove()` are
how a terminal's and a file's panel join and leave, `holds()` is what stops a panel a restored
arrangement already carries being added a second time, `enforce_placement()` puts back a panel
dropped where its class forbids, and `restore()` rebuilds a saved arrangement or answers that it
could not. `dump()` writes a file panel's payload through `file_payload()` and `rebuild()` reads it
back through `file_from_payload()`, which is the one pair that keeps the shape on disk in one place;
a `Kb` document writes and reads the same payload under its own panel name, so `rebuild()` tells the
two apart by name before handing the key back as a `PanelKind::Kb` rather than a `PanelKind::File`.
`ui/dock/skin.rs` implements the component library's
three renderer traits and draws every pixel the library would otherwise style: the tab strip at the
same height as `ui::kit::tab_strip`, the active tab marked on its bottom edge, the dot beside a tab,
the close wherever the panel allows one — through `ui::dock::close_panel()` rather than the group,
because the component library refuses to give up a region's last panel and Ubiq collapses the
emptied region instead — the drop indicator and the strips that resize a region —
and, at the right end of a tab strip whose group holds a terminal or the console, the `+` that asks
the host for a new pane and the chevron that asks for the menu of what else it could reach. The
control is the `NewPane` value `AppState::for_project` hands the skin: one closure per half, plus a
third — `AppState::is_pane_region()` — that answers whether a group is the pane region's, because a
group says which node it is and nothing about where it sits, and the control has to stay on the
strip of a region the user has emptied. The `+` is drawn only while that window holds a project. The
chevron crosses the same seam a tab's right-click does, and for the same reason: a tab's right-click
crosses it through the `with_tab_menu` builder and its `TabMenuRun` type, in
`crates/ubiq/src/ui/tab_menu.rs` — the one module every kind of tab's menu is painted from. The
skin cannot name `AppState`, so the tab's `PanelKind` and the click's point are handed across rather
than a file's key alone, and the window paints the menu over the dock. Ubiq writes no drag, no drop
geometry and no layout serialisation.

`AppState` holds the dock's half of that. `dock()` hands it to the shell; `regions_open()` and
`toggle_region()` are the titlebar's three switches, both going through the dock rather than a flag
beside it; `panel()` builds a kind's panel the first time it is asked for. Opening an empty region
is the gesture that fills it: the bottom starts a pane; the right in IDE opens a fresh chat tab and
in Git the changes panel; the left in Git opens the refs explorer. An empty left in IDE is the
exception, since the explorer panel is in the tree the moment IDE mode is entered, so opening a
closed left reveals it rather than spawning anything. The titlebar offers the left and right
switches in IDE and in Git (`WorkbenchState::has_side_panels`). `new_terminal()` is the titlebar's
own New terminal shortcut: it calls `toggle_region()` when the bottom is shut, and spawns a pane
itself only when the region was already open or was reopened onto panes still in it, so opening
onto true emptiness is never given two panes by two different callers. `reveal_pane_region()` is
the runner's version of the same errand and deliberately not `toggle_region()`: the pane it is
clearing the way for is what fills the region, so opening it must not spawn a shell beside that
pane. It switches to the IDE when the mode has no pane region (`RailMode::has_pane_region`), then
*queues* the open — `settle_pane_region()` answers it at the end of the frame, after
`settle_mode()` has forced the incoming mode's own regions and after `hide_emptied_regions()` has
had its say, because both would otherwise shut the region again in the same frame. A chevron beside it opens
the same new-pane menu the terminal `+` offers, with its shells and runnable tools. `open_new_agent_direct()` is
the titlebar's New agent shortcut, making the same `aim_start()` call `pick_new_agent_menu()` makes
for the `+` menu's first row, with that menu's own first stage skipped. `open_teams_add_agent()` is
the Teams toolbar's, and the one that can aim somewhere other than the active project:
`pick_teams_add_agent()` answers its `MenuId::TeamsAddAgent` menu by index into
`window_projects()`, and `start_teams_agent()` writes the chosen project into
`AppState::new_agent_project` *after* `aim_start()` has run, because `aim_start()` begins by
clearing the aim and the project is part of it. `take_startable_new_agent()` reads that override in
place of `project(cx)`, filtered against the projects the window still holds — a project let go
between the form going up and the user confirming it falls back to the active one rather than
sending a `StartConversation` with nowhere to land — and spends it at send time, so the next form
does not inherit it. A panel reaches the dock
through a `Window` and a message does not come with one, so both halves of a panel's life queue and
are drained in `render`: `settle_visibility()` builds each panel's `Visibility` and pushes
`is_drawn()` into it, along with the layout a file panel writes into its payload — pushed rather
than read back, because the dock asks both while `AppState` is mid-update — then `settle_layout()`
rebuilds a saved arrangement and puts the layouts it carried back on the files, then
`settle_panels()` drains the `PanelEdit` queue. `sync_file_panels()` squares the dock's file panels
with the incoming project's open files when the window changes which project it is pointed at,
because the files are a project's and the panels are the window's — and does the same for the
knowledge base's open documents (`KbState::docs`) in the same pass, on the same reasoning: `Kb`
panels for a project just left are queued to close, and one for each of the incoming project's
already-open documents is queued to open. That is the same
device the pending focus and the arrived files already use. A `DockEvent::LayoutChanged`
subscription enforces placement, then `hide_emptied_regions()` closes an edge region the change just
emptied — its last panel closed, or dragged to another region — before the layout is written down,
so what is remembered is what is on screen. It tells that apart from a region a caller just opened
empty on purpose (the left, above) through `region_had_content`: the flag it ticks per region on
every change, so the auto-hide only fires on the edge from holding something to holding nothing, not
on an emptiness that was already there. A restore is the one place that edge is armed by hand:
`settle_layout()` ends in `collapse_empty_regions()`, because the arrangement a project switch
installed is final and an open region it left empty is one the project on screen has no use for —
except a region the pending-panel queue is about to fill, which `collapse_empty_regions()` reads
off `pending_panels` and leaves unarmed, so a region a queued panel opens onto is never closed
before that panel lands. `layout_blob()` is what the subscription writes. A rail-mode switch is the same queue:
`set_rail_mode()` writes the outgoing mode's arrangement down through `remember_view()`, then hands
the incoming mode's saved blob to `pending_layout`, or — for a mode never arranged — queues its
`ModeLayout::default_for` flags as `pending_regions`, which `settle_mode()` forces on the frame. A
start is the same thing said by the host: the window opens on the IDE's default tree and learns
which mode the project was left in only when `Preferences` arrives, so `apply_preferences()` hands
that mode's blob to `pending_layout` and, for a mode never arranged, its `default_for` flags to
`pending_regions` and Git's own panels to `queue_git_furniture()` — a project left in Git opens on
Git's screen, side panels and all, rather than wearing the IDE's regions.

**The titlebar's overflow chevron is one more menu on the same `MenuId` device.**
`WorkbenchState::overflow_menu` holds the point it opened at, exactly as the new-pane and tab menus
hold theirs, and `WorkbenchState::overflow_rows(has_project, capture_offered)` builds the row list a
project and a capture backend narrow: remote host, then web export and capture when a project is
open (capture only when the platform offers it too), then settings always. `AppState::open_overflow_menu`,
`pick_overflow_menu` and `dismiss_overflow_menu` are the same three-verb shape every menu in the
window follows, and `crate::ui::overflow_menu` paints it from the window root beside the tab menu
and the new-pane menu, for the same reason: the titlebar draws the chevron but does not own what the
menu offers.

The rest is one module per area: `rail.rs`, `titlebar.rs`, `project_menu.rs`, `status_bar.rs`,
`size.rs`, `explorer.rs`, `editor.rs`, `terminal.rs`, `empty.rs`, `chat/`, `agents/`, `orchestration/`, `teams/`
and `board/`, with `work.rs` beside them for the one thing all four of the last draw. The project picker is
its own module rather than a `Picker`, because a project row carries actions and a confirmation and
is not just a value. The clone modal is `ui/clone.rs` on `kit::modal_sized`, over `state/clone.rs`
(`CloneState`, `CloneMode`, and the notes `stage_note()` and `clone_error_note()` turn a stage or an
error into a line) with `app/clone.rs` carrying the traffic and linking `receive_repo()` into the
`app/wire.rs` chain. It reuses the kit and nothing else: `kit::menu::Picker` for the connection and
for the branch, `kit::field` for the filter and the URL, `kit::check_box` for ephemeral and shallow,
and `app/projects.rs`'s own `choose_folder()` for the destination. `state/navigator.rs`'s `rows()`
short-circuits on `parse_repo_url` the way it already does on a `ubiq://` link, into a `Group::Clone`
of one row; `NavRow` carries an optional `action` that `app/nav.rs`'s `press_navigator()` checks
before it looks a destination up. `AppState::project_holds` takes `cx` and its `Holds` carries
`ephemeral`, which is what raises the discard confirmation on the picker's row, and `agents`, which
is every conversation `Conversation::running` answers yes for — a live harness, not an unloaded or
ended transcript. That ephemeral flag is read off `ProjectSnapshot::ephemeral` rather than worked out
from the record: the host answers with the same test it deletes by, so the warning and the deletion
cannot disagree.
The indexing level is `ui/settings.rs`'s `index_level_choice()` for the application-wide default
and `ui/sink/project.rs`'s `index_row()` for a project's override, both drawn with `choice_pill`;
`app/settings.rs::set_index_level` and `app/projects.rs::set_project_index` are the traffic, the
second sending an `UpdateProject` carrying only an `IndexChange`. The host resolves the two into
one answer in `coordinator.rs::index_level` and acts on it in `settle_index`, which is called
whenever either could have moved. The index itself is `crates/ubiq-host/src/index/`: one thread
owning one open index per project, fed by the same walk content search runs and by the watcher's
own batches, handing out a read handle that carries no writer. It lives in a host-owned `index/`
directory beside the interface's `ui/` workarea, which Forget and the orphan collector already
remove.

Project settings' General page carries the rail's badge override: a three-character field,
`AppState::project_initials_input`, clamped as it is typed rather than only on Save. Empty is "no
override" and leaves the badge on the name's own first letter, the way it always read; a value
wins outright, in both the header's own preview mark and — once saved — the rail's badge in
`ui/rail.rs`'s `project_badges()`. `fill_project_form` prefills it from `ProjectRecord::initials`
on open; `commit_project_settings` sends it as `Message::SetProjectInitials` alongside the
`UpdateProject` for an existing record. A project still being created has no record yet for an
override to belong to, so the field is not prefilled and nothing is sent for it until the project
exists and is edited again.

The same dialog's own search excludes are `ui/sink/project.rs`'s `search_excludes_row()`: one
removable row per pattern, an `Add folder…` button that raises the native chooser and relativizes
the chosen folder against the project's own root (a folder outside it, or the root itself, is
silently dropped), and a field that commits a typed gitignore-style pattern — `*.log`, `**/build`
— on Enter. Every add or remove, and the explorer's own `Exclude from search` / `Add to search`
menu picks, all go through `app/projects.rs::set_project_search_excludes`, which sends the whole
list in one `UpdateProject` and applies the snapshot at once rather than waiting on the host's
echo.

**The dialog is also where a project says which repositories inside it are its business.**
`repos_row()` draws a Repositories section under the excludes: the project's own repository first,
always managed and with no switch beside it — it is the repository the project *is* — and then one
row per repository the last working-tree walk found inside it, each with a tick box and a
`submodule` tag where the outer repository pins it. A nested repository starts **ignored** (`D112`):
ticking it sends the whole list through `app/projects.rs::set_project_managed_repos`, the same
immediacy the excludes get, and the host redoes the observation at once so the tree and the Git
screen follow without a restart. The rows come from the window's own `OpenProject::git_repos` — what
a walk found, not what the shared record says — so the section tells a project this window holds
without showing that it has to be opened before it can be listed.

An ignored repository is not drawn faintly or filtered late: the host never opens it, so it has no
status to carry. Its folder gets no branch chip, no badge of its own, and not even the untracked
mark the outer repository would otherwise give it, because the folder of every repository found is
dropped from the outer repository's account either way. `ExplorerState::apply_git` takes only the
managed ones into `git_repos`, which is the boundary it draws a branch on and the place inherited
status stops.

Project settings is `ui/sink/project.rs`: the sink draws it on the page, the
shell paints the same dialog over the window when a project is being created or edited. Application
settings is `ui/settings.rs`: the titlebar's gear raises it, `state/settings.rs` holds the overlay
and the Ui-layer schema, and a toggle sends `SetSettings`. Its Command line section is
`ui/settings.rs`, `command_line()` — the row and its button — over `candidate_list()`, the
directories the host considered; `app/settings.rs` is the traffic, `ask_cli_shortcut()` out and
`apply_cli_shortcut()` back, with `set_settings_nav()` asking a `Query` whenever the section is
opened. `SettingsState::cli` in `state/settings.rs` holds the answer, and it is `None` until one
arrives, which is what lets the section say it is looking rather than say there is no shortcut. Its
Assistance section is `assist()` over `assist_provider_choice()`, and it reads the same way for the
same reason: `SettingsState::assist` is `None` until `Assist` answers, so the chip says it is
checking rather than say there is no backend. The choice is a row of `assist_pill()`s — Off,
On-device, then one for each provider configured below — and the On-device pill is the only one
`AssistInfo::switchable()` gates, dimmed and inert unless the backend is available or off by the
user's own choice. It is the only one, because a machine whose local model is not there must still
be able to switch *away* from it: gating the row would strand a user on a backend that cannot
answer. One `setting_row` follows it — `Name conversations`, a `check_box` on
`AppState::toggle_auto_name_conversations`, which writes the Host layer like the provider row above
it because the host is what does the reading. It is dimmed and inert while `assist` is `Off`, since
a naming runs through that provider and nothing else, and the control that would make it run is
the row directly above, so nothing is trapped by it. No second provider picker: a naming is the
fast model's work at whichever backend is selected. The sentence under the row is the host's own, and
the interface authors no sentence about a platform. The
half that knows anything about a path is the host's `crates/ubiq-host/src/cli_shortcut.rs`:
`handle()` takes the action, `candidates()` and `install_dir()` decide where, `script()` writes the
launcher, `marked_target()` reads the marker line, and `state()` is the answer all three actions
share. Its tests cover the three things that would be silent if they broke — the script is written
marked and executable, a bundle is launched through `open -a` where a bare binary is exec'd, and
remove takes Ubiq's own file and leaves a foreign `ubiq` alone. Shared primitives are in `ui/kit/`;
the conventions behind that split are `tech/ui-and-design.md`'s.

The providers under the Assistance row are `ai_providers()`, one `ai_provider_row()` each: the name
and the kind, a badge saying whether a key is filed and another saying no model is chosen, and Test,
Edit and Remove. `ai_form()` is both the add and the edit — a `kit::modal` rather than a
`prompt_modal`, because it asks several questions — and its boxes are `form_field()`, `app_form()`'s
own field shape lifted into a free function because a provider's fields are built in two places and
a closure holding the render context cannot live across both. An add asks for kind, name, endpoint and key
alone. Writing the record is what files the key and makes the host list that provider's models, so
an add has no model question to ask and needs none: by the time Edit opens there is a list to choose
from. Each of the two model rows is a `model_row()` — a box, then a `model_picker()` over what the
provider answered, then one Refresh, the only place `ListAiModels { refresh: true }` is sent. **The
box is the field and the picker fills it**, rather than the picker being the only way in, because a
provider that will not list its models must still be usable: Azure OpenAI addresses deployments
rather than models, and a listing can fail at a proxy or on a key scoped to inference alone. An
empty smart box means the fast model, which is what its picker's first row writes. The key box is empty every time the form opens, because
the host never sends a key back and this half has none to draw; on an edit that same blank box is
the only way to say keep the one that is filed. A provider with a key and no model is a real record
in between, and its row is where that reads.

`ai_test()` is the reason a suggestion streams at all. It sends `SuggestSubject::ProviderCheck` for
the fast or the smart model — `test_role_pill()` picks which, the Smart pill inert where no smart
model is configured — and appends each `SuggestChunk` to the answer it draws, so a user who has
just typed a key watches a first token arrive instead of a spinner, which is the whole of what the
modal is for. The `Suggestion` that ends the id replaces the accumulated text with the host's
trimmed whole; closing the modal or pressing Rerun sends `CancelSuggest` for the id in flight, so a
provider nobody is watching stops being read. `ai_remove()` is a `confirm_modal` on the danger edge,
because the record's key goes with it. `ui/shell.rs` paints the three in that order — form, test,
removal question — and `app/shell.rs::cancel_dialog` peels them the other way up, because the
question raised last is the one on top.

`app/settings.rs` carries that traffic: `set_assist_provider()`, the
`open_ai_form()`/`save_ai_form()`/`close_ai_form()` group, `refresh_ai_models()` and `run_ai_test()`
out, and `receive_assist()` in `app/wire.rs` back, which drops any suggestion whose id is not the
one the interface minted before it asked. `GetAssist` goes once on attach and again whenever the
provider changes, because availability is asked and never inferred (`D84`); `GetAiProviders` goes on
every arrival at the section, so a provider written from another window is drawn without a restart.
A refusal is an `AiProviderError` in the section's own banner rather than a dialog of its own, in
the same shape the Harnesses section's refusals take, because nothing was changed on the way to
failing.

Vim mode is split the way the rest of the window is not: a pure command set in `state/vim/`
(`step.rs` for the dispatch, `motion.rs`, `object.rs` and `search.rs` under it) that takes a `&str`
and a byte range and answers with a list of `Effect`s, and one driver in `app/vim.rs` that connects
it to a real input. The engine names no gpui at all, which is what lets `tests/vim.rs` drive the
whole command set with plain strings and no window. The driver registers
`App::intercept_keystrokes` — not `observe_keystrokes`, which fires after dispatch and cannot
consume anything — resolves the focused input through the component's `WindowExt::focused_input`,
and applies the effects through `set_selected_range` and `replace`. Undo and redo are the exception:
the component's undo stack is not reachable, so those two go back out as its own actions.

State types live under `crates/ubiq/src/state/`: `workbench.rs` for the rail mode, the open menu, the
project settings dialog, the application settings overlay, what was typed into the picker's and the
explorer's filters, and the menus that came later — `MenuId::Size` for the status bar's size
popover, `WorkbenchState::size_presets` for the pill row it and the Size settings section share and
`WorkbenchState::size_prompt` for the `SizePrompt::Save` / `Rename { name }` naming question both
raise, `MenuId::ViewerKind` for the file-kind chip beside it, `MenuId::Tab` with the tab's `PanelKind` and anchor in
`WorkbenchState::tab_menu` for a tab's right-click, `MenuId::NewPane` with its anchor
in `WorkbenchState::new_pane_menu` and its rows in `WorkbenchState::shells` for the new-pane
control's chevron, and `MenuId::Overflow` with its anchor in `WorkbenchState::overflow_menu` and its
rows in `WorkbenchState::overflow_rows` for the titlebar's own chevron, and `MenuId::Kb` with its row
in `KbState::menu` for the KB explorer's right-click — its own id rather than `MenuId::Explorer`
reused, because both panels can be on screen and only one menu is open; `settings.rs` for the Ui-layer
schema, the overlay's nav, and how a blob is read;
`explorer.rs` for the tree, the list, the keyboard and the right-click menu, drawing through the
shared chrome in `ui/kit/files.rs`; `editor.rs`
for the open files; `logs.rs` for the console's filter; `work.rs` for one project's work as the host
describes it; `agents.rs` for the columns' view of that work, `orchestration.rs` for `[Teams]`'s
graph and `teams.rs` for `Teams`'s own, independent copy of it; `board.rs` for the board's.

`state/work.rs` is the projection and nothing else: the sessions, agents and tasks of one project as
the window last heard the host describe them, and a flag saying whether it has heard at all. The
records are `ubiq_proto::work`'s own, carried across the bus rather than rebuilt beside it.
`replace_all()` takes a `WorkList` whole; `apply_task()` and `apply_agent()` replace on id and answer
whether the record was new, which is what tells the arrangement there is something to find a place
for; `forget_task()` answers whether there was anything to drop. Replacing on id is the property the
whole family rests on — a projection that appended on a re-send is the duplicate-card bug. The
questions the two screens ask of the records live here too, because the host has no use for them:
`members()` is the agents serving a task, `now()` picks the one it speaks through, `pulse()` reduces
everything happening in a task to the state its card's edge carries, and `count()` is what the status
bar counts by bucket. The free `fraction()` and `tokens_label()` are two values a card prints, worked
out at draw time for the reason `state/when.rs` renders how long ago something was rather than
storing it.

`state/file_picker.rs` is the picker itself, and nothing in it reads a disk. `PickerRequest` is the
whole of what a caller says — owner, title, root, prefilter, kind, count, commit and modality — and
`FilePickerState::open` roots the forest it was handed at the requested folder, opens the top of it
and holds the rest: the view, the filter, which folders are open, what has been picked in pick order,
and the size the corner drag has put it at. `rows()` is the only thing the screen reads, and it
arranges the same set two ways. The forest is handed in rather than fetched — `PickerNode` is the
shape a `DirListing` becomes — which is what lets the sink raise a picker with no project open, and
it can also grow after the picker is open: `set_forest` replaces the whole top level on a re-root,
and `fill_node` gives one folder real children once an answer for it lands. `PickerNode::dir_unfetched`
is the placeholder a folder holds until then — `needs_load()` tells it apart from one already
listed, and `expanded_needing_load()` is what a caller asks to know which open folders still owe a
listing. `readable`, `hidden` and `truncated` ride on every node for the same reason `size` does:
an entry a source could not open draws dim with no twisty and does not accept a click,
`show_hidden`/`set_show_hidden` is the toggle that keeps a dotfile like `.config` reachable while
keeping one off by default, and a folder's own truncation is a row's mark rather than a silent
under-count. `crate::app::host_browse` is the one caller that fills a forest this way today, over a
remote host's filesystem before any project exists on it — see the Behaviour section above and the
host browse family (`tech/transport-contract.md`) — walking the tree straight from an already-loaded
explorer's `forest_from_explorer` still needs none of it. A child's path joins with the separator
its parent uses, since the host runs Windows or Unix independently of this window; the host strips
the verbatim prefix before a path ever reaches the wire (`crates/ubiq-host/src/host_path.rs`).

`AppState` holds `file_picker: Option<FilePickerState>`, `picker_filter` and `picker_scroll`, because
exactly one dialog may be up per window and the field above its rows is one of the window's like every
other. `open_file_picker` empties that field, raises the dialog and gives the field the keyboard;
`press_picker_key` hands one key to `FilePickerState::press` and acts on the `Pressed` that comes
back — scrolling the cursor into view, committing, dismissing, or answering false so the key goes on
to whoever else wants it. `ui::file_picker::key_bindings` is where the keystrokes are named, and it
is called from `install_key_bindings` **after** `gpui_component::init`: the library's input binds the
arrows, `enter` and `escape` for itself at the deepest node in the tree, so each key is bound twice —
once for the dialog, once for the field inside it — and the second predicate wins the tie by being
registered later.

`click_picker_row` asks the picker what the click meant and commits on the spot when the request said
a single pick is final on it; `commit_file_picker` and `cancel_file_picker` take the dialog down and
route the answer by `PickerRequest::owner` — the sink's page, the composer's attachments (taken as
`picked_with_sizes()`, so a tag knows how big its file is without anything reading a disk), and
`PickerOwner::HostProject`, which sends `AddProject` to the host `AppState::host_browse` names
rather than writing back into any field of the picker's own, and `PickerOwner::KbFolder`, which
folds the chosen folder into the "Add source" form instead of sending anything — a source is
committed when that form is confirmed, as one whole-list `SetKbSources`. `ui/file_picker.rs` draws it, painted
from `ui/sink/mod.rs` for the same reason the modal is: where a dialog is asked for is not where it
is painted. `crates/ubiq/tests/file_picker.rs` asserts every rule above with no frame at all, over
the sink's own fixture tree.

A row keyed by a ULID takes its element id from `ui::eid`, or `ui::eid2` for a row two ids deep like
a step inside a task, because a ULID is not a `u64` and the tuple form the rest of the window uses
cannot carry one. That convention is
[`../tech/ui-and-design.md`](../tech/ui-and-design.md)'s.

`state/nav.rs` is the value and everything pure about it: `Destination`, `View`, `Locus`,
`same_place()` which compares everything but the locus, `persistable()` which is false only for a
chat, `label()`, the `History` stack with `record()`, `back()`, `forward()` and `peek()` over a
`Fate` predicate the caller supplies, the `Bookmark` record — serialised with its destination as
text, so what keeps an old bookmark readable is the parser rather than a set of serde variant names,
and one that no longer parses is dropped alone rather than costing the blob — and `resolve_anchor()`.
`state/nav/text.rs` is the `ubiq://` form: `Display`, `FromStr`, and `resolve_relative()` for a link
written inside a document, where `..` pops because a document holds paths a human wrote while the
host only ever holds paths it handed out. `app/nav.rs` is the router. `current_destination()` matches
the rail mode, reads the screen's own selection and asks `where_locus()`; `navigate()` resolves the
project — handing the whole destination to another window's `AppState` and raising that window when
it is the one holding the project — sets the rail mode, calls one `reveal_*`, and keeps the
project's recents, which is why they are kept in exactly one place. `settle_nav()` runs once a frame
from `Render` beside `settle_board()` and is the single push site, with no `cx.notify()` of its own
or the frame spins. `reveal_ide()` brings a tab forward, turns a markdown tab's layout from
Preview to Source when the locus names a `Line` or a `Span` — an `Anchor` is left alone, since a
heading slug is what the preview itself draws — and drives the buffer's `set_selected_range` — the
same call every vim motion scrolls its caret in with — stashing `pending_goto` for a file whose
bytes have not landed. `app/editor.rs` applies that goto once the contents arrive, after any
reload restore rather than folded into it, so the asked-for line wins the caret instead of being
scrolled back by the restore. `mark_bookmarks()` runs on the same arrival,
re-stamps what moved and lights what it found through one `TextDecorationCollection` per buffer.
`open_task_chat()` in `app/board.rs` is a `navigate()` call.
`ViewPrefs` carries `bookmarks` and `recents`, both defaulted and written through `store_prefs()`;
`graph_scroll` on `AppState` is the tracked handle a `Viewport` locus reads and writes.
`state/navigator.rs` builds the rows in a free `rows()` naming neither `AppState` nor a window, and
`ui/navigator.rs` draws them `deferred` and `anchored` off the titlebar's field, with the key
context and every handler on the **field**, because the keyboard is in the input inside it and a
deferred panel is nowhere on the focus path. The list hangs from a zero-sized absolute box at the
field's bottom-left corner rather than from wherever the layout put the deferred child: `anchored`
reads its position from that, so the list drops just under the field with its left edge on the
field's, instead of landing in the middle of the centred row and covering the text being typed. `ui::on_link()` is the one handler
`ui/viewer/markdown.rs`, `ui/conversation/mod.rs` and `ui/board/form.rs` each hand to their
`TextView`. `crates/ubiq/tests/nav.rs`, `tests/nav_text.rs`, `tests/bookmarks.rs` and
`tests/navigator.rs` assert all of it without a frame.

## Failure

| What happens | Result |
|---|---|
| A filter matches nothing | The panel says nothing matches; the filter field keeps what was typed. Hits still filling in the background appear as their listings land |
| Every edge region is closed | The rail, titlebar and status bar remain; the centre region fills the dock |
| A region is closed while it holds the console | The console goes with the region and comes back where it was left. Nothing leaves the tree |
| The console's tab is closed | It leaves the arrangement, and the new-pane menu's `Logs` row is what brings it back |
| The user empties a region by closing its last panel or dragging it out | The region closes itself; the titlebar's switch for it reads as closed |
| A project's saved arrangement leaves an edge region open with nothing in it | The region is collapsed as the restore settles, the same as one the user just emptied |
| The user opens an empty side region from the titlebar | It fills with the mode's own furniture for that side — Git's refs or changes, KB's documents, Tasks' task, Agents' list — or, where the side is the chat's home in that mode, with a fresh chat tab attached to nothing |
| The user opens an empty side region a mode has no furniture for | It stays open and empty — whatever was there was dragged away on purpose |
| A panel is dropped in a region its class forbids | It is moved back to its home region on the same edit, so the drop reads as refused |
| A saved arrangement is from another version, or is unreadable | It is discarded whole and the window opens on the default arrangement |
| A saved arrangement names a pane the window still holds | The pane comes back where it was — its group, its split and its tab position |
| A saved arrangement names a pane the window does not hold | The panel is dropped and the tree normalises around the gap. Layout persists; harnesses do not |
| A saved region's every panel was dropped | The region is installed empty, at the size and open state the blob says. The tab strip is still where a pane is opened from |
| The pane region is opened with nothing in it | A pane is started in it, the platform's default shell, as a bare click on `+` would |
| The last project in a window is closed | The window stays, on the empty state. Its harnesses are killed with the project, and what it remembered is written down |
| A project with terminals or running agents is closed | The row asks first, and closes only on a second, explicit click |
| An ephemeral project is closed | The row says the clone will be discarded and closes only on a second, explicit click. The host deletes the folder when it forgets the record |
| A clone is cancelled, or fails | The modal names the reason and stays open with what was typed. The partial destination is removed, and no project appears |
| A repository URL is pasted that is `ssh` | The modal names it as unsupported and offers nothing to click. https is the only transport a clone runs over |
| A connection's provider has no repository listing | The modal says the provider is unsupported. A pasted URL still works, because it needs no connection |
| The repository listing is cut short | The modal says so, and a filter that matches nothing against a cut-short listing asks the provider to search rather than claiming there is nothing |
| A project open in another window is opened here | It leaves that window, which stays open on the empty state if it held nothing else. Its panes and its conversations move with it, still running; a pane the user had detached arrives detached, and the scrollback does not survive the move |
| More than 26 windows are open | The 27th and beyond are named `#`; nothing else changes |
| The last window is closed | The application quits. Closing one of several does not |
| A rail mode has no screen | The empty page names the mode and says it is not built |
| A screen over the work is opened with no project | The centre draws nothing. All four are views of one project's work and there is none; the rail, titlebar and status bar stay |
| A project's folder is deleted, renamed or unmounted | The next probe marks the row; the record stays and the window keeps its last screen |
| A marked project is located again | The record keeps its id, colour and history; only its path moves |
| A folder already in the catalogue is added again | The picker points at the project that is there; no duplicate appears |
| The catalogue is empty | The window stays open on the empty state, which offers to add a project |
| The catalogue file is corrupt | It is preserved under a timestamped name, the session starts empty, and one error says so |
| The catalogue cannot be written | Changes hold for the session and one error says they are not durable |
| A window's view state is corrupt or from another schema | It is discarded and the window opens on defaults |
| A vim operator is abandoned with Escape | The half-typed command is dropped and nothing is deleted |
| Escape is pressed in Normal mode with nothing half-typed | Vim does not claim it, so the modal the textarea sits in still closes |
| A vim count runs past the end of the buffer | The cursor stops at the last line and the count is spent |
| `ciw` is pressed where there is no word | The operator is cancelled and the cursor does not move |
| Vim mode is switched off mid-command | The mode and the half-typed command are both reset, so switching it back on starts in Normal |

## Related docs

- [`workbench-ide.md`](./workbench-ide.md) — IDE mode: the explorer, the editor tabs and the viewers
- [`workbench-git.md`](./workbench-git.md) — Git mode: refs, history, changes and the diff
- [`workbench-agents.md`](./workbench-agents.md) — Agents mode: the columns, the bench and the New agent form
- [`workbench-teams.md`](./workbench-teams.md) — `Teams` and `[Teams]`: the graph, its arrangements and the tasks drawer
- [`workbench-tasks.md`](./workbench-tasks.md) — Tasks mode: the board, its cards, the task panel and the plan
- [`workbench-sink.md`](./workbench-sink.md) — Sink mode: the kitchen sink and its fixtures
- [`panes-and-terminals.md`](./panes-and-terminals.md) — what a terminal panel actually is
- [`chat.md`](./chat.md) — the chat tabs, which survive every mode switch
- [`stats.md`](./stats.md) — the Control screen the rail fills the centre with, and the usage meter
- [`../tech/ui-and-design.md`](../tech/ui-and-design.md) — the tokens and the component conventions
- [`../tech/architecture.md`](../tech/architecture.md) — who owns which state, and why the interface holds no truth
- [`../tech/decisions.md`](../tech/decisions.md) — `D47`, why the agents and the work are two screens
- [`../backlog.md`](../backlog.md) — what the shell still lacks

## Next steps

- Keyboard navigation for the rail and the tabs.
- Make the titlebar's command field run a command — the navigator and the palette are still
  unbuilt.
