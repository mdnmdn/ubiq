---
id: feat-workbench
title: The workbench
kind: feature
status: draft
summary: The window's shell — the activity rail and its modes, the dock of movable panels the user arranges around the centre, the file explorer and editor a project owns, the Git screen of refs, history and uncommitted changes, the agents screen of parallel columns, the orchestration graph and the tasks board the rail's other built modes hold, the kitchen sink the application tests itself against, the file picker any screen raises to choose a path, the empty state a window with no project shows, and the status bar that reports on all of it.
read_when: you are changing the window layout, a rail mode, where a panel may sit or when it is drawn, the explorer, the editor tabs, what a file panel draws, which viewer draws it, how a diagram is rendered or cached, saving a file, the Git screen's refs, history, change lists or diff, the agents screen's columns, what a tab drag means or what the bench is, the orchestration screen's graph, how it arranges itself, its inspector or its tasks, the tasks board's columns, cards or task panel, the kitchen sink's pages or fixtures, the file picker a screen raises to choose a path, application settings, or the status bar
updated: 2026-09-03
verified: 2026-09-03
code_anchors: [crates/ubiq/src/app.rs, crates/ubiq/src/state/when.rs, crates/ubiq/src/state/prefs.rs, crates/ubiq-host/src/projects.rs, crates/ubiq/src/ui/shell.rs, crates/ubiq/src/state/dock.rs, crates/ubiq/src/ui/dock/mod.rs, crates/ubiq/src/ui/dock/skin.rs, crates/ubiq/tests/dock.rs, crates/ubiq/tests/mode_restore.rs, crates/ubiq/src/ui/terminal.rs, crates/ubiq/src/ui/logs.rs, crates/ubiq/src/ui/rail.rs, crates/ubiq/src/ui/titlebar.rs, crates/ubiq/src/ui/project_menu.rs, crates/ubiq/src/ui/explorer.rs, crates/ubiq/src/ui/editor.rs, crates/ubiq/src/ui/viewer/mod.rs, crates/ubiq/src/ui/viewer/diff.rs, crates/ubiq/src/ui/viewer/markdown.rs, crates/ubiq/src/ui/viewer/diagram.rs, crates/ubiq/src/ui/viewer/scene.rs, crates/ubiq/src/ui/viewer/viewport.rs, crates/ubiq/src/ui/viewer/image.rs, crates/ubiq/src/state/diagrams.rs, crates/ubiq/src/state/viewport.rs, crates/ubiq/src/state/scene.rs, crates/ubiq/tests/diagrams.rs, crates/ubiq/tests/viewport.rs, crates/ubiq/tests/scene.rs, crates/ubiq/src/ui/empty.rs, crates/ubiq/src/state/sink.rs, crates/ubiq/src/ui/sink/mod.rs, crates/ubiq/src/ui/sink/docs.rs, crates/ubiq/src/ui/sink/style.rs, crates/ubiq/src/ui/sink/files.rs, crates/ubiq/src/ui/sink/settings.rs, crates/ubiq/src/ui/settings.rs, crates/ubiq/src/state/settings.rs, crates/ubiq/src/ui/kit/settings.rs, crates/ubiq/tests/settings.rs, crates/ubiq/src/ui/sink/project.rs, crates/ubiq/tests/sink.rs, crates/ubiq/src/state/file_picker.rs, crates/ubiq/src/ui/file_picker.rs, crates/ubiq/tests/file_picker.rs, crates/ubiq/src/ui/status_bar.rs, crates/ubiq/src/state/mod.rs, crates/ubiq/src/state/workbench.rs, crates/ubiq/src/state/windows.rs, crates/ubiq/src/state/explorer.rs, crates/ubiq/tests/explorer.rs, crates/ubiq/src/ui/kit/files.rs, crates/ubiq/src/state/editor.rs, crates/ubiq/src/state/git.rs, crates/ubiq/src/ui/git/mod.rs, crates/ubiq/src/ui/git/refs.rs, crates/ubiq/src/ui/git/history.rs, crates/ubiq/src/ui/git/changes.rs, crates/ubiq/src/ui/git/diff.rs, crates/ubiq/tests/git.rs, crates/ubiq/src/state/agents.rs, crates/ubiq/src/state/conversation.rs, crates/ubiq/src/ui/conversation/mod.rs, crates/ubiq/tests/conversation.rs, crates/ubiq/src/state/orchestration.rs, crates/ubiq/src/state/layout.rs, crates/ubiq/src/state/board.rs, crates/ubiq/src/state/work.rs, crates/ubiq/src/state/sample.rs, crates/ubiq/src/ui/agents/mod.rs, crates/ubiq/src/ui/agents/sidebar.rs, crates/ubiq/src/ui/agents/column.rs, crates/ubiq/src/ui/orchestration/mod.rs, crates/ubiq/src/ui/orchestration/graph.rs, crates/ubiq/src/ui/orchestration/inspector.rs, crates/ubiq/src/ui/orchestration/tasks.rs, crates/ubiq/src/ui/work.rs, crates/ubiq/src/ui/board/mod.rs, crates/ubiq/src/ui/board/detail.rs, crates/ubiq/src/ui/board/form.rs, crates/ubiq/tests/agents.rs, crates/ubiq/tests/orchestration.rs, crates/ubiq/tests/board.rs]
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

**The rail selects what the middle of the window is for.** Eight destinations in two groups:
`Control` and `Sink` under `APP`, and `IDE`, `Git`, `Agents`, `Orchestration`, `KB` and `Tasks`
under `PROJECT`. Exactly one
is active, and the active one is shown by the accent colour on both its icon and its label. The
group is not decoration: a `PROJECT` mode with no folder open draws the page saying so, and an `APP`
mode answers whether or not one is open.

**Six modes are built.** What the rail selects between is the centre. Git fills it with the
repository, Agents with the parallel columns, Orchestration with the graph, Tasks with the board,
Sink with the kitchen sink, and the centre panel's tab is named for the mode. IDE fills it with the
open files, one panel each, and the centre panel steps aside for as long as any is open. `Control`
and `KB` render one empty page naming the mode and what it will hold. This is a stated gap, not an error state.

**The explorer and the chat belong to IDE mode.** They are IDE furniture and leave together when the
mode changes; the console, the terminals and the centre panel itself outlive a mode switch. The chat
is written to be reused by the other screens later, but is not shared furniture today. A screen that
wants a panel of its own brings it: the orchestration screen's inspector and tasks drawer, and the board's
task panel, are drawn inside the centre panel rather than in the dock, which is why they toggle
instead of dragging and go with the mode.

**Two screens stand over the same records, and the split is the point** — `D47`. Agents is where
the user *talks to* the agents; Orchestration is where the user *arranges* them. A graph is a map of
who spawned whom, a column is a transcript and a composer, and neither screen draws the other's
view. Both read one projection of what the host reports about a project.

**A column is a place to talk to an agent, not a place an agent lives.** Which column an agent's
conversation is drawn in is the interface's own fact: no message carries it and no drop sends one,
the same rule the graph's card positions follow. What the host owns is which agents exist and what
each is doing, and the screen reads that and never writes it.

**A column holds tabs, and more than one tab is a group.** Dragging a tab onto another column puts
the two agents in one strip, which is how a hand-off is read — the plan and the build side by side,
one column wide. Dragging it past the last column gives it a column of its own again, and a tab
alone in its column is what that gesture produces anyway, so the drop changes nothing. The column a
drop would group into lights up, and the strip at the end of the row lights only for a drop that
would do something: a target that promises a change it will not make is worse than one that stays
dark. The strip over the columns counts them — how many there are, how many agents they hold and how
many of them are grouped — and names both gestures, because neither leaves a mark on the interface
to be discovered from. Its controls are `Close all` and `New agent`. `Close all` benches every agent
on screen — `bench_agent` for every tab in every column, the same thing a tab's own close already
does, not `EndConversation` — and is shown only when there is something on screen to bench; a row
with no columns gets no button rather than one that would silently do nothing. `New
agent` reads the harness library's agent types, the same `ListAgentTypes` answer the new-pane menu
reads so the two lists cannot disagree. A pick does not start anything by itself — it raises a
naming prompt (`ui::agents::new_agent_naming`, over `WorkbenchState::naming_agent`), the window
between choosing a harness and identity and typing what to call the conversation, where leaving
costs nothing because no harness exists yet. Confirming sends `StartConversation` with that name,
or `None` if the field was left empty, which the host reads as "use the harness's own label" —
the way every conversation's name worked before this prompt existed. A type whose binary is not on
this machine is listed and refuses the click rather than being started into a failure. What a pick
eventually makes is a conversation rather than a pane — the same question asked of the other face
of a workspace, and a conversation has no size.

**Closing a tab benches the agent; it does not end it.** This is the one place the screen
deliberately reads differently from a terminal pane, whose close kills the harness behind it —
[`panes-and-terminals.md`](./panes-and-terminals.md). A tab is a view onto a conversation, so taking
it off screen leaves the agent running: the sidebar still lists it, marked `bench`, and one click
brings it back. Nothing on this screen kills an agent — `Close all` benches the whole row the same
way a single close does. `AppState::end_conversation` and `Message::EndConversation` still exist for
a future explicit "end this agent for good" action, but nothing drawn today calls it.

**The bench is computed, not stored.** It is every agent the host reports that no column is showing,
so an agent the host stops reporting stops being listed with nothing to clean up.

**The screen lays itself out once, and every listing after that only prunes.** The first `WorkList`
gives one column per session that has an agent in it, holding every agent in that session in the
order the host listed them: each column is a piece of work, and a session running several agents
arrives grouped rather than spread across the row. The bench therefore starts empty and fills only
from the user's own closes. Every later `WorkList` or `AgentChanged` drops the tabs naming agents the
host has forgotten, and the columns that empties, and does nothing else — an arrangement the user has
changed is not something an arriving record may undo. An arriving agent is listed on the bench rather
than put in a column.

**A column owns a composer for its life.** The window holds a fixed pool of `COMPOSER_SLOTS` text
areas — one per column plus `CHAT_SLOT`, the chat panel's own slot past the last column's — because
one is built before the first frame and its subscription is held for the window's life. A column is
given a **slot** when it opens and keeps it, so what was typed at one agent never moves into a field
addressed at another, and a freed slot's draft is cleared because the slot is handed to the next
column that opens. The placeholder names the agent the column is showing. Enter sends, Shift-Enter
inserts a newline, and cmd/ctrl+Enter sends too — the same `secondary-enter` binding a multi-line
`submit_on_enter` field already answers the same way as a bare Enter, so there is nothing extra to
wire, only a hint to show for it. `AppState::agent_for_slot` is what "sends" resolves the agent
through on every surface — the chat panel's own selection for `CHAT_SLOT`, a column's active tab
otherwise — so the Enter key and the composer's own button never disagree about who a slot is
addressed at.

**One control does Send, Stop or Enqueue, depending on the turn.** Idle sends, exactly as
`prompt_agent` always has. A turn already running with the draft empty offers Stop, which cancels
it. A turn already running with something typed offers Enqueue instead of writing into a harness
mid-turn: the draft is held on the conversation's own `queued` list and the composer clears, the
same way a send clears it. `AppState::send_or_enqueue` is the one function behind all three — the
button's click and the Enter key both call it — and it is what a queued row's turn ending drains:
`Message::ConversationUpdate`'s handler pops the front of the queue and sends it as a plain
`PromptAgent` the instant `apply` leaves the conversation `Idle`. A queued prompt is drawn as its own
small row above the field, oldest first, each with an edit (loads it back into the composer) and a
delete (drops it); the block draws nothing when the queue is empty.

**The ceiling is on columns, not on tabs.** Eight columns fit the row. Grouping into a column that
is open always works, however many tabs it holds; a split that would need a ninth is refused and
leaves the tab where it was, because the room is checked before the tab comes off its column. A
click in the sidebar is never refused: with the row full, a benched agent is grouped into the
focused column rather than given one of its own, because "show me this agent" is a request the
screen can honour whatever the row looks like.

**A column draws a live conversation when there is one and a mock when there is not**, both in the
same list and the same sidebar — `G94`. A live one is drawn by
`crates/ubiq/src/ui/conversation/mod.rs`, the one interface for talking to an agent, and it knows
nothing about the screen hosting it: the chat panel and the kitchen sink adopt it by passing a
different `ConversationView` — an id prefix, a composer slot, whether a footer and a composer come
with it — rather than by growing a renderer each, which would drift the frame a tool block gained a
field. A block is markdown, a thinking block, or a tool call whose header carries a verb from the
tool's kind, its target and its status, and which expands onto its output or its diff.

**Nothing writes into a transcript.** The composer sends — `PromptAgent` to a live agent,
`SendToAgent` to a mock — and appends nothing itself; the user's own line appears when the harness
echoes it back, which is what the harness received rather than what was typed at it. A screen that
drew its own half of a conversation would be inventing the other half too.

