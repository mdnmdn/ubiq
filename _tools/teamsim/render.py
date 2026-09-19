"""The diagnostic PNG: one arrangement, at true scale, shrunk to fit one look.

Composed straight with Pillow — no SVG, no cairo. Dark on light, because this is a picture an agent
reads numbers off, not product UI. The dashed red rectangle is the scenario's viewport at true
scale: whatever runs outside it is scrolling the user has to do.
"""

from __future__ import annotations

import math
from typing import Optional, Sequence

from PIL import Image, ImageDraw, ImageFont

from algos import (
    CARD_HEIGHT,
    CARD_WIDTH,
    GROUP_LABEL,
    RING_PAD,
    SUB_HEIGHT,
    SUB_WIDTH,
    Arrangement,
    Rect,
)

BG = (246, 246, 244)
INK = (26, 30, 36)
DIM = (104, 114, 126)
SESSION_LINE = (140, 148, 158)
SESSION_FILL = (236, 238, 240)
TASK_LINE = (118, 132, 150)
TASK_FILL = (223, 230, 238)
CARD_FILL = (255, 255, 255)
CARD_LINE = (60, 68, 80)
SUB_FILL = (247, 242, 228)
SUB_LINE = (150, 132, 84)
RING_LINE = (176, 158, 104)
SPAWN = (44, 96, 160)
EXTRA = (176, 78, 40)
VIEWPORT = (198, 44, 44)
CAPTION = 66
PAD = 28.0

_FONTS: dict[int, ImageFont.ImageFont] = {}


def font(size: float) -> ImageFont.ImageFont:
    px = max(6, int(round(size)))
    if px not in _FONTS:
        try:
            _FONTS[px] = ImageFont.load_default(size=px)
        except TypeError:  # Pillow without a sized default font
            _FONTS[px] = ImageFont.load_default()
    return _FONTS[px]


#: Characters the bundled default font has no glyph for, and what to draw instead.
_FOLD = str.maketrans({"—": "-", "–": "-", "’": "'", "“": '"', "”": '"'})


def _clip(draw: ImageDraw.ImageDraw, text: str, size: float, room: float) -> str:
    text = text.translate(_FOLD)
    f = font(size)
    if draw.textlength(text, font=f) <= room:
        return text
    while text and draw.textlength(text + "…", font=f) > room:
        text = text[:-1]
    return text + "…" if text else ""


def _dash(
    draw: ImageDraw.ImageDraw,
    a: tuple[float, float],
    b: tuple[float, float],
    colour: tuple[int, int, int],
    width: int = 1,
    on: float = 9.0,
    off: float = 6.0,
) -> None:
    span = math.hypot(b[0] - a[0], b[1] - a[1])
    if span <= 0:
        return
    ux, uy = (b[0] - a[0]) / span, (b[1] - a[1]) / span
    at = 0.0
    while at < span:
        end = min(at + on, span)
        draw.line(
            [(a[0] + ux * at, a[1] + uy * at), (a[0] + ux * end, a[1] + uy * end)],
            fill=colour,
            width=width,
        )
        at = end + off


def _dash_rect(draw: ImageDraw.ImageDraw, rect: Rect, colour, width: int = 1) -> None:
    x, y, w, h = rect
    corners = [(x, y), (x + w, y), (x + w, y + h), (x, y + h)]
    for ix in range(4):
        _dash(draw, corners[ix], corners[(ix + 1) % 4], colour, width)


def render(arr: Arrangement, stats: dict, max_w: int = 1600, max_h: int = 1180) -> Image.Image:
    """One arrangement as a PNG, captioned with its metrics."""
    bbox = arr.bbox
    vw, vh = arr.scenario.viewport
    wide = max(bbox[2], vw) + PAD * 2.0
    tall = max(bbox[3], vh) + PAD * 2.0
    scale = min(max_w / wide, max_h / tall, 1.0)

    gw, gh = max(1, int(wide * scale)), max(1, int(tall * scale))
    img = Image.new("RGB", (gw, gh + CAPTION), BG)
    draw = ImageDraw.Draw(img)

    def px(x: float) -> float:
        return (x - bbox[0] + PAD) * scale

    def py(y: float) -> float:
        return (y - bbox[1] + PAD) * scale + CAPTION

    def box(rect: Rect) -> list[tuple[float, float]]:
        return [(px(rect[0]), py(rect[1])), (px(rect[0] + rect[2]), py(rect[1] + rect[3]))]

    _caption(draw, arr, stats, gw)
    draw.line([(0, CAPTION - 1), (gw, CAPTION - 1)], fill=SESSION_LINE)

    for session in arr.sessions:
        draw.rectangle(box(session.rect), fill=SESSION_FILL, outline=SESSION_LINE, width=1)
        _text(
            draw,
            session.rect,
            f"{session.session.name}"
            + (f"  ·  {session.session.branch}" if session.session.branch else ""),
            scale,
            px,
            py,
            INK,
            13,
        )

    for held in arr.tasks:
        if not held.rect:
            continue
        draw.rectangle(box(held.rect), fill=TASK_FILL, outline=TASK_LINE, width=1)
        title = held.task.title + (f"  ·  {held.task.shape}" if held.task.shape else "")
        _text(draw, held.rect, title, scale, px, py, INK, 12)

    for conn in arr.conns:
        a, b = (px(conn.a[0]), py(conn.a[1])), (px(conn.b[0]), py(conn.b[1]))
        if conn.kind == "spawn":
            draw.line([a, b], fill=SPAWN, width=max(1, int(2 * scale)))
        else:
            _dash(draw, a, b, EXTRA, max(1, int(2 * scale)))

    for card in arr.cards:
        if card.ring:
            _dash_rect(draw, (px(card.ring[0]), py(card.ring[1]), card.ring[2] * scale, card.ring[3] * scale), RING_LINE)
        for _, name, at in card.subs:
            rect = (at[0], at[1], SUB_WIDTH, SUB_HEIGHT)
            draw.rounded_rectangle(
                box(rect), radius=max(1, int(5 * scale)), fill=SUB_FILL, outline=SUB_LINE, width=1
            )
            _lines(draw, rect, [name, "delegate"], scale, px, py, [INK, DIM], [13, 10])

        rect = (card.at[0], card.at[1], CARD_WIDTH, CARD_HEIGHT)
        draw.rounded_rectangle(
            box(rect), radius=max(1, int(7 * scale)), fill=CARD_FILL, outline=CARD_LINE, width=1
        )
        agent = card.agent
        _lines(
            draw,
            rect,
            [agent.name, agent.role, agent.harness, agent.activity],
            scale,
            px,
            py,
            [INK, DIM, DIM, DIM],
            [14, 11, 10, 10],
        )

    # Last, so nothing draws over it: the viewport at true scale, from the arrangement's corner.
    _dash_rect(draw, (px(bbox[0]), py(bbox[1]), vw * scale, vh * scale), VIEWPORT, 2)
    return img


