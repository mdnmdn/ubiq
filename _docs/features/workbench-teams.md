---
id: feat-workbench-teams
title: The Teams graph modes
kind: feature
status: draft
summary: The rail's graph mode — `Teams`, over a project or the whole window, drawing its cards' conversations in the dock — the twelve arrangements the canvas computes for itself, the hexagonal status mark, the filters, the drag model and the tasks drawer.
read_when: you are changing the Teams screen — its graph, how it arranges itself, a card or a delegate row, the span, the filters or the tasks drawer
updated: 2026-10-25
verified: 2026-10-09
code_anchors: [crates/ubiq/src/app/teams.rs, crates/ubiq/src/app/teams_span.rs, crates/ubiq/src/state/teams.rs, crates/ubiq/src/ui/teams/mod.rs, crates/ubiq/src/ui/teams/graph.rs, crates/ubiq/src/ui/teams/status.rs, crates/ubiq/src/ui/teams/tasks.rs, crates/ubiq/src/ui/kit/menu.rs, crates/ubiq/tests/teams.rs, crates/ubiq/tests/teams_view.rs, crates/ubiq/src/state/layout.rs, crates/ubiq/src/state/shapes.rs, crates/ubiq/src/ui/sink/teamsim.rs, crates/ubiq/src/state/teamsim.rs]
depends_on: [feat-workbench, tech-ui]
review_cycle: monthly
---

# The Teams graph modes

## Purpose

The graph mode is where the user *arranges* the agents rather than talks to them: a canvas of
cards on a dotted ground, one per agent, fenced into the task each serves, with a tasks drawer
underneath. It has two rail entries, `Teams` and `All Teams`, which are the same screen over two
spans. The columns that hold the conversations are [Agents mode](./workbench-agents.md).

## Behaviour

**`Teams` draws only the agents this window holds a live `Conversation` with.**
`state::teams::live_work` drops a session with no surviving agent and a task no live agent serves or
holds a step in, so a project with nothing running draws as empty. Each project keeps its own
arrangement in `OpenProject::teams`; the window span has a second arrangement of its own,
`AppState::teams_window`, which belongs to the window rather than to any project, so the two spans
are two arrangements over two different sets of cards and switching span hands back the other one
intact rather than rewriting the one that was up. The earlier graph screen, `[Teams]`
(`RailMode::TeamsOld`), is gone (`D205`): a saved preference standing in it opens on `Teams`.

**The `Teams` screen is scoped by a span, and the span is the window's own fact** — like the zoom,
read off the rail, persisted nowhere and sent nowhere. The arrangement is not: see below.
`state::teams::TeamsSpan` is `Project` or `Window`: the Teams rail entry is `Project` and a second
rail entry (`RailMode::TeamsAll`) is `Window`. The toolbar has no span control; its first filter is
the Session picker under the project span and the Projects picker under the window span. Under `Project` the canvas is the active project's
live work and nothing else. Under `Window` it is every project the window holds, in the window's
own project order — picker order, which never moves — with each project's live work concatenated
into one canvas.

**Under the window span a card and a session pill wear the project they belong to**: the project's
initials, with the project's tint as the pill's coloured left edge, the same face the rail draws its
project badges with, **and every top-level thing on the canvas is fenced in that same tint**: a task
container takes the colour of the project that owns it, and a card no container encloses gets a
dashed fence of its own, so where one project's work stops is read off the canvas rather than off
the chips one card at a time. Nothing is fenced twice — a card inside a container already sits in
its project's colour — and a container lit as a drop target keeps `accent` while it is in that
state, because that is the answer the canvas is giving for the moment. Under the project span the
canvas carries no chip and no project fence — a screen about one project does not need to say
which, and one colour repeated over every card is not a distinction. **The rail and the titlebar go on meaning the active project.** They
are the window's answer to which project the other modes are about, and a span set on one screen
does not rewrite them: the active project keeps its filled badge, and the cross-project canvas is a
wider view *from* that project rather than a window with no project.

**A hand-over stays inside one project.** Under the window span the canvas draws every held
project's task containers, but only the carried card's own project's containers light up under it:
the rest are not drop targets, and a card dropped clear of one keeps the position it was let go at
and changes hands with nobody. The alternative — a container that accepts a foreign card — sends
the host an `AssignAgent` naming one project, that project's agent and another project's task, which
the host refuses after the canvas has already drawn the move.

