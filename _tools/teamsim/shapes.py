"""The arrangements this round added, which have no counterpart in `layout.rs` yet.

`algos.py` is the port — constant for constant, test for test — and nothing that is not in the Rust
belongs in it. These five are the exploration on top: five ways to arrange one container's cards, and
one way to pack containers that never assumes there is a root.

Every one of them grows **downwards**. A connector leaves the bottom of a card and arrives at the top
of the next one (`_bottom` / `_top` in `algos.py`), so an arrangement that seats a child above its
parent draws a line backwards through the canvas whatever else it wins. So the radial shapes here are
all half shapes: the downward sector, never the full turn.

| Key | One container's cards | A card's delegates |
|---|---|---|
| `organic` | children fanned under their parent, bowed and jittered, then pushed apart | a jittered fan |
| `multiradial` | every parent is its own hub: children on a downward semicircle round it | a fan arc |
| `spider` | one hub per container, cards on concentric downward arcs, spokes to the parent | a fan arc |
| `hex` | cards on a honeycomb lattice, a row per hand-off depth, rows half a cell apart | a staggered grid |
| `islands` | disjoint families packed as separate islands, no root assumed | a grid |

`islands` is the one that answers "what if there is no single parent node": it groups by what is
*connected*, at both levels — the cards inside a container, and the containers inside a session — and
packs each group as its own island against the screen's rectangle.
"""

from __future__ import annotations

import hashlib
import math
from typing import Callable, Optional, Sequence

from algos import (
    ALGOS,
    CARD_GAP_X,
    CARD_GAP_Y,
    CARD_HEIGHT,
    CARD_WIDTH,
    EPS,
    SUB_DROP,
    SUB_GAP,
    SUB_NARROW,
    SUB_NARROW_HEIGHT,
    SUB_NARROW_WIDTH,
    TASK_GAP,
    VIEW_ASPECT,
    Agent,
    Algo,
    Contents,
    Packing,
    Point,
    Rings,
    Size,
    _apart_enough,
    _members,
    _radial_slot,
    break_to,
    card_box,
    card_lead,
    pack_best,
    remap,
    ring_grid,
    stack_aspect,
    without_empties,
)

# ── the constants these shapes added ──────────────────────────────────────────

#: Half the downward sector a fan, a hub's brood and a spider's arc spread over, in degrees either
#: side of straight down. Past 90 a slot climbs back above the card it hangs off, which is the one
#: thing none of these shapes may do.
FAN_SPREAD = 74.0
#: The most delegates one fan arc seats before a second arc opens outside it.
FAN_SEATS = 7
#: The most arcs a fan ring opens before it gives up and crowds the last one.
FAN_ARCS = 4

#: How far an organic slot is nudged off its regular seat, as a fraction of the step to the next.
#: Enough to read as grown rather than drawn; not enough to make two of them touch.
ORGANIC_JITTER = 0.3
#: How much lower the ends of an organic brood hang than its middle, as a fraction of `CARD_GAP_Y`.
#: This is the bow that makes a row of children read as a spray rather than as a row.
ORGANIC_BOW = 1.4

#: A honeycomb's long row. Rows alternate `HEX_COLS` and `HEX_COLS - 1`, offset half a cell.
HEX_COLS = 3

#: Half the sector a spider's web spans, in degrees either side of straight down.
SPIDER_SPREAD = 80.0

#: The room between two islands. Twice what sits between two containers, because the gap is the only
#: thing saying they are not one arrangement.
ISLAND_GAP = TASK_GAP * 2.0

#: How many passes the overlap relaxation makes before it settles for what it has.
SPREAD_ROUNDS = 80


# ── the odds and ends every shape here uses ───────────────────────────────────


def seeded(key: str, salt: int = 0) -> float:
    """A repeatable number in `[-1, 1]` for a name. The jitter has to survive a re-render."""
    digest = hashlib.blake2b(f"{key}/{salt}".encode(), digest_size=8).digest()
    return int.from_bytes(digest, "big") / float(1 << 63) - 1.0