**The run pill, the activity badge and the context ring are read off the stream** the window holds
rather than asked for, because asking would be a round trip per token.

**A column's footer reports the harness, and a ring only where there is one.** The harness, the
model, what the turn has cost, the context used out of the size the harness reported, and — where
Claude Code's `rate_limit_event` has arrived — how full the rolling five-hour window is. **No ring
is drawn when no context window was reported** — a ratio over an invented denominator reads as a fact
and is not one, and `G96` names who reports none; the rate-limit pill is guarded the same way. A
mock's footer draws no mode chip, because `WorkAgent` carries none — `G80`.

**Wherever a harness is named in passing, it is one glyph, not its label.** `kit::HARNESS_GLYPH` —
a single placeholder standing in for every harness alike, since none has a real icon yet — replaces
the harness text in the column footer's pill, the sidebar's secondary line and the chat panel's row.
Only the *choosing* surfaces still spell the label out in full: the new-agent menu's rows and the
settings page's harness list, where the full name is what a reader needs to make the pick. The
conversation's own name — what item above's naming prompt sets — is unaffected either way; the
glyph only ever stands in for the harness identifier next to it.

**The sidebar lists everything the host reports, not what is on screen.** That is the point of it: a
column is one conversation and there are only ever a few of them, so the list is the one place a
whole project is visible at once, and a benched agent is in it, marked, rather than gone. A session
is a group with a bar down its left edge, and the bar carries the worst thing happening under it —
error over waiting over running over ended — so a folded session still says it has a failing agent,
the same rule `WorkProjection::pulse` follows for a task's card. Its note line is the title of a task
in that session, read off the work rather than carried on the session, because a session has no
description on the wire. A session with no agents in it is not drawn at all. One click reveals: an
agent in a column comes to the front of it, and a benched one opens a column of its own — or joins
the focused column when the row is already full. The row folds its session, and the header's one
control folds every session or opens every one.

**The orchestration screen is one field of state and everything else.** A selection is either a **session**
— a named piece of work — or an **agent**, which is one workspace: one running harness, one
terminal. Which session the graph draws, what the inspector reports and which tasks the drawer lists
are all functions of that one field, so the three cannot disagree about what the user is looking at.

**What a thing is and where it is drawn are separate, and the graph arranges itself.** No position
is authored anywhere. Position is held apart from the definitions and held relative — a task owns an
origin, an agent owns an offset inside the task it serves. Sessions stack down the canvas, each one
clear of the last; inside a session, containers flow left to right and wrap; inside a container,
cards stack by how far they are from whoever started the work, which draws the three shapes without
naming them: one agent is one card, a chain is a column, a coordinated task is a coordinator over a
row of workers. **An agent nobody gave work to sits above its session's containers**, which is where
an agent coordinating a whole session's work belongs and where one coordinating the project ends up;
that row is stacked by the same rule, because it holds a spawn tree of its own. A card whose parent
is in another container stays on its own container's top row, and the connector is drawn across the
boundary — which is what lets one agent parent every session's master without sinking any of them a
level.
The toolbar's tidy control asks for the arrangement again, discarding every hand-placed position —
the only thing that undoes a drag.

**The graph draws a project's agents as cards on a dotted ground, and opens on all of them.** A
card carries the agent's name, role, state, the one line it says, its branch and its token count, in
the colour of the state's bucket. Zoom scales positions, cards and type together, so the graph reads
the same at every step.

**Two filters narrow it, and both clear.** The session row leads with `all` and then names each
session with the count of agents under it; the four bucket pills — running, waiting, ended, error —
decide which states are drawn. Any pill may be the last one turned off, because **a bucket row with
none lit is not filtering**: the row means "narrow it to these", and narrowing to nothing is what an
untouched row already does. That is what makes an empty canvas honest — it means an empty project,
never a filter the user cannot find their way back out of — and one control at the end of the strip
puts both filters back at once, drawn only while there is something to put back.

**Which session is drawn and which is selected are two questions.** Picking a session from the row
does both, because narrowing to one and looking at it are the same gesture; `all` does neither to the
selection, so "show me all of it" never means "stop looking at this". The inspector and the tasks
drawer follow the selection, and go on reporting one session while every session is on screen.

**A task is an outline round the cards serving it, not a container they sit in.** The dashed box is
computed each frame from where its cards are, so dragging a card takes the outline with it, and a
task with no cards drawn has no box — because nobody serves it, or because a filter hid every card
that does. Its shape — direct, chain or coordinated — is
printed on the outline: whether the agents run in order is a fact about the task, not about any one
of them.

**A card is carried, and dropping it inside another task's outline moves it there.** The card itself
follows the pointer rather than a ghost, and the box it would land in lights up while it is in the
air. Which task it landed in is worked out from where it came to rest, not from what took the drop:
an outline takes no clicks. Where a card sits is the interface's own fact and is written on the spot;
which task it *serves* is the host's, so the drop asks and the outline redraws round the card when
the answer lands — `D41`. A card put down on open ground, or back in the box it came from, asks
nothing and stays where it was let go of.

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

**The composer is real, and nothing answers it.** What is typed goes to the host, which puts it in
the selected agent's thread and answers with the agent carrying it — the line appears because the
host said it did, not because the interface wrote it there. Nothing replies, and the thread says in
as many words that nothing is listening: a fabricated reply is the one thing a screen with a mock
behind it must not draw. Enter sends, Shift-Enter inserts a newline, and the draft is the
orchestration screen's own rather than the chat's or a column's.

**The board and the graph are two views of one set of tasks.** The graph answers "who is doing
what"; the board answers "what is there, and where has it got to" — the same tasks, at the scale of
the project rather than of one session. Nothing is copied between them, and the one set is the
host's, held per project: a task ticked on the board is ticked in the drawer under the graph, and
`Show in graph` is one click because the two screens are two questions about one set of facts.

**A project's sessions and agents are the host's mocks; its tasks are written down.** The mock is
minted per project and made again at every boot, so nothing an agent says outlives the process. A
task belongs to the project instead: it survives the window that made it and the restart after it. A
project's tasks are seeded from the fixture exactly once — an absent store and an empty one are
different things, so a user who deletes every task gets an empty board back rather than the fixture
again — and where that is kept and how is `D39`'s, not this document's.

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
filtered down to the one card just asked for. `New task` cannot select what it asked for, because
the id is the host's to mint: the task that arrives is the one selected.

**A card carries the worst thing happening in its task.** Its left edge is the state the user would
want to be told first: a failed sub-task beats one waiting on a person, which beats one moving. The
line under it names the agent the task speaks through — the coordinator of a coordinated task,
whoever is holding it now for any other shape — and clicking that name opens its conversation. A
task nobody has started says so, and counts its sub-tasks instead.

**The task panel reports one task whole, and edits it in place.** Its session and whether that
session is a worktree, its shape in a sentence, who it speaks through, its description, and every
sub-task with the agent that has it and where that has got to. Ticking one is a change to the task
rather than to the view of it; unticking lands on idle, because nothing here can know what its owner
would go back to doing.

**The form edits everything about a task except where it has got to.** Its title, its description,
its priority, its shape and the session it belongs to, and its sub-tasks — added at the foot of the
list, renamed in place, ticked and removed. Priority and shape are rows of pills, which are the
report and the control at once because each has three fixed values; the session is a picker, because
that list is as long as the project has sessions and it grows. Handing a task to no session at all
is a row in that picker rather than an absence the user has to find the way back to.

**One field is open at a time.** The panel is a report first, and a panel where every field is a
text box has stopped reporting. A field opens on a click and closes on a commit.

**Enter or the ✓ commits, the ✕ discards, and losing focus does neither** — the field stays open.
A blur fires before the click that caused it, so a field that committed on blur could not be
cancelled by the button beside it. Selecting another card discards as well, because the field was
about the card it was typed on.

**An empty title is refused where it was typed and never sent.** It is a slip rather than an
intention — the posture Send takes on an empty draft. A description may be emptied, because clearing
one is a thing to mean. And a value equal to the one the host holds sends nothing at all: the
message set is for acts, and re-asserting a title is not one.

**A status has no control.** The column a card is in is drawn on the panel and not offered there: a
column is a stage, and a card only ever changes column by being moved, so a picker for it would be a
second way to do the one thing the drag is for. `BLOCKED` is derived from the sub-tasks, so there is
nothing to offer there either. This is the one place the panel deliberately stops short of what it
reports.

**Delete asks first, and a sub-task's × does not.** A task is the one thing on the panel that cannot
be retyped, so it takes the question the picker's Forget takes: the first click asks, the second
sends, and any other click on the panel withdraws it. A sub-task's title is one line, so its × goes
straight through.

**A description is Markdown, rendered by default**, with one control that swaps it for the source —
because a description is read far more often than written. The preview sits *inside* edit mode
rather than instead of it, so Save is still there and the draft is not lost, and what it renders is
the draft rather than the record: seeing what has just been typed is the point of the control. A
task with no description says so rather than dropping the section, on the rule the status bar and
the explorer's git marks both follow. On a **card** it is one mark saying a description exists and
nothing more — what a card carries is fixed, and a folded card keeps only its shape, its title and
whose session it is.

**Every change is a message, and the card says it is waiting.** Nothing on either screen writes to
the work: a field sends, the host answers, and the panel goes on reporting the task the host last
confirmed, so a refusal leaves nothing to unwind. A drop asks the same way — the card stays in the
column it came from, drawn muted and saying so, until the answer comes, so a slow host does not read
as a drag that failed. The mark comes off on any answer naming that task, the old column included, so
a refusal cannot leave a card stuck on its way somewhere. Why the interface asks rather than writing
first is the state ownership rule in [`../tech/architecture.md`](../tech/architecture.md).

**A refusal ends whatever asked for it.** What the host would not do is said on the panel, in its
own sentence rather than the project picker's, because a task that would not move is not a fact
about the catalogue and has to be said where the user was looking. It also puts the open field away,
takes the waiting mark off, gives up on selecting a `New task` that never arrived and withdraws an
unanswered delete question — so nothing is left in a state that cannot resolve. The next thing the
host confirms clears the sentence: a report about a change that did not happen is stale the moment
one does.

**Both ways out of a task lead to a screen over the agents, and they lead to different ones.**
`Show in graph` switches to Orchestration and points the graph at whoever is doing the task;
`Open …'s chat` switches to Agents and reveals that agent in a column, because a conversation is what
was asked for. A task the user wants to intervene in is a conversation with an agent, and the
conversation is a column.

**The Git screen is what version control knows, whole.** The refs down the left, the history in the
middle, the uncommitted changes on the right and the comparison under both. It is the same facts
the explorer's badges and the status bar's branch carry, at the size they can be read at: the tree
answers "is this file changed" and this screen answers "what has this repository been doing".

**The screen reads and never writes.** Fetch, pull, push, branch, stash, undo and the commit button
are drawn as the shape the screen will have when there is a write family behind them, and none of
them takes a click; the toolbar carries a `read-only` chip that says why. What is typed into the
commit box is kept with the project, because a message is worth keeping even when nothing can send
it.

**The uncommitted row is the top row of the history.** What is not committed yet is selected the
same way a commit is and is what the screen opens on; picking a commit points the panel beside it
at that commit instead. The panel says what the log said and no more — a commit's own file list
needs a log the git family does not carry.

**The change lists are the pair, not the projection.** A path whose index differs from HEAD is in
the staged list and a path whose worktree differs from the index is in the unstaged one, so a path
that is both is in both — which is exactly what one badge on an explorer row cannot say.
Conflicted paths are a list of their own, drawn first. A row takes the colour the explorer paints
the same path in, so the two never disagree.

**Picking a changed path compares it, and the comparison is the host's.** The pane under the
history draws the hunks `DiffProjectFile` answers with, through the same renderer a diff tab uses.
An unstaged row is compared against the index and a staged or conflicted row against HEAD, because
the file family offers those two and no index-against-HEAD. The pane shuts rather than the history
shrinking, and switches between unified and side by side.

**The history and the refs are fixtures.** The branch list, the remotes, the tags, the stashes, the
submodules and every commit on the screen are invented, because the git family carries no refs list
and no log — `G70`. Everything else on it is answered: the branch, the tracking counts, the
in-progress operation, the totals, the changed paths and the diff. The search over the history and
its `my commits` filter are real controls over the fixture, and both clear together.

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
rather than a tree; the chat is hidden, because a conversation about nothing is a fiction. The `+`
at the end of the pane region's tab strip is not offered, because there is no folder to start a
harness in — but the chevron beside it is, and its menu offers the console: the one panel a window
with no project has a reason to show.

**The explorer, the open files and the terminals belong to a project, not to the window.** A window
holds one set of each per project open in it, and switching between them is a lookup: the tree, the
tabs and which terminal panels are drawn all change together, and nothing is re-read or rebuilt. The
terminals of the projects behind keep running and keep their scrollback.

