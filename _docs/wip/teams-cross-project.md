---
id: wip-teams-cross-project
title: Teams across projects
kind: wip
status: current
summary: How the Teams screen draws every open project's agents at once — a second rail entry in the APP group that the span is read off, one merged projection built from each project's `live_work`, an owner map that answers "whose agent is this" for every write the screen makes, and what the rail, the titlebar and a `ubiq://` link keep meaning when the canvas is about more than one project.
read_when: you are changing what the Teams screen is scoped to, or adding a reader that must work when the canvas spans several projects
updated: 2026-09-21
verified: 2026-09-21
code_anchors: [crates/ubiq/src/state/teams.rs, crates/ubiq/src/app/teams.rs, crates/ubiq/src/app/shell.rs, crates/ubiq/src/app/mod.rs, crates/ubiq/src/ui/teams/mod.rs, crates/ubiq/src/ui/teams/graph.rs, crates/ubiq/src/ui/teams/inspector.rs, crates/ubiq/src/state/nav/text.rs, crates/ubiq/tests/teams.rs]
depends_on: [feat-workbench, tech-ui, wip-teams-layout-spike]
---

# Teams across projects

A window holds every open project's state at once — `AppState.projects` is a whole `OpenProject` per
project, not a cache of the active one. What is single-project is not the data but the *reach*:
every accessor funnels through `open_project(cx)`, which is `self.projects.get(&self.project(cx)?)`,
and a screen built on it draws one project because that is the only project its readers can see.

The Teams screen is the one screen that does not. This is how one canvas draws them all.

Registered as `D154`.

## The shape

**A second rail entry, not a control on the canvas.** One screen, two entries: `RailMode::Teams` in
the PROJECT group is the active project's agents, and `RailMode::TeamsAll` — labelled "All Teams" —
is every project the window holds, on one canvas. The entry the rail is on *is* the span, so there
is nothing on the toolbar that says which span is up and nothing on `AppState` that could disagree
with the rail.

`TeamsAll` sits in the APP group, between `Control` and `Sink`, because a canvas about every open
project is a fact about the window rather than a view onto one project — the same reason those two
are there. It is the one APP entry that still wants a project: a canvas about every open project has
nothing to draw when there are none, so `ui/dock/mod.rs`'s centre arm keeps it behind `has_project`
with the PROJECT screens rather than answering ahead of the no-project case.

**Three facts make the whole feature.**

1. `TeamsSpan` — `Project` or `Window` — **derived, never stored**. `AppState::teams_span()` reads
   `workbench.rail_mode`: `TeamsAll` is `Window`, everything else is `Project`. A window's own fact,
   like the zoom and the arrangement: nothing outside this window has an opinion about it, and it is
   not sent anywhere.
2. `AppState.teams_window: TeamsView` — the window span's own view, beside the per-project ones in
   `OpenProject.teams`. Two spans are two arrangements over two different sets of cards, and a
   shared `TeamsView` would mean switching span threw the other's layout away.
3. `AppState.teams_owner: HashMap<AgentId, ProjectId>` — which project each drawn card belongs to.
   A merged projection loses the one thing every *write* the screen makes needs, and this is where
   it is kept instead.

Everything else is those three reaching the readers that already exist.

## The projection

`state::teams::live_work(work, live)` already returns an **owned** `WorkProjection` — a narrowing of
one project's records to the agents this window holds a conversation with, not a field of anything.
That is what makes this cheap: a cross-project projection is the same kind of value, built by
concatenating one `live_work` per project.

```rust
/// Every open project's live work in one projection, and who owns each card.
pub fn window_work(projects: &[(ProjectId, &WorkProjection, &[AgentId])]) -> (WorkProjection, HashMap<AgentId, ProjectId>)
```

**Nothing collides.** `AgentId`, `SessionId` and `TaskId` are ULIDs minted per record, so two
projects' agents never share an id and the merged lists need no renaming, no prefixing and no
composite key. The projection the canvas measures is a `WorkProjection` like any other, which is why
`Layout`, every packer, `fence`, `sub_slot` and the whole of `state::layout` are untouched by this
change.

