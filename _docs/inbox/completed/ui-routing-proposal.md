---
id: inbox-routing
title: Proposal — navigation, links, history and bookmarks
kind: proposal
status: proposal
summary: One addressable value for every place the user can be — a destination naming a project, a view, an item and a locus the view alone reads — so a link opens it, a back stack returns from it, and a bookmark keeps it.
read_when: you are deciding how one part of the interface sends the user to another, what a link is, or where back, forward and bookmarks live
updated: 2026-09-05
depends_on: [feat-workbench, tech-ui, tech-architecture, inbox-panels, inbox-viewers]
---

# Proposal — navigation

Ubiq has five screens and no way to get from one to another except by hand. A task card can say
`Show in graph` because somebody wrote a function that knows both ends of that jump; a diff cannot
say "open this file at this line", a log record cannot say "the pane that wrote me", and a chat
message that mentions `src/app.rs` is text. Nothing remembers where the user was a moment ago, and
nothing keeps a place they want to come back to tomorrow.

This proposes one value — a **destination** — that names any place in the interface, and one call
that goes there. Everything else in this document falls out of that: a **link** is a destination
with a label, **history** is a stack of destinations arrived at, a **bookmark** is a destination
written down, and the titlebar's `⌘K` field is a thing that turns text into destinations.

The hard part is not the routing. It is that a place inside a view is not the same kind of thing from
one view to the next — a line in an editor, a rectangle in an image viewer, a heading in a rendered
document, a node in a graph — and a router that knows about lines has already lost. §3 is that
problem, and the rest is shaped around it.

## 1. Where it stands

**Two jumps exist, and each is a function that knows both ends.** `show_task_in_graph()` and
`open_task_chat()` in `crates/ubiq/src/app/board.rs` are what the board's task panel calls; between them
they set the graph's selection, the inspector's tab and the rail mode. They are correct and they do
not compose: a third jump is a third function, and neither can be recorded or written down.

**Where the user is has no value.** It is nine fields in eight places: `WindowSlot::active` in the
process-wide registry, `WorkbenchState::rail_mode`, `EditorPaneState::active`,
`ExplorerState::selected`, `BoardState::selected` with `show_detail`, `AgentsState::selection` with
`tab`, `ChatState::active`, `AppState::dock_tab`, and `OpenProject::focused_pane`. Nothing reads
them together, and `crates/ubiq/src/ui/status_bar.rs` — which comes closest, branching on the rail
mode to decide which facts it has — reads each one separately.

**Nothing remembers.** No back stack, no visited list, no bookmarks; the picker's history group is
"projects no window is holding", which is a different word. There is deliberately no breadcrumb, and
[`../../features/workbench.md`](../../features/workbench.md) says why.

**The `⌘K` field is a stub.** `command_input` is rendered by `ui/titlebar.rs` as a 420px field with
a search icon and a decorative hint, wired to nothing. That is `G16`.

**Persistence carries a subset of position and calls it furniture.** `ViewPrefs` in
`crates/ubiq/src/state/prefs.rs` stores the rail mode, the panel flags and sizes, the open files
and which was active, the expanded folders and the selected row. It carries no caret, no scroll, no
graph or board selection, and no dock tab. The caret is not state at all: `cursor_line_column()`
reads it live out of the buffer for the status bar and nothing keeps it.

One asymmetry matters, and §9 comes back to it: every contract id — a project's, a pane's, a
session's, a task's, an agent's — is a ULID that outlives the process, and a chat tab's is a `u64`
this window minted, which does not.

## 2. The destination

**A destination is a value that names a place, and building one touches nothing.** It is inert: it
opens no file, activates no project and notifies no view. Somebody hands it to the router, and the
router is the only thing that acts.

| Part | Holds | Absent means |
|---|---|---|
| `project` | The `ProjectId` the place belongs to | — |
| `view` | Which screen, and its own key for the item inside it | — |
| `locus` | Where inside that item — §3 | Wherever the view already was |

The view arm carries the item, because the two are never useful apart:

