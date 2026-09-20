"""The block-positioning algorithms of the teams graph, ported from Rust, plus `adaptive`.

The first part is a port of `crates/ubiq/src/state/layout.rs`: the same constants, the same
function names, the same arithmetic. Nothing there is an improvement on the Rust — where the two
disagree the port is wrong and is what gets fixed, because the point of the port is that a rendered
PNG can be trusted to show what the app does.

The arrangements express themselves through `ALGOS`, one entry each: how cards sit inside one
container (`inside`), where a card's delegates go inside its own fence (`ring`), and how the
containers of a session are packed (`pack`). A new arrangement is one dict entry plus three small
functions.

An arrangement may also carry a `grow`, which is what `adaptive` is: it never lays a session out at
all. It places one arriving block at a time against the arrangement already on screen, and no
placed block moves unless the arriving one would overlap it — then by the smallest delta that
clears it, and only for the blocks actually in the way.
"""

from __future__ import annotations

import copy
import json
import math
from dataclasses import dataclass, field
from pathlib import Path
from typing import Callable, Iterable, Optional, Sequence

# ── the constants, verbatim ────────────────────────────────────────────────────

CARD_WIDTH = 264.0
CARD_HEIGHT = 140.0

GROUP_PAD = 22.0
GROUP_LABEL = 26.0

CARD_GAP_X = 32.0
CARD_GAP_Y = 44.0
TASK_GAP = 56.0
LAYOUT_MARGIN = 24.0

LAYOUT_WIDTH = 1_320.0

RING_PAD = 14.0
SUB_WIDTH = CARD_WIDTH
SUB_HEIGHT = 96.0
SUB_GAP = 12.0

SUB_DROP = 16.0

EPS = 0.01

# ── the constants the spike added ─────────────────────────────────────────────
#
# **A block is drawn at the size the app draws it, everywhere.** An arrangement earns its height by
# where it puts a block, never by shrinking one: a delegate is `SUB_BOX`, a card is
# `CARD_WIDTH × CARD_HEIGHT`, and no ring shape, no packer and no renderer scales either. A ring
# that runs wide is the packers' problem, and they read it from the ring's own slots.

#: What a delegate box measures. The one delegate size there is.
SUB_BOX = (SUB_WIDTH, SUB_HEIGHT)

#: How many rows a grid ring's columns may reach before it opens another column.
GRID_ROWS = 4

#: The clear sector at the top **and** the bottom of a radial ring, in degrees either side of the
#: vertical, kept free because that is where a card's connectors arrive and leave. The delegates
#: take the two side flanks, which is also why the fence comes out wide and short.
RADIAL_CLEAR = 30.0
#: The breathing room between the card and its first radial ring, and between the two rings.
RADIAL_GAP = 12.0
#: The most delegates one radial ring is ever asked to seat, however much room the arithmetic finds.
RADIAL_SEATS = 10

#: How many further waves an arriving block's push makes after the first one, while two containers
#: are still on top of each other. Bounded rather than "until clean", because a push that cannot
#: settle must stop somewhere, and the picture is the thing that says whether it did.
PUSH_WAVES = 4

#: The viewport an arrangement is read in when the scenario does not say otherwise.
VIEW_ASPECT = 1.6
#: How much wider than the bare area sum a viewport-aware shelf's first guess runs, for the ragged
#: end of every row. It is only a candidate — `pack_view_shelf` scores it against the others.
SHELF_SLACK = 1.15

Point = tuple[float, float]
Size = tuple[float, float]
Rect = tuple[float, float, float, float]
Packing = tuple[list[Point], Size]
Rings = dict[str, int]


# ── the records the layout reads ──────────────────────────────────────────────


@dataclass
class Agent:
    """A `WorkAgent`, cut down to what the layout reads plus what the renderer draws."""

    id: str
    session: str
    task: Optional[str]
    parent: Optional[str]
    name: str = "card"
    role: str = "Implementer"
    harness: str = "claude-code"
    activity: str = "writing"
    subagents: list[tuple[str, str]] = field(default_factory=list)


@dataclass
class Task:
    id: str
    session: Optional[str]
    title: str = "task"
    shape: Optional[str] = None


@dataclass
class Session:
    id: str
    name: str = "session"
    branch: str = ""
    worktree: bool = False


@dataclass
class Link:
    frm: str
    to: str
    kind: str = "handoff"


@dataclass
class Positions:
    """What a human dragged things to. Anything unnamed is placed by the arrangement."""

    tasks: dict[str, Point] = field(default_factory=dict)
    agents: dict[str, Point] = field(default_factory=dict)
    subagents: dict[str, Point] = field(default_factory=dict)

    def any(self) -> bool:
        return bool(self.tasks or self.agents or self.subagents)


@dataclass
class Scenario:
    name: str
    note: str = ""
    viewport: Size = (1600.0, 1000.0)
    sessions: list[Session] = field(default_factory=list)
    tasks: list[Task] = field(default_factory=list)
    agents: list[Agent] = field(default_factory=list)
    links: list[Link] = field(default_factory=list)
    positions: Positions = field(default_factory=Positions)
    source: Optional[Path] = None


FORMAT = "ubiq.teamsim/1"


def _point(raw) -> Point:
    return (float(raw[0]), float(raw[1]))


def load(path: Path) -> Scenario:
    """Read one scenario file. A file whose `format` is not ours is refused."""
    raw = json.loads(Path(path).read_text())
    if raw.get("format") != FORMAT:
        raise ValueError(f"{path}: not {FORMAT} (got {raw.get('format')!r})")
    port = raw.get("viewport") or {}
    placed = raw.get("positions") or {}
    return Scenario(
        name=raw.get("name") or Path(path).stem,
        note=raw.get("note", ""),
        viewport=(float(port.get("w", 1600)), float(port.get("h", 1000))),
        sessions=[
            Session(
                id=s["id"],
                name=s.get("name", s["id"]),
                branch=s.get("branch", ""),
                worktree=bool(s.get("worktree")),
            )
            for s in raw.get("sessions", [])
        ],
        tasks=[
            Task(
                id=t["id"],
                session=t.get("session"),
                title=t.get("title", t["id"]),
                shape=t.get("shape"),
            )
            for t in raw.get("tasks", [])
        ],
        agents=[
            Agent(
                id=a["id"],
                session=a["session"],
                task=a.get("task"),
                parent=a.get("parent"),
                name=a.get("name", a["id"]),
                role=a.get("role", ""),
                harness=a.get("harness", ""),
                activity=a.get("activity", ""),
                subagents=[(s["id"], s.get("name", s["id"])) for s in a.get("subagents", [])],
            )
            for a in raw.get("agents", [])
        ],
        links=[
            Link(frm=l["from"], to=l["to"], kind=l.get("kind", "handoff"))
            for l in raw.get("links", [])
        ],
        positions=Positions(
            tasks={k: _point(v) for k, v in (placed.get("tasks") or {}).items()},
            agents={k: _point(v) for k, v in (placed.get("agents") or {}).items()},
            subagents={k: _point(v) for k, v in (placed.get("subagents") or {}).items()},
        ),
        source=Path(path),
    )


def aspect_of(scenario: Scenario) -> float:
    """The shape of the screen this arrangement is read on — `w / h`, and 1.6 by default."""
    w, h = scenario.viewport
    return (w / h) if h > 0.0 else VIEW_ASPECT


def rings_of(agents: Sequence[Agent]) -> Rings:
    """How many delegates each card is drawing. A card with none is not in the map."""
    return {a.id: len(a.subagents) for a in agents if a.subagents}


# ── the ring ──────────────────────────────────────────────────────────────────


def sub_slot(ix: int) -> Point:
    """Where the `ix`-th delegate starts, as an offset from its parent card's top-left."""
    return (0.0, CARD_HEIGHT + SUB_DROP + ix * (SUB_HEIGHT + SUB_GAP))