**A project leaving the window takes its panes with it.** Closing it, moving it to another window, or
forgetting it kills the harnesses running in it — a pane's working directory is that project's
folder, and no other window can adopt a running emulator. What the project *remembers* is written
down first, so reopening it brings its files back.

**The project picker is a small manager, not a list of values.** It searches on name and path, and
divides into three groups, top to bottom: **open in this window**, **open in another window** with
the letter of the window holding each, and **history** — everything open nowhere, with how long ago
it was. A group with no rows is not drawn. A project's row moves between the groups as the project
moves between windows, in every window's picker at once.

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

**A 3-dot next to the title chip opens project settings for the project this window is showing.**
The path stays as it is. Documentation and Integrations are drawn and disabled. Save writes the
name and colour through `UpdateProject`.

**Each group's rows carry the actions that group needs.** In this window: click to point the window
at it, `Close` to close it, `ExternalLink` to send it to a window of its own. In another window:
click to bring that window to the front — which is how the user moves between windows — or
`ArrowLeft` to take the project into this one. In history: click to open it here, or `ExternalLink`
to open it in a new window. Closing a project that still has terminals running turns the row into a
question rather than taking the click.

**The middle of the titlebar is one field for finding and for doing.** File search and commands go
to the same place, marked `⌘K`. The field draws itself with the kit's shared text-entry container —
a surface with a coloured left edge, and a bottom underline while it holds the keyboard. There is
no breadcrumb: the titlebar says which project, the tab strip says which file, and repeating it in
the middle bought nothing.

**The window's body is a dock, and the user arranges it.** Everything between the chrome is a tree
of tabbed groups: dropping a tab in the middle of a group tabs it there, dropping it on a group's
edge makes a row or a column, a divider drags, and a group's zoom control fills the region with
whatever it is displaying and gives it back. The window fixes no arrangement — it draws the
titlebar, the rail, the dock and the status bar, and what is inside the dock is the user's answer.

**Every area in the dock is a panel.** One per pane for the terminals, one per open file in IDE
mode, and one each for the explorer, the chat, the log console — which is
[`logs.md`](./logs.md) — and the centre.

**Placement is a property of the kind of panel, not a special case.** The explorer and the chat sit
in the left or the right region and nowhere else, because an explorer squeezed into the bottom is a
sixty-pixel tree and a chat in a centre column stops being a conversation. A terminal and the
console take the centre or the bottom. The centre panel takes the centre. A panel dropped where its
class forbids is moved back to its home region on the same edit, so the drop reads as refused rather
than half-taken. A file takes the centre, like the centre panel: the open files *are* the centre in
IDE mode, so a file dragged to a border would leave nothing behind it.

**There is no top region.** The dock has a centre and three edges — left, right and bottom — so
"docked above the editor" is a split at the top of the centre rather than a region of its own.

**A panel with nothing to show is hidden, not removed.** It keeps its place in the tree and its tab
slot, and comes back where it was left. The explorer and the chat leave with IDE mode, the chat also
wanting a project; a terminal is hidden while its project is not the one on screen, so its harness
goes on running and keeps its scrollback; a file panel is hidden while its tab is not one the
project on screen holds. The console is always drawn, and **the centre panel steps aside in IDE mode
for as long as a file is open** — the same machinery, which is what brings it back where it was left
when the last tab closes rather than rebuilding it somewhere else.

**The titlebar's switches open and close the dock's edge regions**, and read the dock rather than a
flag beside it — so a region the user emptied by dragging its last panel out reads as closed. The
left and right switches are drawn only in IDE mode, because the side regions are IDE furniture and
hide in place in every other mode; the bottom switch is offered in every rail mode. **Opening the pane region with nothing in it starts a pane**: the region exists to hold them, it
opens empty, and a switch that gave the user a bar of nothing would not have answered what was
asked. The `+` that opens another one sits at the right end of that region's tab strip, drawn in a
group that holds a terminal or the console, in the pane region even when it holds neither, and only
while a project is open — because a pane runs in a project's folder. Beside it is a chevron, drawn
whether or not a project is, which opens the menu of what else can be reached here: the shells this
machine has, and a row that puts the console on screen. What the rows are and what a click does is
`feat-panes`'s. Past a divider, the titlebar offers search and a bell (both stubs), a gear that
raises application settings, and the theme switch. The gear lights while settings are up. Settings
are interface-wide, so the overlay opens with no project.

**Application settings is a page overlay, not a one-question modal.** It is `SETTINGS_WIDTH` by
`SETTINGS_HEIGHT`, clamped to the viewport, with a left nav and a scrolling body; switching
sections does not resize the panel. Toggles persist as they are flipped — there is no Save. Opening
it dismisses project settings, and the reverse. Three sections ship: **File explorer** (whether a
single click opens a preview tab), **Editor** (whether a new markdown file opens in preview or
source), and **Harnesses** (whether an agent is confined to its project, over the accounts
registered here and an Add button that signs a new one in). The kitchen sink still draws the larger
fixture nav; that page is how the furniture is looked at, not how the application is configured.

**An account row is a name and the harnesses it can start.** That is the whole of what the
interface knows about an identity: no credential and no path reaches it, because neither crosses
the bus. A row with nothing beside it says "not signed in" rather than nothing at all — an account
can reference an environment variable instead of a captured session, and the two are different
answers. The list is asked for on every open, so an account signed in from elsewhere appears
without a restart.

**Signing in is a modal with a real terminal in it, because the harness runs its own login.** Add
harness asks two things — which harness, and what to call the identity — and then the harness's own
flow runs in a pane inside the modal, browser round-trip included. A modal rather than a tab on
purpose: an OAuth flow wants the whole of the user's attention for the half-minute it takes, and a
login that scrolled away behind a pane is a login nobody finishes. Abort is always available and
always safe — a flow that wrote no credential captured nothing, and the host says so rather than
recording a half-made account, so starting again is free.

The login modal is painted from the window root rather than from the settings page that raised it,
for the reason every overlay there is: a login outlives the page. Closing settings mid-flow must
not take the harness's sign-in with it. Its pane belongs to no project and gets no dock panel — the
modal is the only thing that draws it, which is also what keeps one emulator from being rendered in
two places at once.

**The isolation toggle is the one setting the host acts on**, so it is the only row that writes the
Host layer rather than the interface's own — an agent runs under a policy, and the half that spawns
the pane is the half that has to know. Every other row is a `UiSettings` field. Which harnesses opt
out of confinement is not here at all: that belongs to the harness library, which already has the
shape for it. See [`../tech/agent-manager.md`](../tech/agent-manager.md).

**The explorer draws the project's folder, one directory at a time, in the same two arrangements
the file picker uses.** Opening a project asks the host for its top level; expanding a folder asks
for that folder's children, and only then. A repository's `node_modules` is therefore one row
rather than a walk, and a tree the user never opens costs nothing. The tree is the folders that
have been opened, indented; the list is every match the host has already named, flat and sorted by
name without case, each row saying which folder it came from. One filter field sits over both and
what was typed survives the toggle. A filter finds rather than prunes: every folder already listed
is walked while one is typed, and a folder with nothing matching under it drops out instead of
drawing as empty. **The listings are cached in the window, filled in the background when the
project opens**, so a search reads what is already named rather than waiting on the host. The walk's
skip set is left alone — `node_modules` stays one row — and a cached listing does not expand the
tree: collapsing is still not forgetting, and an unfiltered tree still shows only what the user
opened. **A filter is debounced and walked off the frame, on a background thread.** Typing a letter does
not clone or walk the cache on the keystroke: the field keeps the draft, the last result stays on
screen, and after a short pause one snapshot — an `Arc` of the tree, not a copy — is walked on the
background executor. Clearing the field is immediate. Clicking a folder in
the tree expands it; clicking a file opens it; a folder in the list is only where the cursor lands.

**The explorer is worked from the keyboard, and the field keeps it the whole time.** The keys that
drive the rows are bound against the field as well as against the panel, so nothing has to be
tabbed to. `up` and `down` move a cursor bar through the rows and stop at the ends; `right` opens
the folder the cursor is on and then steps into it, `left` shuts it and then steps out to the
folder holding it; `enter` opens a file in a temporary preview and toggles a folder, `shift-enter`
opens the file permanently; `escape` closes the right-click menu,
then clears the filter, then is handed back. What the panel has no answer for it hands back, which
is how `left` and `right` are the field's own caret keys again in the flat list. The cursor is not
the open file: the accent is the file that is open, the keyboard's bar is only where the next key
lands.

**A right-click on a row raises a menu at the pointer.** A file offers Open, Open diff vs HEAD, Copy
path, Copy full path, Open in Finder and prepares Rename and Delete; a folder offers Expand or
Collapse, Copy path, Copy full path, Open in Finder, Refresh and prepares New file, New folder, Rename
and Delete; a click on the empty panel offers Collapse all and prepares New file and New folder. The
four that create or remove a path are drawn and do nothing: nothing on the bus creates or removes a
path yet, and a menu that hid those rows would have nowhere to put them when the host grows the
family. `Open in Finder` — Explorer or File Manager on other platforms — reveals the file or its
folder in the system's file manager, and `Refresh` asks the host to list that folder again. The menu
is the window's one open menu, dismissed by a click outside it or by escape.

**A row the host will not follow is drawn and does nothing.** A symlink leading out of the project or
nowhere, a socket, a device, a pipe: the row appears, faint, and takes no click. Drawing it is the
point — a tree with rows missing is a tree that lies about what is in the folder.

**The explorer holds project-relative paths and nothing else.** No absolute path reaches the
interface, for the same reason no file descriptor does: the folder the tree describes is the host's,
and the two need not be on one machine.

**The explorer states git position by colour and by badge.** Modified, untracked, conflicted, staged
and ignored each take a colour from the status group and a single-letter badge, the colour so it
reads at a glance and the badge so it does not rely on colour alone. The badge sits at the row's
right edge, aligned under its fellows by a spacer after the name, with no separate status dot — the
badge already says with colour whatever a dot would. The host's working-tree map
fills those in: a path in the map gets a status, a path not in it is clean, and until a map has
arrived every row is unmarked because nothing has been read. An untracked or ignored directory
paints every child the same, because git does not look inside and a child not in the map is not
clean. Clean and unread draw the same on the row; the status bar's branch is how a repository is
known. A project that is not a repository draws no badges and no branch.

**A tab exists from the click that asked for the file.** It appears at once, says it is reading, and
fills when the bytes arrive — so a click has an effect, a second click cannot ask for the same file
twice, and a read that fails has somewhere to say so. Bytes that arrive for a project the window has
since switched away from are still put in their tab; bytes for a tab that has been closed are
dropped.

**Each open file owns its buffer.** Switching tabs and switching projects both leave a buffer exactly
as it was, with its undo history, its selection and its scroll — nothing is copied in or out. The
active tab is marked on its bottom edge. The tab's title takes its colour from the repository, the
same map the explorer draws on, and a small dot at the tab's right edge reports the *file*: reading,
saving, a failed save, or an unsaved edit. A clean, idle tab draws no dot.

**A file opens in a temporary preview on a single click, permanently on a double-click**, when
*Open files in previews* is on — which is the default, and a setting. Off, a single click and
`enter` open permanently too. `shift-enter` opens permanently either way; `shift` or `cmd` with a
click, and a double-click — on the explorer row *or on the tab itself* — open permanently. Only one
preview tab exists at a time — opening another preview replaces it and closes its panel. A preview
is promoted to permanent by its first edit, or by the explicit gesture that opens it permanently; a
single click that merely brings the tab to the front does not promote it. A preview tab is drawn in
italics with a faint background so it reads as tentative at a glance.

**Every open file in a project wraps together.** Whether the file editors soft-wrap long lines is a
project's preference, written into the same view prefs as its font size, and one flip brings every
already-open buffer into line rather than waiting for a reopen. A buffer with no wrap preference
uses its editor's own default, which is to wrap.

**With no file open, the centre shows the brand, big and soft.** The page that is about to hold a
file says so without furniture — no button, no menu, just Ubiq's own mark at 200px and half opacity
on the window's ground. The mark is theme picked, the blue on a light palette and the white on a
dark one, exactly as the rail's mark is on its swatch.

**Each open file is a panel, so its tab is the dock's.** There is no tab strip of the editor's own:
a file's tab belongs to the group it sits in, which is what lets a file be dragged beside another
into a row, a column or another group — two files side by side is a drag rather than a mode. Making
a file's tab the displayed one is what makes it the active file, and closing it is what closes the
tab — and the keyboard moves with it, so clicking a file in the explorer or on a tab puts the caret
in that file's buffer.

**A row of tabs scrolls when it runs out of room.** Too many tabs for the strip squeeze into a
scrollable band rather than shrinking the tabs past readability; the active tab scrolls into view,
and chevrons at the strip's ends nudge the band by a step — always present, so an overflowed strip
can be told to be pushable, and a no-op when the strip has nothing more to show. The scroll offsets
only the tabs, so the strip's chrome — the active tab's underline, the `+` that opens a terminal —
stays put.