**A mission has no mark of its own on the canvas.** The canvas draws project fences, task containers,
agent cards and delegate rings; a mission's parent task is a task container like any other, its
subtasks are containers beside it, and a card dropped on either is an ordinary `AssignAgent`. A
mission is read, started and steered from its side panel, the board and the Agents sidebar.

**A task container waiting on prerequisites wears the waiting colour.** `TaskRecord::ready` is the
one readiness implementation both halves of the app call (M20), and a container whose task is not
ready takes `warning` through `ui::work::work_state_colour` — the same token the mission panel's
progress bar and the Tasks tab give that state (M26) — so a stalled branch reads from the canvas rather
than only from a mission surface. A lit or carried container still keeps `accent`: that is the
answer the canvas is giving for the moment.

**The far end of the toolbar makes something, and it is a split button.** `+ Add agent` is gone;
in its place is a `+` and a chevron, the titlebar's own new-terminal pattern reused rather than
built again (§9). The `+` does what `+ Add agent` always did — New agent, asking which
project when the canvas spans more than one, by a menu whose rows wear the same project face the
cards do, and going straight to the form when the window holds one. The chevron opens a second
menu whose one row is **New agent**, asking the same project question in a second stage before
raising the standard dialog (`teams_create_overlay()` in `ui/teams/mod.rs`); the chevron's tooltip
reads "New agent or mission" although no mission row is offered (`G439`). It sits past the flexible gap
with the view controls rather than among the pills, because an action is not a filter. It is one
of two ways into the New agent form that names a project — the sidebar's Missions header offers
New agent too — every other way in starts the agent in the window's active project and has no
project field at all.

**Teams draws its card a shade shorter than the layout engine's plain `CARD_HEIGHT`, because a row
that is often empty is not worth reserving space for on every card.**
`state::layout::TEAMS_CARD_HEIGHT` is `CARD_HEIGHT - 16.0`, and `Layout::auto`/`Layout::place_new`, `Algo::inside` and every packer beneath them (`ring_box`,
`card_box`, `stack`, `column`, the `shapes.rs` `inside_*` family) take the plain card's own size as
an explicit `card: (f32, f32)` argument rather than reading `CARD_HEIGHT` off the constant, so one
engine lays both graphs out at their own size: `TeamsView::relayout`/`absorb_new` pass
`(CARD_WIDTH, TEAMS_CARD_HEIGHT)`. **A card reads the agent label (T-283):** row 1 is the agent's title with the standard
agent tooltip, row 2 the harness's mark and the identity (`definition · harness · model`), and the
third row — the title of the task it is assigned, never its name — is drawn only where there is one;
the activity is the corner chip's. A delegate's card reads the same shape at its own grain: its
`Task` description, then `kind · model` (`SubagentTab::identity`), and the composer's agent
switcher draws the same identity faint beside each delegate. The footer sits flush under its own
content rather than stretched by a flex that used to fill the shorter card's spare height.