def _clash(at: Point, box: Size, other: Point, other_box: Size, gap: float) -> tuple[float, float]:
    """How far two boxes overlap once `gap` is owed between them. Both positive means they clash."""
    dx = min(at[0] + box[0] + gap - other[0], other[0] + other_box[0] + gap - at[0])
    dy = min(at[1] + box[1] + gap - other[1], other[1] + other_box[1] + gap - at[1])
    return dx, dy


def spread(spots: dict[str, Point], boxes: dict[str, Size], gap: float) -> None:
    """Push clashing boxes apart sideways until nothing touches, in place.

    Sideways only, and by half the overlap each: a shape here earns its look from where it put a card
    vertically — under its parent, on its arc, in its row — and buying room by moving a card down the
    page would spend the one axis the whole spike is about.
    """
    keys = list(spots)
    for _ in range(SPREAD_ROUNDS):
        moved = False
        for i in range(len(keys)):
            for j in range(i + 1, len(keys)):
                a, b = keys[i], keys[j]
                dx, dy = _clash(spots[a], boxes[a], spots[b], boxes[b], gap)
                if dx <= EPS or dy <= EPS:
                    continue
                step = dx / 2.0 + EPS
                left, right = (a, b) if spots[a][0] <= spots[b][0] else (b, a)
                spots[left] = (spots[left][0] - step, spots[left][1])
                spots[right] = (spots[right][0] + step, spots[right][1])
                moved = True
        if not moved:
            return


def contents_of(
    spots: dict[str, Point], boxes: dict[str, Size], leads: dict[str, Point]
) -> Contents:
    """Card boxes worked out anywhere at all, normalised into a container's own frame."""
    if not spots:
        return Contents()
    x0 = min(at[0] for at in spots.values())
    y0 = min(at[1] for at in spots.values())
    x1 = max(at[0] + boxes[a][0] for a, at in spots.items())
    y1 = max(at[1] + boxes[a][1] for a, at in spots.items())
    cards = [
        (a, (at[0] - x0 + leads[a][0], at[1] - y0 + leads[a][1])) for a, at in spots.items()
    ]
    return Contents(cards, x1 - x0, y1 - y0)


def family(members: Sequence[Agent]) -> tuple[list[str], dict[str, list[str]]]:
    """The roots of one container's cards and each card's children, cycles broken.

    Only a parent that is *in this container* is an edge — a card whose spawner sits in another
    container is a root here, which is what makes the islands real rather than one graph.
    """
    ids = [m.id for m in members]
    known = set(ids)
    parent = {
        m.id: m.parent for m in members if m.parent in known and m.parent != m.id
    }

    kids: dict[str, list[str]] = {i: [] for i in ids}
    roots: list[str] = []
    for ident in ids:
        seen = {ident}
        up = parent.get(ident)
        while up is not None and up not in seen:  # a parent chain that loops has no root
            seen.add(up)
            up = parent.get(up)
        if up is not None:
            parent.pop(ident, None)
        if ident in parent:
            kids[parent[ident]].append(ident)
        else:
            roots.append(ident)
    return roots, kids


def leaves_of(root: str, kids: dict[str, list[str]]) -> int:
    """How many leaves hang off a node — the weight a wedge of the sector is shared out by."""
    brood = kids.get(root, [])
    return 1 if not brood else sum(leaves_of(k, kids) for k in brood)


def as_islands(
    blocks: Sequence[tuple[dict[str, Point], dict[str, Size]]], target: float, gap: float
) -> dict[str, Point]:
    """Several independent groups of boxes, packed against the screen and merged into one frame."""
    sizes: list[Size] = []
    framed: list[dict[str, Point]] = []
    for spots, boxes in blocks:
        if not spots:
            framed.append({})
            sizes.append((0.0, 0.0))
            continue
        x0 = min(at[0] for at in spots.values())
        y0 = min(at[1] for at in spots.values())
        framed.append({a: (at[0] - x0, at[1] - y0) for a, at in spots.items()})
        sizes.append(
            (
                max(at[0] + boxes[a][0] for a, at in spots.items()) - x0,
                max(at[1] + boxes[a][1] for a, at in spots.items()) - y0,
            )
        )

    at, _ = without_empties(sizes, lambda kept: pack_best(kept, gap, target, commit=True))
    out: dict[str, Point] = {}
    for offset, spots in zip(at, framed):
        for ident, spot in spots.items():
            out[ident] = (offset[0] + spot[0], offset[1] + spot[1])
    return out