def ring_drop(count: int) -> float:
    """How much room a card with `count` delegates needs under it, fence included."""
    if count == 0:
        return 0.0
    rows = float(count)
    return SUB_DROP + rows * SUB_HEIGHT + (rows - 1.0) * SUB_GAP + RING_PAD


def fence(at: Point, subs: Sequence[Point]) -> Optional[Rect]:
    """The fence round a card and its delegates, in the caller's frame. `None` when it has none."""
    if not subs:
        return None
    x0, y0 = at
    x1, y1 = at[0] + CARD_WIDTH, at[1] + CARD_HEIGHT
    for slot in subs:
        x0 = min(x0, slot[0])
        y0 = min(y0, slot[1])
        x1 = max(x1, slot[0] + SUB_WIDTH)
        y1 = max(y1, slot[1] + SUB_HEIGHT)
    return (x0 - RING_PAD, y0 - RING_PAD, (x1 - x0) + RING_PAD * 2.0, (y1 - y0) + RING_PAD * 2.0)


def ring_rows(count: int) -> list[Point]:
    """The ring every from-scratch arrangement wears: one delegate to a row under the card."""
    return [sub_slot(ix) for ix in range(count)]


def grid_cols(count: int) -> int:
    """How many columns a grid ring takes: one, until one would run past `GRID_ROWS` rows.

    A delegate is a full `SUB_WIDTH` wide, so one column is exactly as wide as the card and is the
    shape to stay in while it fits. Past `GRID_ROWS` deep the ring opens another column rather than
    growing down — width is the cheap axis, and it is the only one bought here.
    """
    if count <= 0:
        return 1
    return max(1, math.ceil(count / float(GRID_ROWS)))


def ring_grid(count: int) -> list[Point]:
    """Delegates in a grid under the card, at full size, centred on it.

    One column is card-wide and `GRID_ROWS` deep at most; past that a second column opens beside it,
    then a third. Nothing is scaled to make the ring fit — a wide ring is reserved for by `ring_box`
    and packed round by every packer, which is what lets a ring be wider than its card at all.
    """
    if count <= 0:
        return []
    cols = grid_cols(count)
    span = cols * SUB_WIDTH + (cols - 1) * SUB_GAP
    left = (CARD_WIDTH - span) / 2.0
    top = CARD_HEIGHT + SUB_DROP

    out: list[Point] = []
    for ix in range(count):
        row, col = divmod(ix, cols)
        wide = min(cols, count - row * cols)  # a short last row is centred in the grid
        inset = (span - (wide * SUB_WIDTH + (wide - 1) * SUB_GAP)) / 2.0
        out.append(
            (
                left + inset + col * (SUB_WIDTH + SUB_GAP),
                top + row * (SUB_HEIGHT + SUB_GAP),
            )
        )
    return out


def _radial_slot(turn: float, depth: int) -> Point:
    """One delegate on the ray `turn` degrees clockwise from straight up, hugging the card.

    Not an ellipse: the box is pushed out along the ray only as far as it takes to clear the card's
    **rectangle**, grown by the box and the gap. A ring of these follows the card's own outline, so
    nothing sits on the card and the ring stays as tight as the card is.
    """
    rad = math.radians(turn)
    dx, dy = math.sin(rad), -math.cos(rad)
    wide = (CARD_WIDTH + SUB_WIDTH) / 2.0 + RADIAL_GAP + depth * (SUB_WIDTH + SUB_GAP)
    tall = (CARD_HEIGHT + SUB_HEIGHT) / 2.0 + RADIAL_GAP + depth * (SUB_HEIGHT + SUB_GAP)
    out = min(
        wide / abs(dx) if abs(dx) > 1e-9 else math.inf,
        tall / abs(dy) if abs(dy) > 1e-9 else math.inf,
    )
    return (
        CARD_WIDTH / 2.0 + out * dx - SUB_WIDTH / 2.0,
        CARD_HEIGHT / 2.0 + out * dy - SUB_HEIGHT / 2.0,
    )


def _radial_flank(seats: int, depth: int, left: bool) -> list[Point]:
    """One side's delegates, spread evenly down the flank between the two clear sectors."""
    if seats <= 0:
        return []
    lo, hi = RADIAL_CLEAR, 180.0 - RADIAL_CLEAR
    if left:
        lo, hi = 180.0 + RADIAL_CLEAR, 360.0 - RADIAL_CLEAR
    step = (hi - lo) / seats
    return [_radial_slot(lo + (ix + 0.5) * step, depth) for ix in range(seats)]


def _radial_ring(seats: int, depth: int) -> list[Point]:
    """One whole ring, its delegates shared between the right flank and the left."""
    right = (seats + 1) // 2
    return _radial_flank(right, depth, False) + _radial_flank(seats - right, depth, True)


def _apart_enough(slots: Sequence[Point]) -> bool:
    for i in range(len(slots)):
        for j in range(i + 1, len(slots)):
            a, b = slots[i], slots[j]
            gap_x = max(a[0], b[0]) - min(a[0], b[0]) - SUB_WIDTH
            gap_y = max(a[1], b[1]) - min(a[1], b[1]) - SUB_HEIGHT
            if max(gap_x, gap_y) < SUB_GAP - EPS:
                return False
    return True


def _radial_room(depth: int) -> int:
    """How many delegates ring `depth` seats while they stay a readable `SUB_GAP` apart."""
    room = 1
    for seats in range(2, RADIAL_SEATS + 1):
        if not _apart_enough(_radial_ring(seats, depth)):
            break
        room = seats
    return room


