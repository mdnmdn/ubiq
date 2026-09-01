---
id: inbox-graph-layout
title: Proposal — the graph's automatic arrangement
kind: proposal
status: proposal
summary: One arrangement rule applied at three scales — size a task's cards, pack them into their workspace, then pack the workspaces on the canvas with the coordinator lifted above them when there is one — computed innermost-first over the cards actually drawn, with the frame that owns each box, replacing the single top-down flow that puts the project's coordinator halfway down the page, shares one origin between containers and lets a filter move geometry it was not computed from.
read_when: you are changing how the agents screen arranges itself, where a coordinator card sits, how containers and workspaces are packed, or when an arrangement is recomputed
updated: 2026-09-01
depends_on: [feat-workbench, tech-ui, inbox-agent-graph-final]
---

# Proposal — the graph's automatic arrangement

The agents screen arranges itself: nothing on the canvas is authored, and
`crates/ubiq/src/state/layout.rs` decides where every card goes. It does it in one pass from the
outside in — workspaces in the order their first agent happens to appear, each one's loose cards
stacked above its containers, containers flowing left to right until a constant says wrap — and the
pass produces a canvas the project's own fixture does not survive. The coordinator every root
answers to is drawn a third of the way down the page with four connectors climbing back up into the
work above it. Five containers share two origins. A filter that hides one card moves outlines it was
never computed from, and moves the drop targets with them.

This proposes the arrangement be computed **the other way round**: size the innermost thing first,
and let every enclosing frame be packed from sizes that are already known. One rule at three scales,
the coordinator lifted out of the flow at each scale and centred over what answers to it, and every
box owned by the pass that placed it rather than re-derived by the view from a different set of
cards.

## 0. Which words this uses

This document is about a nesting, so it uses the nesting's names from
[`agent-graph-final.md`](./agent-graph-final.md) §1 — **workspace** for the working area that holds
tasks, **task** for the unit of intent that holds agents, **agent** for one running harness. Today's
code and today's glossary still say *session* for the workspace and *workspace* for the agent; that
rename is that proposal's to make and this one does not wait for it. Where a line names a type it is
today's: `SessionId` is the workspace's id, `WorkAgent::task` is the task an agent serves.

**Coordinator** here means a card that spawned other cards and serves no task of its own — the
project manager in the fixture, or a workspace's own lead. It is not
[the coordinator](../product/glossary.md) that owns processes; that one draws nothing and has no
opinion about any of this.

## 1. Where it stands

The pass is `Layout::auto` (`crates/ubiq/src/state/layout.rs:56`) over `arrange` (`:129`) over
`stack` (`:222`), and it runs over three different populations in this order: **placement over every
record**, then **geometry over the visible ones** (`GraphView::bounds_excluding`,
`crates/ubiq/src/state/agents.rs:379-406`), then **the canvas extent over whatever came out of the
second** (`crates/ubiq/src/ui/agents/graph.rs:119-131`). The population narrows at each step and
nothing feeds back. That is the root of most of what follows.

**The coordinator is drawn inside whichever workspace it belongs to.** `arrange:142-152` takes the
agents of *this workspace* with no task, stacks them, and puts the stack above *this workspace's*
containers. In the fixture the coordinator is a member of the fifth workspace
(`crates/ubiq-host/src/work/mock.rs:378-391`) and parents every other workspace's master, so the one
card everything answers to is drawn under four workspaces of work.
`crates/ubiq/src/ui/kit/canvas.rs:73-74` draws a link as a cubic whose control points are pulled
*down* out of the parent and *up* into the child, so every one of those connectors is an S-bend
through its own endpoints. Nothing constrains a parent's workspace to be placed above its children's;
the graph reads correctly today only when the coordinator happens to be `agents[0]`.

It can also be pulled *into* a container: the host gives every mock agent the task its workspace has
in flight (`crates/ubiq-host/src/work/mod.rs:614-624`), the coordinator included, so whenever the
fifth workspace has work in progress the project manager is drawn as a worker inside that outline.