| Arm | Key | Reaches |
|---|---|---|
| `Ide { key }` | An editor tab key | The editor, that file in front |
| `Explorer { path }` | A project-relative path | The tree, that row revealed and selected |
| `Terminal { pane }` | A `PaneId` | The dock, that pane focused |
| `Logs` | — | The dock's log console |
| `Graph { selection, tab }` | A session or an agent | The orchestration graph, that thing selected, the inspector on that tab |
| `Agents { agent }` | An agent | The agents columns, that agent's transcript |
| `Tasks { task }` | A task | The board, that card's panel open |
| `Chat { chat }` | A conversation | The chat panel, that conversation |
| `Git` | — | The Git screen |
| `Control` / `Kb` | — | The two modes that are still an empty page |

**A file's identity is a tab key, not a path.** A file and its diff are two tabs over one path
(`tab_key(path, Subject)` in `crates/ubiq/src/state/editor.rs`), so the arm carries the key; for a
plain file the key *is* the path, which is why §5's text form is unaffected by the difference.

**The graph and the agents columns are two arms, not one.** The graph is the map and the columns
are the transcript, and a link that says "show me this agent" means a different screen from one
that says "show me this agent on the graph". **The kitchen sink has no arm at all**: it is the test
bench, it has no project behind it, and a screen with no project is not a place.

Three rules hold over the whole set.

**A destination names a project, never a window.** Which window shows a project is the registry's
answer and the user's, not the link's — a project is open in exactly one window, so naming one
would be naming the same place twice and letting the two disagree.

**Every screen answers two questions, and one that cannot is not a place.** `where` returns the
destination the screen is at, including its locus; `reveal` takes one and gets there. A new screen
owes both, and until it has them it cannot be linked to, bookmarked or put in the history — one
function pair, and it is the price of a rail mode.

**No absolute path is in a destination, ever.** For the same reason no file descriptor crosses into
the interface: the folder is the host's, and the two need not be on one machine. This is
[`../../tech/architecture.md`](../../tech/architecture.md)'s second rule, and navigation does not get an
exemption from it.

## 3. The locus, and why the router never reads one

A place inside a view is view-shaped. The editor's is a line; an image viewer's is a rectangle and a
zoom; a rendered document's is a heading; the graph's is a card and a camera; a terminal's is an
offset into scrollback. A router that understands lines will understand rectangles next, and then
it is every viewer's business rolled into one file.

**So the locus has a shared grammar and a private meaning.** The grammar is shared because a
destination has to survive being written to disk and typed into a field, and something has to parse
the text back. The meaning is the view's alone: **nothing outside the view that owns a kind ever
matches on a locus.**

| Kind | Text form | Read by |
|---|---|---|
| `Line { line }` | `L42` | The editor, a Markdown source, a diff |
| `Span { from, to }` | `L42-58` | The editor, a diff hunk |
| `Anchor { slug }` | `heading-slug` | A rendered document, a table of contents |
| `Viewport { x, y, scale }` | `v=0.5,0.25,2` | An image viewer, the graph, an Excalidraw scene |
| `Node { key }` | `n=worker-3` | The graph, a rendered diagram |

The kinds are a closed set, because the interface is one binary and a closed set is checkable. The
openness that matters is not in the type: **a view handed a locus of a kind it does not understand
ignores it** and opens where it would have anyway. An image viewer given `L42` shows the image.

**A locus is a hint, and navigation always arrives.** A line past the end of a file, a renamed anchor,
a deleted node, a viewport onto an image since replaced by a smaller one: none of these is an error,
a dialog or a refusal. The view opens at its best guess and the user is somewhere real. What
navigation must never do is leave them where they were with nothing said — that is indistinguishable
from a dead click.

**Two prerequisites are real and small.** The graph's pan is not tracked state —
`ui/orchestration/graph.rs` uses a bare scrolling container — so a `Viewport` locus on the graph
needs a handle before it can be
read or written. And nothing in Ubiq sets a caret; revealing a line means driving the open file's
`EditorState`, which is the one thing navigation needs from the component library that
`crates/ubiq/src/ui/editor.rs` does not use yet.

