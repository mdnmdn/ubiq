---
id: wip-teams-layout-spike
title: Teams block positioning
kind: wip
status: current
summary: Why the teams graph grows into a tower nobody can read in a rectangular viewport, what the two halves of the spike measured — a Python tool that renders an arrangement and scores it, and a teamsim section of the kitchen sink that drives the production arrangements from the same scenario file — what the measurements say to change, and what five further shapes (organic, multiradial, spider, hex, islands) came out at.
read_when: you are changing how the teams graph arranges its blocks, adding an arrangement, or picking up what this spike left open
updated: 2026-09-22
verified: 2026-09-22
code_anchors: [crates/ubiq/src/state/layout.rs, crates/ubiq/src/state/teamsim.rs, crates/ubiq/src/ui/sink/teamsim.rs, crates/ubiq/src/ui/kit/blocks.rs, crates/ubiq/src/ui/teams/graph.rs, _tools/teamsim/algos.py, _tools/teamsim/shapes.py, _tools/teamsim/FORMAT.md]
depends_on: [feat-workbench, tech-ui]
---

# Teams block positioning

The teams graph had four arrangements and all four grew downward. A canvas that runs five viewport
heights is not read, it is scrolled past, and the whitespace beside it is the room the arrangement
never used. This spike is the two things needed to fix that honestly: a way to see an arrangement,
and a way to measure one.

## The two halves

**`_tools/teamsim/`** is a Python port of [`state/layout.rs`](../../crates/ubiq/src/state/layout.rs)
that renders an arrangement to a PNG and scores it. The port is faithful constant for constant, and
the Rust unit tests are ported with it, so the picture can be trusted to be the picture the
application draws. An arrangement is three small functions in a registry — what a container's cards
do, what a card's delegates do, and how the containers pack — so trying a fifth is one entry.

**The `teamsim` section of the kitchen sink** drives the *production* arrangements, not a copy of
them: a scenario is projected into `WorkAgent`, `TaskRecord` and the ring counts, and `Layout::auto`
is the one call that places anything. A card can be given a delegate, two cards can be linked, and
anything can be dragged; the drag writes back through `Layout::place_task` / `place_agent` /
`place_sub`, exactly as the teams canvas does.

Both read the same file, [`_tools/teamsim/FORMAT.md`](../../_tools/teamsim/FORMAT.md): the scenarios
are the sink's presets and the tool's fixtures, and a scenario copied out of the sink is one the tool
can render.

**Neither draws its own canvas.** The block, fence and connector drawing lifted out of
`ui/teams/graph.rs` into [`ui/kit/blocks.rs`](../../crates/ubiq/src/ui/kit/blocks.rs), which takes
rects and labels and knows nothing about an agent; the teams graph draws through it, which is the
only thing that keeps the two from drifting.

## What the measurements found

The tower has one dominant cause and two that compound it.

**A card's delegates were a single column at full card width.** Six delegates made a card 650pt
taller than its neighbour, and `row_extent` charges the whole row for the tallest card in it, so the
room beside the ring was lost as well. This is most of the height in every scenario that has
delegates at all. The fix is not a smaller delegate: a block is drawn at the size the app draws it,
and the saving has to come from *where* the ring puts a block rather than from shrinking one — a
two-axis fold, spending width before it spends height, is what a column cannot do.

**The packers could not see a ring that was not a column.** `ring_drop` reports a *height*, so a
ring wider than its card is invisible to the arrangement. Nothing could reserve room for a shape
other than the one already there — and a delegate the user has *dragged* already overlaps the card
beside it, because `fence()` grows round wherever a delegate went and no packer is told.

**Nothing knew the viewport is a rectangle.** `score()` charged a packing for being far from
*square*, and `pack_shelf` wrapped against a constant 1320pt whatever the window was.

## What reads better

Two ring shapes, measured over seven scenarios: a **grid** of full-size delegates that folds to a
second card-wide column rather than growing one column taller, and a **radial** ring that seats them
around the card with the top and bottom sectors left clear for the connectors.