**A card that has delegates draws every one of them, one full-width row underneath it per delegate,
inside a fence that is never placed but always derived.** `state::layout::SUB_WIDTH` equals
`CARD_WIDTH` and `SUB_HEIGHT` is 112 — taller than a plain Teams card's own row so a delegate's
command line and its footer both have room, though still shorter than `CARD_HEIGHT` so it keeps
reading as the smaller card the module doc promises — so `state::layout::sub_slot` stacks delegates
one under the next rather than wrapping a second column. `state::layout::fence` computes the dashed
box round the card and wherever its delegates now sit, from the card's own position and the
delegates' positions, so dragging a delegate out to the side resizes the fence on the next frame
rather than leaving it behind. There is no ceiling and no `+N` overflow count: a card with eight
delegates draws eight full-width cards, stacked eight rows deep. `Layout` carries a delegate's
offset against the card that spawned it in `subs`, keyed by the id of the `Task` call that spawned
it, so dragging the parent card — or the task container round it — carries its delegates the same
way it already carries its own cards. The packers take a `Rings` map (`state::layout::Rings`, an
agent's delegate count) and add `ring_drop` — one row per delegate — under the tallest fence in each
row, so the row below a card with delegates clears the fence instead of drawing under it.

**The ring's own fence is drawn only where nothing else already outlines the card — one
project-tinted fence, not two (T-105).** A ringed card whose task already sits inside a container,
or whose card is fenced alone because no container reaches it, is already outlined at `RING_PAD` off
`card_bounds` — which itself wraps the ring — so `ui/teams/graph.rs`'s render pass tracks which task
ids a container actually drew (`boxed_tasks`) and skips `state::layout::fence`'s own box for any
ringed card whose task is boxed, or that the window span's loose fold already reaches; only a ringed
card neither a container nor a loose fence reaches — no task on the canvas — still gets it. Before
this a ringed card inside a container drew both the container's own dashed box and the ring's,
a shade lighter, round the same cards. Where the ring fence *is* drawn it is snug against the card,
in `theme::fade(theme::accent_muted(), 0.8)` and never `active` — the task container is what lights
up as a drop target and draws its own `theme::border()` outline. **`TeamsView::solo_bounds` and
`bounds_excluding` — the lone card's own fence and the task container's own box — measure their own
margin off `RING_PAD` rather than `GROUP_PAD`** (T-105): `card_bounds` already wraps a ringed member
at `RING_PAD`, and each of these is the *only* outline drawn round what it encloses, so it sits
`RING_PAD` off the tightest thing inside it rather than doubling the margin with `GROUP_PAD` stacked
on top of a ring's own.
`TeamsSelection` gains a third case, `Subagent { agent, subagent }`, named by the id of the `Task`
call that spawned the delegate rather than a card of its own — see `D140` for why a delegate is
never a `WorkAgent`. Clicking a delegate's card sets that selection and points the shared chat at the
delegate through the existing `AppState::view_conversation_agent`, the same call a top-level card's
click already made; picking one up to drag it does the same. A delegate's card is draggable and
carried the same way an agent's is, but a drop under it is never a hand-over — `end_carry` answers
only for an agent card, and no container lights up under a carried delegate. The canvas extent folds
in every delegate card and every fence alongside the agent cards and task boxes it already measured;
the canvas itself is unbounded, so a delegate dragged anywhere stays reachable by panning.

**A delegate's card is drawn like an agent's, at its own size, in the same four rows.** The
hexagonal status mark and the delegate's name lead; the harness icon and its model (through
`conversation::short_model_label`, the cut the composer's chip already makes —
`claude-haiku-4-5-20251001` is `haiku`, and a harness that is not Claude keeps its id whole, so one
project never spells a model two ways) follow beneath; a line naming its current command comes
next — the running tool's title, or the last non-blank line of its prose or thought stripped of
markdown (`Thinking` / `Writing` only where nothing is left; Claude cards read the same), read off the
delegate's own last block in the transcript (`Conversation::subagent_activity`) and carried onto
`SubagentTab::activity`, or `need you` — `need you \u{d7}N` above one request — where it is blocked
on a permission answer (`SubagentTab::waiting`), in place of the activity rather than beside it. A
delegate with no line of its own yet, or no model the harness named, draws neither. The footer
carries the one ring a delegate's own transcript can state anything about — how much of what its
subtree has spent came back from cache — and its token count, a **subtree total**: the delegate's
own reports plus every delegate it spawned, never a sibling's or the parent's, banked by
`Conversation::subagent_tokens`; the top-level card is the overall total. A usage report that
names only an instance id leaves the parent's context ring and model alone. There is no
per-delegate context ring beside it, on the same rule that keeps a parent's occupancy off a
delegate's card at all (`G96`, `G416`). **The state's own `status_chip` is neither in the first row nor in
the footer's flow (T-99).** It is pinned to the card's own bottom-right corner, flush with the
block's edge past every row's own padding, so it never has to fight the footer for space and the
first row says only whose card it is. The full model id is a hover away on the card, and readable
in full in the conversation the dock now opens for it.

**The screen is one field of state and everything else.** A selection is either a **session** — a
named piece of work — or an **agent**, which is one workspace: one running harness, one terminal.
Which session the graph draws and which tasks the drawer lists are both functions of that one
field, so the two cannot disagree about what the user is looking at; selecting an agent also opens
its conversation in a `Chat` panel in the right dock.

**What a thing is and where it is drawn are separate, and the graph arranges itself.** No position
is authored anywhere. Position is held apart from the definitions and held relative — a task owns an
origin, an agent owns an offset inside the task it serves. Sessions stack down the canvas, each one
clear of the last, whichever arrangement is on. **An agent nobody gave work to sits above its
session's containers**, which is where an agent coordinating a whole session's work belongs and
where one coordinating the project ends up; that row is stacked as a spawn tree of its own. A card
whose parent is in another container stays on its own container's top row, and the connector is
drawn across the boundary — which is what lets one agent parent every session's master without
sinking any of them a level.

