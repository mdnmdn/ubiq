# `_tools/teamsim/`

The teams graph's block positioning, rendered as PNGs an agent can look at, so the arrangements can
be iterated on without building the app.

`algos.py` is a port of `crates/ubiq/src/state/layout.rs` — the same constants, the same function
names, the same arithmetic — plus `adaptive`, the incremental arrangement this spike exists to work
out. `--selftest` runs the Rust file's own tests, ported, and is the evidence a picture can be
trusted to show what the app does.

`shapes.py` is the other half: the arrangements with **no** counterpart in the Rust yet. They are
kept out of `algos.py` so that file stays readable as the port, and they register themselves into
`ALGOS` at the bottom of it.

## Running it

```
just teamsim --list                                     # scenarios and algorithms
just teamsim --selftest                                 # the ported Rust tests
just teamsim --all-scenarios --algo all --sheet         # every cell, plus out/sheet.png
just teamsim --scenario kitchen --algo adaptive --incremental
just teamsim --all-scenarios --algo all --json          # the numbers, for diffing runs
```

PNGs land in `_tools/teamsim/out/` (gitignored), one per scenario × algorithm, plus `sheet.png` —
the contact sheet is the image to open while iterating. `metrics.json` is written every run.
`--incremental` also renders two frames of each growing arrangement, so a scenario's growth reads
left to right beside the from-scratch tidies of the same graph.

A scenario is one JSON file in `scenarios/`, specified by `FORMAT.md`. The same files are the sink's
presets, so anything added here is selectable in the app.

## The arrangements

| Key | Does |
|---|---|
| `flow` | containers in record order, wrapping across the canvas |
| `packed` | the tightest fit — wide rows fold to about `ceil(sqrt(n))` |
| `tree` | containers hang under whoever spawned them |
| `columns` | one card per row |
| `adaptive` | never lays a session out: places each arriving block into what is already on screen |
| `multiline` | delegates in a grid under their card; every fold aimed at the screen's shape |
| `radial` | delegates round their card; containers packed against the screen's rectangle |

The first four are `Layout::auto`. `adaptive` reads the scenario as a history — `FORMAT.md`'s
arrival order — and places one block at a time: a delegate takes the first free slot in its parent's
ring, an agent the first free x in its depth band, a container a first-fit gap near its parent, a
session the space below the last one. **No placed block moves unless the arriving one overlaps it,
and then by the smallest delta that clears it, and only for the blocks in the way.** Hand-placed
`positions` are honoured the same way: whatever a human dragged wins over what the arrangement
worked out.

And the five in `shapes.py`, none of which is in `layout.rs`. Every one of them grows **downwards**:
a connector leaves the bottom of a card and arrives at the top of the next one, so the radial shapes
here are half shapes — the downward sector, never the full turn.

| Key | One container's cards | A card's delegates |
|---|---|---|
| `organic` | children sprayed under their parent, bowed at the ends, then pushed apart | a jittered fan |
| `multiradial` | every parent its own hub: children on downward arcs round it | a fan arc |
| `spider` | one hub per container, cards on concentric downward arcs, spokes to the parent | a fan arc |
| `hex` | cards on a honeycomb, a row per hand-off depth, rows half a cell over | a staggered grid |
| `islands` | disjoint families packed as separate islands, no root assumed | a grid |

All five pack their containers with `pack_islands`, which is the other thing this round added: the
container forest read as **components** rather than as a tree, each component packed on its own and
the components packed against the screen. It is what an arrangement needs when there is no single
parent node — which is the ordinary case, not the exception.

## The metrics

Printed as a table, stamped into each image's caption, and written to `out/metrics.json`.