def _cards(
    members: Sequence[Agent], rings: Rings, ring, sub: Size
) -> tuple[dict[str, Size], dict[str, Point]]:
    boxes = {m.id: card_box(m.id, rings, ring, sub) for m in members}
    leads = {m.id: card_lead(m.id, rings, ring, sub) for m in members}
    return boxes, leads


# ── the rings: what a card's delegates do ─────────────────────────────────────


def fan_arc(seats: int, depth: int) -> list[Point]:
    """One downward arc of delegates, hugging the card's rectangle the way a radial ring does."""
    if seats <= 0:
        return []
    if seats == 1:
        return [_radial_slot(180.0, depth)]
    lo, hi = 180.0 - FAN_SPREAD, 180.0 + FAN_SPREAD
    step = (hi - lo) / (seats - 1)
    return [_radial_slot(lo + ix * step, depth) for ix in range(seats)]


def fan_room(depth: int) -> int:
    """How many delegates arc `depth` seats while they stay a readable `SUB_GAP` apart."""
    room = 1
    for seats in range(2, FAN_SEATS + 1):
        if not _apart_enough(fan_arc(seats, depth)):
            break
        room = seats
    return room


def fan_arcs(count: int) -> list[int]:
    """How the delegates share out between the arcs: the inner one first, then one outside it."""
    out: list[int] = []
    left = count
    while left > 0 and len(out) < FAN_ARCS:
        seats = min(left, fan_room(len(out)))
        out.append(seats)
        left -= seats
    if left > 0 and out:
        out[-1] += left  # past four arcs it is a crowd whatever we do; do not grow the box for it
    return out


def ring_fan(count: int) -> list[Point]:
    """Delegates on a downward arc under the card, the outer arcs outside the inner ones.

    The radial ring puts them on the card's two flanks and leaves the top and the bottom clear. This
    one does the opposite: it takes the bottom, because under `multiradial` and `spider` a card's
    children are already out to the sides and the room below it is the room nobody else wants.
    """
    slots: list[Point] = []
    for depth, seats in enumerate(fan_arcs(count)):
        slots += fan_arc(seats, depth)
    return slots


def _organic_slots(count: int, scale: float) -> list[Point]:
    """The fan's own seats, nudged by `scale`. At `scale = 0` it *is* `ring_fan`, which is the whole
    point: `fan_room` proved those seats are far enough apart, so the backing-off below terminates."""
    slots: list[Point] = []
    for depth, seats in enumerate(fan_arcs(count)):
        lo, hi = 180.0 - FAN_SPREAD, 180.0 + FAN_SPREAD
        step = (hi - lo) / max(seats - 1, 1)
        for ix in range(seats):
            base = lo + ix * step if seats > 1 else 180.0
            turn = base + seeded(f"turn/{depth}", ix) * step * ORGANIC_JITTER * scale
            at = _radial_slot(turn, depth)
            slots.append((at[0], at[1] + seeded(f"drop/{depth}", ix) * SUB_GAP * scale))
    return slots


def ring_organic(count: int) -> list[Point]:
    """The fan, off its regular seats — a spray rather than an arc.

    The nudge is seeded off the slot, so a re-render draws the same picture: an arrangement that
    moves every time it is looked at cannot be judged against the one before it. And it is backed off
    until nothing touches, because a crowded arc is what the jitter is *for* — two delegates drawn one
    over the other is not organic, it is broken, and an eye reads it as one card with a shadow.
    """
    for scale in (1.0, 0.7, 0.45, 0.25, 0.0):
        slots = _organic_slots(count, scale)
        if _apart_enough(slots):
            return slots
    return slots


def hex_rows(count: int) -> list[int]:
    """A honeycomb's rows: `HEX_COLS`, then one fewer, then `HEX_COLS` again."""
    out: list[int] = []
    left = count
    wide = True
    while left > 0:
        seats = min(left, HEX_COLS if wide else HEX_COLS - 1)
        out.append(seats)
        left -= seats
        wide = not wide
    return out