**The arrangement is computed bottom-up, and it is one of twelve (`Algo::ALL`).** The cards inside a
container are packed first, which is what fixes the container's size; those fixed boxes are then
packed against each other. A fence is therefore always tight round its own cards, and no arrangement
reserves canvas it does not fill. **Five arrangements remain** (`Algo::ALL`), each pairing its own
`ring` (delegate placement inside a fence) with the fold rule below. A name a settings file holds
for an arrangement since removed (Flow, Packed, Tree, Columns, Adaptive, Multiline, Organic) loads
as **Radial**, the default (`#[serde(other)]` on `Algo`).

| Arrangement | What it does |
|---|---|
| **Radial** | Delegates ring their card, packed against the screen's rectangle |
| **Multiradial** | Every parent is a hub, its children set out on arcs below it |
| **Spider** | One hub per container, cards on arcs, spokes drawn back to the parent |
| **Hex** | Cards laid on a honeycomb, a row per hand-off depth, rows offset half a cell |
| **Islands** | Disjoint families drawn as separate islands; no root is assumed |

**Which arrangement is on is the toolbar's dropdown, and picking one applies it.** The control
reads as the arrangement it is showing, and a pick asks for it at once — there is no second button
to press. What it computes discards every hand-placed position, which makes it the only thing that
undoes a drag. The choice is drawn on the window's own `TeamsView`, exactly like zoom, but unlike
zoom it does not stop there: a pick also writes `UiSettings::teams_algo`
(`AppState::set_teams_layout`) and sends it down the Ui settings layer, so it is the app-wide
default from that point on — the next project opened, and the same project on the next run, both
open in it. A card that arrives afterwards on the canvas that made the pick is given the place that
arrangement would have given it, and nothing already on the canvas moves.

**A fresh graph opens in the arrangement `UiSettings::teams_algo` names**, seeded once when a
project is entered (`AppState::sync_projects`) rather than read on every relayout — a canvas the
user has since repicked an arrangement for keeps it, whatever the setting says afterwards (the
toolbar pick that changed it included: only a *later* pick on a *different* canvas moves it again).
The setting can also be reached directly, as a row on the Appearance page
(`AppState::set_teams_default_algo`), the same `Picker` the toolbar's own dropdown draws from
`Algo::ALL` — the two writers share the one field, so there is one fact rather than two to keep in
sync. The toolbar also carries a **Rearrange** button beside **Fit** — the toolbar's own name for
`AppState::tidy_teams`, the full relayout that already existed for a picked arrangement to run
through, now reachable on demand: hiding the done delegates or the finished cards leaves gaps a
fresh tidy closes.

**Every arrangement folds the loose block to the screen's shape, the
same fold a container's own cards get.** The row of agents with no task — a session's coordinators,
or the window span's masters — used to be stacked raw, one row per hand-off depth however wide it
ran, whichever arrangement was on; it now takes the arrangement's own
`target` through `stack_aspect`, exactly as `Algo::inside` already folds a container's. Left
unfolded, a session with several top-level agents drew that row running off the right of the
canvas while the containers under it wrapped correctly — the top-level counterpart of the ring fold
[`wip-teams-layout-spike`](../wip/teams-layout-spike.md) describes.

**The canvas is infinite: pan and zoom, not scroll.** `TeamsView::pan` (screen px, unbounded) and
`zoom` (`ZOOM_MIN` 0.25 to `ZOOM_MAX` 4.0) are the whole transform; `ui::kit::blocks::pannable` is
the clipped frame and `Board::free` the board placed at the pan. Wheel and a trackpad's two-finger
scroll pan on both axes (`AppState::wheel_teams`; shift turns a vertical wheel horizontal);
Cmd/Ctrl+wheel and a trackpad **pinch** (`gpui::PinchEvent` via `on_pinch`, `AppState::pinch_teams`)
zoom around the pointer (`TeamsView::zoom_at`). **Screen to canvas happens in one place,
`TeamsView::to_canvas`** (after `in_frame`, which uses the frame origin the grid layer records each
paint); drags, the carried card's drop point and the sand all go through it. The grid is
`canvas::pan_grid`, painted from the frame, pan and zoom so it always fills the viewport, and doubled
in pitch while closer than `GRID_MIN_PITCH`. The toolbar's `-`/`+` zoom around the viewport centre. The other boards (WBS, the sink's teamsim) still use the bounded `scroller`.

