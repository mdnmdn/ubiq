"""The block-positioning algorithms of the teams graph, ported from Rust, plus `adaptive`.

The first part is a port of `crates/ubiq/src/state/layout.rs`: the same constants, the same
function names, the same arithmetic. Nothing there is an improvement on the Rust — where the two
disagree the port is wrong and is what gets fixed, because the point of the port is that a rendered
PNG can be trusted to show what the app does.

The arrangements express themselves through `ALGOS`, one entry each: how cards sit inside one
container (`inside`), where a card's delegates go inside its own fence (`ring`), and how the
containers of a session are packed (`pack`). A new arrangement is one dict entry plus three small
functions. An arrangement may also carry a `refine`, which is what `adaptive` is: it starts from the
arrangement that exists — the scenario's hand-placed `positions`, otherwise `flow` — and moves as
little as it can.
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
    for sub in subs:
        x0 = min(x0, sub[0])
        y0 = min(y0, sub[1])
        x1 = max(x1, sub[0] + SUB_WIDTH)
        y1 = max(y1, sub[1] + SUB_HEIGHT)
    return (x0 - RING_PAD, y0 - RING_PAD, (x1 - x0) + RING_PAD * 2.0, (y1 - y0) + RING_PAD * 2.0)


def ring_rows(count: int) -> list[Point]:
    """The ring every from-scratch arrangement wears: one delegate to a row under the card."""
    return [sub_slot(ix) for ix in range(count)]


# ── what a card and a row take up ─────────────────────────────────────────────


def card_extent(agent: str, rings: Rings) -> float:
    return CARD_HEIGHT + ring_drop(rings.get(agent, 0))


def row_extent(row: Sequence[str], rings: Rings) -> float:
    return max((card_extent(a, rings) for a in row), default=0.0)


def row_width(cards: int) -> float:
    return cards * CARD_WIDTH + max(cards - 1, 0) * CARD_GAP_X


def break_at(cards: int) -> int:
    """How many cards a wrapped row takes before it folds: the side of the smallest square."""
    return int(max(math.ceil(math.sqrt(cards)), 1))


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


def stack(members: Sequence[Agent], rings: Rings) -> Contents:
    """Roots on the top row, their children on the next. One row per hand-off depth."""
    rows = rows_by_depth(members)
    if not rows:
        return Contents()

    width = max((row_width(len(row)) for row in rows), default=0.0)

    cards: list[tuple[str, Point]] = []
    y = 0.0
    height = 0.0
    for row in rows:
        start = (width - row_width(len(row))) / 2.0
        for ix, agent in enumerate(row):
            cards.append((agent, (start + ix * (CARD_WIDTH + CARD_GAP_X), y)))
        tall = row_extent(row, rings)
        height = y + tall
        y += tall + CARD_GAP_Y
    return Contents(cards, width, height)


def stack_wrapped(members: Sequence[Agent], rings: Rings) -> Contents:
    """The same depth rows, each folded into a near-square block instead of one long line."""
    rows = rows_by_depth(members)
    if not rows:
        return Contents()

    per_line = [break_at(len(row)) for row in rows]
    width = max((row_width(n) for n in per_line), default=0.0)

    cards: list[tuple[str, Point]] = []
    y = 0.0
    height = 0.0
    for row, per in zip(rows, per_line):
        for start_ix in range(0, len(row), per):
            chunk = row[start_ix : start_ix + per]
            start = (width - row_width(len(chunk))) / 2.0
            for ix, agent in enumerate(chunk):
                cards.append((agent, (start + ix * (CARD_WIDTH + CARD_GAP_X), y)))
            tall = row_extent(chunk, rings)
            height = y + tall
            y += tall + CARD_GAP_Y
    return Contents(cards, width, height)


def column(members: Sequence[Agent], rings: Rings) -> Contents:
    """One card per row, in spawn order."""
    rows = rows_by_depth(members)
    if not rows:
        return Contents()

    cards: list[tuple[str, Point]] = []
    y = 0.0
    height = 0.0
    for row in rows:
        for agent in row:
            cards.append((agent, (0.0, y)))
            tall = card_extent(agent, rings)
            height = y + tall
            y += tall + CARD_GAP_Y
    return Contents(cards, CARD_WIDTH, height)


def _members(task: str, agents: Sequence[Agent]) -> list[Agent]:
    return [a for a in agents if a.task == task]


def inside_stack(task: str, agents: Sequence[Agent], rings: Rings) -> Contents:
    return stack(_members(task, agents), rings)


def inside_wrapped(task: str, agents: Sequence[Agent], rings: Rings) -> Contents:
    return stack_wrapped(_members(task, agents), rings)


def inside_column(task: str, agents: Sequence[Agent], rings: Rings) -> Contents:
    return column(_members(task, agents), rings)


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


def pack_skyline(sizes: Sequence[Size], width: float, gap: float) -> Packing:
    """Best-fit skyline packing into a container `width`. Positions come back in `sizes` order."""
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
            grown = max(extent[0], x + w - gap) * max(extent[1], y + h - gap)
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


def score(extent: Size, content: float) -> float:
    """How bad a packing is — smaller is better."""
    OFF_SQUARE = 0.35
    WASTED = 0.5

    area = extent[0] * extent[1]
    if area <= 0.0:
        return 0.0
    aspect = max(extent[0] / extent[1], extent[1] / extent[0])
    waste = min(max((area - content) / area, 0.0), 1.0)
    return area * (1.0 + OFF_SQUARE * (aspect - 1.0) + WASTED * waste)


def pack_best(sizes: Sequence[Size], gap: float) -> Packing:
    """Pack against a handful of container widths and keep the best-scoring result."""
    content = sum(w * h for w, h in sizes)
    widest = max((s[0] for s in sizes), default=0.0)
    side = math.sqrt(content)

    best: Optional[tuple[Packing, float]] = None
    for ratio in (0.7, 1.0, 1.4, 1.9):
        packed = pack_skyline(sizes, max(side * ratio, widest), gap)
        marked = score(packed[1], content)
        if best is None or marked < best[1]:
            best = (packed, marked)
    return best[0] if best else ([], (0.0, 0.0))


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


def pack_flow(sizes: Sequence[Size], parents: Sequence[Optional[int]]) -> Packing:
    return pack_shelf(sizes, LAYOUT_WIDTH - LAYOUT_MARGIN, TASK_GAP)


def pack_packed(sizes: Sequence[Size], parents: Sequence[Optional[int]]) -> Packing:
    return without_empties(sizes, lambda kept: pack_best(kept, TASK_GAP))


def pack_forest(sizes: Sequence[Size], parents: Sequence[Optional[int]]) -> Packing:
    kept_parents = remap(sizes, parents)
    return without_empties(sizes, lambda kept: pack_tree(kept, kept_parents, TASK_GAP))


# ── the whole arrangement ─────────────────────────────────────────────────────


@dataclass
class Placed:
    """One card on the canvas: its offset, its delegates' slots, and what those work out to."""

    agent: Agent
    offset: Point
    slots: list[Point] = field(default_factory=list)
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
    before: dict[str, Point] = field(default_factory=dict)
    before_order: list[str] = field(default_factory=list)
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
    """What one card reserves: itself, and its delegates without the ring's own padding.

    For the default one-per-row ring this is exactly `CARD_HEIGHT + ring_drop(n)` tall, which is
    what the Rust packers leave room for — so the drawn picture and the reserved space agree, and a
    reflowed ring reserves what it actually takes.
    """
    rect = (card.at[0], card.at[1], CARD_WIDTH, CARD_HEIGHT)
    if card.ring:
        x, y, w, h = card.ring
        rect = _union([rect, (x + RING_PAD, y + RING_PAD, w - RING_PAD * 2.0, h - RING_PAD * 2.0)])
    return rect