## 4. Links

**A link is a destination and a label, and the thing drawing it does not know how to open it.** It
emits the value and the router opens it — which is what makes a link cheap enough to put everywhere,
and why `Show in graph` stops being a function about the board.

What can emit one, once this exists:

| Surface | Link |
|---|---|
| A task card, and the task panel | Its agent, its session, its graph position |
| An agent card, and the inspector | Its pane, its task, its branch's files |
| A diff hunk | The file at the hunk's first line |
| The explorer, and the editor's tabs | Themselves — which is what makes them bookmarkable |
| A log record | The pane, project or file it names |
| A rendered Markdown document | Its own relative links, and its headings |

**One exclusion, and it is not negotiable: Ubiq does not scan terminal output for links.** Terminal
bytes are opaque — no VT parsing, no pattern matching over a harness's screen, no clickable path in a
pane. A harness that wants to send the user somewhere says so through the chat, where the transcript
is structured, or through a task. Making a pane's bytes linkable means reading them, and reading them
is the thing Ubiq does not do.

**Following a link is one call, whatever drew it.** Same call for a card, a bookmark, a `⌘K` result
and a back press — which is what keeps the history honest, because there is one place that records
an arrival.

## 5. The text form

A destination needs to survive being written down: in a bookmark on disk, in a Markdown document, in
a chat message, in the `⌘K` field, on the clipboard behind a `Copy link` action.

```
ubiq://<project-id>/<view>[/<item>][#<locus>]
```

| Written | Example |
|---|---|
| A file at a line | `ubiq://01J7…/ide/crates/ubiq/src/app/wire.rs#L1712` |
| A range | `ubiq://01J7…/ide/README.md#L10-24` |
| A heading in a rendered document | `ubiq://01J7…/ide/_docs/INDEX.md#catalogue` |
| A task | `ubiq://01J7…/tasks/<task-id>` |
| An agent's transcript | `ubiq://01J7…/agents/<agent-id>` |
| An agent on the graph, inspector on chat | `ubiq://01J7…/graph/a:<agent-id>/chat` |

**The view slug decides the arity, not the number of segments**, which is what lets a multi-segment
file path go unescaped. `ide` and `explorer` take everything up to the fragment; `terminal`,
`tasks`, `chat` and `agents` take exactly one bare id; `graph` takes `s:<session-id>` or
`a:<agent-id>` — the prefix is forced, because both are 26-character ULIDs and the text alone
cannot tell them apart — and then optionally `chat` or `tasks`; `control`, `kb`, `git` and `logs`
take none, and a trailing segment on one of them is a different string rather than a refinement.

**The project is written as its id and shown as its name.** A ULID is unreadable and a name is not
stable: a project renamed, recoloured or moved on disk keeps its id, which is exactly what a bookmark
from three weeks ago needs. Wherever a link is drawn, the name is what the user reads.

**Parsing is total.** A string that does not parse is not a link — no error, no toast, no partial
navigation to the project it happened to name.

**A relative link inside a document resolves against the document.** A Markdown link whose target is
`../src/app.rs#L200` is a destination in that file's project, at that path — which is how a rendered
document becomes navigable without inventing a second syntax. Anything that escapes the project root
is not a link, the same rule the file family already enforces on the wire.

## 6. History

**One back stack per window, spanning every project that window has shown.** Not per view and not per
project: the user who clicked from a task to a file to a pane expects one back press to undo the last
of those, and three stacks cannot agree on which was last.

**An entry is recorded on arrival, not on request**, so it never names a place that was not drawn.

| The user… | The stack |
|---|---|
| Follows a link, a card, a bookmark or a `⌘K` result | Pushes |
| Changes rail mode from the rail | Pushes |
| Opens a file from the explorer, or brings a tab forward | Pushes |
| Switches project in the picker | Pushes |
| Moves the caret, scrolls, pans the graph, folds a column | Updates the current entry's locus in place |
| Presses back or forward | Moves the cursor. Pushes nothing |
| Types in a filter, toggles a panel, changes the palette | Nothing. Not a place |