def ring_hex(count: int) -> list[Point]:
    """Delegates on a honeycomb under the card: rows of three and two, offset half a cell.

    The cells are laid out on the hexagonal lattice, but a delegate is a rectangle, so consecutive
    rows are a full box apart rather than interlocking — a honeycomb of rectangles is a staggered
    grid, and pretending otherwise would draw one delegate over another. What the stagger buys is a
    ring that reads as a cluster rather than as a table, three wide and short.
    """
    if count <= 0:
        return []
    cell = SUB_NARROW_WIDTH + SUB_GAP
    span = HEX_COLS * SUB_NARROW_WIDTH + (HEX_COLS - 1) * SUB_GAP
    left = (CARD_WIDTH - span) / 2.0
    top = CARD_HEIGHT + SUB_DROP

    out: list[Point] = []
    for row, seats in enumerate(hex_rows(count)):
        wide = seats * SUB_NARROW_WIDTH + (seats - 1) * SUB_GAP
        x = left + (span - wide) / 2.0
        for col in range(seats):
            out.append((x + col * cell, top + row * (SUB_NARROW_HEIGHT + SUB_GAP)))
    return out


# ── the insides: what one container's cards do ────────────────────────────────


def _bows(brood: Sequence[str], boxes: dict[str, Size], target: float) -> list[list[str]]:
    """A brood too wide for the screen, folded into several bows instead of one long spray.

    Nine children on one fan is a 3400pt row: the shape is lovely and the canvas scrolls sideways for
    two screens to show it. `break_to` is the same judgement the grid ring makes, at the cards.
    """
    widest = max(boxes[c][0] for c in brood)
    tallest = max(boxes[c][1] for c in brood)
    per = break_to(len(brood), (widest, tallest), target)
    return [list(brood[at : at + per]) for at in range(0, len(brood), per)]


def _brood_organic(
    at: Point,
    box: Size,
    brood: Sequence[str],
    boxes: dict[str, Size],
    spots: dict[str, Point],
    target: float,
) -> None:
    """One parent's children, fanned under it, bowed at the ends and nudged off their seats."""
    if not brood:
        return
    widest = max(boxes[c][0] for c in brood)
    step = widest + CARD_GAP_X
    y = at[1] + box[1] + CARD_GAP_Y
    for bow_row in _bows(brood, boxes, target):
        middle = (len(bow_row) - 1) / 2.0
        for ix, child in enumerate(bow_row):
            off = (ix - middle) * step
            bow = (abs(ix - middle) / middle if middle > 0.0 else 0.0) * CARD_GAP_Y * ORGANIC_BOW
            spots[child] = (
                at[0]
                + box[0] / 2.0
                - boxes[child][0] / 2.0
                + off
                + seeded(child, 1) * step * ORGANIC_JITTER,
                y + bow + abs(seeded(child, 2)) * CARD_GAP_Y * ORGANIC_JITTER,
            )
        y += max(boxes[c][1] for c in bow_row) + CARD_GAP_Y + CARD_GAP_Y * ORGANIC_BOW


def _brood_radial(
    at: Point,
    box: Size,
    brood: Sequence[str],
    boxes: dict[str, Size],
    spots: dict[str, Point],
    target: float,
) -> None:
    """One parent's children on downward arcs round it — the hub of a multiradial.

    A hub with more children than one arc seats opens a second arc outside the first rather than
    pushing the first one out to a radius that seats them all: the radius that seats nine children is
    most of a screen, and nothing is inside it.
    """
    if not brood:
        return
    centre = (at[0] + box[0] / 2.0, at[1] + box[1] / 2.0)
    sector = math.radians(2.0 * FAN_SPREAD)
    lo = 180.0 - FAN_SPREAD

    reach = math.hypot(box[0], box[1]) / 2.0 + CARD_GAP_Y + max(boxes[c][1] for c in brood) / 2.0
    left = list(brood)
    while left:
        room, run, seats = sector * reach, 0.0, 0
        for child in left:  # an arc is filled by what the children actually measure, not the widest
            run += boxes[child][0] + CARD_GAP_X
            if seats and run > room:
                break
            seats += 1
        step = (2.0 * FAN_SPREAD) / seats
        for ix, child in enumerate(left[:seats]):
            rad = math.radians(lo + (ix + 0.5) * step)
            spots[child] = (
                centre[0] + reach * math.sin(rad) - boxes[child][0] / 2.0,
                centre[1] - reach * math.cos(rad) - boxes[child][1] / 2.0,
            )
        reach += max(boxes[c][1] for c in left[:seats]) + CARD_GAP_Y
        left = left[seats:]


