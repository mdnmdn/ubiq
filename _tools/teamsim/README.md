# `_tools/teamsim/`

The teams graph's block positioning, rendered as PNGs an agent can look at, so the arrangements can
be iterated on without building the app.

`algos.py` is a port of `crates/ubiq/src/state/layout.rs` — the same constants, the same function
names, the same arithmetic — plus `adaptive`, the incremental arrangement this spike exists to work
out. `--selftest` runs the Rust file's own tests, ported, and is the evidence a picture can be
trusted to show what the app does.

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

`adaptive` wins if its readability numbers stay near a from-scratch tidy's while `mv_mean` stays at
zero and `order` stays `yes`.

## The ring shapes

A card's delegates are its ring. `ring` answers one slot per delegate; `ring_box` unions those slots
with the card and pads them, and that box — **width and height, not a drop** — is what every packer
and every `inside` reserves. A ring wider than its card used to be invisible to the packers; it is
not any more, which is what let the other two shapes exist at all.

| Shape | Worn by | A ring of six measures |
|---|---|---|
| `ring_rows` | `flow`, `packed`, `tree`, `columns` | 264 × 806 — one full-width delegate per row |
| `ring_grid` | `multiline`, `adaptive` | 264 × 410 — two narrow columns, exactly card-wide |
| `ring_radial` | `radial` | 568 × 336 — narrow boxes on the card's two flanks |

`ring_grid` takes `SUB_NARROW_WIDTH = (CARD_WIDTH - SUB_GAP) / 2` and `SUB_NARROW_HEIGHT = 72`, two
to a row, so the fence is exactly as wide as the card. It widens to three columns rather than run
past `GRID_ROWS` rows — width is the cheap axis. `ring_radial` leaves the sectors straight up and
straight down clear, because that is where a card's connectors arrive and leave, and spreads the
delegates down the two side flanks; past one ring's worth a second opens outside it. Each slot is
pushed out along its ray only as far as it takes to clear the card's **rectangle**, so the ring
follows the card's outline instead of ballooning into an ellipse.

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
   `ring_grid` under `adaptive` took the whole corpus from 12.4 screen-heights to 10.2 with
   displacement still at exactly zero — `wide-coordination` went from 1.52 screens tall to 1.12.
   Nothing else in the spike bought as much.
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

**`adaptive` wears `ring_grid`.** It is strictly better than `ring_rows` on every scenario, with
`mv_mean` at 0.0 and `order` `yes` throughout, so the incremental contract is untouched.
`ring_radial` under `adaptive` was marginally shorter overall but moved blocks on four of seven
scenarios, which is the one thing `adaptive` is not allowed to do.

The arrangement to put in front of a user is **`adaptive` with the grid ring**. `multiline` reaches
the same shape on `wide-coordination` (1536 × 936, balance 0.0) and is the better from-scratch tidy,
but `adaptive` gets there without moving anything anyone was already looking at. `radial` is the
most compact for a card with six-plus delegates and the prettiest at one card per container, and the
hardest to follow once two constellations sit side by side; it is a per-card option, not a canvas.

Whitespace was chosen over density throughout. `deep-delegation`/`multiline` fills 0.68 × 0.63 of a
screen and leaves the rest empty; that is the right answer against the 3.13 screens of scrolling the
same graph took before.

## Adding an algorithm

One entry in `ALGOS` in `algos.py` and three small functions: `inside(task, agents, rings, ring, sub,
target)` arranges one container's cards, `ring(count)` says where a card's delegates go inside its
own fence, and `pack(sizes, parents, target)` packs a session's containers. A ring shape that draws
delegates at something other than `SUB_BOX` says so with the entry's `sub`, and the renderer, the
metrics and every reservation follow. An arrangement that grows rather than tidies
sets `grow` as well and leaves the other three as its starting shape. Nothing else changes — the
renderer, the metrics and the CLI read the registry.