**Containers share origins.** A task with no cards is given an origin and `continue`s without
advancing `x` (`layout.rs:166-169`), so every empty container in a workspace lands on the same point
— in the shipped mock, three containers at `(46, 404)` and two at `(46, 646)`. A task with no
workspace at all is never visited (`:157`) and falls through to `task_origin`'s constant
`(46, 72)` (`:96-101`), which is inside the coordinator's own card. Assigning an agent to any of
them — which `assign_agent` (`work/mod.rs:527`) accepts — drops its card and its outline on top of
another container.

**The outline is measured from cards the placement never saw.** `arrange` reserves `box_w`/`box_h`
from every member (`:172-173`) and throws both away; the view rebuilds the box each frame from the
members that pass `visible` (`agents.rs:388`). Turning off a bucket pill therefore shrinks a
container round the cards that are left and moves its top-left, while its neighbours stay on origins
reserved for the full set. It moves the **drop targets** with it: `task_at` (`agents.rs:361-372`)
hit-tests the same filtered boxes, so a card dropped where a hidden card used to sit lands on open
ground and no `AssignAgent` is sent.

**A box is the hull of its members, so one dragged card stretches it across the canvas.** A card put
down on open ground keeps its membership (`end_carry`, `agents.rs:336-352`), and its container's
outline now spans from the container to wherever the card was left — over every container in
between, and over their drop regions.

**A deleted task teleports its cards to the corner.** The host takes its agents off it
(`work/mod.rs:197-215`), `TaskDeleted` touches the layout not at all (`app.rs:1241-1262`), and the
`AgentChanged`s that follow reach `place_new`, which fills only keys it has never seen — so a pair
stored as an offset inside a container is now read as a point on the canvas (`layout.rs:109-116`),
and a card whose offset was `(0,0)` jumps to `(0,0)`.

**Cards are bucketed by depth rather than grouped under a parent.** `stack` puts every agent at depth
*d* on row *d* in the projection's order (`:242-249`) and centres each row over the widest
(`:251-260`). Two leads in one container interleave their workers on one row, and the connectors
cross. A row never wraps: a coordinated task with eight workers is a 2,336pt box that overflows the
window and forces the next container to wrap immediately.

**Nothing that wraps knows how wide the canvas is.** `LAYOUT_WIDTH` (`:37`) is `1_320.0`, consulted
only between containers (`:174`), and the pass is computed at 100% zoom while the view multiplies by
the zoom — so the flow wraps at 1,056 screen points in a 2,000-point window at 0.8, and never
re-wraps when the window, the inspector or the drawer changes the width available. A row advances by
its tallest box, so a one-card container beside a three-row one leaves two card-heights of hole.

**The canvas order is not the toolbar's order.** Layout discovers workspaces by first appearance in
`work.agents` (`:59-67`); the session pills iterate `work.sessions` (`ui/agents/mod.rs:117-118`), so
the mock's canvas reads 5, 1, 2, 3, 4 while its pills read 1, 2, 3, 4, 5 — and a re-ordered `WorkList`
can silently reorder the whole canvas on the next tidy.

**Four more, smaller.** `place_new` adopts the arriving key's slot from a *tidy* arrangement of
everything (`:86-94`), so a new card lands where a tidy canvas would have put it however the canvas
on screen looks — and a task that changes workspace keeps its old slot entirely (`app.rs:1228-1231`).
`Layout` cannot forget anything, so deleted tasks and vanished agents keep their entries for the life
of the window. Every `TaskCreated`, `TaskChanged` and `AgentChanged` — including one per line typed
into the composer — runs the whole `O(tasks × agents)` pass for one key. And the module's own doc
comment (`:51-55`) says each workspace is laid out from the same top-left corner because only one is
on screen at a time, which the code four lines below contradicts.

## 2. What this decides

1. The arrangement is computed **innermost first**: a task's cards are sized before its container is
   placed, a workspace is sized before it is placed, the canvas is packed last.
2. A **coordinator is lifted out of the flow at the scale it coordinates** and centred over the span
   of what answers to it. A project coordinator sits above every workspace; a workspace's lead sits
   above that workspace's containers; a task's lead sits above its workers — the only one of the
   three that already works.
3. **A canvas with no coordinator is the ordinary case**, not a special one: the band is absent and
   the workspaces start at the margin.