| Name | Means |
|---|---|
| `bbox_w`, `bbox_h` | the whole arrangement's bounding box |
| `aspect` | `bbox_w / bbox_h` |
| `vp_asp` | that aspect against the viewport's — under 1 is taller than a screen wants |
| `scr_w`, `scr_h` | bounding box over viewport: how many screens of scrolling each way |
| `fill` | drawn card and delegate area over bounding-box area. **Reported, not optimised for** — this spike is about readability, and a dense canvas is not a readable one |
| `balance` | the coefficient of variation of the drawn area per viewport-sized tile. A tower leaves most tiles empty and scores badly; one tile always scores 0, because it fits on a screen |
| `cross` | pairwise connector-segment crossings |
| `l_max`, `l_mean` | the longest and the mean connector |
| `mv_mean`, `mv_max`, `unmov` | displacement. For a grown arrangement it is what each arriving block cost the blocks already placed, summed over the growth; for a from-scratch one it is how far that tidy threw the arrangement that existed |
| `order` | whether the containers' reading order — left to right, top to bottom — survived |

`adaptive` wins if its readability numbers stay near a from-scratch tidy's while `order` stays `yes`
and nothing moves that the arriving block did not actually overlap. `mv_mean` is 0.0 on five of the
seven scenarios and 152.0 on `deep-delegation` and `kitchen` — the two with hand-placed containers.
A container a human placed is put back by `apply_positions` after every arrival, so when a card's
ring grows sideways into one, the block that grew is the one that gives way, and that displacement is
the contract working rather than the contract broken.

## The ring shapes

A delegate is always `SUB_BOX = SUB_WIDTH × SUB_HEIGHT` — 264 × 96, the size the app draws it. No
arrangement scales a block; a ring earns its height by *where* it puts a delegate, never by
shrinking one. A card's delegates are its ring. `ring` answers one slot per delegate; `ring_box`
unions those slots with the card and pads them, and that box — **width and height, not a drop** — is
what every packer and every `inside` reserves. A ring wider than its card is the packers' problem,
and they already read it off the ring's own slots.

| Shape | Worn by | A ring of six measures |
|---|---|---|
| `ring_rows` | `flow`, `packed`, `tree`, `columns` | 264 × 806 — one full-width delegate per row |
| `ring_grid` | `multiline`, `adaptive` | 568 × 482 — two card-wide columns |
| `ring_radial` | `radial` | 844 × 384 — full-width boxes on the card's two flanks |
| `ring_fan` | `multiradial`, `spider` | 1396 × 370 — two downward arcs, the outer one outside the inner |
| `ring_organic` | `organic` | 1396 × 370 — the same arcs, off their seats |
| `ring_hex` | `hex` | 568 × 590 — rows of two, staggered half a cell |

`ring_grid` takes `grid_cols(count) = max(1, ceil(count / GRID_ROWS))`: one column, exactly card-wide
because a delegate is a full `SUB_WIDTH`, until it would run past `GRID_ROWS = 4` rows, then a second
column opens beside it, then a third — width is the cheap axis. `ring_radial` leaves the sectors
straight up and straight down clear, because that is where a card's connectors arrive and leave, and
spreads the delegates down the two side flanks; past one ring's worth a second opens outside it. Each
slot is pushed out along its ray only as far as it takes to clear the card's **rectangle**, so the
ring follows the card's outline instead of ballooning into an ellipse.

`ring_fan` is `ring_radial` turned the other way up: it takes the **bottom** sector and leaves the
flanks, because under `multiradial` and `spider` a card's children are already out to the sides and
the room below it is the room nobody else wants. At 1396 × 370 for six it is the widest ring here and
the shortest, which is the trade this whole spike is built on — a narrower `FAN_SPREAD` buys back
width and spends height, and height is the axis that scrolls. `ring_organic` is the same arcs with a
seeded nudge, backed off until nothing touches: two delegates drawn one over the other read as one
card with a shadow, which is not organic, it is broken. `HEX_COLS` is 2, not 3: three full-width
delegates across is an 816pt ring under a 264pt card, more sideways scrolling than the stagger buys
back.

`ring_pad` is what keeps the reserved box honest: `RING_PAD` goes on each side the delegates actually
push past the card, and nowhere else. A one-per-row ring only ever passes the card downwards, so it
comes out at exactly `CARD_HEIGHT + ring_drop(n)` — the Rust's arithmetic, unchanged.