**A tab says what it is looking at.** A file's change against a version-control base is a tab of its
own beside the file rather than something the file's tab switches into, so opening a comparison
never takes over what is being read or edited. The tab is named for the file with what it is looking
at after it, and what identifies it — in the saved arrangement and in what the project remembers —
is that pair rather than the path alone.

**A viewer is a pure function of bytes and a kind.** What draws a file is chosen once from its
extension when the tab opens, and nothing about it looks at a path again: `.md` gets the Markdown
view, `.mmd` and `.mermaid` the diagram, `.excalidraw` the scene, the image extensions the picture
itself, and everything else the highlighted buffer — which is the general case rather than a
fallback. A comparison against version control is not a viewer kind: it is what the tab is *looking
at*, so a diff opens beside the file rather than inside it.

**A viewer with a source to show has a three-way toggle, and it persists.** Source, Preview or
Split, in a strip above the body, drawn only by the three viewers that have both halves — the
buffer has nothing to toggle to and an image has no source. Split shows the file's own buffer, not a
copy of it, so switching costs nothing and loses no undo history. Which of the three is on screen
belongs to the file rather than to the strip, and it is written into the saved arrangement and into
what the project remembers, so a document reopens in the layout it was left in. A new markdown file
opens in Preview or Source as the Editor setting says; mermaid still opens in Preview. Already-open
tabs keep the layout they were left in.

**A Markdown preview scrolls, and YAML frontmatter is a bar above it rather than part of it.** A
document that opens with a `---` block draws it as a collapsible bar at the head of the preview —
collapsed to its first few field names, expanded to the raw YAML — and the bar keeps its height
while the document below it takes the rest of the panel and scrolls inside it. Whether the bar is
open belongs to the file, like the layout toggle. A document without frontmatter is the same view
without the bar.

**A diagram is drawn in the interface, on a background thread.** A Mermaid document is just text;
the bus already carries a file's bytes, so nothing about a diagram crosses it. The window renders it
with a Mermaid implementation of its own, and it renders it **off the frame thread**: layout is
superlinear and a large graph has been measured at two seconds, which would be two seconds of dead
keystrokes in every pane. A diagram a frame found it needed is queued while the frame is built,
handed to the background executor once it is done, and lands as an update on the next frame after
that. Until then the panel says it is drawing, and the window stays live.

**A drawn diagram is cached twice: in the window, and in the project's workarea.** The key is the
source's hash, the palette, and a marker for the renderer's version — the palette because the
renderer bakes its colours into the markup, so light and dark are two pictures rather than one
recoloured. The workarea is a directory the host reserves for the interface and never reads inside,
and it arrives on every project message as an absolute path rather than being composed from the
config root, which is what makes a host on another machine a change of value rather than of code.
**The cache is disposable**: deleting the workarea costs re-renders and loses nothing.

**A diagram or a scene in a panel can be panned and zoomed.** It opens fitted to the panel, aspect
ratio preserved, with a margin. The wheel zooms about the pointer, a drag pans, a double-click or a
pinch-out to the floor restores the fit. The camera belongs to the tab, not the file, and is not
written down. A fence inside a Markdown document is not a panel: it is drawn at the picture's own
size and the document is what scrolls.

**An Excalidraw scene sits on Excalidraw's own white canvas.** A file that names a canvas colour
keeps it; a file that names none — `transparent`, an absent key — gets the format's default white
rather than the window's ground, so a diagram drawn on paper stays on paper in a dark window. The
scene is painted into a canvas that fills the panel; a canvas that only laid out to its content
would draw the whole scene into a few pixels at the top of the pane.

**A fenced diagram in a Markdown document goes through the same renderer.** A ```` ```mermaid ````
fence is drawn by the diagram viewer and a ```` ```excalidraw ```` one by the scene viewer — one
renderer per format, two call sites for each. A Mermaid fence resolves against the same cache the
panel uses, so a document with several fences fills in as each of them lands rather than waiting for
all of them. A picture is drawn at its own size, which the renderer reads out of the SVG's
`viewBox`, rather than stretched to whatever box it landed in.

**`⌘S` writes the active file back, and names the version it read.** A save the host refuses because
the file moved under it is reported on the tab and in the status bar, and the file is left alone —
Ubiq is not the only thing editing these files, and the agents in the panes are the other one. There
is no merge: resolving a conflict is the user's.

**A file that cannot be edited honestly is not offered for saving.** A read the host cut short at its
byte ceiling is readable and unsavable, because writing a prefix back would shorten the file. A file
whose bytes are not text says so instead of drawing them.

**A dirty tab asks before closing.** The first click on its × turns the tab into a question; only a
second one discards the edit. Bringing the tab forward again withdraws the question. The panel comes
back to its group to ask, because the dock takes a closed tab out before the window hears about it.

**A right-click on a file tab raises a menu over the window.** Close, Close Others, Close Left, Close
Right, Close All, Copy Full Path, Open in Finder, Save and Word Wrap — the two *closes* and the
surround closes anchoring on the tab that was clicked, Copy Full Path copying the file's project
path to the clipboard, Open in Finder revealing it (or its folder) in the system's file manager,
Save writing the file behind one tab rather than only the active one, and a dirty
tab in a bulk close still asked for rather than silently closed. The menu is the window's one open
menu, painted at the window root because the dock's skin cannot name `AppState`, and it is dismissed
by a click outside it or by escape.

**A project's text size is one knob, and the status bar holds it.** A dropdown at the status bar's
right scales the file editor, the terminal panes and the explorer tree together — `FONT_SIZES`, a
hand-picked ladder of point sizes rather than every integer, because a size is chosen by eye. The
size is remembered with the project, so a zoom survives a restart. `cmd-=` and `cmd-shift-=` zoom in
and `cmd--` zoom out by whole points, within the range the chrome admits; the dropdown shows the
nearest ladder entry when a nudge lands between them.

**An explorer row is sized from its text, not from a constant.** The row's height and the tree's
per-level indent are both derived from the size the row draws at — `kit::row_height()` and
`kit::row_indent()`, floored so the twisty and the kind icon never touch the edges and capped so the
tree does not become a column of buttons at the top of the range. A zoom therefore makes the tree
taller as well as larger, and a small size gives a genuinely denser list rather than small text in
the old box. The file picker and the ref list draw the same chrome at `kit::ROW_FONT`, because they
are dialogs rather than a project's workspace and no project's zoom reaches them.

**The status bar reports facts, not intentions, and an absent fact is drawn as absent.** It reports
on whatever is on screen, so the rail mode decides which set of facts it has. In IDE mode with a
project open: the active file's project-relative path, what its save is doing when that is worth a
word, the project's branch (or detached short id, or unborn name) with ahead/behind and working-tree
totals when those exist, the caret's real one-based line and column, the file's language, encoding
and line ending, the harness and mode the composer is set to, and the active file's text size. A project that is not a repository
prints nothing git-related, and a branch with no upstream draws no `0/0`. The caret and the language
go with the file, so a window with no file open reports neither rather than a position in nothing.
With no project open it says so and stops. On the two screens over the agents there is no file and
no caret to report, so it counts instead. On the orchestration screen: how many sessions and agents
there are, and how the agents are spread across the four states, each count in its state's colour. A
count of zero is drawn as zero rather than dropped — "no agent is failing" is a fact, and it is the
one the user is checking for. On the agents screen it reports on the field rather than on the
project: how many columns there are, how many agents they hold, how many of them are grouped, how
many are on the bench, and the same four states over the agents in those columns. The strip reports
on what is on screen, and the bench is exactly the difference between the field and what the host
reports. At the right it names each harness behind the columns once, which is the one fact about
them the columns' own footers say only one at a time. On the board it counts the work instead: how
many cards are in each column, how many sub-tasks are done across the cards on screen, and how many
of them nobody can finish without the user — over the cards the filters leave, because a count that
disagrees with what is drawn is worse than none. Whichever set it is showing, it says where
Ubiq is writing whenever that is not the usual `~/.config/ubiq` — a config root you cannot see is a
foot-gun.

**The window remembers the arrangement it was left in, and which files were open in it.** The whole
dock as it serialises itself — the tree, the axes, the sizes and which tab of each group was
displayed — the rail mode, the files open in the centre with which of them was in front, the folders
the explorer had expanded and the row it had selected all belong to a project; the palette belongs
to the interface. Both are stored by the host, which keeps them as an opaque blob it never reads, so
the schema stays the interface's own. The arrangement is remembered **per rail mode**: each mode
keeps its own record of which regions were on screen and its own dock blob, so the IDE's side
panels are not undone by a visit to the sink, and a visit back does not summon them where the sink
was arranging. A mode with no record of its own opens on its defaults — the IDE with its two side
regions on and the pane region put away, every other mode with all three off — and **so does a
project that has never been arranged in the mode it is entered in**, so a project reached from
another one does not inherit that one's edges and then have them written down as its own. A blob it
cannot parse is discarded and the window opens on defaults. Which mode the window is in is written
down when the mode is chosen rather than when the arrangement next changes, so two modes that
arrange nothing between them still reopen in the right one.

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
holds that the restored arrangement does not name is put back in its home region rather than lost.

**A panel writes down what it is looking at, not what it drew.** A file panel carries its tab and
the layout its viewer was left in, a terminal panel carries its pane's id, and nothing else — never
a parsed scene, a computed diff or a rendered diagram. Each of those is a function of bytes: the host will send the file again, and the
scene, the diff and the picture are made from it again, the last of them off the workarea's cache. Every other panel is
its name and nothing more.

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

**The kitchen sink is the application's own test bench, and the one screen with nothing behind it.**
It is under `APP` because it is about Ubiq rather than about a folder: it opens on a first run with an
empty catalogue, it asks the host for nothing, and it looks the same in every window. Eight pages,
selected by a strip along the top — the plain buffer, one per special viewer, the style reference,
the file picker, then the two settings layouts composed from the kit.

**Its documents are fixtures, drawn by the viewer their name implies, and a fixture is not a file.**
A page's document is a name and a constant, and the name carries an extension, so the viewer and the
highlighter are picked by exactly the rule a real path goes through rather than by a second table
that could disagree with it. The Markdown fixture carries a fence of each diagram kind, which is how
the fence renderers are exercised without opening a file, and what a viewer is handed is the
**buffer's** current text rather than the constant, so typing in the source half of a split redraws
the drawn half. But a fixture has no path, no version and nothing to save, so no tab, no dirty mark
and no save state is drawn for one: all the sink keeps per document is which of its viewer's layouts
is on screen, toggled by the same three pills a file panel offers.

**The style reference draws every token and every primitive, and it is where one with no other call
site gets one.** Each specimen carries the name a call site reaches it by, so a token whose value is
wrong in one palette, a control whose off state reads as absent, or a surface whose coloured edge
floats inside its container shows up here before it shows up on a screen. Its controls are wired to
real state, because a control that cannot hold a value is not being tested — one value drives the
stepper, the meter and the ring — and nothing they hold means anything.

**Its modals are the window's only ones, and nothing behind them happens.** Three shapes — a
question, a form, and something irreversible — told apart by what their edge says and by the colour
of the confirming button. Both buttons dismiss and neither claims anything: a fixture that pretended
to close a pane would be the one thing a screen with nothing behind it must not draw. The shape a
modal is drawn in belongs to the UI-and-design document, linked below, with every other surface's.

**The settings pages are layouts, not a settings screen.** Application settings is a left nav of
kit rows — Appearance, Harnesses, Agent defaults, and the three quieter destinations — and a body
of the same controls the style reference already draws: `choice_pill` for a pinned theme or a
density, `check_box` for a boolean, `stepper` and `meter` for a number, `card` for a permission
mode, `Picker` for a dropdown, `slab` for a harness that opens. Project settings is that same
furniture in the shape of a dialog: a coloured left edge, a nav, a form. On the sink, Cancel puts
the fixture back and Save writes nothing, because the sink has no project behind it. Over the
workbench the same dialog is the create and edit surface: only General is enabled, the path is
immutable, Create sends `AddProject`, and Save sends `UpdateProject`. Its colour row is a strip of
swatches plus a picker — saturation/value, a hue bar, and a `#RRGGBB` field — so a custom colour is
chosen rather than only indexed. A field that holds the keyboard is underlined on its bottom edge
as well as marked on the left.

**The picker page raises the file picker in each of the shapes a screen can ask for one.** The picker
takes a request — files or folders, one answer or several, the folder it is rooted at, a prefilter
like `*.md`, whether a single pick is final on the click or on the button, and whether it holds the
window or goes away on an outside click — and the page draws those seven fields as pill rows over a
button that raises the dialog out of them. The tree it opens over is a fixture like every other page's,
because the sink has no project behind it, and what the dialog handed back is printed under the
button: a cancelled dialog and one that answered with nothing are different answers, and the readout
says which it was. The dialog itself belongs to the window rather than to the page — exactly one may
be up, whichever screen asked — so the answer is routed back to whoever raised it rather than
returned.