Three refinements make that table behave the way an editor does.

**The current entry's locus is kept current, so back returns to where the user left rather than where
they entered.** Without it, coming back to a file lands at the line the link named however far the
user then scrolled, which is the most annoying thing a back button can do.

**One push site, and it is not the router.** Most departures never reach the router at all — the
rail calls `set_rail_mode`, the explorer calls `select_file`, the dock calls `activate_file` — so
asking the outgoing screen on the way out would cover only the arrivals the router already sees.
Instead `settle_nav` runs once a frame from `Render`, reads what the window is *drawing*, and either
records a new place or refreshes the standing entry's locus. That is the table above with no call
sites to keep in step, and it subsumes the outgoing-locus refresh as the same act.

**A movement inside one view is not an entry until it is a jump.** Typing and scrolling are not
navigation: a move within one view pushes only when it came through the router — a link, a search
result, a bookmark — and never when it came from the caret.

**Arriving where you already are pushes nothing.** The locus is applied, the entry is updated, and
the stack is the length it was. Forward is truncated by a push, as everywhere.

The stack is bounded — 64 entries, oldest dropped — and **not persisted**: one restored into a window
whose projects, tasks and agents have all moved on is a list of places that no longer exist. What is
worth keeping across a restart is the recents list in §8, which is a set rather than a spine.

**Back never moves a project between windows.** An entry whose project is now held by another
window is skipped, and the control says so rather than yanking the project across. An entry whose
project has been forgotten is dropped from the stack entirely.

Back and forward are two actions bound beside `SaveFile` in `crates/ubiq/src/app/mod.rs` — `⌃-` and
`⌃⇧-`, the editor convention — and two controls in the titlebar, each disabled when its end of the
stack is empty and each carrying the name of where it would go.

## 7. Bookmarks

**A bookmark is a destination the user wrote down, with a name and an optional note.** One action
toggles one on wherever the user currently is, which is `where` and nothing more; the name defaults
to what the destination is called — the path and line, the task's title, the agent's name — and is
editable.

**Bookmarks belong to the project their destination names.** They ride the project's opaque view blob
through `SetPreferences` under `Scope::Project`, beside the furniture `ViewPrefs` already stores, and
the host neither parses nor validates them — the schema stays the interface's, exactly as
[`../../tech/transport-contract.md`](../../tech/transport-contract.md) has it. Forgetting a project forgets
its bookmarks, which is what "Forget" promises.

**A line number rots, so a file bookmark is anchored as well as numbered.** It keeps the trimmed
text of the line it was made on, capped at 120 characters, beside the line number. On opening:

| The anchored line… | What happens |
|---|---|
| Still reads the same at that number | Go there |
| Is found within 200 lines either side | Go there, and re-stamp the number |
| Is nowhere in the file | Open at the remembered number, and mark the bookmark **adrift** |
| Is past the end of the file | Open at the last line, and mark it adrift |

An adrift bookmark is drawn as adrift and is not silently repaired, because a bookmark that quietly
points at the wrong line is worse than one that says it lost its place. Loci with no text to anchor
to — a viewport, a node, a task — carry no anchor and none is invented for them.

**The list is drawn where the project's other content is.** A collapsible section at the head of the
explorer panel, project-scoped, each row its name and its destination's label, with the adrift mark
where it applies.

**A bookmarked line is marked on the line, not in the gutter.** The component library's only public
decoration surface is `TextDecorationCollection` over byte ranges; its gutter is a private element
hard-coded to line numbers and fold icons. So the mark is a highlight over the line itself, and the
thing that says a file holds bookmarks while their lines are off screen is a count chip on the
file's tab. Under
[`./movable-panels-proposal.md`](./movable-panels-proposal.md) it becomes a panel of its own, and can
be dragged beside the file it points into; nothing above changes when it does.


## 8. Recents, and the navigator

The router sees every arrival, so it can keep two things from them. The stack in §6 is a spine —
ordered, with a cursor, thrown away with the window. **Recents is a set** — the last 32 distinct
destinations per project, most recent first, persisted in the same blob as the bookmarks, and it is
what makes `⌘K` useful on the first keystroke after a restart.

