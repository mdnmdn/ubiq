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

## Adding an algorithm

One entry in `ALGOS` in `algos.py` and three small functions: `inside(task, agents, rings)` arranges
one container's cards, `ring(count)` says where a card's delegates go inside its own fence, and
`pack(sizes, parents)` packs a session's containers. An arrangement that grows rather than tidies
sets `grow` as well and leaves the other three as its starting shape. Nothing else changes — the
renderer, the metrics and the CLI read the registry.