**The toolbar's own `+`, beside Rearrange and Fit, opens the right dock's agent panel with nothing
attached.** Before any card is picked, `Teams` holds no `Chat` panel in the right dock at all —
unlike `Ide` and `Kb`, where a persistent agent's tab joins that dock at project entry whether or
not the region is open (`AppState::settle_persistent_chat`). `AppState::open_teams_new_agent_panel`
reveals the same panel `select_in_teams` would have opened for a picked card, sharing its
reuse-or-mint step, so the two never grow a second tab between them; the revealed tab's own header
then offers *New agent* or *attach existing*, the same chevron every chat tab carries.

**The graph draws a project's agents as cards on a dotted ground, and opens on all of them.** A
card carries the agent's name, role, state, the one line it says, its branch and its token count, in
the colour of the state's bucket. Zoom scales positions, cards and type together, so the graph reads
the same at every step.

**A card and its delegates read one status pair.** `state::status` holds the two dictionaries both halves speak — a `Lifecycle` and a
`Doing` — and `agent_status` and `delegate_status` are the two derivations onto them.
`agent_status` reads the live `Conversation` first, because its `run` and `stop_reason` carry the
distinction the record cannot: a harness whose last turn ended but is still running answers `Idle`,
where `WorkAgent`'s `Ended` covers both that and a harness gone for good; it falls back to the
record only for an agent it holds no conversation with. That fallback never reads `Working`: the
host registers every agent as `Thinking` and never writes the field again, so a record's working
activity reads `Idle · Unknown` — otherwise every agent the window does not hold would pulse, and
one endless pulse redraws the whole window on every display frame.

**The mark is a hexagon, and it carries both halves at once — a static outline and a separately
animated core (T-99).** `ui::teams::status::status_mark(status, side, id)` over
`kit::hex_mark(id, border, fill, side, pulse)`: the outer hexagon is a *stroke only* — no fill of
its own, so the block behind it shows through and the mark never becomes a second card background
— coloured by the lifecycle through `ui::work::lifecycle_colour`, and a smaller filled hexagon
layered over it carries the activity or the result, grey (`theme::text_faint`) rather than
`doing_colour`'s own answer once `Doing::Done` (T-106) — a delegate that finished clean does not
keep reading as still going, and a Teams card's own edge and its `status_chip` take the same
grey-for-done reading through `ui::teams::status::card_colour`, so the mark, the edge and the chip
cannot disagree — `tech/ui-and-design.md` states the rule once, for all three. So a lifecycle
transition changes the border without
destroying the activity reading, and a result arriving changes the fill without claiming the
execution is still going: a completed delegate is a grey outline round a grey core, a failed one
round a danger-red core. An execution whose activity nothing reports draws the outline alone rather
than a guessed colour. **The core alone pulses, and only while `Lifecycle::Working`** — not idle,
not done, not waiting on the reader, the three restful readings a moving core would cry wolf over;
`id` names the animation so two marks on one canvas never share a clock. **The chip is no longer
beside the mark** — it is the card's own bottom-right corner badge, pinned past the footer's flow —
but it reads the same pair: `status_chip` carries the glyph ahead of the compact word
(`Status::chip`), and `Status::label` is the precise pair the tooltip says. `Writing` and `Tools`
wear `UbiqIcon::PaneWriting` and `UbiqIcon::PaneTools`, drawn in the registry's `pane` set for
exactly these readings. **An agent card and a delegate card go through the same two functions** —
since the dictionaries were unified there is nothing left at that layer to tell them apart. A
delegate's
tooltip still leads with the state's word ahead of its name, type, model and thinking level.
Neither the tasks drawer nor a connector reads the pair: each colours by
`activity_colour(agent.activity)`, so both report `Ended` where the card reports `Idle` (`G279`).