**The dialog is worked from the keyboard, and the field keeps it the whole time.** It opens with the
focus in the filter — typing a name is the first thing a picker is for — and the keys that drive the
rows are bound against the field as well as against the dialog, so nothing has to be tabbed to.
`up` and `down` move a cursor bar through the rows and stop at the ends; `right` opens the folder the
cursor is on and then steps into it, `left` shuts it and then steps out to the folder holding it;
`enter` ticks the row where several may be chosen and *is the answer* where one was asked for;
`secondary-enter` — cmd on macOS, ctrl elsewhere — hands back what has been ticked; `escape` closes
the dialog with nothing. What the dialog has no answer for it hands back, which is how `left` and
`right` are the field's own caret keys again in the flat list.

**The cursor is not the selection, and the two are drawn apart.** The accent is what will come back;
the keyboard's bar is only where the next key lands, in `selected` with a `border_focus` edge. A row
that is both keeps the accent fill and takes the focus edge — what a dialog hands over outranks where
its cursor happens to be. The cursor follows the mouse too, so an arrow after a click carries on from
the row that was clicked, and a row it is moved onto is scrolled into view.

**Both arrangements are the same set, and which one is on screen is the user's.** The tree is the
folders that have been opened, indented, each file reporting its size; the list is every match under
the root, flat and sorted by name without case, each row saying which folder it came from. One filter
field sits over both and what was typed survives the toggle, because a user who cannot find something
in the tree switches to the list to look for the same thing. A filter finds rather than prunes: every
folder is walked while one is typed, and a folder with nothing matching under it drops out instead of
drawing as an empty row. A folders-only picker draws no files at all, and the prefilter never hides a
folder — a folder it hid would take the files under it with it.

## Contract

**Projects cross the bus.** The catalogue belongs to the host, and the workbench holds a projection
of it: `ListProjects`, `AddProject`, `ForgetProject`, `UpdateProject`, `LocateProject`,
`OpenedProject` and `RefreshProject` going out, `ProjectList`, `ProjectAdded`, `ProjectChanged`,
`ProjectForgotten` and `ProjectError` coming back, `GetPreferences`/`SetPreferences` behind
everything the window remembers, and `GetSettings`/`SetSettings` behind how it behaves. A chosen folder reaches the host as a path in `AddProject` or
`LocateProject`; Add also carries the name and colour from project settings. The choosing itself is
the platform's, and crosses nothing. The full family is
[`../tech/transport-contract.md`](../tech/transport-contract.md).

**Files cross the bus too.** The explorer and the editor are projections of the host's answers, not
state of their own: `ProjectTree`, `ReadProjectFile` and `WriteProjectFile` going out, and
`ProjectTreeListing`, `ProjectFileContents`, `ProjectFileWritten` and `ProjectFileError` coming back.
Every one of them names a project and a path inside it, and each answers only the window that asked.
The blob behind what a project remembers grows the arrangement, the open files, the active one, the
expanded folders and the selected row, and the host neither parses nor validates any of it.

**The work crosses the bus as well, and every message names a project.** Going out: `ListWork`,
`CreateTask`, `UpdateTask`, `MoveTask`, `AssignTask`, `DeleteTask`, `AddStep`, `RenameStep`,
`RemoveStep`, `MoveStep`, `ToggleStep`, `AssignAgent` and `SendToAgent`. Coming back: `WorkList`,
`TaskCreated`, `TaskChanged`, `TaskDeleted`, `AgentChanged` and `WorkError`. A project is open in one
window at a time, so each answer reaches only the window that asked, and the three screens over the
work draw from the same projection of it. What no message carries is the arrangement over the
records — which column an agent's conversation is drawn in, and where a card sits. The full family, with its payloads and its rules, is
[`../tech/transport-contract.md`](../tech/transport-contract.md).

**A live conversation is a family of its own, and every message in it names an agent.** Going out:
`StartConversation`, `PromptAgent`, `CancelTurn`, `AnswerPermission`, `SetAgentConfig` and
`EndConversation`, with `ListAgentTypes` behind the `New agent` menu. Coming back:
`ConversationStarted`, `ConversationUpdate`, `ConversationEnded` and `ConversationError`. An update
is a delta rather than a record, so the transcript is a fold the window keeps and the host never
re-sends; the family's payloads and its ordering rule belong to the transport contract.

One fixture is left. `crates/ubiq/src/state/sample.rs` holds the chat, the one screen with no
transport family behind it: its composer sends to nothing and its reply is canned, which is
[`chat.md`](./chat.md)'s. The terminals have a family of their own, in
[`panes-and-terminals.md`](./panes-and-terminals.md).

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
| Chat | `ui/chat/` | Edge | Right, at `CHAT_WIDTH` | `ChatState` |
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
| Orchestration screen | `ui/orchestration/mod.rs` | The centre panel in Orchestration mode | Fills it; its toolbar takes `TITLEBAR_HEIGHT` | `GraphView`, over the project's `WorkProjection` |
| Orchestration graph | `ui/orchestration/graph.rs` | The orchestration screen, beside the inspector | Grows; scrolls to the extent of its cards | `GraphView` and its `Layout` over the same projection, and `CARD_WIDTH`/`CARD_HEIGHT` in `state/layout.rs` |
| Inspector | `ui/orchestration/inspector.rs` | The orchestration screen, right | `INSPECTOR_WIDTH`, fixed | `GraphView::selection`, and `agent_input` on `AppState` |
| Tasks drawer | `ui/orchestration/tasks.rs` | The orchestration screen, under the graph | `TASKS_HEIGHT` open, its header shut | `GraphView::tasks_open` |
| Tasks board | `ui/board/mod.rs` | The centre panel in Tasks mode | Fills it; its columns scroll sideways | `BoardState` over the project's `WorkProjection`, and `COLUMN_WIDTH`/`COLUMN_SHUT` |
| Task panel | `ui/board/detail.rs` | The board, right | `TASK_PANEL_WIDTH`, fixed | `BoardState::selected`, `show_detail` and `editing`, and the window's four form entities |
| Kitchen sink | `ui/sink/mod.rs` | The centre panel in Sink mode, project or no project | Fills it; its page strip takes the tab strip's own height | `SinkState`, on the window rather than on a project |
| Sink documents | `ui/sink/docs.rs` | The kitchen sink, on four of its eight pages | Fills it | The fixture in `state/sink.rs` its page names, and the window's buffer for it |
| Style reference | `ui/sink/style.rs` | The kitchen sink, on its fifth page | Fills it; scrolls | `SinkState`, and the theme itself |
| Picker page | `ui/sink/files.rs` | The kitchen sink, on its sixth page | Fills it; scrolls | `SinkState::picker`, and the fixture tree in `state/sink.rs` |
| Settings | `ui/sink/settings.rs` | The kitchen sink, on its seventh page | Fills it; nav plus a scrolling body | `SinkState::settings`, and the window's settings fields |
| Project settings | `ui/sink/project.rs` | The kitchen sink, on its eighth page | A dialog-shaped panel in the page | `SinkState::project`, and the window's project-name fields |
| File picker | `ui/file_picker.rs` | Over the whole window, wherever it was raised | `DEFAULT_WIDTH` by `DEFAULT_HEIGHT`, resized from its corner grip and floored at `MIN_WIDTH`/`MIN_HEIGHT` | `AppState::file_picker`, and the window's `picker_filter` |
| Empty page | `ui/empty.rs` | The centre panel in `Control` and `KB` mode, and with no project open | Fills it | `RailMode`, or nothing at all |

Two rules hold across the three tables. **The chrome does not move and the panels do** — the
titlebar, the rail and the status bar each take one fixed constant and are the frame the dock is
drawn inside, while a region opens at `EXPLORER_WIDTH`, `CHAT_WIDTH` or `DOCK_HEIGHT` and keeps
whatever the user drags it to from then on. And **a screen's furniture is the screen's**: the
orchestration screen's inspector and drawer, the agents screen's sidebar and the board's task panel
take one fixed constant each, are shown and
hidden from the screen they belong to rather than from the titlebar's switches, and leave with the
mode.

To add a panel: a `PanelKind` variant with its class, its home, its permanent name and its rule in
`is_drawn()`, an arm in `ui::dock::body`, and the area module under `ui/`. One that a saved
arrangement cannot rebuild from its name alone — as a file cannot, every file panel answering
`ubiq.file` — also writes a payload in `dump()` and is read back out of it in `rebuild()`. To add a
rail mode: a variant on `RailMode`, its group, label, note and icon, and the arm in `ui::dock`'s
`centre()` that says what the centre panel draws — and, for a mode under `APP`, an arm that answers
before `centre()`'s no-project case rather than after it.

## What a window owns

A window is one `AppState`, and inside it one `OpenProject` per project open in that window. The
split between them is the feature's spine. **A project owns what is about that project** — its
explorer tree, its open files and their buffers, its panes, which of them holds the keyboard, the
furniture it was last left in, the work the host last described to it, the repository as the host
last described it, and the graph's, the columns', the board's and the Git screen's own views of
those. **The window owns what is about the window** — the dock and one
panel per kind in it, the palette, the chat, the log console, and one flat map from pane id to
emulator, because an emulator does not care which list draws it.

The task panel's four fields, the board's filter and the Git screen's search and commit box sit
across that line: the entities are the
**window's**, because a text field is a component the window builds and there is one of each, while
the text in them is the **project's**, because it is about the task that project has open. So a
project switch refills them from whichever project came forward. That is the `file_filter` precedent
one step on — there both the field and what was typed into it are the window's; here only the field
is.

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

`AppState` in `crates/ubiq/src/app.rs` is the root view. It owns the window's own state — the dock
and the panels in it, the chat, the console, the emulators, the component library's `TextareaState`
and `InputState` entities and the subscriptions that keep them mirrored — and a map of `OpenProject`
keyed by `ProjectId` holding everything that belongs to a project. Every mutator ends in
`cx.notify()`.

`sync_projects()` is the one place the map is reconciled against the registry, and it is idempotent:
it drops the projects the window no longer holds through `drop_project()`, builds an `OpenProject` for
each new one, and calls `enter_project()` when the active project changed. It runs from the
`observe_global` subscription rather than from each call site, so a project taken by *another* window
reaches this one down the same path as a local change. `enter_project()` is where a project's
furniture reaches the window — the rail mode it was left in and that mode's arrangement — and it
starts no pane, which is why a window opening on nothing starts no harness.

Accessors read through the active project and tolerate its absence: `open_project()`, `explorer()`,
`editor()`, `work()`, `agents()`, `graph()`, `board()`, `panes()` and `focused_pane()` each answer
for a window with no project without a caller having to check, and `work_mut()`, `agents_mut()`,
`graph_mut()` and `board_mut()` are the writing twins of the four over the work. `drop_project()` writes the project's blob, parks a copy against a
reopen in the same session, and kills its panes. `ui_font_size_or_default()` is the project's live
text size (or `theme::EDITOR_FONT_SIZE`), and `set_ui_font_size()` and `nudge_ui_font_size()` are
the two ways it changes — the former reconfiguring every already-open emulator through
`ui::terminal::config()` as well as writing the value down, so a zoom reaches panes that are on
screen. `toggle_editor_wrap()` flips a project's wrap and brings every open buffer into line, and
`remember()` writes the explorer's filter down with the rest of the view prefs. The file-tab
context menu's handlers are `open_file_tab_menu()`, `pick_file_tab_menu()`, the four
`close_editor_tabs*` helpers, `save_file()` and `dismiss_file_tab_menu()`.

`state/git.rs` is the Git screen's view of one project's repository, held on `OpenProject` beside
the graph's and the board's: which sidebar sections are shut, which ref and which commit are
selected, what is typed in the search and the commit box, which changed path the diff is about and
what it is compared against. It holds no working-tree records — `staged()`, `unstaged()` and
`conflicted()` take the `GitEntry` pairs the host sent, which `OpenProject::git_entries` keeps whole
beside the projection the explorer got — and `settle()` drops a selection whose path has gone
clean.
`Side::base()` is where a list's comparison base is decided, and `RefRow` and `CommitRow` are the
fixtures `state/sample.rs` seeds, which is the whole of what the screen invents. Its four widths and
the graph's lane pitch are constants there, the way the board's and the columns' are theirs.

`ui/git/` draws it, one file per area: `mod.rs` is the frame and the toolbar, `refs.rs` the
sections and their rows — the file list's own row chrome, so a ref reads the way a path does —
`history.rs` the search, the lanes and the commits, `changes.rs` the three lists and the commit
box, and `diff.rs` the pane under both, which hands the hunks to `ui/viewer/diff.rs` rather than
drawing them again. The toolbar's branch wording is `ui/status_bar.rs`'s `operation_label()` and
`capped()`, shared so the strip and the screen cannot say different things about one repository.