**The `⌘K` field becomes the thing that turns text into destinations**, which is its first real job
and half of `G16`. It offers groups, filtered together by whatever is typed:

| Group | Rows |
|---|---|
| Recent | This project's recents, most recent first |
| Bookmarks | This project's bookmarks, adrift ones marked |
| Files | The explorer's tree, matched on path — the filter that already exists, given a second home |
| Tasks | Titles, across sessions |
| Agents | Names and roles |
| Commands | Everything the interface can do that is not a place |

A pasted `ubiq://` URI resolves to a single row naming where it goes, which is how a link from a chat,
a commit message or another machine gets followed. One naming a project this catalogue does not have
says so and offers nothing — it cannot know the path of a folder it has never seen.

## 9. Which places can be written down

**A destination can be written down when every id in it outlives the process that minted it.** That
is a property of the arm, and it is checked in one place: `Destination::persistable`.

**A file is fine.** Its identity is a tab key over its project-relative path, which is what the file
family carries, what `ViewPrefs` stores, and what `ExplorerState` selects on. A destination writes
the same string.

**A task, an agent, a session and a pane are fine too.** Every contract id in
`crates/ubiq-proto/src/ids.rs` is a ULID newtype with `Display` and `FromStr` — `ProjectId`,
`PaneId`, `SessionId`, `TaskId` and `WorkspaceId`, which is what an agent id is. They mean the same
thing after a restart, so a bookmark to a task is a bookmark to that task, and the whole of
`ubiq://` is printable and parseable for them.

**A chat is the one that is not.** `ChatId` is a `u64` minted by the window in
`crates/ubiq/src/state/dock.rs`, and it means nothing tomorrow. A chat destination is followed and
put in the history, and it is never written into a bookmark or a recents list.

Stating the limit is the point, and stating it once: **an arm with no printable id is never offered
a bookmark by construction**, because the serialised form of a destination is its `ubiq://` text and
nothing else.

## 10. Where it lives

A `nav` module under `crates/ubiq/src/state/` holds the value, the parser, the printer, the stack and
the bookmark store. The router is a small set of methods on `AppState` beside the ones that already
exist — `navigate`, `back`, `forward`, `toggle_bookmark` — each ending in `cx.notify()` like every
other mutator in that file, and each recording the arrival.

Each screen contributes its `where` and `reveal` beside the module that draws it: `ui/editor.rs`,
`ui/explorer.rs`, `ui/terminal.rs`, `ui/agents/mod.rs`, `ui/board/mod.rs`, `ui/logs.rs`. The router
matches on the view arm and calls one of them; it contains no knowledge of what any of them do.

Under [`./movable-panels-proposal.md`](./movable-panels-proposal.md) a destination still names a
view rather than a panel, and `reveal` means "focus the group already holding this, or open one" —
the router does not gain a panel argument, and a link never says where on screen to put the thing it
opens. Under [`./file-viewers-proposal.md`](./file-viewers-proposal.md) the viewer chosen for a file
is the thing that reads the locus, which is why §3's kinds line up with the viewers that document
proposes: `Line` for the source, `Anchor` for the preview, `Viewport` for an image or a scene,
`Node` for a rendered diagram.

**Nothing crosses the bus for navigation.** The router issues the intents that already exist —
activating a project through the registry, reading a file through the file family — and adds no
message of its own. The one thing it stores rides a blob the host already carries and does not read.

## 11. Failure

Every row is a place navigation still arrives, or a place it declines to start — never a dialog.

| The destination… | What happens |
|---|---|
| Names a project no window holds | The window following the link opens it, and the entry is its own |
| Names a project another window holds | That window is raised and navigates; the follower's stack is untouched |
| Names a project that has been forgotten | Not a navigation. The history entry is dropped, the bookmark marked dead |
| Names a file that is gone | The tab opens and reports the read failure, as any open does |
| Names a task, agent or session that is gone | The view opens on its own default and says the item is gone |
| Names a pane whose harness exited | The pane is shown as it is — an exited harness keeps its pane |
| Carries a locus of a kind the view does not know | The locus is dropped; the view opens where it would have |
| Carries a line past the end of a file | The view opens at the last line |
| Is a string that does not parse | Nothing navigates, and the text was never a link |
| Is where the user already is | The locus is applied and nothing is pushed |