4. **A frame owns its box.** The pass answers where a container is *and how big it is*; the view
   stops deriving one from the cards it can see.
5. The pass runs over the **cards that are drawn**, so a filter cannot move geometry it was not
   computed from, and an outline and a drop target cannot disagree.
6. Wrapping happens against a **width the canvas actually has**, not a constant.
7. A relayout has a **scope**: an arrival re-places one frame, a tidy re-places the canvas.

What it does not decide: nothing here crosses the bus, and `D41` stands — position is the
interface's own fact, membership is the host's. Nothing here changes what a card carries, what a
container prints, or what a drag means.

## 3. The model: three frames, one rule

Four levels, three of them frames that own an origin and a size:

```
canvas
 ├── coordinator band          0..n cards — absent when nobody coordinates across workspaces
 └── workspace frame           one per workspace with anything drawn in it
      ├── coordinator band     the workspace's own lead, when it has one
      └── task frame           one per task with cards in it
           └── card            fixed CARD_WIDTH × CARD_HEIGHT
```

**Every frame answers the same two questions** — *how big am I*, and *where do my children sit inside
me* — and answers the first without knowing where it will be put. That is the whole change. Today
`arrange` needs a `y` before it can place anything, so it can never use a size it has not yet
computed, which is why it computes `box_w`/`box_h` and throws them away.

Position stays relative and gains one level: a card owns an offset in its task, a task owns an offset
in its workspace, a workspace owns an origin on the canvas. Dragging a workspace comes free, a
container dragged inside its workspace stays inside it, and the two-meanings-in-one-map problem —
an offset that is absolute for a taskless card and relative for every other — goes away, because a
taskless card is now a member of its workspace's band and the band is a frame like any other.

## 4. The algorithm

Three passes, innermost first. Each is a pure function from records to a size and a set of offsets:
no window, no zoom, no theme, no colour.

### 4.1 A task's cards — a tidy tree, not a row of buckets

The cards in one container are a forest: edges are `parent` links landing inside the same container,
and a card whose parent is outside it is a root here — the rule
`an_orchestrator_parents_each_sessions_master_without_sinking_it`
(`crates/ubiq/tests/agents.rs:443`) already pins.

Lay each root out as a **tidy tree**: children in a row under their parent, sibling subtrees packed
left to right with `CARD_GAP_X` between them, the parent centred over the span of its own children,
roots packed left to right with the same gap. A row of children wider than the target width wraps
into a block under its parent rather than running off the canvas.

That draws the three shapes without naming them, which is what the current rule intends and does not
achieve: `Direct` is one node, `Chain` comes out as a column, `Coordinated` is a lead over its own
workers. Unlike depth-bucketing, a worker sits under the lead that spawned it, two leads do not
interleave, and no connector inside a container crosses another.

The frame's size is the extent of the placed cards, plus `GROUP_PAD` on every side and `GROUP_LABEL`
above — and **at least the width its label needs**, so a long title no longer overruns the box it is
printed on.

### 4.2 A workspace — the lead on top, the containers packed under it

Take the sizes §4.1 produced.

The workspace's own coordinator — an agent of this workspace with no task, that cards in this
workspace answer to — is lifted into a band above the containers. Several such cards are themselves a
forest and get §4.1's tidy tree.

The containers are then packed into rows against the target width (§6) by **shelf packing in the
workspace's task order**: fill a row until the next box would exceed the width, start the next row a
`TASK_GAP` below the tallest box in the one above. Order is the task order rather than tallest-first,
because a stable arrangement is worth more here than a tight one — a task that moves column on the
board must not jump across the canvas.

Two refinements, both cheap: **order the containers by their root's parent** — sort by the horizontal
position of the card their root answers to, one barycentre pass, so the connectors leaving the band
fan out instead of crossing — and **centre the band over the packed rows**, so a lead over a
three-container row sits over the middle of it rather than its left edge. The workspace's size is the
extent of its band and its rows together, plus its own label.

### 4.3 The canvas — the project coordinator on top, the workspaces packed under it