def _grown(
    members: Sequence[Agent],
    boxes: dict[str, Size],
    brood_of: Callable[
        [Point, Size, Sequence[str], dict[str, Size], dict[str, Point], float], None
    ],
    target: float,
) -> dict[str, Point]:
    """Every root grown into its own frame by `brood_of`, the frames packed as islands.

    The two shapes that grow a tree outward — `organic` and `multiradial` — differ only in where a
    parent puts its children, so that is the one thing they hand in.
    """
    roots, kids = family(members)
    blocks: list[tuple[dict[str, Point], dict[str, Size]]] = []
    for root in roots:
        spots: dict[str, Point] = {root: (0.0, 0.0)}
        rank = [root]
        while rank:
            node = rank.pop(0)
            brood = kids.get(node, [])
            brood_of(spots[node], boxes[node], brood, boxes, spots, target)
            rank += brood
        spread(spots, boxes, CARD_GAP_X)
        blocks.append((spots, boxes))
    return as_islands(blocks, target, CARD_GAP_X * 2.0)


def inside_organic(
    task: str, agents, rings: Rings, ring=ring_organic, sub: Size = SUB_NARROW, target: float = VIEW_ASPECT
) -> Contents:
    """Children sprayed under their parent, bowed at the ends, then pushed apart until they fit."""
    members = _members(task, agents)
    if not members:
        return Contents()
    boxes, leads = _cards(members, rings, ring, sub)
    return contents_of(_grown(members, boxes, _brood_organic, target), boxes, leads)


def inside_multiradial(
    task: str, agents, rings: Rings, ring=ring_fan, sub: Size = SUB_NARROW, target: float = VIEW_ASPECT
) -> Contents:
    """Every parent its own hub, its children on the downward half of a circle round it."""
    members = _members(task, agents)
    if not members:
        return Contents()
    boxes, leads = _cards(members, rings, ring, sub)
    return contents_of(_grown(members, boxes, _brood_radial, target), boxes, leads)


def _wedges(
    roots: Sequence[str], kids: dict[str, list[str]], lo: float, hi: float
) -> dict[str, tuple[float, int]]:
    """Each card's angle and its depth: a node takes the middle of the wedge its leaves earn it.

    This is what makes a web read: a whole subtree sits in one wedge of the sector, so a spoke never
    crosses the wedge next door, and a card is always somewhere under its parent.
    """
    out: dict[str, tuple[float, int]] = {}
    weight = {root: leaves_of(root, kids) for root in roots}
    total = sum(weight.values()) or 1

    pending: list[tuple[str, float, float, int]] = []
    at = lo
    for root in roots:
        span = (hi - lo) * weight[root] / total
        pending.append((root, at, at + span, 0))
        at += span
    while pending:
        node, left, right, depth = pending.pop()
        out[node] = ((left + right) / 2.0, depth)
        brood = kids.get(node, [])
        if not brood:
            continue
        share = sum(leaves_of(k, kids) for k in brood) or 1
        cursor = left
        for child in brood:
            span = (right - left) * leaves_of(child, kids) / share
            pending.append((child, cursor, cursor + span, depth + 1))
            cursor += span
    return out