**Order is the window's project order**, `WindowSlot.projects` — picker order, which never moves —
so a relayout puts the same project's cards in the same region twice running. `AppState::
window_projects` is the one place it is read, sorted by `ProjectId` — creation order, ULIDs being
what they are — when the window has no slot in the registry, because a `HashMap`'s order permutes
on rehash and an order that moved between two reads would move every card with it.

**The window's view is laid out where the project's is.** Every wire arm that learns of an arrival
lays the arriving project's `TeamsView` out over its own narrowed projection, and
`AppState::settle_window_layout` lays `teams_window` out over the merged one beside it — `relayout`
where the arm relayouts, `absorb_new` where it absorbs. Whatever span is up: the view belongs to
the window rather than to the screen, so a span switched to after the work arrived finds the cards
already placed rather than piled on the default offset.

**The registry is ahead of the window by a frame.** `sync_projects` builds an `OpenProject` after
the registry has already listed the project, so the span's project list is `WindowSlot.projects`
narrowed to the ones this window has actually built — an unfiltered list would name a project whose
state is not there yet. `AppState::teams_projects` is the one answer to "which projects is this
screen about", and every reader asks it rather than the registry.

**`teams_projects` cannot answer whether a start has a project to choose between**, which is why
`AppState::teams_project_choice` sits beside `teams_span()` in `app/teams_span.rs`: the span is
`Window` *and* the window holds more than one project it has built state for. `teams_projects`
answers with the projects the span is actually drawing — under `TeamsSpan::Project` always the
active project alone — so neither half implies the other, and the add-agent control asks this one.

## Where the switch lives

One place: the accessors in `app/shell.rs`. Every reader on the screen already goes through them,
and none of `ui/teams/*` calls `open_project` at all.

| Accessor | Span `Project` | Span `Window` |
|---|---|---|
| `teams(cx)` | `open.teams` | `self.teams_window` |
| `teams_mut(cx)` | `open.teams` | `self.teams_window` |
| `teams_work(cx)` | `live_work` of the active project | `window_work` of every held project |
| `teams_over_work(cx)` | `(&mut open.teams, live_work)` | `(&mut self.teams_window, merged)` |

A window span with no project open is `None`, exactly as the project span is — the screen draws
nothing rather than an empty graph to explain.

## What the owner map is for

A merged projection answers every *read* the screen makes. Four writes need to know whose agent they
are about, and each one asks `teams_owner`:

- **`end_teams_carry`** sends `Message::AssignAgent { project_id, .. }` against the dropped card's
  own project. Under the window span the card may belong to any project the window holds, and
  filing it against the active one would move somebody else's agent onto a task it cannot serve.
  The pair the drop answers with is narrowed a step earlier, in `move_teams_carry`: the map names
  the carried card's project, its tasks are the only containers `TeamsView::carry_to` will light
  up, and a foreign container is therefore never offered and never re-anchored to. `state::teams`
  is told *which tasks*, never *whose* — the comparison is the caller's, which is what keeps that
  module free of the word project.
- **`select_in_teams`** points the shared transcript at the selection through
  `view_conversation_agent`, which resolves the project with `project_of_agent` before it writes —
  pointing a foreign card's transcript at a delegate is a write into the project that holds it.
- **`ui::teams::graph` and `ui::teams::inspector`** read `app.teams_conversation(agent, cx)` for the
  rings and for the transcript, which resolves through the owner map under the window span and is
  the active project's answer under the project span.
- **`settle_teams`** gathers `conversation.subagents()` for the ring map. Under the window span it
  gathers from every held project rather than from `open_project(cx)`.

The map is rebuilt in `settle_teams`, alongside the rings, from the same pass that builds the
projection — write-if-changed, so a settled canvas does not touch state every frame.

## The composer

**One selection, one composer, one slot.** `TEAMS_SLOT` is a single slot in the fixed
`COMPOSER_SLOTS` pool and stays that way: the inspector reports on one selection whatever the span
is, so there is nothing here that wants a slot per project. `agent_for_slot(TEAMS_SLOT)` already
answers `teams(cx)?.agent_in_focus()`, which is the merged view's answer under the window span with
no change at all. What does change is the send: the path behind the composer resolves the
conversation's project through the owner map rather than assuming the active one.

## What a card says about its project

Under the window span, and only under it, a card wears its project: the project's initials, with
`theme::project_tint(temporary, colour, custom_colour)` as the chip's **coloured left edge** rather
than as a separate square beside the letters. A square would have been a second mark on a chip that
is already only two characters wide, and the edge carries the same colour in a third of the room.
`ui/project_face.rs`'s `ProjectFace` and `project_face(id, cx)` are the one place it is built,
shared with `ui/rail.rs`'s project badges. Nothing new in `theme.rs` — the places a project already
wears its colour are the pattern, and this is one more.

The session pills carry the same chip, because two projects can name a session the same thing and a
row of bare names would be two pills that look like one.

**The chip is not the whole story: the canvas is fenced by project too.** A chip answers "whose is
this card" one card at a time, which is the wrong shape for the question the window span actually
raises — *where does this project's work stop*. So under the window span every top-level thing on
the canvas wears a dashed fence in `ProjectFace`'s tint: a task container takes the colour of the
project that owns it, and a card no container encloses gets a fence of its own. Nothing is fenced
twice — a card inside a container is not fenced again, because the box round it already carries the
colour — and a project the registry cannot face draws no fence rather than a colourless one.

**The drop state still wins the container's outline.** A container lit as a drop target, or being
carried, keeps `accent` and the heavier dashes `Fence`'s `active` draws: "let go here" is an answer
the canvas gives for a moment and the project's colour is one every card inside it already gives.
The project tint is therefore the resting colour, in place of `theme::border`.

Both readings live in `state::teams` — `TeamsView::fenced_tasks` names the containers and the card
each takes its colour from, `TeamsView::fenced_alone` names the loose cards, and `solo_bounds`
measures a lone fence with a container's padding so the two sit the same distance from what they
hold. Both answer with nothing under `TeamsSpan::Project`, which is how "the project span is
unchanged" is a claim `crates/ubiq/tests/teams.rs` makes without a frame. The colour stays in
`ui::teams::graph`, which is the rule that module has always kept: state names no colour.

Under the project span the canvas is unchanged, chip, fences and all: a screen about one project
does not need to say which, and one colour repeated over every card is not a distinction.

## What the rail, the titlebar and a link keep meaning

**The rail and the titlebar go on meaning the active project.** They are the window's answer to
"which project are the other eight modes about", and a span the user set on one screen does not get
to rewrite them. The active project keeps its filled badge; the cross-project canvas is a wider view
*from* that project, not a window with no project.

**A link names the project of what it points at.** The grammar is
`ubiq://<project-id>/teams/<kind>:<id>/<tab>[/<subagent>]` and it stays exactly that: a teams
destination names the project that owns the *selection*. Two of the three arms own an agent and
resolve through `project_of_agent`; the session arm owns none, and resolves through
`project_of_session` — the first held project with an agent under that session, which is the same
answer the session pills' chip is read off. Under the window span the active project is the wrong
answer for either: a selected session can belong to any project the canvas draws, and a link naming
the project on screen would point at work that project has never held. The span is not in the
address, because the span is not part of the place: the same agent, read on a canvas showing one project and on a canvas showing six, is the
same agent. Following a link therefore lands on the selection in whatever span the window is in, and
a link built while the window span is up is still a link somebody in the project span can follow.

## The reach the span opens up

The Teams inspector draws the whole conversation component at `TEAMS_SLOT`, so the window span puts
a foreign agent behind every listener in `ui/conversation/`, not only behind the four writes above.
Each of those resolves the agent's own project: `app/agents.rs`'s queue trio, `cancel_turn`,
`answer_permission`, `fork_conversation`, `reveal_permission`, `recall_last_message`, the config
picker and the panel and disclosure toggles all take `project_of_agent`, and the three lifecycle
rows read the record through `AppState::teams_agent`, `work()`'s span-aware sibling. The transcript
list body and `ui/conversation/info.rs` read through `teams_conversation` and `teams_agent` for the
same reason — a guard moved on the panel and not on what it wraps draws a panel with nothing in it.

**The rule this leaves behind:** a reader on a path the Teams screen can reach may not ask
`self.project(cx)`. `G326` names the three surfaces where that is still open, and why two of them
are a decision rather than a missing sibling.

## Where this stands

| # | What | Files |
|---|---|---|
| W1 | `TeamsSpan`, `window_work`, the `AppState` fields, the four accessors switching on the span | `state/teams.rs`, `app/mod.rs`, `app/shell.rs`, `app/boot.rs` |
| W2 | The writes: `settle_teams`, `end_teams_carry`, `select_in_teams`, `teams_conversation`, the composer's send, and the conversation component's readers | `app/teams.rs`, `app/agents.rs`, `app/nav.rs`, `ui/conversation/` |
| W3 | The toolbar toggle, the project chip on cards and session pills | `app/teams_span.rs`, `ui/project_face.rs`, `ui/teams/mod.rs`, `ui/teams/graph.rs`, `ui/teams/status.rs` |
| W4 | Tests and the documents this owes | `crates/ubiq/tests/teams.rs`, `_docs/features/workbench.md`, `_docs/tech/decisions.md`, `_docs/backlog.md` |

All four are in the tree. What is open is `G325` — following a teams link that names a
non-active project activates that project on arrival, which is right for the other eight modes and
is a side effect under the window span, where the card the link names is on screen either way.