The same rule, one scale up. A **project coordinator** is a card that serves no task and whose
children are in more than one workspace. It is drawn in a band at the top of the canvas, outside every
workspace frame, centred over everything below it — the fix for §1's first and worst defect, and one
that holds however the records happen to be ordered. Several such cards, or a coordinator answering to
another coordinator, are §4.1's tidy tree again.

The workspaces are then packed with §4.2's shelf packing, using the sizes §4.2 computed, **in
`work.sessions` order** — the toolbar's order, so the canvas and the session pills finally agree —
wrapping at the target width, and ordered by the same barycentre rule when there is a band above them
to fan out of.

**A workspace is drawn as a frame**: a rectangle with the workspace's name above it, in the same
idiom as a task's outline but one weight lighter. This is new drawing and it is what makes packing
legible — two workspaces side by side without it read as one field of containers. It also gives the
workspace's drag a handle, the way a container's empty ground is its handle today.

### 4.4 What the pass produces

One `Layout`, as today, but carrying **a box per frame** rather than an origin per frame: where a
task or a workspace is, and how big it is. `GraphView::bounds_of` becomes a lookup instead of a fold
over visible cards, which closes the filter defects in one move — the outline, the drop target and
the placement are then the same three numbers, computed once.

## 5. No coordinator, and other honest shapes

**No coordinator at all** is the common case and costs nothing: the band is empty, takes no height,
and the workspaces start at `LAYOUT_MARGIN`. Nothing else in the pass changes — which is the test of
whether the band is a special case or simply the first row. It is the first row. **A coordinator a
filter has hidden** leaves its children as roots, and the band is absent for the same reason; the
children do not move up into it.

**A coordinator with no children** — a card that serves no task and spawned nothing — is not a
coordinator. It goes in its workspace's band, where today's rule already puts it. **Several project
coordinators**, each parenting a different set of workspaces, are a forest in the band, with the
workspaces ordered so each cluster sits under its own parent. **A parent cycle** cannot hang the
pass: the walk is bounded by the edge count, as `depth_of` (`:286`) already is, and a card in a cycle
is a root.

**A container with no cards drawn** — nobody serves it, or a filter hid everyone who does — is not
placed and takes no room, and a card dropped into it lands in the frame that drop created rather than
on a shared constant. That is §1's shared-origins defect, fixed by not answering the question until
there is something to answer it about: `task_origin` for an unplaced task is the canvas's next free
slot, not `(46, 72)`. **Nothing to draw** is a canvas with no frames, which the view already handles
with its two empty states.

## 6. Where the target width comes from

The pass needs one number it cannot derive: how wide it may grow before wrapping.

**The window measures, the pass is told.** The graph is drawn inside a scrolling container whose
bounds the view has; the canvas width at 100% zoom — the panel's width over the zoom — is written
into the graph's state when it changes, and `relayout` reads it. A pass that has never been told a
width falls back to `LAYOUT_WIDTH`, so every test that builds a `GraphView` by hand keeps working.

Two properties this has to keep. The target is the width **at 100% zoom**, so zooming scales a
finished arrangement and never rearranges it, exactly as today. And it is a *target*, not a clamp: a
single container wider than the panel is placed rather than squeezed, and the canvas scrolls.

## 7. Build it or take a crate

The pass is two jobs. §4.1 and the two bands are **edge-driven** — trees laid out so connectors read.
§4.2 and §4.3 are **box packing** — rectangles of known size wrapped into rows. A general graph-layout
crate is built for the first and does not do the second: a layered engine handed several unconnected
workspaces spreads them along one rank, which is the opposite of the wrap this proposal exists to
get.

What is available, checked against this use:

| Crate | Version | License | Weight | What it gives here |
|---|---|---|---|---|
| `taffy` | 0.13, **already in the tree** | MIT | none — `gpui` depends on it | Flexbox wrap over sized boxes: §4.2 and §4.3's packing, exactly |
| `dagre` | 0.1.1 (2026-05) | Apache-2.0 | one dep (`log`) at `default-features = false` | Layered top-to-bottom **with compound nodes**: containers get computed bounds, crossings minimised |
| `dugong` | 0.8.0-alpha.5 | MIT/Apache-2.0 | `rustc-hash`, `serde`, `serde_json`; MSRV 1.95 | The same dagre semantics, from a repository that is alive |
| `rust-sugiyama` | 0.4.0 | MIT | `petgraph 0.8` — a new dependency here | Layered layout, **no clusters**, so §4.2 stays ours anyway |
| `layout-rs` | 0.1.3 | MIT | bundles a DOT parser and an SVG backend | Unusable: no nested graphs, and it renders |
| `forceatlas2`, `fdg` | — | AGPL-3.0 / unmaintained since 2022 | — | Force-directed is the wrong shape for a spawn tree, and the first licence is one a desktop product cannot take |