**Three filters narrow it, all of them clear, and the first two are now the same shape.** Sessions
and states are each a set, several on at once, so both are a `kit::MultiPicker` —
`MenuId::TeamsSessions` and `MenuId::TeamsBuckets` — a chip saying what is ticked, opening a list
that stays down while a second is ticked. Under the window span a session *is* a project, which is
why the toolbar reads `MenuId::TeamsSessions` as "projects", labels the section accordingly, and
dots each row in the owning project's tint (`T-142`); under the project span every session belongs
to the one project and the dots carry nothing to say. Ticking several sessions or several projects
narrows to their union, the way the row of chips it replaced already did. The states control's row
carries its bucket's own colour instead. Any row in either control may be the last one turned off,
because **a control with none ticked is not filtering**: it means "narrow it to these", and
narrowing to nothing is what an untouched control already does — which is why each reads as `all
sessions`/`all projects` or `all states` when empty. That is what makes an empty canvas honest — it
means an empty project, never a filter the user cannot find their way back out of. No toolbar
control puts every filter back at once (`T-258`) — each is already undone where it is set, and the
one that did that drew and vanished mid-typing, shoving every control to its right sideways under
the pointer. `AppState::clear_teams_filters` and `TeamsView::filtered` stay, because
`crates/ubiq/tests/teams.rs` pins the state behaviour they name.

**The tasks drawer is a flat list of tasks.** A task that is not ready wears the same `waits on`
mark the board draws.

**The third is `Hide done`, and it is about delegates rather than cards.** A tick box after the
states control, on `Teams` alone — a toggle rather than a set, which is why it stayed a tick box
when the states became one control. The states hide whole cards; a card's ring goes on growing under it
for the life of the conversation, because every delegate a transcript ever named is still named
there — so a long session ends as a wall of finished boxes round the three that are working, which
is the clutter it answers. **Done is a state the data already carries, not a timer**: a delegate *is*
its spawning `Task` call, that call's `ToolStatus` reaches `Completed` when the delegate returns, and
`Doing::Done` — with lifecycle `Ended`, because a delegate that came back is not running — is that
reading. Nothing infers an ending from how long a delegate has been
quiet (`G283`). `Failed` and `Unknown` stay on screen — an error is what a reader came to find, and a
delegate whose spawning call the transcript does not hold is not a delegate anything says is over.
`TeamsView::drawn_delegates` is the one place the rule is applied, read both by the canvas that draws
the boxes and by `settle_teams`, which writes the ring counts the arrangement packs against — so a
hidden delegate stops taking room the next time the graph is laid out. Like the states control, the
tick box does not itself relayout: a filter narrows what is drawn, and throwing every hand-placed
card away is `Tidy`'s job. **It is the one Teams filter that survives a restart**, carried in
`ViewPrefs::teams_hide_done` beside the board's two, because it is a reading preference about a
canvas the user comes back to; which session and which buckets are showing are questions a reader
asks now, and are not written down.