def radial_rings(count: int) -> list[int]:
    """How the delegates split between the rings: one, until a readable spacing runs out."""
    if count <= 0:
        return []
    inner = _radial_room(0)
    if count <= inner:
        return [count]
    # Past one ring, share them out evenly rather than filling the inner one — an even constellation
    # reads as one card's delegates, a full inner ring with two strays hanging off it does not.
    share = min(inner, (count + 1) // 2)
    return [share, count - share]


def ring_radial(count: int) -> list[Point]:
    """Narrow delegates round the card rather than under it, on its two side flanks.

    The sectors straight up and straight down are left clear, because that is where the card's
    connectors arrive and leave — a delegate never sits on one. What is left is the two flanks, so
    the fence comes out wide and short, which is the trade a rectangular viewport wants: width is
    cheap, height is not. Past one ring's worth, a second opens outside the first.
    """
    out: list[Point] = []
    for depth, seats in enumerate(radial_rings(count)):
        out += _radial_ring(seats, depth)
    return out


# ── what a card and a row take up ─────────────────────────────────────────────


def ring_pad(held: Rect, card: Rect) -> Rect:
    """`RING_PAD` added on each side the delegates actually push past the card, and nowhere else.

    A one-per-row ring only ever passes the card downwards, so this is the bottom-only pad the Rust
    reserves today, arithmetic for arithmetic. A ring that reaches sideways gets its pad there too,
    which is what keeps one card's fence from touching its neighbour's.
    """
    left = RING_PAD if held[0] < card[0] - EPS else 0.0
    top = RING_PAD if held[1] < card[1] - EPS else 0.0
    right = RING_PAD if held[0] + held[2] > card[0] + card[2] + EPS else 0.0
    bottom = RING_PAD if held[1] + held[3] > card[1] + card[3] + EPS else 0.0
    return (held[0] - left, held[1] - top, held[2] + left + right, held[3] + top + bottom)


def ring_box(slots: Sequence[Point]) -> Rect:
    """The room a card and its delegates take, in the card's own frame.

    The union of the card and the delegate boxes, padded by `ring_pad` — the same reading as
    `content_rect`, so what a packer reserves is exactly what the picture draws, whatever shape the
    ring is in.
    """
    card = (0.0, 0.0, CARD_WIDTH, CARD_HEIGHT)
    if not slots:
        return card
    held = _union([card] + [(s[0], s[1], SUB_WIDTH, SUB_HEIGHT) for s in slots])
    return _union([card, ring_pad(held, card)])


def card_box(agent: str, rings: Rings, ring: Callable[[int], list[Point]]) -> Size:
    """What one card reserves, in **both** axes — the fix a ring wider than its card needs.

    Derived from the ring function's own slots, so a new ring shape is reserved for by every packer
    and every `inside` the moment it exists, with nothing else to change.
    """
    held = ring_box(ring(rings.get(agent, 0)))
    return (held[2], held[3])


def card_lead(agent: str, rings: Rings, ring: Callable[[int], list[Point]]) -> Point:
    """Where the card itself sits inside that box. `(0, 0)` unless the ring reaches past it."""
    held = ring_box(ring(rings.get(agent, 0)))
    return (-held[0], -held[1])


def card_extent(agent: str, rings: Rings) -> float:
    return CARD_HEIGHT + ring_drop(rings.get(agent, 0))


def row_extent(row: Sequence[str], rings: Rings) -> float:
    return max((card_extent(a, rings) for a in row), default=0.0)


def row_width(cards: int) -> float:
    return cards * CARD_WIDTH + max(cards - 1, 0) * CARD_GAP_X


def row_span(row: Sequence[str], boxes: dict[str, Size]) -> float:
    """How wide a row of cards is once each one is as wide as its own ring."""
    if not row:
        return 0.0
    return sum(boxes[a][0] for a in row) + CARD_GAP_X * (len(row) - 1)


def break_at(cards: int) -> int:
    """How many cards a wrapped row takes before it folds: the side of the smallest square."""
    return int(max(math.ceil(math.sqrt(cards)), 1))


def break_to(cards: int, box: Size, target: float) -> int:
    """How many cards a row takes before it folds, so the block it makes reads at `target`.

    The wrap `packed` uses aims at a square, because nothing told it the screen is a rectangle.
    This one is told.
    """
    if cards <= 1:
        return 1
    best, mark = 1, math.inf
    for per in range(1, cards + 1):
        lines = math.ceil(cards / per)
        wide = per * box[0] + (per - 1) * CARD_GAP_X
        tall = lines * box[1] + (lines - 1) * CARD_GAP_Y
        off = abs(math.log((wide / tall) / target)) if tall > 0.0 else math.inf
        if off < mark - 1e-9:
            best, mark = per, off
    return best


# ── one container's contents ──────────────────────────────────────────────────


@dataclass
class Contents:
    cards: list[tuple[str, Point]] = field(default_factory=list)
    width: float = 0.0
    height: float = 0.0


def rows_by_depth(members: Sequence[Agent]) -> list[list[str]]:
    """The cards of one set, gathered into a row per hand-off depth."""
    if not members:
        return []
    ids = {m.id for m in members}
    parents: dict[str, str] = {}
    for agent in members:
        if agent.parent is not None and agent.parent in ids:
            parents[agent.id] = agent.parent

    rows: list[list[str]] = []
    for agent in members:
        depth = depth_of(agent.id, parents)
        while len(rows) <= depth:
            rows.append([])
        rows[depth].append(agent.id)
    return rows


def depth_of(agent: str, parents: dict[str, str]) -> int:
    """How many hand-offs deep an agent is. Bounded, so a parent chain that loops stops."""
    depth = 0
    at = agent
    while at in parents:
        depth += 1
        at = parents[at]
        if depth > len(parents):
            break
    return depth


def _boxes(members: Sequence[Agent], rings: Rings, ring) -> dict[str, Size]:
    return {m.id: card_box(m.id, rings, ring) for m in members}


def _leads(members: Sequence[Agent], rings: Rings, ring) -> dict[str, Point]:
    return {m.id: card_lead(m.id, rings, ring) for m in members}


def _lay(
    chunks: Sequence[Sequence[str]],
    boxes: dict[str, Size],
    leads: dict[str, Point],
    width: float,
) -> Contents:
    """Rows of cards, each centred in `width`, each as tall and as wide as its own ring needs."""
    cards: list[tuple[str, Point]] = []
    y = 0.0
    height = 0.0
    for chunk in chunks:
        x = (width - row_span(chunk, boxes)) / 2.0
        for agent in chunk:
            cards.append((agent, (x + leads[agent][0], y + leads[agent][1])))
            x += boxes[agent][0] + CARD_GAP_X
        tall = max((boxes[a][1] for a in chunk), default=0.0)
        height = y + tall
        y += tall + CARD_GAP_Y
    return Contents(cards, width, height)


def stack(members: Sequence[Agent], rings: Rings, ring=ring_rows) -> Contents:
    """Roots on the top row, their children on the next. One row per hand-off depth."""
    rows = rows_by_depth(members)
    if not rows:
        return Contents()
    boxes = _boxes(members, rings, ring)
    width = max((row_span(row, boxes) for row in rows), default=0.0)
    return _lay(rows, boxes, _leads(members, rings, ring), width)


def _chunks(rows: Sequence[Sequence[str]], per_line: Sequence[int]) -> list[list[str]]:
    out: list[list[str]] = []
    for row, per in zip(rows, per_line):
        out += [list(row[at : at + per]) for at in range(0, len(row), per)]
    return out


def stack_wrapped(members: Sequence[Agent], rings: Rings, ring=ring_rows) -> Contents:
    """The same depth rows, each folded into a near-square block instead of one long line."""
    rows = rows_by_depth(members)
    if not rows:
        return Contents()
    boxes = _boxes(members, rings, ring)
    chunks = _chunks(rows, [break_at(len(row)) for row in rows])
    width = max((row_span(c, boxes) for c in chunks), default=0.0)
    return _lay(chunks, boxes, _leads(members, rings, ring), width)


def stack_aspect(
    members: Sequence[Agent],
    rings: Rings,
    ring=ring_rows,
    target: float = VIEW_ASPECT,
) -> Contents:
    """The same depth rows, folded to the shape of the screen rather than to a square."""
    rows = rows_by_depth(members)
    if not rows:
        return Contents()
    boxes = _boxes(members, rings, ring)
    per_line = [
        break_to(len(row), max((boxes[a] for a in row), key=lambda b: b[0] * b[1]), target)
        for row in rows
    ]
    chunks = _chunks(rows, per_line)
    width = max((row_span(c, boxes) for c in chunks), default=0.0)
    return _lay(chunks, boxes, _leads(members, rings, ring), width)


def column(members: Sequence[Agent], rings: Rings, ring=ring_rows) -> Contents:
    """One card per row, in spawn order."""
    rows = rows_by_depth(members)
    if not rows:
        return Contents()
    boxes = _boxes(members, rings, ring)
    width = max((b[0] for b in boxes.values()), default=CARD_WIDTH)
    return _lay([[a] for row in rows for a in row], boxes, _leads(members, rings, ring), width)


def _members(task: str, agents: Sequence[Agent]) -> list[Agent]:
    return [a for a in agents if a.task == task]


def inside_stack(
    task: str, agents, rings: Rings, ring=ring_rows, target: float = 1.0
) -> Contents:
    return stack(_members(task, agents), rings, ring)


def inside_wrapped(
    task: str, agents, rings: Rings, ring=ring_rows, target: float = 1.0
) -> Contents:
    return stack_wrapped(_members(task, agents), rings, ring)


def inside_column(
    task: str, agents, rings: Rings, ring=ring_rows, target: float = 1.0
) -> Contents:
    return column(_members(task, agents), rings, ring)


def inside_aspect(
    task: str, agents, rings: Rings, ring=ring_rows, target: float = VIEW_ASPECT
) -> Contents:
    return stack_aspect(_members(task, agents), rings, ring, target)


# ── the packers ───────────────────────────────────────────────────────────────


def pack_shelf(sizes: Sequence[Size], width: float, gap: float) -> Packing:
    """Boxes in the order given, wrapping onto a new shelf when the next would pass `width`."""
    at: list[Point] = []
    x = y = row_h = 0.0
    extent = [0.0, 0.0]
    for w, h in sizes:
        if w <= 0.0 and h <= 0.0:
            at.append((x, y))
            continue
        if x > 0.0 and x + w > width:
            x = 0.0
            y += row_h + gap
            row_h = 0.0
        at.append((x, y))
        x += w + gap
        row_h = max(row_h, h)
        extent[0] = max(extent[0], x - gap)
        extent[1] = max(extent[1], y + row_h)
    return at, (extent[0], extent[1])


def longest(size: Size) -> float:
    return max(size[0], size[1])


def ledge(sky: Sequence[Point], x: float, w: float) -> float:
    """How high a box starting at `x` and `w` wide has to sit to clear the fence under it."""
    y = 0.0
    for ix, (sx, height) in enumerate(sky):
        end = sky[ix + 1][0] if ix + 1 < len(sky) else math.inf
        if end <= x + EPS or sx >= x + w - EPS:
            continue
        y = max(y, height)
    return y


def height_at(sky: Sequence[Point], x: float) -> float:
    for sx, height in reversed(sky):
        if sx <= x + EPS:
            return height
    return 0.0


def raise_fence(sky: list[Point], x: float, w: float, top: float) -> None:
    """Raise the skyline over `[x, x + w)` to `top`, which is what placing a box does to it."""
    right = x + w
    beyond = height_at(sky, right)
    kept = [n for n in sky if n[0] < x - EPS or n[0] > right + EPS]
    kept.append((x, top))
    kept.append((right, beyond))
    kept.sort(key=lambda n: n[0])
    out: list[Point] = []
    for node in kept:
        if out and (node[0] <= out[-1][0] + EPS or abs(node[1] - out[-1][1]) < EPS):
            continue
        out.append(node)
    sky[:] = out


def pack_skyline(sizes: Sequence[Size], width: float, gap: float, commit: bool = False) -> Packing:
    """Best-fit skyline packing into a container `width`. Positions come back in `sizes` order.

    The rule that picks between free slots is the greedy "smallest bounding box so far", which is
    the Rust's. It has one failure the spike found: a set of boxes of much the same width stacks
    into a single column whatever `width` it is given, because starting a second column always grows
    the box more than another row does. `commit` says the container width is a decision already
    taken, so a slot is judged on the **height** it costs — which is what makes sweeping widths in
    `pack_best` mean anything. The four production arrangements do not pass it.
    """
    order = sorted(range(len(sizes)), key=lambda ix: (-longest(sizes[ix]), ix))

    room = width + gap
    sky: list[Point] = [(0.0, 0.0)]
    at: list[Point] = [(0.0, 0.0)] * len(sizes)
    extent = [0.0, 0.0]

    for ix in order:
        w, h = sizes[ix][0] + gap, sizes[ix][1] + gap
        best: Optional[tuple[float, float, float]] = None
        for node in range(len(sky)):
            x = sky[node][0]
            if x + w > room + EPS:
                continue
            y = ledge(sky, x, w)
            wide = width if commit else max(extent[0], x + w - gap)
            grown = wide * max(extent[1], y + h - gap)
            if best is None or (
                grown < best[0] - EPS
                or (
                    grown < best[0] + EPS
                    and (y < best[2] - EPS or (y < best[2] + EPS and x < best[1]))
                )
            ):
                best = (grown, x, y)
        if best is None:
            x, y = 0.0, max((n[1] for n in sky), default=0.0)
        else:
            _, x, y = best
        raise_fence(sky, x, w, y + h)
        at[ix] = (x, y)
        extent[0] = max(extent[0], x + w - gap)
        extent[1] = max(extent[1], y + h - gap)
    return at, (extent[0], extent[1])


def score(extent: Size, content: float, target: float = 1.0) -> float:
    """How bad a packing is — smaller is better.

    `target` is the aspect the packing is wanted at: `1.0` is the square the Rust asks for today,
    and a viewport's `w / h` is what a screen actually is. The penalty is the distance from that
    shape either way, so a ribbon and a tower are both punished, and a rectangle is not.
    """
    OFF_TARGET = 0.35
    WASTED = 0.5

    area = extent[0] * extent[1]
    if area <= 0.0:
        return 0.0
    ratio = (extent[0] / extent[1]) / target
    off = max(ratio, 1.0 / ratio)
    waste = min(max((area - content) / area, 0.0), 1.0)
    return area * (1.0 + OFF_TARGET * (off - 1.0) + WASTED * waste)


def pack_best(
    sizes: Sequence[Size], gap: float, target: float = 1.0, commit: bool = False
) -> Packing:
    """Pack against a handful of container widths and keep the best-scoring result.

    The widths tried hang off the side of the `target`-shaped rectangle of the same area, so asking
    for a 1.6:1 screen tries wider containers than asking for a square — and `target = 1.0` is the
    square, arithmetic for arithmetic.
    """
    content = sum(w * h for w, h in sizes)
    widest = max((s[0] for s in sizes), default=0.0)
    side = math.sqrt(content * target)

    best: Optional[tuple[Packing, float]] = None
    for ratio in (0.7, 1.0, 1.4, 1.9):
        packed = pack_skyline(sizes, max(side * ratio, widest), gap, commit)
        marked = score(packed[1], content, target)
        if best is None or marked < best[1]:
            best = (packed, marked)
    return best[0] if best else ([], (0.0, 0.0))


def shelf_width(sizes: Sequence[Size], target: float) -> float:
    """How wide a shelf has to be for what it holds to come out at `target`.

    The side of the `target`-shaped rectangle of the same area, loosened by `SHELF_SLACK` because a
    shelf wastes the ragged end of every row, and never narrower than the widest box on it.
    """
    content = sum(w * h for w, h in sizes)
    widest = max((s[0] for s in sizes), default=0.0)
    return max(math.sqrt(content * target) * SHELF_SLACK, widest)


def brood(children: Sequence[int], span: Sequence[float], gap: float) -> float:
    """How wide a row of children is, gaps included."""
    if not children:
        return 0.0
    return sum(span[c] for c in children) + gap * (len(children) - 1)


def pack_tree(sizes: Sequence[Size], parents: Sequence[Optional[int]], gap: float) -> Packing:
    """A forest, tidy-tree style: a parent centred over the span of its children."""
    n = len(sizes)
    if n == 0:
        return [], (0.0, 0.0)

    parent = list(parents)
    for node in range(n):
        at_ix = node
        steps = 0
        while parent[at_ix] is not None:
            up = parent[at_ix]
            if up == node or steps > n:
                parent[node] = None
                break
            at_ix = up
            steps += 1
        if parent[node] == node:
            parent[node] = None

    children: list[list[int]] = [[] for _ in range(n)]
    for node, up in enumerate(parent):
        if up is not None:
            children[up].append(node)

    depth = [0] * n
    for node in range(n):
        at_ix = parent[node]
        while at_ix is not None:
            depth[node] += 1
            at_ix = parent[at_ix]

    deepest = max(depth, default=0)
    row_h = [0.0] * (deepest + 1)
    for size, row in zip(sizes, depth):
        row_h[row] = max(row_h[row], size[1])
    row_y = [0.0] * (deepest + 1)
    for row in range(1, deepest + 1):
        row_y[row] = row_y[row - 1] + row_h[row - 1] + gap

    span = [s[0] for s in sizes]
    for node in sorted(range(n), key=lambda node: -depth[node]):
        if children[node]:
            span[node] = max(span[node], brood(children[node], span, gap))

    at: list[Point] = [(0.0, 0.0)] * n
    cursor = 0.0
    extent = [0.0, 0.0]
    pending: list[tuple[int, float]] = []
    for node in range(n):
        if parent[node] is None:
            pending.append((node, cursor))
            cursor += span[node] + gap
    while pending:
        node, left = pending.pop()
        x = left + (span[node] - sizes[node][0]) / 2.0
        y = row_y[depth[node]]
        at[node] = (x, y)
        extent[0] = max(extent[0], x + sizes[node][0])
        extent[1] = max(extent[1], y + sizes[node][1])

        under = left + (span[node] - brood(children[node], span, gap)) / 2.0
        for child in children[node]:
            pending.append((child, under))
            under += span[child] + gap
    return at, (extent[0], extent[1])


def without_empties(sizes: Sequence[Size], pack: Callable[[Sequence[Size]], Packing]) -> Packing:
    """Pack only the containers that are actually drawn, and answer positions for all of them."""
    kept = [ix for ix in range(len(sizes)) if sizes[ix][0] > 0.0 or sizes[ix][1] > 0.0]
    dense = [sizes[ix] for ix in kept]
    packed, extent = pack(dense)

    at: list[Point] = [(0.0, 0.0)] * len(sizes)
    for dense_ix, ix in enumerate(kept):
        at[ix] = packed[dense_ix]
    return at, extent


def remap(sizes: Sequence[Size], parents: Sequence[Optional[int]]) -> list[Optional[int]]:
    """The parent relation over drawn containers, renumbered to match `without_empties`."""
    dense: list[Optional[int]] = [None] * len(sizes)
    next_ix = 0
    for ix in range(len(sizes)):
        if sizes[ix][0] > 0.0 or sizes[ix][1] > 0.0:
            dense[ix] = next_ix
            next_ix += 1
    out: list[Optional[int]] = []
    for ix in range(len(sizes)):
        if dense[ix] is None:
            continue
        up = parents[ix]
        out.append(dense[up] if up is not None else None)
    return out


def task_forest(tasks: Sequence[Task], agents: Sequence[Agent]) -> list[Optional[int]]:
    """Which container hangs under which."""
    parents: list[Optional[int]] = [None] * len(tasks)
    for ix, task in enumerate(tasks):
        members = [a for a in agents if a.task == task.id]
        member_ids = {m.id for m in members}
        for member in members:
            spawner = member.parent
            if spawner is None:
                continue
            if spawner in member_ids:
                continue
            up_agent = next((a for a in agents if a.id == spawner), None)
            above = None
            if up_agent is not None and up_agent.task is not None:
                above = next((i for i, t in enumerate(tasks) if t.id == up_agent.task), None)
            if above is not None and above != ix:
                parents[ix] = above
                break
    return parents


def pack_flow(
    sizes: Sequence[Size], parents: Sequence[Optional[int]], target: float = 1.0
) -> Packing:
    return pack_shelf(sizes, LAYOUT_WIDTH - LAYOUT_MARGIN, TASK_GAP)


def pack_packed(
    sizes: Sequence[Size], parents: Sequence[Optional[int]], target: float = 1.0
) -> Packing:
    return without_empties(sizes, lambda kept: pack_best(kept, TASK_GAP))


def pack_forest(
    sizes: Sequence[Size], parents: Sequence[Optional[int]], target: float = 1.0
) -> Packing:
    kept_parents = remap(sizes, parents)
    return without_empties(sizes, lambda kept: pack_tree(kept, kept_parents, TASK_GAP))


def pack_view_shelf(
    sizes: Sequence[Size], parents: Sequence[Optional[int]], target: float = VIEW_ASPECT
) -> Packing:
    """A shelf as wide as the screen wants, which keeps record order and still fills a rectangle.

    The widths worth trying are the ones that actually change where the rows break — the running
    total of the boxes in record order — plus `shelf_width`'s area-derived guess. `score` against
    the target picks between them, which is the same judgement `pack_best` makes and saves inventing
    a slack constant that would be right for one scenario and wrong for the next.
    """
    kept = [s for s in sizes if s[0] > 0.0 or s[1] > 0.0]
    if not kept:
        return pack_shelf(sizes, LAYOUT_WIDTH - LAYOUT_MARGIN, TASK_GAP)
    content = sum(w * h for w, h in kept)
    widest = max(s[0] for s in kept)

    widths = {widest, shelf_width(kept, target)}
    run = -TASK_GAP
    for w, _ in kept:
        run += w + TASK_GAP
        widths.add(max(run, widest))

    best: Optional[tuple[Packing, float]] = None
    for width in sorted(widths):
        packed = pack_shelf(sizes, width, TASK_GAP)
        marked = score(packed[1], content, target)
        if best is None or marked < best[1]:
            best = (packed, marked)
    return best[0] if best else pack_shelf(sizes, widest, TASK_GAP)


def pack_view_best(
    sizes: Sequence[Size], parents: Sequence[Optional[int]], target: float = VIEW_ASPECT
) -> Packing:
    """The tightest fit, aimed at the screen's rectangle instead of at a square."""
    return without_empties(sizes, lambda kept: pack_best(kept, TASK_GAP, target, commit=True))


# ── the whole arrangement ─────────────────────────────────────────────────────


@dataclass
class Placed:
    """One card on the canvas: its offset, its delegates' slots, and what those work out to."""

    agent: Agent
    offset: Point
    slots: list[Point] = field(default_factory=list)
    depth: int = 0
    at: Point = (0.0, 0.0)
    subs: list[tuple[str, str, Point]] = field(default_factory=list)
    ring: Optional[Rect] = None


@dataclass
class TaskBox:
    task: Task
    origin: Point = (0.0, 0.0)
    rect: Optional[Rect] = None
    size: Size = (0.0, 0.0)  # what the packer reserved, before anything was drawn


@dataclass
class SessionBox:
    session: Session
    rect: Rect


@dataclass
class Conn:
    a: Point
    b: Point
    kind: str


@dataclass
class Arrangement:
    scenario: Scenario
    algo: "Algo"
    origins: dict[str, Point] = field(default_factory=dict)
    sessions: list[SessionBox] = field(default_factory=list)
    tasks: list[TaskBox] = field(default_factory=list)
    cards: list[Placed] = field(default_factory=list)
    conns: list[Conn] = field(default_factory=list)
    bbox: Rect = (0.0, 0.0, 0.0, 0.0)
    band: dict[str, float] = field(default_factory=dict)  # a session's top, per session id
    lines: dict[tuple[str, int], list[float]] = field(default_factory=dict)
    before: dict[str, Point] = field(default_factory=dict)
    before_order: list[str] = field(default_factory=list)
    frames: list["Arrangement"] = field(default_factory=list)
    growth: Optional[dict] = None
    passes: int = 0


def _union(boxes: Iterable[Rect]) -> Rect:
    xs0: list[float] = []
    ys0: list[float] = []
    xs1: list[float] = []
    ys1: list[float] = []
    for x, y, w, h in boxes:
        xs0.append(x)
        ys0.append(y)
        xs1.append(x + w)
        ys1.append(y + h)
    if not xs0:
        return (0.0, 0.0, 0.0, 0.0)
    return (min(xs0), min(ys0), max(xs1) - min(xs0), max(ys1) - min(ys0))


def content_rect(card: Placed) -> Rect:
    """What one card takes up on the canvas, the same reading `ring_box` reserves by.

    `card.ring` is the fence: the union of the card and its delegates with `RING_PAD` all round.
    Inset it back to that bare union and re-pad it with `ring_pad`, and the answer is
    `CARD_HEIGHT + ring_drop(n)` for the one-per-row ring the Rust has today — and the right box
    for a ring that reaches sideways, which is the whole point of the change.
    """
    rect = (card.at[0], card.at[1], CARD_WIDTH, CARD_HEIGHT)
    if card.ring:
        x, y, w, h = card.ring
        held = (x + RING_PAD, y + RING_PAD, w - RING_PAD * 2.0, h - RING_PAD * 2.0)
        rect = _union([rect, ring_pad(held, rect)])
    return rect


# ── the registry ──────────────────────────────────────────────────────────────


@dataclass(frozen=True)
class Algo:
    """One arrangement: three small functions, the words the menu row carries, and a refinement."""

    key: str
    label: str
    hint: str
    inside: Callable[..., Contents]
    ring: Callable[[int], list[Point]]
    pack: Callable[..., Packing]
    grow: Optional[Callable[["Arrangement"], int]] = None
    #: Whether the packer is handed which container hangs under which. Most do not read it.
    forest: bool = False


def algo_of(name: "str | Algo") -> Algo:
    if isinstance(name, Algo):
        return name
    try:
        return ALGOS[name]
    except KeyError:
        raise SystemExit(f"unknown algorithm {name!r} — have {', '.join(ALGOS)}")


def layout_auto(scenario: Scenario, algo: "str | Algo") -> Arrangement:
    """The whole arrangement, ready to draw.

    A from-scratch arrangement is `Layout::auto` and ignores the scenario's `positions`. One with a
    `grow` is built up a block at a time in arrival order instead. Either way the arrangement that
    already existed is kept, because that is what a from-scratch tidy's displacement is measured
    against.
    """
    algo = algo_of(algo)

    if algo.grow:
        out = Arrangement(scenario=scenario, algo=algo)
        out.passes = algo.grow(out)
    else:
        out = _build(scenario, algo)
        _finish(out)

    entry = _input(scenario)
    out.before = {card.agent.id: card.at for card in entry.cards}
    out.before_order = _reading_order(entry)
    return out


def _input(scenario: Scenario) -> Arrangement:
    """The arrangement a from-scratch tidy displaces: `flow`, with whatever a human moved over it."""
    entry = _build(scenario, ALGOS["flow"])
    apply_positions(entry)
    _finish(entry)
    return entry


def apply_positions(out: Arrangement) -> None:
    """Lay the scenario's hand-placed positions over whatever the arrangement worked out."""
    placed = out.scenario.positions
    for task, origin in placed.tasks.items():
        if task in out.origins:
            out.origins[task] = origin
    for card in out.cards:
        if card.agent.id in placed.agents:
            card.offset = placed.agents[card.agent.id]
        for ix, (sid, _) in enumerate(card.agent.subagents):
            moved = placed.subagents.get(f"{card.agent.id}/{sid}")
            if moved is not None and ix < len(card.slots):
                card.slots[ix] = moved


def _build(scenario: Scenario, algo: Algo) -> Arrangement:
    """`Layout::auto`: every session from scratch, stacked, reading only the definitions."""
    agents = scenario.agents
    tasks = scenario.tasks
    rings = rings_of(agents)

    order: list[str] = []
    for session in [a.session for a in agents] + [t.session for t in tasks if t.session]:
        if session not in order:
            order.append(session)

    out = Arrangement(scenario=scenario, algo=algo)
    y = LAYOUT_MARGIN
    for session in order:
        out.band[session] = y
        y = _arrange(out, session, y, agents, tasks, algo, rings)
    return out


def _arrange(
    out: Arrangement,
    session: str,
    top: float,
    agents: Sequence[Agent],
    tasks: Sequence[Task],
    algo: Algo,
    rings: Rings,
) -> float:
    """`Layout::arrange`: the loose block along the top, then the containers under it."""
    y = top
    by_id = {a.id: a for a in agents}

    loose = [a for a in agents if a.session == session and a.task is None]
    block = stack(loose, rings, algo.ring)
    if block.cards:
        for agent, offset in block.cards:
            _card(out, by_id[agent], (LAYOUT_MARGIN + offset[0], y + offset[1]), algo)
        y += block.height + TASK_GAP

    boxes = [t for t in tasks if t.session == session]
    target = aspect_of(out.scenario)
    contents = [algo.inside(t.id, agents, rings, algo.ring, target) for t in boxes]
    sizes: list[Size] = [
        (0.0, 0.0)
        if not c.cards
        else (c.width + GROUP_PAD * 2.0, c.height + GROUP_PAD * 2.0 + GROUP_LABEL)
        for c in contents
    ]

    wants_forest = algo.forest or algo.key == "tree"
    parents = task_forest(boxes, agents) if wants_forest else [None] * len(boxes)
    at, extent = algo.pack(sizes, parents, target)

    for ix, task in enumerate(boxes):
        origin = (
            LAYOUT_MARGIN + at[ix][0] + GROUP_PAD,
            y + at[ix][1] + GROUP_PAD + GROUP_LABEL,
        )
        out.origins[task.id] = origin
        out.tasks.append(TaskBox(task=task, origin=origin, size=sizes[ix]))
        for agent, offset in contents[ix].cards:
            _card(out, by_id[agent], offset, algo)

    if extent[1] > 0.0:
        y += extent[1] + TASK_GAP
    return y


def _card(out: Arrangement, agent: Agent, offset: Point, algo: Algo) -> None:
    out.cards.append(Placed(agent=agent, offset=offset, slots=algo.ring(len(agent.subagents))))


def _finish(out: Arrangement) -> None:
    """Work every position out from the origins and the offsets, and derive what encloses them."""
    for card in out.cards:
        origin = out.origins.get(card.agent.task, (0.0, 0.0)) if card.agent.task else (0.0, 0.0)
        card.at = (origin[0] + card.offset[0], origin[1] + card.offset[1])
        card.subs = [
            (sid, name, (card.at[0] + slot[0], card.at[1] + slot[1]))
            for (sid, name), slot in zip(card.agent.subagents, card.slots)
        ]
        card.ring = fence(card.at, [s[2] for s in card.subs])

    inside: dict[str, list[Rect]] = {}
    for card in out.cards:
        if card.agent.task:
            inside.setdefault(card.agent.task, []).append(content_rect(card))

    for box in out.tasks:
        box.origin = out.origins.get(box.task.id, box.origin)
        rects = inside.get(box.task.id)
        if not rects:
            box.rect = None
            continue
        held = _union(rects)
        box.rect = (
            held[0] - GROUP_PAD,
            held[1] - GROUP_PAD - GROUP_LABEL,
            held[2] + GROUP_PAD * 2.0,
            held[3] + GROUP_PAD * 2.0 + GROUP_LABEL,
        )

    by_id = {s.id: s for s in out.scenario.sessions}
    out.sessions = []
    for session in out.band:
        held = _union(
            [content_rect(c) for c in out.cards if c.agent.session == session]
            + [b.rect for b in out.tasks if b.rect and b.task.session == session]
        )
        if held[2] <= 0.0 and held[3] <= 0.0:
            continue
        out.sessions.append(
            SessionBox(
                session=by_id.get(session, Session(id=session, name=session)),
                rect=(
                    held[0] - GROUP_PAD,
                    held[1] - GROUP_PAD - GROUP_LABEL,
                    held[2] + GROUP_PAD * 2.0,
                    held[3] + GROUP_PAD * 2.0 + GROUP_LABEL,
                ),
            )
        )

    _connect(out)
    out.bbox = _union([b.rect for b in out.sessions] + [content_rect(c) for c in out.cards])


def _connect(out: Arrangement) -> None:
    """Parent edges, then the extra links that are drawn but never move a card."""
    placed = {card.agent.id: card.at for card in out.cards}
    out.conns = []
    seen: set[tuple[str, str]] = set()
    for card in out.cards:
        up = card.agent.parent
        if up is None or up not in placed:
            continue
        seen.add((up, card.agent.id))
        out.conns.append(Conn(a=_bottom(placed[up]), b=_top(card.at), kind="spawn"))
    for link in out.scenario.links:
        if link.frm not in placed or link.to not in placed:
            continue
        if link.kind == "spawn" and (link.frm, link.to) in seen:
            continue
        out.conns.append(Conn(a=_bottom(placed[link.frm]), b=_top(placed[link.to]), kind=link.kind))


def _bottom(at: Point) -> Point:
    return (at[0] + CARD_WIDTH / 2.0, at[1] + CARD_HEIGHT)


def _top(at: Point) -> Point:
    return (at[0] + CARD_WIDTH / 2.0, at[1])


def _reading_order(out: Arrangement) -> list[str]:
    drawn = [b for b in out.tasks if b.rect]
    return [
        b.task.id for b in sorted(drawn, key=lambda b: (round(b.rect[1], 1), round(b.rect[0], 1)))
    ]


# ── adaptive: the incremental arrangement ─────────────────────────────────────
#
# Nothing here lays a session out. A block arrives, finds a place against what is already on
# screen, and pushes only what it overlaps — by the smallest delta that clears it. A session is
# never re-packed, and the ripple is capped: no block moves twice for one arrival.

def room_of(scenario: Scenario) -> float:
    """How wide the canvas is worth filling: the screen, and never narrower than `LAYOUT_WIDTH`."""
    return max(scenario.viewport[0], LAYOUT_WIDTH)


def wrap_at(room: float) -> int:
    """How many cards a row band takes before it wraps onto a second line inside the same band."""
    return max(
        1,
        int((room - LAYOUT_MARGIN * 2.0 - GROUP_PAD * 2.0 + CARD_GAP_X) // (CARD_WIDTH + CARD_GAP_X)),
    )


#: The wrap on the canvas `LAYOUT_WIDTH` describes — what the arithmetic answers with no screen.
WRAP = wrap_at(LAYOUT_WIDTH)


def _drawn(out: Arrangement, session: str) -> list[TaskBox]:
    return [b for b in out.tasks if b.rect and b.task.session == session]


def _loose(out: Arrangement, session: str) -> list[Rect]:
    return [
        content_rect(c) for c in out.cards if c.agent.session == session and not c.agent.task
    ]


def _apart(a: Rect, b: Rect, gap: float) -> bool:
    return (
        a[0] + a[2] + gap <= b[0] + EPS
        or b[0] + b[2] + gap <= a[0] + EPS
        or a[1] + a[3] + gap <= b[1] + EPS
        or b[1] + b[3] + gap <= a[1] + EPS
    )


def _close(a: Rect, b: Rect, gx: float, gy: float) -> bool:
    return not (
        a[0] + a[2] + gx <= b[0] + EPS
        or b[0] + b[2] + gx <= a[0] + EPS
        or a[1] + a[3] + gy <= b[1] + EPS
        or b[1] + b[3] + gy <= a[1] + EPS
    )


def _away(at: Rect, other: Rect, gx: float, gy: float) -> Point:
    """The smallest delta that clears `other` of `at`, in the direction it already lies."""
    down = at[1] + at[3] + gy - other[1]
    up = other[1] + other[3] + gy - at[1]
    right = at[0] + at[2] + gx - other[0]
    left = other[0] + other[2] + gx - at[0]
    dy = down if other[1] + other[3] / 2.0 >= at[1] + at[3] / 2.0 else -up
    dx = right if other[0] + other[2] / 2.0 >= at[0] + at[2] / 2.0 else -left
    return (0.0, dy) if abs(dy) <= abs(dx) else (dx, 0.0)


def _move(out: Arrangement, box: TaskBox, dx: float, dy: float) -> None:
    """Move one container, which moves its cards with it and nothing else."""
    x, y = out.origins[box.task.id]
    out.origins[box.task.id] = (x + dx, y + dy)
    _finish(out)


def _wave(out: Arrangement, session: str, rect: Rect, skip: set[str]) -> None:
    """One ripple out from `rect`. Each container moves once in it, so the ripple ends.

    A container a human placed is never in the ripple: `apply_positions` puts it back after every
    arrival, so moving it is a move that does not happen, and the block that grew into it is the one
    that has to give way.
    """
    pinned = set(out.scenario.positions.tasks)
    queue: list[Rect] = [rect]
    while queue:
        at = queue.pop(0)
        for box in _drawn(out, session):
            if box.task.id in skip or box.task.id in pinned:
                continue
            if not _close(at, box.rect, TASK_GAP - EPS, TASK_GAP - EPS):
                continue
            dx, dy = _away(at, box.rect, TASK_GAP, TASK_GAP)
            skip.add(box.task.id)
            _move(out, box, dx, dy)
            queue.append(box.rect)


def _clashing(out: Arrangement, session: str) -> Optional[TaskBox]:
    """The first container, in reading order, that another one is still sitting on top of."""
    drawn = sorted(_drawn(out, session), key=lambda b: (round(b.rect[1], 1), round(b.rect[0], 1)))
    for ix, box in enumerate(drawn):
        for other in drawn[ix + 1 :]:
            if _close(box.rect, other.rect, TASK_GAP - EPS, TASK_GAP - EPS):
                return box
    return None


def _push(out: Arrangement, session: str, rect: Rect, skip: set[str]) -> None:
    """Push the containers `rect` overlaps clear of it, and keep pushing until none overlap.

    One wave moves each container at most once, which is what keeps the ripple finite — but a
    container that grew sideways twice needs the one beside it moved twice, and a wave that has
    already spent its move on it leaves the two on top of each other. So a wave that settles with a
    clash left in it is followed by another, seeded from the container that is *earlier* in reading
    order, so the block a user has been looking at longest is the one that stays put.
    """
    _wave(out, session, rect, skip)
    for _ in range(PUSH_WAVES):
        clash = _clashing(out, session)
        if clash is None:
            return
        _wave(out, session, clash.rect, {clash.task.id})


def _push_cards(out: Arrangement, task: str, rect: Rect, skip: set[str]) -> None:
    """The same, for the cards inside one container: only what the grown block now overlaps."""
    queue: list[Rect] = [rect]
    while queue:
        at = queue.pop(0)
        for card in out.cards:
            if card.agent.task != task or card.agent.id in skip:
                continue
            held = content_rect(card)
            if not _close(at, held, CARD_GAP_X - EPS, CARD_GAP_Y - EPS):
                continue
            dx, dy = _away(at, held, CARD_GAP_X, CARD_GAP_Y)
            skip.add(card.agent.id)
            card.offset = (card.offset[0] + dx, card.offset[1] + dy)
            _finish(out)
            queue.append(content_rect(card))


def _arrivals(scenario: Scenario) -> list[tuple]:
    """A scenario read as a history: the blocks in the order they turned up."""
    tasks = {t.id: t for t in scenario.tasks}
    events: list[tuple] = []
    seen_s: set[str] = set()
    seen_t: set[str] = set()

    def session(sid: Optional[str]) -> None:
        if sid and sid not in seen_s:
            seen_s.add(sid)
            events.append(("session", sid))

    for agent in scenario.agents:
        session(agent.session)
        if agent.task and agent.task in tasks and agent.task not in seen_t:
            seen_t.add(agent.task)
            events.append(("task", tasks[agent.task]))
        events.append(("agent", agent))
        for sub in agent.subagents:
            events.append(("sub", agent, sub))
    for task in scenario.tasks:
        if task.id not in seen_t:
            session(task.session)
            seen_t.add(task.id)
            events.append(("task", task))
    return events


def grow(out: Arrangement) -> int:
    """Place every block in arrival order. Answers how many arrived, and keeps a few frames."""
    events = _arrivals(out.scenario)
    marks = {len(events) // 3, 2 * len(events) // 3}
    stats: dict = {"arrivals": 0, "moves": [], "pairs": 0, "still": 0, "order_kept": True}

    for ix, event in enumerate(events):
        was = {card.agent.id: card.at for card in out.cards}
        order = _reading_order(out)

        kind = event[0]
        if kind == "session":
            _arrive_session(out, event[1])
        elif kind == "task":
            out.tasks.append(TaskBox(task=event[1]))
        elif kind == "agent":
            _arrive_agent(out, event[1])
        else:
            _arrive_sub(out, event[1], event[2])
        # A human's own placement wins over whatever the arrangement worked out for it.
        apply_positions(out)
        _finish(out)

        stats["arrivals"] += 1
        for card in out.cards:
            if card.agent.id not in was:
                continue
            stats["pairs"] += 1
            gone = math.hypot(card.at[0] - was[card.agent.id][0], card.at[1] - was[card.agent.id][1])
            if gone < 0.5:
                stats["still"] += 1
            else:
                stats["moves"].append(gone)
        kept = [t for t in _reading_order(out) if t in order]
        if kept != order:
            stats["order_kept"] = False
        if ix in marks and ix > 0:
            out.frames.append(_snap(out))

    out.growth = stats
    return stats["arrivals"]


def _snap(out: Arrangement) -> Arrangement:
    """A frame of the growth, for the contact sheet."""
    frames, out.frames = out.frames, []
    shot = copy.deepcopy(out)
    out.frames = frames
    return shot


def _arrive_session(out: Arrangement, session: str) -> None:
    """A new session goes below the last one."""
    if session in out.band:
        return
    bottom = [b.rect[1] + b.rect[3] for b in out.tasks if b.rect]
    bottom += [content_rect(c)[1] + content_rect(c)[3] for c in out.cards]
    out.band[session] = LAYOUT_MARGIN if not bottom else max(bottom) + GROUP_PAD + TASK_GAP


def _arrive_agent(out: Arrangement, agent: Agent) -> None:
    """A new agent on a task: the row for its depth, at the first free x inside the container."""
    if agent.task is None:
        _arrive_loose(out, agent)
        return
    box = next((b for b in out.tasks if b.task.id == agent.task), None)
    if box is None:
        return
    depth = _depth_now(out, agent)
    if box.task.id not in out.origins:
        out.origins[box.task.id] = _first_fit(out, box, agent)
    card = Placed(agent=agent, offset=_slot_in(out, box.task.id, depth), depth=depth)
    out.cards.append(card)
    _finish(out)
    if box.rect:
        _push(out, agent.session, box.rect, {box.task.id})


def _arrive_sub(out: Arrangement, agent: Agent, sub: tuple[str, str]) -> None:
    """A new delegate: the first free slot in its parent's ring, in whatever shape is in force."""
    card = next((c for c in out.cards if c.agent.id == agent.id), None)
    if card is None:
        return
    count = len(card.slots) + 1
    # The ring re-flows over the card it belongs to — a grid gains a row, a radial ring re-spaces.
    # Nothing outside this card moves for it; that is still `_push_cards`' business below.
    card.slots = out.algo.ring(count)
    _finish(out)
    if agent.task:
        _push_cards(out, agent.task, content_rect(card), {agent.id})
        box = next((b for b in out.tasks if b.task.id == agent.task and b.rect), None)
        if box:
            _push(out, agent.session, box.rect, {box.task.id})
    else:
        _push(out, agent.session, content_rect(card), set())


def _arrive_loose(out: Arrangement, agent: Agent) -> None:
    """An agent nobody gave work to: the next free place in the block along the session's top."""
    session = agent.session
    up = next(
        (c for c in out.cards if c.agent.id == agent.parent and not c.agent.task),
        None,
    )
    depth = up.depth + 1 if up else 0
    y = out.band.get(session, LAYOUT_MARGIN) + depth * (CARD_HEIGHT + CARD_GAP_Y)
    mates = _loose(out, session)
    x = LAYOUT_MARGIN
    while any(_close((x, y, CARD_WIDTH, CARD_HEIGHT), o, CARD_GAP_X - EPS, CARD_GAP_Y - EPS) for o in mates):
        x += CARD_WIDTH + CARD_GAP_X
    card = Placed(agent=agent, offset=(x, y), depth=depth)
    out.cards.append(card)
    _finish(out)
    _push(out, session, content_rect(card), set())


def _depth_now(out: Arrangement, agent: Agent) -> int:
    """How deep the arriving card is, read off the cards already in its container."""
    up = next(
        (c for c in out.cards if c.agent.id == agent.parent and c.agent.task == agent.task),
        None,
    )
    return up.depth + 1 if up else 0


def _slot_in(out: Arrangement, task: str, depth: int) -> Point:
    """The first free x in a container's depth band, wrapping to a new line in the same band."""
    line = out.lines.get((task, depth))
    if line and line[2] < wrap_at(room_of(out.scenario)):
        offset = (line[1], line[0])
        line[1] += CARD_WIDTH + CARD_GAP_X
        line[2] += 1
        return offset
    bottoms = [
        card.offset[1] + (content_rect(card)[1] + content_rect(card)[3] - card.at[1])
        for card in out.cards
        if card.agent.task == task
    ]
    y = max(bottoms) + CARD_GAP_Y if bottoms else 0.0
    out.lines[(task, depth)] = [y, CARD_WIDTH + CARD_GAP_X, 1]
    return (0.0, y)


def _first_fit(out: Arrangement, box: TaskBox, agent: Agent) -> Point:
    """A new container, first-fit into the gaps this session already has.

    A gap in the same reading band as its parent task wins, then one near it, then the reading end.
    Nothing already placed moves to make the gap.
    """
    session = box.task.session or agent.session
    size = (CARD_WIDTH + GROUP_PAD * 2.0, CARD_HEIGHT + GROUP_PAD * 2.0 + GROUP_LABEL)
    others = [b.rect for b in _drawn(out, session)] + _loose(out, session)
    top = out.band.get(session, LAYOUT_MARGIN)
    if not others:
        return (LAYOUT_MARGIN + GROUP_PAD, top + GROUP_PAD + GROUP_LABEL)

    near: Optional[Point] = None
    if agent.parent:
        up = next((c for c in out.cards if c.agent.id == agent.parent), None)
        if up is not None and up.agent.task:
            held = next((b for b in out.tasks if b.task.id == up.agent.task and b.rect), None)
            if held and held.rect:
                near = (held.rect[0], held.rect[1])
    anchor = near or (LAYOUT_MARGIN, top)

    end = max(o[1] + o[3] for o in others) + TASK_GAP
    bands = sorted({round(o[1], 1) for o in others} | {end})
    xs = sorted({LAYOUT_MARGIN} | {o[0] + o[2] + TASK_GAP for o in others} | {anchor[0]})

    best: Optional[tuple[tuple, Point]] = None
    for y in bands:
        if y < top - EPS:
            continue
        for x in xs:
            if x + size[0] > room_of(out.scenario) - LAYOUT_MARGIN + EPS:
                continue
            rect = (x, y, size[0], size[1])
            if any(_close(rect, o, TASK_GAP - EPS, TASK_GAP - EPS) for o in others):
                continue
            cost = (abs(y - anchor[1]), abs(x - anchor[0]), y, x)
            if best is None or cost < best[0]:
                best = (cost, (x, y))
    x, y = best[1] if best else (LAYOUT_MARGIN, end)
    return (x + GROUP_PAD, y + GROUP_PAD + GROUP_LABEL)


ALGOS: dict[str, Algo] = {
    "flow": Algo(
        "flow",
        "Flow",
        "containers in record order, wrapping across the canvas",
        inside_stack,
        ring_rows,
        pack_flow,
    ),
    "packed": Algo(
        "packed",
        "Packed",
        "the tightest fit, for when the whitespace bothers you",
        inside_wrapped,
        ring_rows,
        pack_packed,
    ),
    "tree": Algo(
        "tree",
        "Tree",
        "containers hang under whoever spawned them",
        inside_stack,
        ring_rows,
        pack_forest,
    ),
    "columns": Algo(
        "columns",
        "Columns",
        "one card per row, for a narrow window",
        inside_column,
        ring_rows,
        pack_flow,
    ),
    "adaptive": Algo(
        "adaptive",
        "Adaptive",
        "places each arriving block into the arrangement already on screen",
        inside_stack,
        ring_grid,
        pack_flow,
        grow=grow,
    ),
    "multiline": Algo(
        "multiline",
        "Multiline",
        "delegates in a grid under their card, everything folded to the screen's shape",
        inside_aspect,
        ring_grid,
        pack_view_shelf,
    ),
    "radial": Algo(
        "radial",
        "Radial",
        "delegates round their card, packed against the screen's rectangle",
        inside_aspect,
        ring_radial,
        pack_view_best,
    ),
}


def _shapes() -> None:
    """The arrangements with no counterpart in `layout.rs` live in `shapes.py`, and join here.

    Imported at the bottom rather than at the top because `shapes.py` reads this file's constants
    and helpers — this is the port, and nothing that is not in the Rust is written above this line.
    The import is for its effect: `shapes.py` adds its own rows to `ALGOS`, so that whichever of the
    two modules is imported first, the registry is whole by the time anyone reads it.
    """
    import shapes  # noqa: F401


_shapes()