The grid is the one to put in front of a user. It improves every scenario, and under the incremental
placement it improves them with *nothing already on screen moving* unless the arriving block truly
lands on it — `wide-coordination` comes out 0.96 screens wide by 1.33 tall, one crossing, nothing
displaced, and the containers in the order they arrived in. The radial ring is the most compact
shape for a card carrying six or more delegates and reads beautifully at one card per container, but
two constellations side by side are hard to follow and it spends a lot of canvas. It belongs as a
per-card choice rather than a canvas-wide arrangement.

## Arriving blocks

A graph is never laid out once. It grows as an agent spawns a delegate, and the arrangement on screen
has to absorb that without rearranging itself. The rule the spike settled on, and measured: **no
placed block moves unless the arriving one would overlap it, and then only by the minimum delta that
clears it, and only for the blocks in the way.** A session is never re-packed.

`Layout::place_new` is where that lives in the application — it tidies a whole arrangement and adopts
positions for unseen keys alone. The measurement that matters is that the containers' reading order
survives and that nothing moves without a block having landed on it, because an arrangement that
wins on shape by reshuffling what the user arranged has failed whatever its numbers say. Displacement
is zero on five of the seven scenarios and 152pt on `deep-delegation` and `kitchen`, which are the
two that pin a container by hand: a container a human placed is put back by `apply_positions` after
every arrival, so when a ring grows sideways into one, the block that grew is the one that gives way.
A push that leaves two containers on top of each other is the failure mode there, and it is why the
push runs further waves until none do rather than moving each container once.

## Five more shapes, and the one that won

The round after the grid ring asked a different question: what if the arrangement is not a stack at
all? [`_tools/teamsim/shapes.py`](../../_tools/teamsim/shapes.py) holds five answers, kept out of
`algos.py` so that file stays readable as the port. They all grow **downwards**, because a connector
leaves the bottom of a card and arrives at the top of the next one, so a shape that seats a child
above its parent draws a line backwards whatever else it wins — the radial ones are half shapes, the
downward sector rather than the full turn.

`organic` sprays a parent's children under it, bowed at the ends and nudged off their seats.
`multiradial` makes every parent its own hub, its children on downward arcs round it. `spider` is one
hub per container with the cards on concentric arcs and the spokes running out to them. `hex` puts
the cards on a honeycomb, a row per hand-off depth, every other row half a cell over. `islands`
assumes no root at all: it groups cards, and containers, by what they are *connected to*, and packs
each group as its own island.

Every block in these numbers is drawn at `SUB_BOX` — 264 × 96, the size the app draws a delegate.
Summed over the seven scenarios:

| | `multiline` | `islands` | `radial` | `hex` | `adaptive` | `organic` | `multiradial` | `spider` |
|---|---|---|---|---|---|---|---|---|
| Σ screens tall | **9.95** | 10.63 | 10.91 | 11.13 | 11.54 | 12.00 | 12.59 | 13.36 |
| Σ screens wide | 6.69 | 6.18 | 6.94 | 7.21 | **5.54** | 7.67 | 7.94 | 8.29 |
| Σ crossings | 9 | 15 | 18 | 19 | **8** | 13 | 23 | 18 |

**`multiline` is the shortest arrangement measured anywhere in this spike**, with `islands` second.
A full-size delegate costs a ring-based shape more than it costs a tidy that just stacks cards under
a card, and that is a fact about one card's ring, not about how containers pack — it does not undo
the case for `islands`: it is still the shape with no privileged root, a container of unrelated cards
is islands rather than a column pretending they are one tree, and `pack_islands` is still a strictly
better default than `pack_forest` at that separate job. `pack_islands` reads the same container
forest `tree` reads, as *components* rather than as a tree: on a graph that is a tree it packs the
one component and behaves like `tree`, and on the ordinary case — a sweep of unrelated tasks — it
does not degenerate into the ribbon `G306` describes. `adaptive` is untouched by any of this: it
still has the fewest crossings of the eight and the least width by a wide margin, because it places
one block at a time into what already fits rather than packing an arc or a fan that has to seat a
whole brood at once.