`petgraph` is in `Cargo.lock` but only down a Linux-only path — `tree_magic_mini → wl-clipboard-rs →
arboard` — so on macOS it is a new dependency, not a free one. `taffy` genuinely is free: `gpui`
compiles it already.

**The recommendation: take `taffy` for the packing, write the tree.** Wrapping sized rectangles into
rows with gaps, alignment and a target width is what a flexbox engine is; it is already compiled;
`Layout` hands it sizes and reads back positions, with no window and no styling in the way. The tree
is the other half and for this graph it is small — the edges are single-parent `parent` links and the
forests are shallow, so a tidy tree over them is on the order of 120 lines.
`crates/ubiq/src/state/scene.rs` sets the precedent for that trade, writing thirty lines of base64
rather than taking a dependency for it.

**When that stops being right, `dagre` is the exit, and taking it must not mean rewriting the
callers.** The moment the graph has edges that skip a rank, a card with two parents, or crossings no
sibling ordering can fix, the right answer is the barycentre and Brandes–Köpf passes a crate already
has. `dagre` handles compound nodes and answers a container's bounds directly, so it can take over
§4.1 *and* §4.2's inner geometry behind the same `Layout` surface, leaving only the wrap. Its risk is
stated rather than discovered: the upstream repository is archived — survivable at Apache-2.0 and 17k
lines, and the reason it is not the first choice today. `dugong` is the same design from a live
repository, in alpha, with a low-level API its own README declines to call a supported surface. So:
no new dependency in phase 1, `taffy` from phase 2, and a named upgrade path.

## 8. What `Layout` keeps, and the one seam that moves

The pass is replaced; the surface it is used through is not. Every caller a rewrite must keep
working:

| Item | Reached from |
|---|---|
| `CARD_WIDTH`, `CARD_HEIGHT`, `GROUP_PAD`, `GROUP_LABEL`, `CARD_GAP_X/Y` | `ui/agents/graph.rs`, `state/agents.rs`, `tests/agents.rs` |
| `Layout::auto` | `GraphView::relayout` (`state/agents.rs:178`) ← `app.rs:1185` (`WorkList`), `app.rs:2622` (tidy) |
| `Layout::place_new` | `app.rs:1204`, `:1231`, `:1272` (`TaskCreated`, `TaskChanged`, `AgentChanged`) |
| `task_origin` | `GraphView::place`, `carry_to`, `end_carry`, `Layout::at` |
| `at` / `offset` | `GraphView::at`, `at_id`, `bounds_excluding` — and the view, every frame |
| `place_task` / `place_agent` | the two drags |

Three change shape: `auto` takes the drawn set and the target width, `at` resolves a frame chain one
level deeper, and **`bounds_of` moves off `GraphView` and onto `Layout`** as the box the pass
computed. That is the seam — `state/agents.rs:375-406` holds the second, disagreeing geometry, and
deleting it is most of the fix. Two things are missing outright and are added: **`Layout::forget`**,
since nothing prunes a deleted task or a vanished agent, and a **scope on a relayout** (§9).

## 9. When an arrangement is recomputed

Today: on `WorkList`, on any record arriving (`place_new`), and on the tidy control. Filters and
resizes do neither.

After this, a relayout names its scope:

| What happened | Scope |
|---|---|
| A filter changed | the canvas — the pass is over the drawn set, so an outline can never disagree with it |
| The panel was resized past a threshold | the canvas, debounced to one per frame |
| A card arrived, or changed task | its container, keeping every other frame's origin |
| A task arrived, or changed workspace | its workspace's packing, keeping every other workspace's origin |
| A task or an agent went away | its frame, and `forget` for what it left behind |
| A card or a container was dragged | nothing — a drag pins what it moved |
| Tidy | the canvas, discarding every pinned position |