def inside_spider(
    task: str, agents, rings: Rings, ring=ring_fan, sub: Size = SUB_NARROW, target: float = VIEW_ASPECT
) -> Contents:
    """One hub per container, its cards on concentric downward arcs, its spokes the parent links.

    A radial tree in a sector rather than a full turn, so every spoke still points down the page.
    Each arc is pushed out far enough to seat the row on it — a web whose outer rings are as crowded
    as its inner ones is not a web, it is a pile.
    """
    members = _members(task, agents)
    if not members:
        return Contents()
    boxes, leads = _cards(members, rings, ring, sub)
    roots, kids = family(members)
    seats = _wedges(roots, kids, 180.0 - SPIDER_SPREAD, 180.0 + SPIDER_SPREAD)

    deepest = max(depth for _, depth in seats.values())
    rows: list[list[str]] = [[] for _ in range(deepest + 1)]
    for ident, (_, depth) in seats.items():
        rows[depth].append(ident)

    sector = math.radians(2.0 * SPIDER_SPREAD)
    reach: list[float] = []
    for depth, row in enumerate(rows):
        tall = max(boxes[a][1] for a in row)
        wide = sum(boxes[a][0] + CARD_GAP_X for a in row)
        want = max(wide / sector, tall / 2.0)
        if depth:
            prev = max(boxes[a][1] for a in rows[depth - 1])
            want = max(want, reach[depth - 1] + prev / 2.0 + CARD_GAP_Y + tall / 2.0)
        reach.append(want)

    spots: dict[str, Point] = {}
    for ident, (turn, depth) in seats.items():
        rad = math.radians(turn)
        spots[ident] = (
            reach[depth] * math.sin(rad) - boxes[ident][0] / 2.0,
            -reach[depth] * math.cos(rad) - boxes[ident][1] / 2.0,
        )
    spread(spots, boxes, CARD_GAP_X)
    return contents_of(spots, boxes, leads)


def inside_hex(
    task: str, agents, rings: Rings, ring=ring_hex, sub: Size = SUB_NARROW, target: float = VIEW_ASPECT
) -> Contents:
    """Cards on a honeycomb lattice: a row per hand-off depth, every other row half a cell over.

    The stagger is what a hex grid is worth here — a card sits between the two above it rather than
    directly under one, so a spoke from either parent reaches it without running past a sibling.
    """
    members = _members(task, agents)
    if not members:
        return Contents()
    boxes, leads = _cards(members, rings, ring, sub)
    roots, kids = family(members)

    seats = _wedges(roots, kids, 0.0, 1.0)  # the DFS order, so a family stays together in its row
    order = sorted(seats, key=lambda a: (seats[a][1], seats[a][0]))
    rows: list[list[str]] = []
    for ident in order:
        depth = seats[ident][1]
        while len(rows) <= depth:
            rows.append([])
        rows[depth].append(ident)

    cell = max(b[0] for b in boxes.values()) + CARD_GAP_X
    tall = max(b[1] for b in boxes.values())
    lattice: list[list[str]] = []
    for row in rows:  # a depth row wider than the screen folds onto the next lattice rows
        per = break_to(len(row), (cell, tall), target)
        lattice += [row[at : at + per] for at in range(0, len(row), per)]

    spots: dict[str, Point] = {}
    y = 0.0
    for depth, row in enumerate(lattice):
        wide = len(row) * cell - CARD_GAP_X
        x = -wide / 2.0 + (cell / 2.0 if depth % 2 else 0.0)
        for ident in row:
            spots[ident] = (x + (cell - CARD_GAP_X - boxes[ident][0]) / 2.0, y)
            x += cell
        y += max(boxes[a][1] for a in row) + CARD_GAP_Y
    return contents_of(spots, boxes, leads)


def inside_islands(
    task: str, agents, rings: Rings, ring=ring_grid, sub: Size = SUB_NARROW, target: float = VIEW_ASPECT
) -> Contents:
    """Each disjoint family of cards arranged on its own, the families packed against the screen.

    No root is assumed and none is privileged: a container holding four unrelated cards is four
    islands, and a container holding one tree is one. The gap between two islands is what says they
    are not the same piece of work — which a single column of cards never says at all.
    """
    members = _members(task, agents)
    if not members:
        return Contents()
    boxes, leads = _cards(members, rings, ring, sub)
    roots, kids = family(members)

    blocks: list[tuple[dict[str, Point], dict[str, Size]]] = []
    for root in roots:
        crew: list[Agent] = []
        rank = [root]
        while rank:
            node = rank.pop(0)
            crew.append(next(m for m in members if m.id == node))
            rank += kids.get(node, [])
        held = stack_aspect(crew, rings, ring, sub, target)
        # `stack_aspect` answers where the *card* goes; the island is packed by its box
        blocks.append(
            ({a: (at[0] - leads[a][0], at[1] - leads[a][1]) for a, at in held.cards}, boxes)
        )
    return contents_of(as_islands(blocks, target, ISLAND_GAP), boxes, leads)


