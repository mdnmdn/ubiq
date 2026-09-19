---
id: wip-teams-layout-spike
title: Teams block positioning
kind: wip
status: current
summary: Why the teams graph grows into a tower nobody can read in a rectangular viewport, what the two halves of the spike measured — a Python tool that renders an arrangement and scores it, and a teamsim section of the kitchen sink that drives the production arrangements from the same scenario file — what the measurements say to change, and what five further shapes (organic, multiradial, spider, hex, islands) came out at.
read_when: you are changing how the teams graph arranges its blocks, adding an arrangement, or picking up what this spike left open
updated: 2026-09-19
verified: 2026-09-19
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
delegates at all.

**The packers could not see a ring that was not a column.** `ring_drop` reports a *height*, so a
ring wider than its card is invisible to the arrangement. Nothing could reserve room for a shape
other than the one already there — and a delegate the user has *dragged* already overlaps the card
beside it, because `fence()` grows round wherever a delegate went and no packer is told.

**Nothing knew the viewport is a rectangle.** `score()` charged a packing for being far from
*square*, and `pack_shelf` wrapped against a constant 1320pt whatever the window was.

## What reads better

Two ring shapes, measured over seven scenarios: a **grid** of narrow delegates that stays exactly as
wide as the card it belongs to, and a **radial** ring that seats them around the card with the top
and bottom sectors left clear for the connectors.

The grid is the one to put in front of a user. It improves every scenario, and under the incremental
placement it improves them with *nothing already on screen moving* — `wide-coordination` goes from
1.5 viewport heights to fitting one screen, with no connector crossings and no block displaced. The
radial ring is the most compact shape for a card carrying six or more delegates and reads beautifully
at one card per container, but two constellations side by side are hard to follow and it spends a lot
of canvas. It belongs as a per-card choice rather than a canvas-wide arrangement.

## Arriving blocks

A graph is never laid out once. It grows as an agent spawns a delegate, and the arrangement on screen
has to absorb that without rearranging itself. The rule the spike settled on, and measured: **no
placed block moves unless the arriving one would overlap it, and then only by the minimum delta that
clears it, and only for the blocks in the way.** A session is never re-packed.

`Layout::place_new` is where that lives in the application — it tidies a whole arrangement and adopts
positions for unseen keys alone. The measurement that matters is that it keeps displacement at
exactly zero and the containers' reading order intact, because an arrangement that wins on shape by
reshuffling what the user arranged has failed whatever its numbers say.

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

Summed over the seven scenarios, in screen-heights of scrolling:

| | `islands` | `multiline` | `hex` | `radial` | `adaptive` | `organic` | `multiradial` | `spider` |
|---|---|---|---|---|---|---|---|---|
| Σ screens tall | **8.77** | 9.41 | 10.05 | 10.21 | 10.23 | 10.81 | 12.01 | 12.18 |

**`islands` is the shortest arrangement measured anywhere in this spike**, and it gets there without
a new ring shape, a new score or a tuned constant — all it does is stop pretending the graph has one
root. `pack_islands` reads the same container forest `tree` reads, as *components* rather than as a
tree: on a graph that is a tree it packs the one component and behaves like `tree`, and on the
ordinary case — a sweep of unrelated tasks — it does not degenerate into the ribbon `G306` describes.
That makes it a strictly better default than `pack_forest`, and it is the one of the five worth
carrying into `layout.rs`.

`hex` comes third and reads the cleanest of the five at a glance; its whole advantage is the stagger,
a card sitting between the two above it rather than under one. The three tree-growing shapes are the
same verdict `radial` got, for the same reason: beautiful for one hub and its workers, hard to follow
the moment two hubs sit side by side, and expensive in canvas — `spider` spends 1.96 screen-widths on
`wide-coordination` at a `fill` of 0.09, because nine cards on one arc need a radius that seats nine
cards and nothing is inside it.

Two things the round taught that were not about any one shape. A shape that spreads still has to
fold: the first cut put a whole brood on one arc and `wide-coordination` came out two screens wide
under `organic` and `hex` both, which `break_to` — the fold the grid ring already uses — halves. And
jitter needs a floor: `ring_organic`'s nudge put two delegates exactly on top of each other, visible
in the PNG and invisible in every number the tool collects. That is the argument for the renderer.

## What this spike did not fix

`Layout::auto` stacks sessions down the page unconditionally, so three sessions are at least three
times as tall as one however well each is arranged. No arrangement can reach it: the session loop is
above them all. It is the largest readability loss left, and it is `G305` in the backlog with the
rest of what the port turned up — `G306` for the tree that becomes a ribbon, `G307` for the packing
that wins on area and loses on shape, `G308` for two smaller ones.