# ── the registry ──────────────────────────────────────────────────────────────


@dataclass(frozen=True)
class Algo:
    """One arrangement: three small functions, the words the menu row carries, and a refinement."""

    key: str
    label: str
    hint: str
    inside: Callable[[str, Sequence[Agent], Rings], Contents]
    ring: Callable[[int], list[Point]]
    pack: Callable[[Sequence[Size], Sequence[Optional[int]]], Packing]
    refine: Optional[Callable[["Arrangement"], int]] = None


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
    `refine` starts from the input arrangement — the positions where they exist, `flow` otherwise —
    and adjusts it. Either way the input is kept, because that is what displacement is measured
    against.
    """
    algo = algo_of(algo)
    entry = _input(scenario)

    if algo.refine:
        out = copy.deepcopy(entry)
        object.__setattr__(out, "algo", algo)
        out.passes = algo.refine(out)
    else:
        out = _build(scenario, algo)
    _finish(out)

    out.before = {card.agent.id: card.at for card in entry.cards}
    out.before_order = _reading_order(entry)
    return out


def _input(scenario: Scenario) -> Arrangement:
    """The arrangement that already exists: `flow`, with whatever a human moved laid over it."""
    entry = _build(scenario, ALGOS["flow"])
    placed = scenario.positions
    for task, origin in placed.tasks.items():
        if task in entry.origins:
            entry.origins[task] = origin
    for card in entry.cards:
        if card.agent.id in placed.agents:
            card.offset = placed.agents[card.agent.id]
        for ix, (sid, _) in enumerate(card.agent.subagents):
            moved = placed.subagents.get(f"{card.agent.id}/{sid}")
            if moved is not None and ix < len(card.slots):
                card.slots[ix] = moved
    _finish(entry)
    return entry


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
    block = stack(loose, rings)
    if block.cards:
        for agent, offset in block.cards:
            _card(out, by_id[agent], (LAYOUT_MARGIN + offset[0], y + offset[1]), algo)
        y += block.height + TASK_GAP

    boxes = [t for t in tasks if t.session == session]
    contents = [algo.inside(t.id, agents, rings) for t in boxes]
    sizes: list[Size] = [
        (0.0, 0.0)
        if not c.cards
        else (c.width + GROUP_PAD * 2.0, c.height + GROUP_PAD * 2.0 + GROUP_LABEL)
        for c in contents
    ]

    parents = task_forest(boxes, agents) if algo.key == "tree" else [None] * len(boxes)
    at, extent = algo.pack(sizes, parents)

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


# ── adaptive: start from what is there and adjust ─────────────────────────────

PASSES = 8
GAIN = 0.01  # the balance improvement a pass has to earn to be worth another


def adaptive(out: Arrangement) -> int:
    """Bounded local passes over the arrangement that exists. Answers how many it took."""
    from metrics import balance

    reflow_rings(out)
    _finish(out)
    best = balance(out)

    passes = 0
    for _ in range(PASSES):
        passes += 1
        moved = gravity(out)
        moved = spill(out) or moved
        _finish(out)
        now = balance(out)
        if not moved or best - now < GAIN:
            break
        best = now
    return passes


def reflow_rings(out: Arrangement) -> bool:
    """Fold a card's tower of delegates into the free space beside it. The card does not move."""
    changed = False
    for card in out.cards:
        count = len(card.subs)
        if count < 4 or card.ring is None:
            continue
        room = _room_right(out, card)
        fits = 1 + max(int((room + SUB_GAP) // (SUB_WIDTH + SUB_GAP)), 0)
        cols = min(break_at(count), max(fits, 1))
        if cols <= 1:
            continue
        card.slots = [
            (
                (ix % cols) * (SUB_WIDTH + SUB_GAP),
                CARD_HEIGHT + SUB_DROP + (ix // cols) * (SUB_HEIGHT + SUB_GAP),
            )
            for ix in range(count)
        ]
        changed = True
    return changed


def _room_right(out: Arrangement, card: Placed) -> float:
    """Clear space to the right of a card's ring, before the next thing drawn in that band."""
    band = (card.at[1] + CARD_HEIGHT, card.ring[1] + card.ring[3])
    right = card.at[0] + CARD_WIDTH
    limit = math.inf
    for other in out.cards:
        if other is card or other.agent.session != card.agent.session:
            continue
        rect = content_rect(other)
        if rect[0] + rect[2] <= right + EPS:
            continue
        if rect[1] + rect[3] <= band[0] + EPS or rect[1] >= band[1] - EPS:
            continue
        limit = min(limit, rect[0] - CARD_GAP_X)
    return limit - right


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


def _shift(out: Arrangement, box: TaskBox, to: Point) -> None:
    """Move one container, which moves its cards with it and nothing else."""
    at = box.rect
    out.origins[box.task.id] = (
        out.origins[box.task.id][0] + to[0] - at[0],
        out.origins[box.task.id][1] + to[1] - at[1],
    )
    _finish(out)


def gravity(out: Arrangement) -> bool:
    """Pull each container straight up into the free space above it, keeping its x."""
    changed = False
    for session in out.band:
        for box in sorted(_drawn(out, session), key=lambda b: (b.rect[1], b.rect[0])):
            rect = box.rect
            limit = out.band[session]
            for other in _drawn(out, session):
                if other is box:
                    continue
                o = other.rect
                if o[0] >= rect[0] + rect[2] - EPS or o[0] + o[2] <= rect[0] + EPS:
                    continue
                if o[1] + o[3] <= rect[1] + EPS:
                    limit = max(limit, o[1] + o[3] + TASK_GAP)
            for o in _loose(out, session):
                if o[0] >= rect[0] + rect[2] - EPS or o[0] + o[2] <= rect[0] + EPS:
                    continue
                if o[1] + o[3] <= rect[1] + EPS:
                    limit = max(limit, o[1] + o[3] + TASK_GAP)
            if rect[1] - limit > 1.0:
                _shift(out, box, (rect[0], limit))
                changed = True
    return changed


def spill(out: Arrangement) -> bool:
    """Move a container that runs past the viewport's height into free space in a higher band."""
    from metrics import balance

    _, vh = out.scenario.viewport
    for session in out.band:
        drawn = _drawn(out, session)
        if len(drawn) < 2:
            continue
        held = _union([b.rect for b in drawn])
        if held[3] <= vh:
            continue
        box = max(drawn, key=lambda b: b.rect[1] + b.rect[3])
        rect = box.rect
        others = [b.rect for b in drawn if b is not box] + _loose(out, session)

        was = balance(out)
        best: Optional[tuple[float, Point]] = None
        for band in sorted({round(o[1], 1) for o in others}):
            if band >= rect[1] - EPS:
                continue
            x = max(
                (
                    o[0] + o[2] + TASK_GAP
                    for o in others
                    if o[1] < band + rect[3] - EPS and o[1] + o[3] > band + EPS
                ),
                default=rect[0],
            )
            to = (x, band)
            moved = (to[0], to[1], rect[2], rect[3])
            if any(not _apart(moved, o, TASK_GAP - EPS) for o in others):
                continue
            _shift(out, box, to)
            now = balance(out)
            if best is None or now < best[0]:
                best = (now, to)
            _shift(out, box, (rect[0], rect[1]))
        if best and was - best[0] > GAIN:
            _shift(out, box, best[1])
            return True
    return False


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
        "keeps the arrangement you have and only adjusts it",
        inside_stack,
        ring_rows,
        pack_flow,
        refine=adaptive,
    ),
}