def _text(draw, rect: Rect, text: str, scale: float, px, py, colour, size: float) -> None:
    """A fence's label, in the room the label band leaves above its contents."""
    px_size = size * scale
    if px_size < 6.5:
        return
    room = rect[2] * scale - 12.0
    draw.text(
        (px(rect[0]) + 6.0, py(rect[1]) + max(1.0, (GROUP_LABEL * scale - px_size) / 2.0)),
        _clip(draw, text, px_size, room),
        fill=colour,
        font=font(px_size),
    )


def _lines(draw, rect: Rect, rows: Sequence[str], scale: float, px, py, colours, sizes) -> None:
    """A card's rows, dropped as the card shrinks rather than overrunning it."""
    y = py(rect[1]) + 5.0 * scale
    room = rect[2] * scale - 12.0
    if room < 34.0:
        return
    for text, colour, size in zip(rows, colours, sizes):
        px_size = size * scale
        if px_size < 5.5 or y + px_size > py(rect[1] + rect[3]) - 2.0:
            return
        if text:
            draw.text(
                (px(rect[0]) + 6.0, y),
                _clip(draw, text, px_size, room),
                fill=colour,
                font=font(px_size),
            )
        y += px_size + 3.0 * scale


def _caption(draw: ImageDraw.ImageDraw, arr: Arrangement, stats: dict, width: int) -> None:
    head = f"{arr.scenario.name}  ·  {arr.algo.label}"
    if arr.growth:
        head += f"  ·  grown over {stats['arrivals']} arrivals"
    draw.text((10, 7), _clip(draw, head, 17, width - 20), fill=INK, font=font(17))
    row = (
        f"bbox {stats['bbox_w']:.0f} x {stats['bbox_h']:.0f}   aspect {stats['aspect']}"
        f"   vs viewport {stats['viewport_aspect']}   screens {stats['screens_w']}w x {stats['screens_h']}h"
        f"   fill {stats['fill']}   balance {stats['balance']}"
    )
    draw.text((10, 28), _clip(draw, row, 12, width - 20), fill=DIM, font=font(12))
    row = (
        f"crossings {stats['crossings']}   link max {stats['link_max']:.0f}"
        f"   mean {stats['link_mean']:.0f}   displaced mean {stats['moved_mean']:.0f}"
        f"   max {stats['moved_max']:.0f}   unmoved {stats['unmoved']}"
        f"   order kept {'yes' if stats['order_kept'] else 'NO'}   ·   {arr.scenario.note}"
    )
    draw.text((10, 45), _clip(draw, row, 12, width - 20), fill=DIM, font=font(12))


def sheet(cells: Sequence[tuple[str, Image.Image]], cols: int, cell_w: int = 620) -> Image.Image:
    """The contact sheet: every cell side by side, which is the image an agent actually looks at."""
    if not cells:
        return Image.new("RGB", (cell_w, 60), BG)
    scaled = []
    for label, img in cells:
        h = max(1, int(img.height * cell_w / img.width))
        scaled.append((label, img.resize((cell_w, h), Image.LANCZOS)))
    rows = math.ceil(len(scaled) / cols)
    row_h = [
        max(img.height for _, img in scaled[r * cols : (r + 1) * cols]) + 22 for r in range(rows)
    ]
    out = Image.new("RGB", (cols * (cell_w + 10) + 10, sum(row_h) + 10), BG)
    draw = ImageDraw.Draw(out)
    y = 10
    for r in range(rows):
        x = 10
        for label, img in scaled[r * cols : (r + 1) * cols]:
            draw.text((x, y), label, fill=INK, font=font(15))
            out.paste(img, (x, y + 18))
            x += cell_w + 10
        y += row_h[r]
    return out
