"""How readable an arrangement is, as numbers.

Everything here is geometry over a finished `Arrangement` — nothing reads the algorithms. The
numbers are what a future run is diffed against, so each one is named once and computed once.
"""

from __future__ import annotations

import math
from typing import Iterable, Sequence

from algos import CARD_HEIGHT, CARD_WIDTH, Arrangement, Rect


def drawn_rects(arr: Arrangement) -> list[Rect]:
    """Every rectangle with ink in it: the cards and the delegates, at this ring shape's size."""
    sw, sh = arr.algo.sub
    out: list[Rect] = []
    for card in arr.cards:
        out.append((card.at[0], card.at[1], CARD_WIDTH, CARD_HEIGHT))
        for _, _, at in card.subs:
            out.append((at[0], at[1], sw, sh))
    return out


def _overlap(a: Rect, b: Rect) -> float:
    w = min(a[0] + a[2], b[0] + b[2]) - max(a[0], b[0])
    h = min(a[1] + a[3], b[1] + b[3]) - max(a[1], b[1])
    return max(w, 0.0) * max(h, 0.0)


def balance(arr: Arrangement) -> float:
    """How unevenly the arrangement fills the viewport-sized tiles its bounding box covers.

    The coefficient of variation of the drawn area per tile. A tower leaves most tiles empty and
    scores badly; a block that fills its bounding box evenly scores near zero. One tile is always
    balanced, which is the right answer: it fits on a screen.
    """
    bbox = arr.bbox
    vw, vh = arr.scenario.viewport
    cols = max(1, math.ceil((bbox[2] - 0.01) / vw))
    rows = max(1, math.ceil((bbox[3] - 0.01) / vh))
    if cols * rows <= 1:
        return 0.0
    rects = drawn_rects(arr)
    tiles: list[float] = []
    for row in range(rows):
        for col in range(cols):
            tile = (bbox[0] + col * vw, bbox[1] + row * vh, vw, vh)
            tiles.append(sum(_overlap(tile, r) for r in rects))
    mean = sum(tiles) / len(tiles)
    if mean <= 0.0:
        return 0.0
    var = sum((t - mean) ** 2 for t in tiles) / len(tiles)
    return math.sqrt(var) / mean


def _crosses(p: Sequence[float], q: Sequence[float], r: Sequence[float], s: Sequence[float]) -> bool:
    """Whether `pq` and `rs` cross at a point interior to both. Shared endpoints do not count."""
    if p in (r, s) or q in (r, s):
        return False

    def side(a, b, c) -> float:
        return (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])

    d1, d2 = side(p, q, r), side(p, q, s)
    d3, d4 = side(r, s, p), side(r, s, q)
    return ((d1 > 0) != (d2 > 0)) and ((d3 > 0) != (d4 > 0))


def crossings(arr: Arrangement) -> int:
    segs = [(c.a, c.b) for c in arr.conns]
    n = 0
    for i in range(len(segs)):
        for j in range(i + 1, len(segs)):
            if _crosses(segs[i][0], segs[i][1], segs[j][0], segs[j][1]):
                n += 1
    return n


def _length(a: Sequence[float], b: Sequence[float]) -> float:
    return math.hypot(b[0] - a[0], b[1] - a[1])


def measure(arr: Arrangement) -> dict:
    """Every number, for the caption, the table and `--json`."""
    bbox = arr.bbox
    w, h = bbox[2], bbox[3]
    vw, vh = arr.scenario.viewport
    rects = drawn_rects(arr)
    ink = sum(r[2] * r[3] for r in rects)
    lengths = [_length(c.a, c.b) for c in arr.conns]

    if arr.growth:
        # Grown a block at a time: displacement is what each arrival cost the blocks already
        # there, summed over the growth, which is the number "only adjustments" has to be small in.
        moves = arr.growth["moves"]
        pairs = arr.growth["pairs"]
        displaced = {
            "moved_mean": round(sum(moves) / len(moves), 1) if moves else 0.0,
            "moved_max": round(max(moves), 1) if moves else 0.0,
            "unmoved": round(arr.growth["still"] / pairs, 3) if pairs else 1.0,
            "order_kept": arr.growth["order_kept"],
        }
    else:
        # Laid out from scratch: displacement is how far it threw the arrangement that existed.
        moved = [
            _length(card.at, arr.before[card.agent.id])
            for card in arr.cards
            if card.agent.id in arr.before
        ]
        displaced = {
            "moved_mean": round(sum(moved) / len(moved), 1) if moved else 0.0,
            "moved_max": round(max(moved), 1) if moved else 0.0,
            "unmoved": round(sum(1 for d in moved if d < 0.5) / len(moved), 3) if moved else 1.0,
            "order_kept": reading_order(arr) == arr.before_order,
        }

    return {
        "scenario": arr.scenario.name,
        "algo": arr.algo.key,
        "cards": len(arr.cards),
        "delegates": sum(len(c.subs) for c in arr.cards),
        "containers": sum(1 for b in arr.tasks if b.rect),
        "bbox_w": round(w, 1),
        "bbox_h": round(h, 1),
        "aspect": round(w / h, 3) if h else 0.0,
        "viewport_aspect": round((w / h) / (vw / vh), 3) if h else 0.0,
        "screens_w": round(w / vw, 2),
        "screens_h": round(h / vh, 2),
        "fill": round(ink / (w * h), 3) if w * h else 0.0,
        "balance": round(balance(arr), 3),
        "crossings": crossings(arr),
        "link_max": round(max(lengths), 1) if lengths else 0.0,
        "link_mean": round(sum(lengths) / len(lengths), 1) if lengths else 0.0,
        **displaced,
        "arrivals": arr.passes if arr.growth else 0,
    }


def reading_order(arr: Arrangement) -> list[str]:
    """The containers left-to-right, top-to-bottom — the order the eye takes them in."""
    drawn = [b for b in arr.tasks if b.rect]
    return [b.task.id for b in sorted(drawn, key=lambda b: (round(b.rect[1], 1), round(b.rect[0], 1)))]


COLUMNS = [
    ("scenario", 18),
    ("algo", 9),
    ("bbox_w", 8),
    ("bbox_h", 8),
    ("aspect", 7),
    ("vp_asp", 7),
    ("scr_w", 6),
    ("scr_h", 6),
    ("fill", 6),
    ("balance", 8),
    ("cross", 6),
    ("l_max", 7),
    ("l_mean", 7),
    ("mv_mean", 8),
    ("mv_max", 8),
    ("unmov", 6),
    ("order", 6),
]

_KEYS = {
    "vp_asp": "viewport_aspect",
    "scr_w": "screens_w",
    "scr_h": "screens_h",
    "cross": "crossings",
    "l_max": "link_max",
    "l_mean": "link_mean",
    "mv_mean": "moved_mean",
    "mv_max": "moved_max",
    "unmov": "unmoved",
    "order": "order_kept",
}


def table(rows: Iterable[dict]) -> str:
    out = [" ".join(name.ljust(w) for name, w in COLUMNS).rstrip()]
    out.append(" ".join("-" * w for _, w in COLUMNS))
    for row in rows:
        cells = []
        for name, w in COLUMNS:
            value = row[_KEYS.get(name, name)]
            if isinstance(value, bool):
                value = "yes" if value else "NO"
            cells.append(str(value).ljust(w))
        out.append(" ".join(cells).rstrip())
    return "\n".join(out)