A scoped relayout is what `place_new` was reaching for and could not express: an arrival gets the
place the *current* arrangement would give it, not the place a hypothetical tidy one would — which
fixes a new card landing on a hand-placed sibling, and a task that changes workspace staying in the
old one's row.

Relayout on a filter change costs the user their arrangement, and the cost is worth stating: the
alternative is an outline that lies about which cards it contains and a drop target that is not where
the outline is. **The pass that runs over the drawn set is the only one that cannot disagree with
itself**, and a hand-placed card a filter has hidden was not on screen to be attached to anyway.

## 10. What it costs

The pass is linear in cards plus the packing, so size is not the question at any scale Ubiq will
see — and the scoped relayout makes the common case cheaper than today's, which runs everything for
one key.

**An arrangement is less stable than today's.** A card that changes bucket changes which cards are
drawn, which changes a container's size, which can change a wrap — so a filter change can move things
the user was not looking at. Shelf packing in a fixed order rather than best-fit keeps this to a
minimum: a container that grows pushes what follows it and nothing else. **The workspace frame is new
drawing** on a screen that is already dense, and it is proposed only because packed workspaces are
not readable without it.

**Two tests change, and both were pinning a defect.**
`two_sessions_are_laid_out_clear_of_each_other` (`tests/agents.rs:395`) asserts a card coordinating
the project sits above *its own workspace's* container — it becomes an assertion that a card
coordinating one workspace sits in that workspace's band, with a new test for a card coordinating
several sitting above all of them. `a_card_dropped_on_open_ground_only_moves` (`:543`) leaves a card
far from its container and asserts nothing about the outline; it gains the assertion that the
container's box is the one the pass computed rather than a hull stretched to the dropped card.

## 11. Phases

1. **The tidy tree inside a container.** `stack` is replaced; nothing outside `layout.rs` changes.
   Crossings inside a coordinated task go, rows wrap, and the chain and direct cases keep the
   geometry the existing tests pin.
2. **Frames own their boxes.** `Layout` carries a box per frame, `bounds_of` becomes a lookup,
   `forget` arrives, and the placement, the outline and the drop target become one set of numbers.
   This is the phase that fixes the shared origins and every filter defect.
3. **The bands and the workspace frames.** The coordinator is lifted at both scales, workspaces are
   packed in the toolbar's order and drawn as frames. The visible fix.
4. **The measured width, and the scoped relayout** — including on a filter change and a resize.

Each phase ships on its own.

## 12. What this asks to be decided

1. **Is a workspace drawn as a frame?** §4.3 says yes; packing is hard to read without it. The
   alternative is one workspace per row, which needs no frame and wastes the canvas.
2. **Does a filter change relayout?** §9 says yes, and it costs the user their hand-placed cards.
3. **Is the target width measured, or just a better constant?** §6 says measured; a constant keeps
   `relayout` a pure function of the records.
4. **Does a workspace become draggable** once it is a frame with an origin? Nearly free, and one more
   thing on the canvas that moves.
5. **How is a project coordinator identified?** §4.3 infers it — no task, children in more than one
   workspace. The alternative is a fact on the record, which is a change to the contract and outside
   this proposal.
6. **Does the graph scroll to the workspace the toolbar selects?** Positions are absolute and
   `show_graph_session` (`app.rs:2546`) neither relayouts nor scrolls, so picking the fourth workspace
   today opens on blank canvas. Adjacent to this proposal rather than part of it, and cheap once a
   workspace is a frame with a known box.

## Related docs

- [`agent-graph-final.md`](./agent-graph-final.md) — the nesting this arranges, and the rename it uses
- [`../features/workbench.md`](../features/workbench.md) — the agents screen, and what it draws today
- [`../tech/decisions.md`](../tech/decisions.md) — `D41`, position is the interface's and membership is the host's
- [`../tech/ui-and-design.md`](../tech/ui-and-design.md) — the tokens and shapes any new frame is drawn in
- [`../backlog.md`](../backlog.md) — where the defects in §1 belong as rows until this lands