## The aspect target

`score` and the shelf used to want a square and a constant `LAYOUT_WIDTH = 1320`. Both now take a
`target` — the scenario's viewport as `w / h`, 1.6 by default — passed explicitly down through
`pack`, never read off a global. `target = 1.0` reproduces the old square arithmetic exactly, which
is how the four production arrangements stay untouched; only `multiline` and `radial` ask for the
screen. `wrap_at` does the same for `adaptive`'s row bands, which used to fold at four cards because
`LAYOUT_WIDTH` said so rather than because the screen did.

## What the iteration concluded

Four rounds of render-look-change, judging the PNGs, not the table:

1. **The ring is the dominant term, and it is a two-axis problem.** Swapping `ring_rows` for
   `ring_grid` under `adaptive` is what takes a card's delegates off one tall column, and it is
   still the largest single win in the spike: the grid folds at `GRID_ROWS` and spends width, which
   is the axis that does not scroll. What it does *not* do any more is buy that by shrinking the
   delegate — the fold is the whole of the saving now.
2. **A radial ring has to hug the card's rectangle, not an ellipse.** The first cut put delegates on
   an ellipse and half of them landed on top of the parent card. Clearing the top **and** bottom
   sectors, not just the top, is what keeps a connector legible — a card's edges both carry one.
3. **The ported skyline's greedy area rule collapses same-width containers into a column** whatever
   width it is handed, so sweeping widths in `pack_best` did nothing until the rule could be told the
   width is already decided (`commit`) and to minimise height instead. `kitchen`/`radial` went 5412
   → 3474 on that one change. This is a real finding about `layout.rs`, not about the spike.
4. **A blind slack constant for the shelf is a scenario-fitting exercise.** Scoring a sweep of shelf
   widths against the target does the same job with nothing to tune — and it is the judgement
   `pack_best` already makes. `deep-delegation`/`multiline` went 1254 → 634 tall.

**`adaptive` wears `ring_grid`.** It is strictly better than `ring_rows` on every scenario and
`order` is `yes` throughout; the only blocks it displaces are the ones a widening ring genuinely
grew into, beside a container a human pinned. `ring_radial` under `adaptive` was marginally shorter
overall but moved blocks on four of seven scenarios for no such reason, which is the one thing
`adaptive` is not allowed to do.

The arrangement to put in front of a user is **`adaptive` with the grid ring**. At full block size,
summed over the seven scenarios, `multiline` is now the shortest of the eight (9.95 screens tall)
with `adaptive` fifth (11.54) — a full-size delegate costs `adaptive` more than it costs a
from-scratch tidy, because the grid ring it wears is now two columns wide rather than narrow-and-two.
`adaptive` still gets there without moving anything anyone was already looking at, which
`multiline`'s from-scratch tidy cannot promise. `radial` is the most compact for a card with
six-plus delegates and the prettiest at one card per container, and the hardest to follow once two
constellations sit side by side; it is a per-card option, not a canvas.

Whitespace was chosen over density throughout. `deep-delegation`/`multiline` comes out 1.25 screens
wide and 0.73 tall at a `fill` of 0.44, which is more sideways room than the graph strictly needs;
that is the right answer against the 3.13 screens of *scrolling* the same graph takes under `packed`
or `tree`.

## What the five shapes concluded

Summed over the seven scenarios, in screen-heights of scrolling — the number the whole spike is
about — and screen-widths beside it, because a shape that buys height with width has to show the
bill. Every block here is drawn at full `SUB_BOX` size, the size the app draws it — nothing in this
table is a narrow-delegate number:

| | `multiline` | `islands` | `radial` | `hex` | `adaptive` | `organic` | `multiradial` | `spider` |
|---|---|---|---|---|---|---|---|---|
| Σ screens tall | **9.95** | 10.63 | 10.91 | 11.13 | 11.54 | 12.00 | 12.59 | 13.36 |
| Σ screens wide | 6.69 | 6.18 | 6.94 | 7.21 | **5.54** | 7.67 | 7.94 | 8.29 |
| Σ crossings | 9 | 15 | 18 | 19 | **8** | 13 | 23 | 18 |