`AppState` holds the screen's two text entities — `git_search` and `git_message` — mirroring them
into the project's view through subscriptions, with `sync_git_fields()` filling them back on the
frame after a project swings in, on the explorer filter's rule. `git_view()`, `git_view_mut()` and
`git_entries()` are the accessors; `select_git_path()` is the one mutator that sends anything, and
it sends `DiffProjectFile` only when the selection actually moved. `refresh_git()` asks for the
overview and the working tree together. `ProjectFileDiffed` feeds whichever of the screen and a diff
tab was waiting on that path and base. The whole of it is tested without a frame in
`crates/ubiq/tests/git.rs`.

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
state. `groups` computes the picker's three lists for one window, and `window_count` is what
`AppState::several_windows()` asks before drawing a letter anywhere. Every `AppState` subscribes with `observe_global`, so a move in one window
redraws the picker in all of them, and reads go through `WindowRegistry::read` rather than
`default_global`, which would notify the observers on a plain read and spin the frame. The registry
is pure logic and is tested without a frame in `crates/ubiq/tests/windows.rs`, which seeds it the
way the host does.

`state/when.rs` renders a row's relative time at draw time from `last_opened_at`, and
`state/prefs.rs` is the schema inside the opaque blob the host stores — one `ModeLayout` per rail
mode in the `ViewPrefs::modes` map, each carrying that mode's region flags and a dock blob of its
own, beside the files and folders a project reopens with, the point size its text is read at
(`ui_font_size`), whether its editors wrap (`editor_wrap`) and the text in its explorer's filter
(`file_filter`) — each new field `#[serde(default)]`, so a field costs the schema nothing. The
number is `3`, because one value in the blob carries a meaning that moves with the build: a
`rail_mode` of `Agents` names the column screen, and an older blob wrote it for the graph. That is
the one case a default cannot rescue — nothing is missing, and the value means something else — so
the blob is discarded whole, and `ui::dock::LAYOUT_VERSION` follows the number so the arrangement
inside it goes with it.

`crates/ubiq/src/ui/shell.rs` assembles the frame and nothing more: the mark and the titlebar in one
row, then the rail beside `AppState::dock()`, then the status bar, and — when one is up — the
project-settings overlay, the application-settings overlay, the login modal, or one of the window's
own menus over all of it: the file-tab menu, the new-pane menu and the new-agent menu. Those last
three are painted here rather than by the surface that opened them, because more than one surface
opens them and what there is to offer is the window's answer, not a page's — the new-agent menu is
`ui::agents::new_agent_menu`, and the agents screen's `New agent` control and the chat panel's
`+ New chat` both raise that one. The mark is drawn by `rail::mark`
in that first row so it sits in the corner above the rail rather than inside it. It fixes no
arrangement — everything between the chrome is the dock's.

`ui/dock/` is the adapter. `state/dock.rs` is the policy over the tree and draws nothing:
`PanelKind` names a panel — a pane id for a terminal, a tab key for a file, nothing at all for the
rest — `class()` says which regions it may sit in, `home()` where it opens and where one put back
goes, `name()` is the permanent key a saved layout is rebuilt from, `tab_key()` is the tab a file
panel draws, `is_drawn()` is the hidden-not-removed rule, and `closable()` says whose tab offers a
close. `is_drawn()` is asked against one `Visibility` — everything the window knows about itself
that a panel could care about — so a new rule is a field on that struct rather than another argument
threaded through the dock. All of it is pure logic and is tested without a frame in
`crates/ubiq/tests/dock.rs`, which is also what makes the cost of renaming a panel visible at the
moment somebody edits the string. `ui/dock/mod.rs` holds `WorkbenchPanel` — a `PanelKind`, a weak
`AppState` handle and a focus handle, and nothing else — whose render delegates through `body()` to
the area functions that already exist, so adding a panel is an arm of a `match` rather than a new
owner of state. `default_layout()` is the arrangement a window opens in, `add()` and `remove()` are
how a terminal's and a file's panel join and leave, `holds()` is what stops a panel a restored
arrangement already carries being added a second time, `enforce_placement()` puts back a panel
dropped where its class forbids, and `restore()` rebuilds a saved arrangement or answers that it
could not. `dump()` writes a file panel's payload through `file_payload()` and `rebuild()` reads it
back through `file_from_payload()`, which is the one pair that keeps the shape on disk in one place. `ui/dock/skin.rs` implements the component library's
three renderer traits and draws every pixel the library would otherwise style: the tab strip at the
same height as `ui::kit::tab_strip`, the active tab marked on its bottom edge, the dot beside a tab,
the close only where the panel allows one, the drop indicator and the strips that resize a region —
and, at the right end of a tab strip whose group holds a terminal or the console, the `+` that asks
the host for a new pane and the chevron that asks for the menu of what else it could reach. The
control is the `NewPane` value `AppState::for_project` hands the skin: one closure per half, plus a
third — `AppState::is_pane_region()` — that answers whether a group is the pane region's, because a
group says which node it is and nothing about where it sits, and the control has to stay on the
strip of a region the user has emptied. The `+` is drawn only while that window holds a project. The
chevron crosses the same seam the file tab's right-click does, and for the same reason. A file tab's right-click crosses the same seam through
the `with_file_tab_menu` builder and its `FileTabMenuRun` type: the skin cannot name `AppState`, so
the tab's key and the click's point are handed across, and the window paints the menu over the dock.
Ubiq writes no drag, no drop geometry and no layout
serialisation.

`AppState` holds the dock's half of that. `dock()` hands it to the shell; `regions_open()` and
`toggle_region()` are the titlebar's three switches, both going through the dock rather than a flag
beside it; `panel()` builds a kind's panel the first time it is asked for. A panel reaches the dock
through a `Window` and a message does not come with one, so both halves of a panel's life queue and
are drained in `render`: `settle_visibility()` builds each panel's `Visibility` and pushes
`is_drawn()` into it, along with the layout a file panel writes into its payload — pushed rather
than read back, because the dock asks both while `AppState` is mid-update — then `settle_layout()`
rebuilds a saved arrangement and puts the layouts it carried back on the files, then
`settle_panels()` drains the `PanelEdit` queue. `sync_file_panels()` squares the dock's file panels
with the incoming project's open files when the window changes which project it is pointed at,
because the files are a project's and the panels are the window's. That is the same
device the pending focus and the arrived files already use. A `DockEvent::LayoutChanged`
subscription enforces placement and writes the layout down, in that order, so what is remembered is
what is on screen; `layout_blob()` is what it writes. A rail-mode switch is the same queue:
`set_rail_mode()` writes the outgoing mode's arrangement down through `remember_view()`, then hands
the incoming mode's saved blob to `pending_layout`, or — for a mode never arranged — queues its
`ModeLayout::default_for` flags as `pending_regions`, which `settle_mode()` forces on the frame.

The rest is one module per area: `rail.rs`, `titlebar.rs`, `project_menu.rs`, `status_bar.rs`,
`explorer.rs`, `editor.rs`, `terminal.rs`, `empty.rs`, `chat/`, `agents/`, `orchestration/` and
`board/`, with `work.rs` beside them for the one thing all three of the last draw. The project picker is
its own module rather than a `Picker`, because a project row carries actions and a confirmation and
is not just a value. Project settings is `ui/sink/project.rs`: the sink draws it on the page, the
shell paints the same dialog over the window when a project is being created or edited. Application
settings is `ui/settings.rs`: the titlebar's gear raises it, `state/settings.rs` holds the overlay
and the Ui-layer schema, and a toggle sends `SetSettings`. Shared primitives are in `ui/kit/`;
the conventions behind that split are in
[`../tech/ui-and-design.md`](../tech/ui-and-design.md).

State types live under `crates/ubiq/src/state/`: `workbench.rs` for the rail mode, the open menu, the
project settings dialog, the application settings overlay, what was typed into the picker's and the
explorer's filters, and the two menus that came later — `MenuId::FontSize` for the status bar's
text-size dropdown, `MenuId::FileTab` with the tab's key and anchor in
`WorkbenchState::file_tab_menu` for the file tab's right-click, and `MenuId::NewPane` with its anchor
in `WorkbenchState::new_pane_menu` and its rows in `WorkbenchState::shells` for the new-pane
control's chevron; `settings.rs` for the Ui-layer
schema, the overlay's nav, and how a blob is read;
`explorer.rs` for the tree, the list, the keyboard and the right-click menu, drawing through the
shared chrome in `ui/kit/files.rs`; `editor.rs`
for the open files; `logs.rs` for the console's filter; `work.rs` for one project's work as the host
describes it; `agents.rs` for the columns' view of that work and `orchestration.rs` for the graph's;
`board.rs` for the board's.

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

`state/orchestration.rs` holds `GraphView`, the orchestration screen's *view* of that work: the selection, the
session being drawn, the showing buckets, the zoom, the composer's draft, what a drag is carrying,
the grains behind it, and the arrangement. It holds no records, which is why every reader takes a
`WorkProjection` as its first argument — the shape `BoardState`'s readers have. Nothing here draws
and nothing names a colour. `visible()` is the two filters together and is the one reader that needs
no projection, because both are answered by the agent in hand; `showing()` treats an empty bucket
list as no filter; `session` absent is every session, and is its own field rather than a reading of
`selection` so that clearing what is drawn does not throw away what is selected — `active_session()`
answers that second question, and scopes nothing. `show_session()`, `toggle_bucket()` and
`clear_filters()` are the three mutations, and `filtered()` is what tells the toolbar and the empty
canvas whether there is anything to clear. `Held` — a card or a container — is what `start_carry()`
takes and what `carry_to()` branches on. `bounds_of()` is the box round a task's cards; `task_at()` is what a drop lands in, and leaves
the carried card out of every box it tests against; `end_carry()` writes the card's new offset
against the container it came to rest in and answers the pair for an `AssignAgent`, touching no
membership itself; `settle_sand()` answers whether the trail still owes a frame.

`state/layout.rs` holds every position, relative: a task's origin and an agent's offset inside it,
absolute only for an agent with no task. `at()` resolves the two. `Layout::auto()` is the whole
arrangement computed from the records alone and discards every hand-placed position, which is why
only `relayout()`, behind the tidy control, may ask for it. `Layout::place_new()` is what an arriving
record gets instead: it takes a tidy arrangement of everything and adopts it for the keys the layout
has never seen, so a new task takes the next container slot in the same flow-and-wrap order and a new
agent the next offset inside its task, with nothing already placed moving and no second geometry to
keep in step. `CARD_WIDTH`, `CARD_HEIGHT`, `GROUP_PAD` and `GROUP_LABEL` live there because the
outlines, the connectors and the hit testing work from them. Both are tested without a frame in
`crates/ubiq/tests/orchestration.rs`, which asserts no position against the fixture — it has none.

`AppState` carries the graph's view as `graph` and the composer as `agent_input`, a `TextareaState`
of its own so the two drafts cannot leak into each other. `start_graph_carry()` selects a card and
does not select a container; `move_graph_carry()` honours reduced motion, moving what is held without
laying a grain; `tidy_graph()` is the tidy control; `end_graph_carry()` sends the `AssignAgent` a
drop asks for; `send_to_agent()` sends the composer's line and appends nothing itself;
`settle_graph()`, called from `render` beside the other end-of-frame passes, ages the trail and puts
down a carry whose drag ended where the canvas's drop handler never sees it. `ui/orchestration/mod.rs`
is the frame; `graph.rs`, `inspector.rs` and `tasks.rs` are its three areas, painted from the layers
in `ui/kit/canvas.rs`.

`ui/work.rs` is the one place a work state becomes a colour, for every screen that draws one.
`activity_colour()` and `bucket_colour()` put the four buckets on the four status tokens — the three
ways of working share the one that means "moving" — and `role_icon()` and `role_mark()` are the glyph
a role wears. `ubiq_proto::work` keeps the words and `theme.rs` keeps the values, so the columns, the
graph, the board and the status bar cannot disagree about what running looks like.

`state/agents.rs` is the other view over that projection, and holds the arrangement rather than any
record: the columns, which one the sidebar's "here" means, which sessions are folded, what is typed
in each composer by slot, and the tab a drag is carrying. A `Column` is a composer slot, an ordered
set of agent ids and which of them is in front, and `grouped()` is the more-than-one-tab rule the
header counts. `arrange()` is the one-column-per-session layout and runs on the first listing;
`prune()` drops the tabs naming agents the host has forgotten and answers whether anything went, so a
re-sent `WorkList` costs no redraw. `reveal()` is the sidebar's one gesture, `open()` a column of its
own, `open_in()` a group, `bench()` a close, and `split_off()` the drop past the last column, which
asks `free_slot()` for room **before** it takes the tab off the column it was in. `reveal()` falls
back to `open_in()` on the focused column when the row is full, which is why the sidebar's click
never fails. `benched()` is the
difference between what the host reports and what the columns hold, and `on_the_field()`,
`grouped()`, `count()` and `has_room()` are what the header, the status bar and the drop targets
read. `COLUMNS_MAX` and `COLUMN_MIN_WIDTH` live here rather than in `theme.rs`, because how many
conversations fit and how narrow one may get are facts about a conversation. Nothing in it draws,
nothing in it names a colour, and it is tested without a frame in `crates/ubiq/tests/agents.rs`.