## 12. Rules this adds

1. **Navigation is the interface's own** — no transport family, no new message.
2. **Every place has a destination, and every screen can say which one it is at.** A screen with no
   `where` and no `reveal` is not linkable, not bookmarkable, and absent from the history.
3. **A destination names a project, never a window.**
4. **Navigation always arrives.** A locus that cannot be honoured is dropped, never refused.
5. **A locus is opaque outside the view that owns its kind.**
6. **Ubiq does not scan terminal output for links.**
7. **Back never moves a project between windows.**

## 13. Phases

1. **The value and the router.** `Destination`, the view arms, `Locus`, the `nav` module, `where` and
   `reveal` for the three built screens, and the two hand-written jumps rewritten as destinations.
   Nothing new appears on screen, and the board's two buttons keep working.
2. **History.** The stack, the coalescing rule, the locus refresh on the way out, the two actions and
   their keys, and the two titlebar controls. Worth having on its own.
3. **The text form.** Parse, print, `Copy link` wherever a destination exists, and relative
   resolution inside a document.
4. **Bookmarks.** The record, the store in the project blob, the toggle, the explorer section, the
   line anchor and the adrift mark. Every arm but a chat can be written down.
5. **The navigator.** `⌘K` over recents, bookmarks, files, tasks and agents, and the URI paste.
6. **Links in content.** The rendered document's links and the chat transcript's mentions become
   destinations. Waits on the viewers, and on the chat's transport family.

Phases 1 and 2 stand alone; 6 cannot start before its two dependencies do.

## 14. What this asks to be decided

Seven decision rows:

- Every place in the interface is one value — a project, a view with its item, and an optional locus
  — and every screen can both produce one and be sent to one.
- A locus has a shared grammar and a private meaning: nothing but the view that owns a kind ever
  matches on it, and a locus that cannot be honoured is dropped rather than refused.
- A destination names a project and never a window; which window shows it is the registry's answer.
- Ubiq does not scan terminal output for links. A harness sends the user somewhere through the chat
  or through a task, never through its screen.
- History is one bounded stack per window, spanning projects, recorded on arrival, not persisted, and
  it never moves a project between windows.
- A bookmark rides the project's opaque view blob, and a file bookmark keeps the text of its line so
  it can say when it has come adrift rather than pointing quietly at the wrong place.
- A destination is written down only when every id in it outlives the process that minted it, which
  is every arm but a chat; a destination is stored as its `ubiq://` text, so compatibility belongs to
  one parser rather than to a set of serde variant names.

Backlog rows this leaves open: a tracked pan handle for the graph, without which a graph viewport
cannot be read or written; caret and scroll restoration on a tab switch, which the same `reveal`
machinery would give almost for free; whether a link should be able to *preview* a destination
without committing the history to it; mouse back and forward buttons, which need a gpui capability
nothing here has checked; cross-project bookmarks, which have nowhere to live while the store is
project-scoped; and the `⌘K` field's other half, the commands, which are not places.

## Related docs

- [`../../features/workbench.md`](../../features/workbench.md) — the screens, the rail modes and the `⌘K` field this gives a job to
- [`../../tech/architecture.md`](../../tech/architecture.md) — the rules §2 and §10 obey
- [`../../tech/ui-and-design.md`](../../tech/ui-and-design.md) — where the controls and marks §6 and §7 add are drawn
- [`../../tech/transport-contract.md`](../../tech/transport-contract.md) — the preferences scope §7 stores bookmarks in
- [`./movable-panels-proposal.md`](./movable-panels-proposal.md) — the dock a `reveal` resolves against
- [`./file-viewers-proposal.md`](./file-viewers-proposal.md) — the viewers that read §3's loci
- [`../../backlog.md`](../../backlog.md) — `G16`, and what this leaves open