`hex` reads the cleanest of the five at a glance; its whole advantage is the stagger, a card sitting
between the two above it rather than under one. The three tree-growing shapes are the same verdict
`radial` got, for the same reason: beautiful for one hub and its workers, hard to follow the moment
two hubs sit side by side, and expensive in canvas — nine cards on one arc need a radius that seats
nine cards and nothing is inside it.

Two things the round taught that were not about any one shape. A shape that spreads still has to
fold: the first cut put a whole brood on one arc and `wide-coordination` came out two screens wide
under `organic` and `hex` both, which `break_to` — the fold the grid ring already uses — halves. And
jitter needs a floor: `ring_organic`'s nudge put two delegates exactly on top of each other, visible
in the PNG and invisible in every number the tool collects. That is the argument for the renderer.

## One block, one size

**A block is drawn at the size the interface draws it, under every arrangement.** There is one
delegate box, `SUB_BOX` — 264 × 96 — and `SUB_NARROW` is gone from both halves along with the
per-`Algo` `sub` size that chose between them: `fence`, `ring_box`, `card_box`, `card_lead`, the
`stack` family and every `inside_*` take a ring and no size. A ring earns its height by *where* it
puts a delegate, never by shrinking one, which is what makes the picture the tool renders the
picture the canvas draws, box for box.

`ring_grid` is what pays for that: `grid_cols` is `ceil(count / GRID_ROWS)`, so the ring is one
card-wide column while it is at most `GRID_ROWS` deep and opens a second column beside it rather
than a fifth row — the two-axis fold, spending the axis that does not scroll. The shapes that seat
delegates round a card seat full boxes and therefore seat fewer per arc, and `HEX_COLS` is 2 rather
than 3 for the same reason: three 264pt delegates across is an 816pt ring under a 264pt card.

The corpus is taller for it — `multiline` 9.95 screen-heights against the 9.41 a narrow delegate
bought, `islands` 10.63 against 8.77 — and that is the price of the rule, paid deliberately. A
canvas that reads at a glance is worth more than a canvas that fits because the blocks on it shrank.

## The top-level row had the same bug the ring did

The grid ring fixed a card's *own* delegates; it said nothing about the row of agents with no task
of their own — a session's coordinators, or the window span's masters — which `Layout::arrange`
lays out through the same `stack` every arrangement used before this spike: one row per hand-off
depth, however wide it runs, never folded. Every arrangement past the four original ones already
folds a *container's* cards to the screen's shape (`stack_aspect`, via `Algo::target`); the loose
block above the containers did not, so a session with several top-level agents — `hex`, `islands`,
`multiradial`, `spider`, `organic`, `adaptive` included — drew that row running straight off the
right of the canvas while the containers under it were already wrapping correctly. This is what a
user reported as "the top fences and agents are distributed horizontally so they easily go outside
on the right" while the nested packing read fine.

The fix is one `if`: `Algo::ORIGINAL` (`Flow`, `Packed`, `Tree`, `Columns`) keeps the raw `stack`,
because that arithmetic is the one nothing may disturb; every other arrangement folds the loose
block with `stack_aspect` at its own `target`, the same call a container's cards already get.
`_tools/teamsim/scenarios/loose-coordinators.json` is the scenario that exercises it — nine agents
with no task, nothing else — and it is what shows the difference: every arrangement drew it at
2676 × 476 (1.67 screens wide) before the fix, and `hex`/`islands`/`multiradial`/`spider`/`organic`
fold it to 900 × 844 (0.56 screens wide) after. `flow`/`packed`/`tree`/`columns` are unchanged, by
design; `adaptive` is unchanged too, but only because the Python tool's `adaptive` key is the
incremental placer this spike built (`grow`, below), not the static `Algo::Adaptive` `_arrange`
takes in the Rust — the Rust one is folded like every other non-original arrangement.

## What this spike did not fix

`Layout::auto` stacks sessions down the page unconditionally, so three sessions are at least three
times as tall as one however well each is arranged. No arrangement can reach it: the session loop is
above them all. It is the largest readability loss left, and it is `G305` in the backlog with the
rest of what the port turned up — `G306` for the tree that becomes a ribbon, `G307` for the packing
that wins on area and loses on shape, `G308` for two smaller ones.