`state/conversation.rs` is one live agent's transcript as the window holds it. `Conversation::apply`
folds a delta in — a chunk extends the block its message id names, a change of id starts a new one, a
patch reaches its call through an index rather than a scan — and `activity()`, `context_pct()`,
`tokens()`, `cost_usd()` and `rate_limit_five_hour_pct()` are what the badge, the ring and the
footer's pills are drawn from; `is_next()` is the gap check. `AppState` holds them per project as `conversations`, kept after the harness ends, and
`refresh_agent_record()` writes the badge, the ring, the token count and the model onto the
`WorkAgent` record, so the sidebar, the graph and a column's header keep one source.
`ui/conversation/mod.rs` draws one — `render()` over a `ConversationView`, then `tool_block()`,
`diff()`, `permission()`, `footer()`, `composer()` and `queue_list()` — `prompt_agent()` sends and
appends nothing, `send_or_enqueue()` is what the composer's one button and the Enter key both call
(send when idle, queue on `Conversation` when a turn is already running and the draft is not empty,
nothing when it is), `steer_column()` resolves the slot's agent through `AppState::agent_for_slot`
and chooses between `send_or_enqueue()` and the mock's path, and `pick_new_agent_menu()` starts a
conversation on the harness at that row. `crates/ubiq/tests/conversation.rs` covers both.

**A conversation is drawn before its harness exists.** `pick_new_agent_menu()` mints the `AgentId`
itself — the `SessionId` precedent — and the host adopts it, so `ConversationStarted` and the
`Conversation` it creates arrive with no process behind them yet; `Conversation.launched` stays
`false` until the harness's own `Started` update sets it. While it is false, `composer()` reads
`conversation.config` for a `model`-category `ConfigOption` and draws a `Picker` dropdown above the
field instead of the footer's read-only pill (a "Discovering models…" note when the list has not
arrived), its own open/shut state on `Conversation.model_menu_open` rather than the window's single
`open_menu` — several pending conversations can each have one open at once — and a pick sends
`AppState::pick_agent_model()` — which also records the pick on `Conversation.chosen_model`, since
the host does not echo a `SetAgentConfig` sent before launch, so the picker has nowhere else to read
its own highlight from, and closes the dropdown. `activity()` reads that pending,
never-run state (`Run::Idle` with no `stop_reason` yet) as `Activity::Thinking` rather than
`Activity::Ended`, matching what the host already reports at registration.

`AppState` carries it as `agents`, the composers as `column_inputs` — a fixed pool of
`COMPOSER_SLOTS` `TextareaState` entities built in the constructor (one per column, plus
`CHAT_SLOT`), each with a subscription that mirrors what is typed onto that slot's draft and steers
the column on a bare Enter — and the sidebar's scroll as `agents_scroll`, its own rather than the
explorer's. `reveal_agent()`, `group_agent_into()`, `bench_agent()`, `select_column_tab()` and
`focus_agent_column()` are the clicks; `start_tab_drag()`, `drop_tab_on()`, `drop_tab_at_end()` and
`settle_tab_drag()` are the drag, the last putting down a tab whose drag ended where no drop handler
sees it. `steer_column()` is the one thing this screen sends through the Enter key, and appends
nothing itself; `close_all_conversations()` is `bench_agent()` for every tab in every column, not
`end_conversation()`. `fill_columns()` gives each composer its placeholder and its draft, drained in
`render` for the reason `fill_task_form()` is: `set_placeholder` and `set_value` both need a window,
and an arriving message, a project switch and a jump from another screen have none. `MenuId::AgentBench`
carries the column its `+` was clicked in, because a row of columns has one each and only one menu
may be open. `open_task_chat()` reveals an agent and switches to Agents mode; `show_task_in_graph()`
switches to Orchestration.
`ui/agents/mod.rs` is the frame — the sidebar, the header strip, the row of columns and the drop
strip at the end — `sidebar.rs` is the list, and `column.rs` is one column, from its tab strip to its
composer.

`state/board.rs` is the board's view of the same projection, and holds nothing that is a fact about a
task: the filter text, which session's pills are on, which task is open, which columns and cards are
shut, the carry, and what the panel is in the middle of doing. `Field` names the one field open — the
title, the description, a step by its id rather than its place in the list, or the field that names
the next one — and `TaskForm` is what was typed into them. `moving` is a drop the host has not
answered, read back by `is_moving()`; `awaiting_new` is a `CreateTask` whose id is not known;
`preview` is the description showing as markdown while it is written; `confirm_delete` is a delete
asked once. `is_editing()`, `edit()` and `stop_editing()` are the one-field-at-a-time rule, `select()`
discards an open field because it was about the card being left, and `needs_fill()` answers whether
the fields still describe the open task — a pure predicate rather than the refill itself, because
writing into the component library's state needs a window and this has to be testable without one.
`column()` is what one column draws, `matches()` is the filter both it and the status bar's counts go
through, and `end_carry()` answers the task and the column it landed in. It is tested without a frame
in `crates/ubiq/tests/board.rs`.

`AppState` carries it as `board`, the filter as `task_filter`, and the panel's four fields as
`task_title_input`, `task_description_input`, `step_title_input` and `new_step_input`. Every edit is
a handler that sends and waits: `begin_task_edit()` opens a field and gives it the keyboard,
`cancel_task_edit()` puts it away and refills it from the record, `commit_task_title()` and
`commit_step_title()` refuse an empty title and send nothing when the value has not changed,
`commit_task_description()` allows an empty one, `set_task_priority()`, `set_task_shape()` and
`set_task_session()` send on the click, `add_task_step()` keeps its field so several can be typed in
a row, `remove_task_step()` goes straight through, `delete_task()` asks the first time and sends the
second, `withdraw_task_delete()` takes the question back, and `toggle_description_preview()` swaps
the markdown for the source. `new_task()` is where the filter field becomes a title and the task is
asked for; `drop_task()` is the column's own drop handler, because the column is the drop target
here; `settle_board()`, beside `settle_graph()` in `render`, puts down a carry whose drag ended
outside every column. `ui/board/mod.rs` is the toolbar, the columns and the cards, and its
`status_colour()` is the one place a column becomes a colour.

`ui/board/detail.rs` is the report and `ui/board/form.rs` the controls, drawn into the same column.
The form is not an area of its own and has no row in the table above: the rule about adding an area
is about something that occupies new space, and this fills the panel that has a row and a
`TASK_PANEL_WIDTH` of its own. It is a second file for the reason `ui/chat/` keeps its transcript
apart from its composer — the report and the controls are two jobs. `title()` and `description()` are
the two fields that open, `pills()` is priority and shape, `session()` is the picker behind
`MenuId::TaskSession`, `step_controls()`, `step_field()` and `new_step()` belong to the sub-task
list, `delete()` is the two-click question, and `refusal()` is where `WorkbenchState::work_error` is
said.

`state/sample.rs` is down to `chat()`. Projects, the file tree, a file's bytes, the panes and the
work all come from the host, and the constructors that invented them are gone.

`state/sink.rs` is the kitchen sink's fixtures and the small state its pages carry, and it is the one
other place a constant stands in for what the host would send — deliberately, and for the opposite
reason `sample.rs` does. The chat's fixture is a screen waiting for a transport family; the sink's is
a screen that will never have one, because a test bench with a project behind it would be testing the
project. It holds four documents as `&'static str`, each under the name that picks its viewer,
`SinkSection` for the page strip, `SinkState` for the layouts and the style reference's controls,
`SinkModal` for which of the three shapes is up, `SettingsDemo` for the application settings page
and `ProjectDemo` for the project settings dialog — its swatch, its custom `0xRRGGBB`, and the
HSV the picker is holding. Nothing in it draws and nothing in it holds a
buffer, which is what lets `crates/ubiq/tests/sink.rs` hand every fixture to the parser or the
renderer that will draw it with no frame — so a fixture that stopped parsing fails the build instead
of drawing an error nobody looks at.

The buffers are the window's: `AppState` builds one `EditorState` per fixture in its constructor,
where there is a window to build one with, keyed by the document's key, plus `sink_input`,
`sink_textarea` and `sink_modal_input`, and the settings pages' own fields (`sink_search`, the
harness name, executable, prompt and env, the project name, description and colour hex). A fixture is a
constant, so that is the whole of their lifecycle — nothing arrives late, nothing is saved, and no
change subscription is needed because there is no baseline to compare against. `ui/sink/` is the
screen: `mod.rs` draws the page strip through `kit::tab_strip` and dispatches on the page,
`docs.rs` draws one fixture through `ui/viewer/` — every viewer reached rather than copied, which
is the whole point of the page — `style.rs` is the reference, `files.rs` is the picker page,
`settings.rs` is the application settings layout and `project.rs` is the project settings dialog.
The modal is raised from `mod.rs` rather than from `style.rs`, because exactly one may be up and
where it is asked for is not where it is painted; the primitive is `kit::modal`, whose shape and
dismissal rules are the UI-and-design document's. The project settings page is not that modal: it
is a wider form. The sink draws it on the page so the whole of it can be looked at; the workbench
raises the same form over the window to create a project or edit the one on screen.

`state/file_picker.rs` is the picker itself, and nothing in it reads a disk. `PickerRequest` is the
whole of what a caller says — owner, title, root, prefilter, kind, count, commit and modality — and
`FilePickerState::open` roots the forest it was handed at the requested folder, opens the top of it
and holds the rest: the view, the filter, which folders are open, what has been picked in pick order,
and the size the corner drag has put it at. `rows()` is the only thing the screen reads, and it
arranges the same set two ways. The forest is handed in rather than fetched, which is what lets the
sink raise a picker with no project open and what will let the host's listings fill the same dialog
when a screen needs one over a real project — `PickerNode` is the shape a `DirListing` becomes.

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
route the answer by `PickerRequest::owner` — one variant today, the sink's page. `ui/file_picker.rs` draws it, painted
from `ui/sink/mod.rs` for the same reason the modal is: where a dialog is asked for is not where it
is painted. `crates/ubiq/tests/file_picker.rs` asserts every rule above with no frame at all, over
the sink's own fixture tree.

A row keyed by a ULID takes its element id from `ui::eid`, or `ui::eid2` for a row two ids deep like
a step inside a task, because a ULID is not a `u64` and the tuple form the rest of the window uses
cannot carry one. That convention is
[`../tech/ui-and-design.md`](../tech/ui-and-design.md)'s.

`state/explorer.rs` holds every piece of tree logic and no frame, which is what makes it testable in
`crates/ubiq/tests/explorer.rs`. `merge()` puts one directory's listing into the tree, matching
entries by name so that a folder re-listed keeps the children and the expanded flags below it, an
entry that has gone is dropped with its subtree, and a new one arrives shut and unlisted — which also
makes an unsolicited listing harmless. `toggle()` answers whether flipping a folder open means the
host has to be asked. `expanded()` is what gets written down and `reopen()` is what reads it back,
opening the folders a blob named as each of their parents arrives. The order the host sorted a
listing in is kept rather than sorted again, so two windows on one project cannot disagree.
`rows()` is the two arrangements — tree and list — and `press()` is every key, so the tests walk
the cursor without a window. `unlisted_for_cache()` is the folders the background fill still cannot
see into, skipping the walk's skip set so a cache does not list `node_modules`. `drawn_rows()` is
what the panel paints: open folders when the field is empty, the last background hits when it is
not, never a walk of the cache on the frame. `ExplorerMenu` is
the right-click: `menu_entries()` is what it offers, and the four actions that wait on the host are
present and not ready. `ui/explorer.rs` draws through `ui/kit/files.rs`, the same chrome the picker
uses, and tints, badges and dots a row from `GitStatus`. `apply_git()` takes a `GitWorkingTree` and
projects each pair onto that enum; `merge()` re-paints so an expand is not unmarked until the next
refresh. A stale generation is discarded. An untracked or ignored directory in the map is inherited
by every child `paint_git()` walks, which is what makes expanding a new folder mark the files
inside it.

`state/editor.rs` names the component library, unlike its neighbours, because a file's buffer *is*
its state: `FileBody` is either `Loading`, the `Text` of a buffer with the bytes the host sent beside
it, the `Diff` the host computed, `Binary`, or a `Failed` read. Dirtiness is that comparison against
the host's bytes, cached off the buffer's own change event rather than recomputed per frame.
`FileLanguage::of()` picks a highlighter from the path's extension, and anything it does not
recognise opens as plain text, which is the general case rather than a fallback. `Subject` is what a
tab is looking at — the file, or a comparison made from it — and `tab_key()` and `from_tab_key()`
are what keep those two tabs rather than one: the key is the path for the file itself and the path
behind a prefix for a comparison, and it is what the saved arrangement and the view prefs name a tab
by.