# ── the packer that never assumes a root ──────────────────────────────────────


def families_of(count: int, parents: Sequence[Optional[int]]) -> list[list[int]]:
    """The connected groups of containers, over the parent relation read as undirected."""
    up = list(range(count))

    def find(at: int) -> int:
        while up[at] != at:
            up[at] = up[up[at]]
            at = up[at]
        return at

    for node, above in enumerate(parents):
        if above is None or above >= count:
            continue
        a, b = find(node), find(above)
        if a != b:
            up[a] = b

    groups: dict[int, list[int]] = {}
    for node in range(count):
        groups.setdefault(find(node), []).append(node)
    return list(groups.values())


def pack_islands(
    sizes: Sequence[Size], parents: Sequence[Optional[int]], target: float = VIEW_ASPECT
) -> Packing:
    """Containers grouped by what they are connected to, each group packed as its own island.

    `tree` needs a root to hang a forest off and gives every container one row of depth whether the
    graph has that shape or not. This one reads the same forest as a set of *components*: two
    sessions' worth of unrelated work comes out as two islands side by side against the screen's
    rectangle, and a graph with no parent edges at all comes out as one island, packed tight.
    """
    kept_parents = remap(sizes, parents)

    def go(kept: Sequence[Size]) -> Packing:
        groups = families_of(len(kept), kept_parents)
        spots: list[Point] = [(0.0, 0.0)] * len(kept)
        blocks: list[tuple[list[int], list[Point]]] = []
        shapes: list[Size] = []
        for group in groups:
            at, extent = pack_best([kept[ix] for ix in group], TASK_GAP, target, commit=True)
            blocks.append((group, at))
            shapes.append(extent)

        outer, _ = pack_best(shapes, ISLAND_GAP, target, commit=True)
        edge = [0.0, 0.0]
        for offset, (group, at) in zip(outer, blocks):
            for ix, spot in zip(group, at):
                spots[ix] = (offset[0] + spot[0], offset[1] + spot[1])
                edge[0] = max(edge[0], spots[ix][0] + kept[ix][0])
                edge[1] = max(edge[1], spots[ix][1] + kept[ix][1])
        return spots, (edge[0], edge[1])

    return without_empties(sizes, go)


# ── the registry rows ─────────────────────────────────────────────────────────
#
# Added to `ALGOS` at the bottom of this file rather than listed in `algos.py`, so that importing
# either module first leaves the registry whole.


EXTRA: dict[str, Algo] = {
    "organic": Algo(
        "organic",
        "Organic",
        "children sprayed under their parent, bowed and nudged off the grid",
        inside_organic,
        ring_organic,
        pack_islands,
        sub=SUB_NARROW,
        forest=True,
    ),
    "multiradial": Algo(
        "multiradial",
        "Multiradial",
        "every parent a hub, its children on the downward half of a circle round it",
        inside_multiradial,
        ring_fan,
        pack_islands,
        sub=SUB_NARROW,
        forest=True,
    ),
    "spider": Algo(
        "spider",
        "Spider",
        "one hub per container, cards on concentric downward arcs, spokes to the parent",
        inside_spider,
        ring_fan,
        pack_islands,
        sub=SUB_NARROW,
        forest=True,
    ),
    "hex": Algo(
        "hex",
        "Hex",
        "cards on a honeycomb, a row per hand-off depth, rows half a cell apart",
        inside_hex,
        ring_hex,
        pack_islands,
        sub=SUB_NARROW,
        forest=True,
    ),
    "islands": Algo(
        "islands",
        "Islands",
        "disjoint families as separate islands, at the cards and at the containers, no root assumed",
        inside_islands,
        ring_grid,
        pack_islands,
        sub=SUB_NARROW,
        forest=True,
    ),
}


ALGOS.update(EXTRA)