1. **At full block size, a plain grid under one root beats grouping by what is connected — but not
   on the axes that matter for a default.** `multiline` is now the shortest arrangement in the
   corpus, ahead of `islands`; a full-size delegate costs a ring-based shape more than it costs a
   tidy that just stacks cards under a card. That does not undo the case for `islands`: it is still
   the shape with no privileged root — a container of unrelated cards is islands, not a column
   pretending they are one tree — and `pack_islands` is still the better packer of the two,
   `pack_forest` included, because it reads the container graph as components instead of assuming a
   spine. `adaptive` is untouched by any of this: it still has the fewest crossings of the eight
   (8, against `multiline`'s 9) and the least width by a wide margin (5.54 screens, next is
   `islands` at 6.18), because it places one block at a time into what already fits rather than
   packing an arc or a fan that has to seat a whole brood at once.
2. **A honeycomb is worth its stagger and nothing else.** `hex` sits in the middle of the pack now
   that every ring seats full-size boxes, and its whole advantage is unchanged: a card sits between
   the two above it rather than under one. A honeycomb of *rectangles* cannot interlock vertically,
   so the rows are a full box apart — the hexagonal part of a hex grid is the offset, not the
   packing.
3. **The pretty shapes cost canvas, and the bill is width.** `spider` is a genuine web — one hub,
   spokes fanning out, every one of them pointing down the page — and it is both the tallest and the
   widest of the eight. Nine cards on one arc need a radius that seats nine cards, and nothing is
   inside it. `multiradial` is the same trade, milder, because a crowded hub opens a second arc
   rather than pushing the first one out. Both are one-hub shapes: beautiful for a lead and its
   workers, hard to read the moment two hubs sit side by side. The same verdict `radial` got, for the
   same reason — and at full delegate size a radial ring's two flanks are a full `SUB_WIDTH` each,
   which is why `radial` is no longer the compact option it was against narrow delegates.
4. **A fan still has to be allowed to fold.** Full-size delegates make a whole brood on one arc even
   more expensive sideways than before — `break_to`, the fold the grid ring already uses, is what
   keeps `organic` and `hex` off the top of the width column. A shape that spreads is not an excuse
   to skip the fold.
5. **Jitter needs a floor.** `ring_organic`'s nudge put two delegates on top of each other on
   `anchors` and `decisions` — visible in the PNG, invisible in every number the tool collects, and
   nothing in the metrics would ever have caught it. The tool renders pictures for a reason.

**`pack_islands` is still the one to carry into `layout.rs`.** `multiline` winning the height sum is
a statement about one card's ring, not about how containers pack; `pack_islands` is a strictly better
default than `pack_forest` at that separate job: on a graph that *is* a tree it packs the one
component and behaves like the tree, and on the ordinary graph — a sweep of unrelated tasks — it does
not degenerate into the ribbon `G306` describes. `adaptive`'s incremental placement is the one to put
in front of a user by default; `hex`, `organic`, `multiradial` and `spider` stay per-canvas choices:
`hex` if the stagger reads better to a human eye than the grid does, the other two for a single hub
and a screenshot.

## Adding an algorithm

An arrangement with a counterpart in `layout.rs` goes in `algos.py`; one without goes in `shapes.py`
and is added to `ALGOS` at the bottom of that file. Either way it is one entry and three small
functions, and nothing else: `inside(task, agents, rings, ring, target)` arranges one container's
cards, `ring(count)` says where a card's delegates go inside its own fence, and
`pack(sizes, parents, target)` packs a session's containers. Every delegate a ring seats is
`SUB_BOX` — the size the app draws it — so a ring earns its shape from where it puts a block, never
from shrinking one; there is no per-arrangement size to plumb through. An arrangement that grows
rather than tidies sets `grow` as well and leaves the other three as its starting shape; one whose
packer reads which container hangs under which sets `forest`. Nothing else changes — the renderer,
the metrics and the CLI read the registry.