**Which sessions are drawn and which is selected are two questions, and the sessions filter answers
only the first.** Ticking a row in `MenuId::TeamsSessions` narrows the canvas and nothing else — a
set has no one row to hand the selection to the way the single pill it replaced once did. The
tasks drawer keeps reporting on whatever `TeamsSelection` last named, which a card
click or a nav link still sets, and goes on reporting it while every session is on screen (`T-142`).
**A route back to "just look at this one session" is a second, opt-in target on the same row**
(`T-146`): `kit::MultiPicker` grew `on_select`, a trailing arrow beside the tick that calls
`select_in_teams(TeamsSelection::Session(id))` without touching `graph.sessions` — the tick still
only narrows what is drawn, the arrow only moves the selection. A session opens no chat panel
(`select_in_teams`'s own rule — it has no conversation of its own), so the effect is on the canvas'
fences and the tasks drawer's `active_session`, exactly as a card click's effect is on the right
dock.

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

**A hand-over is within a project.** Under the window span the canvas draws every held project's
containers, and a card may only be filed into one of its own project's: a foreign container never
lights up under it, so the drop is not offered rather than refused after the fact. Such a drop is
one on open ground — the card stays where it was let go of, and nothing is asked of any host.

**A container is dragged by the ground inside it, and takes everything with it.** The empty space in
a box is the handle, and the cards drawn over it take their own drags, so grabbing a card moves one
agent and grabbing anywhere else moves the task. Only the container's origin moves, so nothing
inside can fall out of step, and a container is never dropped into another one.

**A drag leaves a trail, and it is the only motion on the screen.** Grains fall where the pointer
passed and shrink, drift and fade over the next two-thirds of a second, so a thing that moved reads
as held rather than as a redraw. Cards and containers both shed it; reduced motion skips it.

**`Teams` has no inspector and no composer of its own.** It draws the actual conversation instead,
because it selects only among agents this window holds a live `Conversation` with — but it draws it
in the right dock rather than beside the graph. Selecting a card or a delegate is
`AppState::select_in_teams`'s cue to also call `open_teams_agent_panel`, which reuses the project's
first `Chat` panel — or mints one — and points it at the agent, revealing the dock if it was put
away. **The tab is the project on screen's whoever owns the agent**, because the dock's chat leaves
are exactly the active project's tabs (`sync_chat_panels`) and a tab minted into a background
project draws no panel at all; under the window span the attachment therefore names a card another
held project owns, and the readers behind it — `teams_conversation()`, `teams_agent()`,
`project_of_agent()` — each resolve that project for themselves. From there it is an ordinary chat tab: `ui::conversation::render`, the same transcript, footer
and composer the chat panel and a column use, at that tab's own pooled composer slot — nothing here
is mocked, and nothing about the card's selection is a second, Teams-only conversation surface.

## Implementation

`state/teams.rs` holds `TeamsView`, the screen's *view* of that work: the selection, the sessions
being drawn, the showing buckets, the zoom, what a drag is carrying, the grains behind it, and the
arrangement. It holds no records, which is why every reader takes a `WorkProjection` as its first
argument — the shape `BoardState`'s readers have. Nothing here draws and nothing names a colour.
`visible()` is the filters together and needs no projection, because each is answered by the agent
in hand; `showing()` treats an empty bucket list as no filter; an empty `sessions` is every session,
and is its own field rather than a reading of `selection` so that clearing what is drawn does not
throw away what is selected — `active_session()` answers that second question, and scopes nothing.
`filtered()` is what tells the empty canvas whether there is anything to clear. `TeamsHeld` — a
card, a delegate or a container — is what `start_carry()` takes and what `carry_to()`
branches on. `bounds_of()` is the box round a task's cards; `end_carry()` writes the card's new
offset against the container it came to rest in and answers the pair for an `AssignAgent`,
touching no membership itself; `settle_sand()` answers whether the trail still owes a frame.
`live_work(work, live)` narrows a `WorkProjection` to the agents in `live`, dropping a session with no live agent left and a task
none of them serves or holds a step in. `TeamsSpan` — `Project` or `Window`, defaulting to
`Project` — is the screen's scope, and `window_work` is what `Window` reads: one `live_work` per
project, concatenated into a single `WorkProjection` and returned beside a
`HashMap<AgentId, ProjectId>` owner map saying which project each merged agent came from. Nothing
collides, because `AgentId`, `SessionId` and `TaskId` are ULIDs minted per record, so the merge
renames nothing and `state/layout.rs` knows nothing of the span.

`state/layout.rs` holds every position, relative: a task's origin and an agent's offset inside it,
absolute only for an agent with no task. `at()` resolves the two. `Layout::auto()` is the whole
arrangement computed from the records alone and discards every hand-placed position, which is why
only `relayout()`, behind the tidy control, may ask for it. `Layout::place_new()` is what an arriving
record gets instead: it takes a tidy arrangement of everything and adopts it for the keys the layout
has never seen, so a new task takes the next container slot in the same flow-and-wrap order and a new
agent the next offset inside its task, with nothing already placed moving and no second geometry to
keep in step. `CARD_WIDTH`, `CARD_HEIGHT`, `GROUP_PAD` and `GROUP_LABEL` live there because the
outlines, the connectors and the hit testing work from them. Both are tested without a frame in
`crates/ubiq/tests/teams_view.rs`, which asserts no position against the fixture — it has none.

`AppState` carries the span as `teams_span`, the window span's own `TeamsView` as `teams_window` —
beside the per-project ones in `OpenProject::teams` — and the merged owner map as `teams_owner`.
The switch lives in four accessors in `app/shell.rs`, which every reader on the screen already goes
through: `teams()` and `teams_mut()` hand back the project's view or the window's, `teams_work()`
answers `live_work` over the active project's `work` and `agents.live` or `window_work` over every
project the span names — and `teams_over_work()` is that pairing for a drag. `teams_projects()` is the one answer to which
projects the screen is about: the active project alone under `Project`, and under `Window` the
registry's `WindowSlot::projects` narrowed to the ones this window has built an `OpenProject` for,
because `sync_projects()` runs a frame behind the registry. Every write the screen makes resolves
the agent's own project through `project_of_agent()`, and a read that needs a foreign project's
record goes through `teams_conversation()` and `teams_agent()`. There is no composer of the
screen's own: selecting a card reveals the agent's conversation in an ordinary chat tab in the
right dock (`AppState::open_teams_agent_panel`), because `ui::conversation::render` already owns a
composer of its own.
`end_teams_carry()` files its `AssignAgent` against the dropped card's own project, and
`teams_carry_tasks()` is what keeps that pair honest: under the window span it answers the carried
card's own project's task ids, and `carry_to()` lights up no container outside that set. `None` is
every task, which is the project span's answer, because one project's canvas draws one project's
tasks and the question does not arise. `settle_teams()`, called from `render` beside the other
end-of-frame passes, ages the trail, puts down a carry whose drag ended where the canvas's drop
handler never sees it, and gathers the delegate rings and rebuilds `teams_owner` across every project
`teams_projects()` names. `settle_window_layout()` is the other half of the window span's view:
every `wire.rs` arm that lays a project's `TeamsView` out over its own work calls it afterwards to
lay `teams_window` out over the merged projection, `relayout` where the arm relaid and `absorb_new`
where it absorbed. Without it the window view keeps the empty `Layout` it was built with, every
card answers the same default offset, and the span draws one pile in the corner. It runs whatever
span is up, because the view is the window's rather than the screen's: a span switched to after the
work arrived finds its cards already placed.
`app/teams_span.rs` holds `teams_span()`, read off the rail rather than stored — `RailMode::TeamsAll`
is `Window`, everything else is `Project`, so the span is a second rail entry rather than a toolbar
toggle (`D154`, [`wip/teams-cross-project.md`](../wip/teams-cross-project.md)) — and
`teams_project_choice()` beside it, which says whether a start raised from the canvas has to ask
which project it is for: the window span *and* more than one project open, the window span's own
arithmetic rather than `teams_projects()`, which answers for the span that is up and so would make
the control hide itself the moment it was used. `ui/project_face.rs` is the one place a project's initials and
`theme::project_tint` become an element — `ProjectFace` and `project_face(id, cx)`, shared with
`ui/rail.rs`'s project badges, drawn on a card and a session pill only under the window span.
`ui/teams/mod.rs` is the frame; `graph.rs` and `tasks.rs` are its two areas, painted from the
layers in `ui/kit/canvas.rs` — there is no inspector: selecting a card opens that agent's
conversation in the right dock instead (`AppState::open_teams_agent_panel`).

## Failure

| What happens | Result |
|---|---|
| Nothing is selected | The drawer falls back to the first session, so it does not go blank; the graph draws every session and needs no fallback. `Teams` opens no chat panel until something is |
| Every agent is filtered out | The graph says so, with no way to clear filters from the message (`T-258`) — each is already undone at its own control. It says the opposite thing — that no agent is running in this project — when there was nothing to hide, so the two emptinesses are never confused |
| Every bucket pill is turned off | The row is not filtering, and every card is drawn. This is the way back from having turned them all off, which is why no pill refuses a click |
| A task's cards are all hidden | No outline is drawn for it. The task keeps its place in the drawer's list |
| A card is dropped outside the graph | The next frame puts it down where the drag left it, so it cannot stay stuck to the pointer |
| A card is dropped on open ground | It keeps its task and its parent, and stays where it was put |
| A container is dragged onto another | Nothing is filed anywhere. The outlines overlap until one is moved or the graph is tidied |

## Related docs

- [`workbench.md`](./workbench.md) — the rail, the dock and the panels these modes are drawn inside
- [`workbench-agents.md`](./workbench-agents.md) — the other kind of screen over the same records
- [`workbench-tasks.md`](./workbench-tasks.md) — the board over the same tasks the drawer lists
- [`workbench-sink.md`](./workbench-sink.md) — the teamsim page the arrangements are judged on
- [`../tech/decisions.md`](../tech/decisions.md) — `D47`, why the agents and the work are two screens
- [`../backlog.md`](../backlog.md) — what these modes still lack

## Next steps

- Write the graph's arrangement down, so a hand-placed card survives a restart.