`ui/editor.rs` is what one file draws and the two things its tab asks of it. `render_file()` is the
file panel's body, `label()` and `state_colour()` are what the dock's tab says and the colour of the
dot beside it, `highlighter_language()` is the one place our language enum meets the highlighter's,
and `render()` is what is left of the centre panel in IDE mode: `welcome(app)`, the big, faint brand
mark on the no-file page.
There is no tab strip here — the dock's groups draw those. A body that is not a buffer goes to
`ui/viewer/`, whose `mod.rs` holds the layout toggle and the frame every viewer's body is drawn in
and dispatches on `ViewerKind`: `diff.rs`, `markdown.rs`, `diagram.rs`, `scene.rs` and `image.rs`.
A diagram or a scene in a panel is wrapped by `viewer/viewport.rs`, which is the hits and the
wheel; `state/viewport.rs` is the camera they share — fit, zoom about a point, pan, reset — and
is what `tests/viewport.rs` asserts, because none of it needs a frame. A fence still draws
through `diagram.rs` and `scene.rs` directly, at the picture's own size. None of them reaches a
path or a handle; the camera is keyed by the tab and lives on the window.

`state/diagrams.rs` is the Mermaid renderer and its disk tier, and it is the only place in the
interface that names `merman`. `render()` is one source in and one picture out, sized by `view_box()`
off the SVG's own `viewBox`; `key()` is the content address — the source, the palette and the
renderer's version marker, which moves with the `=` pin in `Cargo.toml`; `cache_dir()` joins the
workarea the host sent with the interface's own subdirectory; `Disk` reads and atomically writes one
entry there, swallowing every IO failure as a cache miss; and `resolve()` is the whole of what runs
on the background executor. `AppState::diagram()` answers what the window holds and queues what it
does not, `drain_diagram_asks()` hands the queue to `cx.background_spawn` once the frame is built —
never from inside one — and `diagram_drawn()` takes each answer back by its key and notifies.

The file path through the two halves: `select_file()` opens a tab, queues its panel and sends
`ReadProjectFile`; `open_diff()` opens a tab on a comparison and sends `DiffProjectFile`;
`toggle_folder()` sends `ProjectTree` when a folder has never been listed; `fill_explorer_cache()`
sends the same message at `CACHE_DEPTH` for folders the background fill cannot see into yet, from
project open, not from a keystroke; `schedule_explorer_filter()` debounces the field and
`spawn_explorer_filter()` walks a snapshot on the background executor;
`save_active_file()` sends
`WriteProjectFile` with the version the read came with. Holding a project also sends `ProjectGit`
and `RefreshProjectGit { full: true }`, a successful save and a pane exit send the full refresh
again, and `ui/status_bar.rs` prints the overview's branch. `activate_file()` and `closed_file_panel()`
are the other direction — the dock deciding which tab is in front and which has gone, which the
editor learns from it rather than the other way round. Contents cannot become a buffer where they
arrive, because a buffer needs a window and a message does not come with one, so they queue and
`attach_arrived_files()` drains them in `render` — the same device the dock's own edits and the
pending focus use, and the one `fill_task_form()` uses for the task panel's fields. It exists for
that reason and no other: `set_value` needs a window and a message does not come with one, so a
selection change, a project switch and a `TaskChanged` for the open task each leave a flag for the
next frame to drain. Its guard is what stops it writing over what is being typed on every frame, and
it fills from the record the host confirmed rather than from what was typed — never while a field is
open. `install_key_bindings()` binds `⌘S`, `cmd-=` and `cmd-shift-=` (zoom in) and `cmd--` (zoom out)
in the `Workbench` key context, then the file picker's and
the explorer's keys — each bound for the surface and for the field inside it, after the component
library's own so they win — and the binary calls it beside its own quit binding.

## Failure

| What happens | Result |
|---|---|
| The last editor tab is closed | The centre panel comes back where it was left and says no file is open, and the status bar reports no caret and no language |
| A saved arrangement names a file | It is rebuilt from the payload beside it, at the tab and the layout it carried. A file panel with no payload names no tab and is dropped, like a terminal |
| A saved arrangement's file panel names a tab the project no longer opens | The panel is hidden rather than drawn, so it keeps its place and comes back if the tab is opened again |
| A file panel is dragged to a border region | It is moved back to the centre on the same edit. The open files are the centre in IDE mode, so a file on a border would leave nothing behind it |
| A filter matches nothing | The panel says nothing matches; the filter field keeps what was typed. Hits still filling in the background appear as their listings land |
| Every edge region is closed | The rail, titlebar and status bar remain; the centre region fills the dock |
| A region is closed while it holds the console | The console goes with the region and comes back where it was left. Nothing leaves the tree |
| The console's tab is closed | It leaves the arrangement, and the new-pane menu's `Logs` row is what brings it back |
| The user empties a region by dragging its last panel out | The titlebar's switch for it reads as closed, because it reports the dock |
| A panel is dropped in a region its class forbids | It is moved back to its home region on the same edit, so the drop reads as refused |
| A saved arrangement is from another version, or is unreadable | It is discarded whole and the window opens on the default arrangement |
| A saved arrangement names a pane the window still holds | The pane comes back where it was — its group, its split and its tab position |
| A saved arrangement names a pane the window does not hold | The panel is dropped and the tree normalises around the gap. Layout persists; harnesses do not |
| A saved region's every panel was dropped | The region is installed empty, at the size and open state the blob says. The tab strip is still where a pane is opened from |
| The pane region is opened with nothing in it | A pane is started in it, the platform's default shell, as a bare click on `+` would |
| The last project in a window is closed | The window stays, on the empty state. Its harnesses are killed with the project, and what it remembered is written down |
| A project with terminals is closed | The row asks first, and closes only on a second, explicit click |
| A project open in another window is opened here | It leaves that window, which stays open on the empty state if it held nothing else. Its panes are killed rather than moved |
| More than 26 windows are open | The 27th and beyond are named `#`; nothing else changes |
| The last window is closed | The application quits. Closing one of several does not |
| A rail mode has no screen | The empty page names the mode and says it is not built |
| The Git screen is opened on a folder that is not a repository | The toolbar says so, the lists are empty and nothing is drawn as clean. No error to dismiss |
| A repository is open and the working tree has nothing to say | The panel says there is nothing to commit, which is the answer clean gives and unread does not |
| A changed path is picked and the host has not answered yet | The pane says it is reading. The last comparison is never left under the new name |
| The path the diff pane is about goes clean | The selection and the comparison go with it, and the pane asks for another path |
| A commit is selected | The panel reports what the log said and states that a commit's file list needs a message the git family does not carry |
| The row of columns is full | A split is refused and the tab stays in the column it was in. A click in the sidebar still brings a benched agent on, grouped into the focused column, and the `+` still groups into the column it belongs to — the ceiling is on columns, not on tabs |
| A benched agent stops being reported | It stops being listed, and there is nothing to clean up: the bench is computed from the work rather than written down |
| An agent arrives after the screen has laid itself out | It is listed on the bench rather than put in a column. Every listing after the first only prunes, because the arrangement is the user's |
| Every agent is on the bench | The field says which control brings one back, rather than an empty row that would read as a project with nothing running |
| A tab is dropped anywhere but a column or the end strip | The next frame puts it down in the column it came from, so it cannot stay stuck to the pointer |
| Nothing is selected on the orchestration screen | The inspector says so and points at the toolbar and the graph. The drawer falls back to the first session, so it does not go blank; the graph draws every session and needs no fallback |
| Every agent is filtered out | The graph says so and offers to show everything. It says the opposite thing — that no agent is running in this project — when there was nothing to hide, so the two emptinesses are never confused |
| Every bucket pill is turned off | The row is not filtering, and every card is drawn. This is the way back from having turned them all off, which is why no pill refuses a click |
| A task's cards are all hidden | No outline is drawn for it. The task keeps its place in the drawer's list |
| A card is dropped outside the graph | The next frame puts it down where the drag left it, so it cannot stay stuck to the pointer |
| A card is dropped on open ground | It keeps its task and its parent, and stays where it was put |
| A container is dragged onto another | Nothing is filed anywhere. The outlines overlap until one is moved or the graph is tidied |
| The composer sends with a session selected, or with nothing | Nothing is sent, and Send reads as disabled while the draft is empty |
| A message is sent to a mock agent | The host puts it in that agent's thread and answers with the agent carrying it. Nothing replies, and the thread says so rather than inventing one |
| A conversation's update does not follow the last one | The window reports the gap and applies the update anyway, because half a transcript is worth more than none |
| A harness reports no context window | No ring is drawn, and the footer reports the harness and the model without one |
| A conversation's harness exits | The transcript stays and the agent takes no further turn. Closing the tab is what takes it off screen |
| The composer is used while a turn is already running | An empty draft offers Stop; a non-empty one is held on `Conversation.queued` instead of being written into the harness mid-turn, and sent automatically the moment the turn ends |
| A screen over the work is opened with no project | The centre draws nothing. All three are views of one project's work and there is none; the rail, titlebar and status bar stay |
| A move is never answered | The card stays in the column it came from, drawn muted and saying it is waiting. Nothing times it out, and the mark comes off on the next answer naming that task |
| An edit is never answered | The field closes and the panel goes on reporting the task the host last confirmed, so the change reads as not having happened. What was typed stays in the form until the selection changes or the project is entered again |
| The host refuses a change to the work | The panel says what it would not do, puts the open field away, takes the waiting mark off, gives up on a `New task` that never arrived and withdraws a pending delete. The next thing the host confirms clears the sentence |
| The selected task is absent from a fresh listing | The panel closes rather than reporting a task nobody holds. The selection is left as it was, so the panel returns if a later listing carries the task again |
| A project's tasks cannot be written | The change holds for the session and one refusal says once that it is not durable. The card moves, so the board and the store disagree until a write succeeds |
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
| The project is not a repository | The explorer's rows stay unmarked and the status bar prints no branch. That is an ordinary answer, not an error |
| A repository exists and cannot be read | Badges clear rather than freeze at the last good answer. A corrupt object database is the case this exists for |
| A working-tree reply is older than one already held | Discarded, so a slow walk cannot repaint what was true earlier |
| A harness exits | The pane stays, and the window asks the host to refresh that project's version control |
| A dirty tab is closed | The panel returns to its group with the tab turned into a question, and takes a second click. Bringing it forward withdraws it |
| Contents arrive for a project the window no longer holds | Dropped. For one it holds but is not showing, they are put in their tab, which is there on the next switch |
| Contents arrive for a tab that has been closed | Dropped, so nothing reopens under the user |
| A remembered folder no longer exists | It is dropped from the restore rather than waited on; the rest of the tree opens |
| A remembered file no longer exists | Its tab opens and reports the failed read, so the loss is visible rather than silent |
| A diagram has been asked for and has not come back | The panel says it is drawing. The window keeps redrawing and keeps taking keystrokes; nothing waits on it |
| A diagram's source will not render | The renderer's own sentence is shown in the danger colour above the source, which is the only place it is any use. Nothing is written down, and the next ask renders again, because it may follow the edit that fixed it |
| A Markdown fence has not resolved yet | The block draws an ellipsis and fills in on the frame its picture lands. The rest of the document is drawn |
| The workarea cannot be read or written | Every failure is a cache miss, never an error: the diagram is rendered again and the picture is drawn. A half-written entry is a miss too, because it carries no usable size |
| The palette is switched with a diagram on screen | The picture is rendered again for the new palette. The two palettes are two entries, so switching back is a cache hit |
| A window has no project yet | A diagram still renders, with the memory tier alone. There is no workarea to write to until the catalogue has arrived |

## Related docs

- [`panes-and-terminals.md`](./panes-and-terminals.md) — what a terminal panel actually is
- [`chat.md`](./chat.md) — the panel that survives every mode switch
- [`../tech/ui-and-design.md`](../tech/ui-and-design.md) — the tokens and the component conventions
- [`../tech/architecture.md`](../tech/architecture.md) — who owns which state, and why the interface asks
- [`../tech/decisions.md`](../tech/decisions.md) — `D47`, why the agents and the work are two screens, and `D42`, the dock they are panels in
- [`../tech/transport-contract.md`](../tech/transport-contract.md) — the project, file, git and work families in full
- [`../backlog.md`](../backlog.md) — what the shell still lacks

## Next steps

- Build the Control and KB screens.
- Reorder a task's sub-tasks, which `MoveStep` names on the wire for exactly that.
- Hand a sub-task to an agent, so `Step.owner` is set by something.
- Write the graph's arrangement down, so a hand-placed card survives a restart.
- Remember which of the board's columns were shut.
- Reach a status change from the keyboard, so a card can move without a drag.
- Keyboard navigation for the rail and the tabs.
- Make the titlebar's command field find a file in the project.
