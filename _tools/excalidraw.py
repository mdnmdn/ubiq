#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = [
#   "pyyaml>=6",
#   "pillow>=10",
# ]
# ///
"""
excalidraw.py — Agent-friendly Excalidraw CLI.

Converts a compact, YAML-based "simple Excalidraw" authoring format to and from
the verbose native ``.excalidraw`` JSON, validates it, and renders SVG/PNG previews.

The native format carries ~25 bookkeeping fields per element (seed, versionNonce,
fractional index, radians, bindings, ...). The simple format only asks for
type/position/size/text/style; this tool generates the rest the way Excalidraw's
importer expects.

Run with uv (auto-installs deps):

    uv run _tools/excalidraw.py <command> [options]

Commands: schema | to-excalidraw | from-excalidraw | to-image | validate
See _docs/simple-excalidraw-spec.md for the full format specification.
"""
from __future__ import annotations

import argparse
import gzip
import json
import math
import re
import sys
import time
from random import Random

try:
    import yaml
except ImportError:  # pragma: no cover - surfaced to user
    sys.stderr.write(
        "error: PyYAML is required. Run via `uv run _tools/excalidraw.py ...` "
        "or `pip install pyyaml`.\n"
    )
    raise SystemExit(2)

# --------------------------------------------------------------------------- #
# Constants & mappings
# --------------------------------------------------------------------------- #

SOURCE = "https://github.com/ubiq/excalidraw-tool"
SPEC_VERSION = 1

# Native element types we emit, keyed by the alias used in the simple format.
TYPE_ALIASES = {
    "rect": "rectangle",
    "rectangle": "rectangle",
    "ellipse": "ellipse",
    "circle": "ellipse",
    "diamond": "diamond",
    "text": "text",
    "arrow": "arrow",
    "line": "line",
    "frame": "frame",
    "draw": "freedraw",
    "freedraw": "freedraw",
}
TYPE_KEYS = set(TYPE_ALIASES)
SHAPE_TYPES = {"rectangle", "ellipse", "diamond"}
CONNECTOR_TYPES = {"arrow", "line"}

# Per-type default size (w, h) when neither `size` nor explicit dims are given.
DEFAULT_SIZE = {
    "rectangle": (180, 80),
    "ellipse": (120, 120),
    "diamond": (160, 90),
    "frame": (800, 500),
    "text": (120, 25),
}

STROKE_WIDTHS = {"thin": 1, "bold": 2, "extra": 4}
FILL_STYLES = {"hachure", "cross-hatch", "solid"}
STROKE_STYLES = {"solid", "dashed", "dotted"}
TEXT_ALIGN = {"left", "center", "right"}
VERTICAL_ALIGN = {"top", "middle", "bottom"}
FONT_FAMILIES = {"hand": 1, "normal": 2, "code": 3}
FONT_FAMILY_NAMES = {1: "hand", 2: "normal", 3: "code"}
ARROWHEADS = {"arrow", "triangle", "dot", "bar", "none"}

# Defaults applied to every element unless overridden by `defaults:` or the element.
ELEMENT_DEFAULTS = {
    "stroke": "#1e1e1e",
    "bg": "transparent",
    "fill": "solid",
    "strokeWidth": "bold",  # -> 2
    "strokeStyle": "solid",
    "roughness": 1,
    "opacity": 100,
    "fontSize": 20,
    "fontFamily": "normal",
    "align": "center",
    "valign": "middle",
}

# Base62 in ASCII-sort order (digits < uppercase < lowercase) for fractional indices.
_B62 = "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz"


# --------------------------------------------------------------------------- #
# Errors
# --------------------------------------------------------------------------- #


class ConversionError(Exception):
    """Raised for malformed simple documents during conversion."""


# --------------------------------------------------------------------------- #
# Small helpers
# --------------------------------------------------------------------------- #


def _now_ms() -> int:
    return int(time.time() * 1000)


def _frac_index(i: int, width: int) -> str:
    """Fixed-width base62 fractional index; lexicographically sortable & unique."""
    digits = []
    for _ in range(width):
        i, r = divmod(i, 62)
        digits.append(_B62[r])
    return "a" + "".join(reversed(digits))


def _frac_width(n: int) -> int:
    width = 2
    while 62 ** width < max(n, 1):
        width += 1
    return width


def _norm_color(value) -> str:
    if value is None:
        return "transparent"
    s = str(value).strip()
    if s.lower() in ("transparent", "none", ""):
        return "transparent"
    if re.fullmatch(r"[0-9a-fA-F]{3}|[0-9a-fA-F]{6}|[0-9a-fA-F]{8}", s):
        return "#" + s
    return s


def _stroke_width(value) -> int:
    if isinstance(value, (int, float)):
        return int(value)
    return STROKE_WIDTHS.get(str(value).lower(), 2)


def _stroke_width_name(value: int) -> str:
    for name, num in STROKE_WIDTHS.items():
        if num == value:
            return name
    return str(value)


def _roundness(value):
    if value in (None, False, "false", "sharp", 0):
        return None
    return {"type": 3}


def _font_family(value) -> int:
    if isinstance(value, int):
        return value
    return FONT_FAMILIES.get(str(value).lower(), 2)


def _deg2rad(value) -> float:
    return float(value or 0) * math.pi / 180.0


def _rad2deg(value) -> float:
    return float(value or 0) * 180.0 / math.pi


def _text_size(text: str, font_size: float) -> tuple[float, float]:
    """Rough metrics good enough for placement/preview (no font loading)."""
    lines = str(text).split("\n")
    width = max((len(line) for line in lines), default=0) * font_size * 0.55
    height = max(len(lines), 1) * font_size * 1.25
    return round(width, 2), round(height, 2)


# --------------------------------------------------------------------------- #
# I/O plumbing (shared by all commands)
# --------------------------------------------------------------------------- #


def read_input(path: str | None, *, binary: bool = False):
    if path:
        mode = "rb" if binary else "r"
        with open(path, mode) as fh:
            return fh.read()
    if sys.stdin.isatty():
        raise ConversionError(
            "no --input given and stdin is a terminal (nothing piped in)"
        )
    return sys.stdin.buffer.read() if binary else sys.stdin.read()


def write_output(path: str | None, data, *, binary: bool = False) -> None:
    if path:
        mode = "wb" if binary else "w"
        with open(path, mode) as fh:
            fh.write(data)
    elif binary:
        sys.stdout.buffer.write(data)
    else:
        sys.stdout.write(data)
        if not data.endswith("\n"):
            sys.stdout.write("\n")


# --------------------------------------------------------------------------- #
# Parsing the simple format into normalized element records
# --------------------------------------------------------------------------- #


def _detect_type(entry: dict) -> tuple[str, str]:
    """Return (type_key, native_type).

    `text` and `frame` are dual-purpose: `text` is both a type and a shape's
    label, and `frame` is both a type and a parent reference. So a "strong" type
    key (rect/ellipse/diamond/arrow/line/draw) always wins; when none is present,
    `text` wins over `frame` (a text element inside a frame is `{text:.., frame:..}`,
    whereas a frame's own label is `name:`, not `text:`)."""
    present = [k for k in entry if k in TYPE_KEYS]
    if not present:
        raise ConversionError(f"element has no type key (one of {sorted(TYPE_KEYS)}): {entry!r}")
    strong = [k for k in present if k not in ("text", "frame")]
    if len(strong) > 1:
        raise ConversionError(f"element has multiple type keys {strong}: {entry!r}")
    if strong:
        key = strong[0]
    elif "text" in present:
        key = "text"
    else:
        key = "frame"
    return key, TYPE_ALIASES[key]


def normalize_element(entry: dict, n: int) -> dict:
    if not isinstance(entry, dict):
        raise ConversionError(f"element must be a mapping, got {type(entry).__name__}: {entry!r}")
    type_key, native = _detect_type(entry)
    props = {k: v for k, v in entry.items() if k != type_key}
    val = entry[type_key]
    if native == "text":
        # For a text element the type-key value is the text *content*; the id is
        # optional via an explicit `id:` prop (text is never a reference target).
        if isinstance(val, dict):
            props = {**val, **props}
        elif val is not None:
            props.setdefault("text", str(val))
        eid = props.pop("id", None)
    elif isinstance(val, dict):
        props = {**val, **props}
        eid = props.pop("id", None)
    elif val is None:
        eid = props.pop("id", None)
    else:
        eid = props.pop("id", str(val))

    parent = props.pop("frame", None) if native != "frame" else None

    # position / size
    at = props.pop("at", None)
    x = props.pop("x", at[0] if at else None)
    y = props.pop("y", at[1] if at else None)
    size = props.pop("size", None)
    w = props.pop("w", props.pop("width", size[0] if size else None))
    h = props.pop("h", props.pop("height", size[1] if size else None))

    return {
        "type": native,
        "id": str(eid) if eid is not None else None,
        "x": None if x is None else float(x),
        "y": None if y is None else float(y),
        "w": None if w is None else float(w),
        "h": None if h is None else float(h),
        "parent": str(parent) if parent is not None else None,
        "props": props,
        "n": n,
    }


# --------------------------------------------------------------------------- #
# to-excalidraw
# --------------------------------------------------------------------------- #


def to_excalidraw(doc: dict, *, rng: Random | None = None) -> dict:
    rng = rng or Random()
    if not isinstance(doc, dict):
        raise ConversionError("top-level document must be a mapping")
    raw_elements = doc.get("elements")
    if not isinstance(raw_elements, list):
        raise ConversionError("document must contain an `elements:` list")

    base_defaults = {**ELEMENT_DEFAULTS, **(doc.get("defaults") or {})}
    layout = doc.get("layout") or {}
    canvas = doc.get("canvas") or {}

    norm = [normalize_element(e, i) for i, e in enumerate(raw_elements)]

    # Assign ids (auto where missing) and check uniqueness.
    used_ids: set[str] = set()
    for i, el in enumerate(norm):
        if el["id"] is None:
            el["id"] = f"el-{i}"
        if el["id"] in used_ids:
            raise ConversionError(f"duplicate element id: {el['id']!r}")
        used_ids.add(el["id"])
    by_id = {el["id"]: el for el in norm}

    _assign_layout(norm, layout)

    # Build native elements. Primary elements keep input order; bound text is
    # inserted right after its container; connectors are appended last.
    out: list[dict] = []
    bound_after: dict[str, list[dict]] = {}
    connectors: list[dict] = []
    group_ids: dict[str, str] = {}
    rand = lambda: rng.randint(1, 2**31 - 1)

    def resolve_groups(value):
        if value is None:
            return []
        names = value if isinstance(value, list) else [value]
        ids = []
        for name in names:
            ids.append(group_ids.setdefault(str(name), f"group-{name}"))
        return ids

    def make_base(el, native, x, y, w, h, props):
        return {
            "id": el["id"],
            "type": native,
            "x": round(x, 2),
            "y": round(y, 2),
            "width": round(w, 2),
            "height": round(h, 2),
            "angle": _deg2rad(props.get("angle", 0)),
            "strokeColor": _norm_color(props.get("stroke", base_defaults["stroke"])),
            "backgroundColor": _norm_color(props.get("bg", base_defaults["bg"])),
            "fillStyle": props.get("fill", base_defaults["fill"]),
            "strokeWidth": _stroke_width(props.get("strokeWidth", base_defaults["strokeWidth"])),
            "strokeStyle": props.get("strokeStyle", base_defaults["strokeStyle"]),
            "roughness": int(props.get("roughness", base_defaults["roughness"])),
            "opacity": int(props.get("opacity", base_defaults["opacity"])),
            "groupIds": resolve_groups(props.get("group")),
            "frameId": el["parent"],
            "roundness": _roundness(props.get("roundness")),
            "seed": rand(),
            "version": 1,
            "versionNonce": rand(),
            "isDeleted": False,
            "boundElements": [],
            "updated": _now_ms(),
            "link": props.get("link"),
            "locked": False,
        }

    for el in norm:
        native = el["type"]
        props = {**base_defaults, **el["props"]}
        if native in CONNECTOR_TYPES:
            connectors.append(el)
            continue

        w = el["w"] if el["w"] is not None else DEFAULT_SIZE.get(native, (120, 60))[0]
        h = el["h"] if el["h"] is not None else DEFAULT_SIZE.get(native, (120, 60))[1]
        x = el["x"] if el["x"] is not None else 0.0
        y = el["y"] if el["y"] is not None else 0.0

        text = props.get("text")

        if native == "text":
            content = "" if text is None else str(text)
            font_size = float(props.get("fontSize", base_defaults["fontSize"]))
            if el["w"] is None or el["h"] is None:
                tw, th = _text_size(content, font_size)
                w = el["w"] if el["w"] is not None else (tw or 10)
                h = el["h"] if el["h"] is not None else th
            base = make_base(el, native, x, y, w, h, props)
            base.update(_text_fields(content, font_size, props, base_defaults, container=None))
            el["_geom"] = (x, y, w, h)
            out.append(base)
            continue

        # shape or frame
        base = make_base(el, native, x, y, w, h, props)
        if native == "frame":
            base["name"] = str(props["name"]) if props.get("name") is not None else None
        el["_geom"] = (x, y, w, h)
        out.append(base)

        # bound text inside shapes/frames
        if text is not None and native != "frame":
            font_size = float(props.get("fontSize", base_defaults["fontSize"]))
            content = str(text)
            tw, th = _text_size(content, font_size)
            tx = x + (w - tw) / 2
            ty = y + (h - th) / 2
            tid = f"{el['id']}-text"
            tprops = {**props, "align": props.get("align", "center"),
                      "valign": props.get("valign", "middle")}
            tbase = {
                "id": tid, "type": "text",
                "x": round(tx, 2), "y": round(ty, 2),
                "width": round(tw, 2), "height": round(th, 2),
                "angle": 0,
                "strokeColor": _norm_color(props.get("textColor", props.get("stroke", base_defaults["stroke"]))),
                "backgroundColor": "transparent",
                "fillStyle": props.get("fill", base_defaults["fill"]),
                "strokeWidth": _stroke_width(props.get("strokeWidth", base_defaults["strokeWidth"])),
                "strokeStyle": "solid", "roughness": int(props.get("roughness", base_defaults["roughness"])),
                "opacity": int(props.get("opacity", base_defaults["opacity"])),
                "groupIds": resolve_groups(props.get("group")),
                "frameId": el["parent"], "roundness": None,
                "seed": rand(), "version": 1, "versionNonce": rand(),
                "isDeleted": False, "boundElements": [], "updated": _now_ms(),
                "link": None, "locked": False,
            }
            tbase.update(_text_fields(content, font_size, tprops, base_defaults, container=el["id"]))
            base["boundElements"].append({"type": "text", "id": tid})
            bound_after[el["id"]] = bound_after.get(el["id"], []) + [tbase]

    # Connectors (need geometry of bound shapes).
    out_connectors = []
    for el in connectors:
        out_connectors.append(_make_connector(el, by_id, base_defaults, rng, resolve_groups))

    # Stitch order: primary + bound text (after container) + connectors.
    ordered: list[dict] = []
    for base in out:
        ordered.append(base)
        for tb in bound_after.get(base["id"], []):
            ordered.append(tb)
    # register bound-arrow references on shapes
    for conn in out_connectors:
        for end in ("startBinding", "endBinding"):
            binding = conn.get(end)
            if binding and binding["elementId"] in by_id:
                target = next((b for b in ordered if b["id"] == binding["elementId"]), None)
                if target is not None:
                    target["boundElements"].append({"type": "arrow", "id": conn["id"]})
    ordered.extend(out_connectors)

    # Fractional indices in final z-order.
    width = _frac_width(len(ordered))
    for i, base in enumerate(ordered):
        base["index"] = _frac_index(i, width)

    return {
        "type": "excalidraw",
        "version": 2,
        "source": SOURCE,
        "elements": ordered,
        "appState": {
            "viewBackgroundColor": _norm_color(canvas.get("background", "#ffffff")),
            "gridSize": canvas.get("gridSize"),
            "theme": canvas.get("theme", "light"),
        },
        "files": {},
    }


def _text_fields(content, font_size, props, base_defaults, container):
    align = props.get("align", base_defaults["align"])
    valign = props.get("valign", base_defaults["valign"])
    if align not in TEXT_ALIGN:
        align = "left"
    if valign not in VERTICAL_ALIGN:
        valign = "top"
    return {
        "text": content,
        "fontSize": float(font_size),
        "fontFamily": _font_family(props.get("fontFamily", base_defaults["fontFamily"])),
        "textAlign": align,
        "verticalAlign": valign,
        "containerId": container,
        "originalText": content,
        "lineHeight": 1.25,
        "autoResize": True,
        "baseline": round(font_size * 0.8, 2),
    }


def _assign_layout(norm: list[dict], layout: dict) -> None:
    """Assign x/y to positionable elements that lack explicit coordinates."""
    mode = (layout.get("mode") or "row").lower()
    gap = float(layout.get("gap", 40))
    start = layout.get("start") or [0, 0]
    cols = int(layout.get("cols", 3))
    cx, cy = float(start[0]), float(start[1])
    row_h = 0.0
    col = 0

    if mode == "none":
        for el in norm:
            if el["type"] in CONNECTOR_TYPES:
                continue
            if el["x"] is None:
                el["x"] = 0.0
            if el["y"] is None:
                el["y"] = 0.0
        return

    for el in norm:
        if el["type"] in CONNECTOR_TYPES:
            continue
        w = el["w"] if el["w"] is not None else DEFAULT_SIZE.get(el["type"], (120, 60))[0]
        h = el["h"] if el["h"] is not None else DEFAULT_SIZE.get(el["type"], (120, 60))[1]
        if el["x"] is not None and el["y"] is not None:
            continue  # explicit position wins
        if mode == "col":
            el["x"], el["y"] = float(start[0]), cy
            cy += h + gap
        elif mode == "grid":
            el["x"] = float(start[0]) + col * (w + gap)
            el["y"] = cy
            row_h = max(row_h, h)
            col += 1
            if col >= cols:
                col = 0
                cy += row_h + gap
                row_h = 0.0
        else:  # row
            el["x"], el["y"] = cx, float(start[1])
            cx += w + gap


def _make_connector(el, by_id, base_defaults, rng, resolve_groups):
    props = {**base_defaults, **el["props"]}
    rand = lambda: rng.randint(1, 2**31 - 1)
    native = el["type"]

    start_binding = None
    end_binding = None
    points_abs = None

    src = props.get("from")
    dst = props.get("to")
    explicit_points = props.get("points")
    explicit_start = props.get("start")
    explicit_end = props.get("end")

    if explicit_points:
        points_abs = [(float(p[0]), float(p[1])) for p in explicit_points]
    elif explicit_start and explicit_end:
        points_abs = [(float(explicit_start[0]), float(explicit_start[1])),
                      (float(explicit_end[0]), float(explicit_end[1]))]
    elif src is not None and dst is not None:
        if src not in by_id or "_geom" not in by_id[src]:
            raise ConversionError(f"arrow `from: {src}` does not reference a known shape")
        if dst not in by_id or "_geom" not in by_id[dst]:
            raise ConversionError(f"arrow `to: {dst}` does not reference a known shape")
        sx, sy = _edge_point(by_id[src]["_geom"], by_id[dst]["_geom"])
        ex, ey = _edge_point(by_id[dst]["_geom"], by_id[src]["_geom"])
        gap = float(props.get("gap", 4))
        dx, dy = ex - sx, ey - sy
        dist = math.hypot(dx, dy) or 1.0
        ux, uy = dx / dist, dy / dist
        sx, sy = sx + ux * gap, sy + uy * gap
        ex, ey = ex - ux * gap, ey - uy * gap
        points_abs = [(sx, sy), (ex, ey)]
        start_binding = {"elementId": src, "focus": 0, "gap": gap}
        end_binding = {"elementId": dst, "focus": 0, "gap": gap}
    else:
        raise ConversionError(
            f"connector {el['id']!r} needs `from`/`to`, `start`/`end`, or `points`"
        )

    ox, oy = points_abs[0]
    rel = [[round(px - ox, 2), round(py - oy, 2)] for px, py in points_abs]
    xs = [p[0] for p in rel]
    ys = [p[1] for p in rel]
    width = max(xs) - min(xs)
    height = max(ys) - min(ys)

    end_head = props.get("endArrowhead", props.get("arrowhead", "arrow" if native == "arrow" else None))
    start_head = props.get("startArrowhead")
    end_head = None if end_head in (None, "none") else end_head
    start_head = None if start_head in (None, "none") else start_head

    return {
        "id": el["id"],
        "type": native,
        "x": round(ox, 2),
        "y": round(oy, 2),
        "width": round(width, 2),
        "height": round(height, 2),
        "angle": 0,
        "strokeColor": _norm_color(props.get("stroke", base_defaults["stroke"])),
        "backgroundColor": _norm_color(props.get("bg", "transparent")),
        "fillStyle": props.get("fill", base_defaults["fill"]),
        "strokeWidth": _stroke_width(props.get("strokeWidth", base_defaults["strokeWidth"])),
        "strokeStyle": props.get("strokeStyle", base_defaults["strokeStyle"]),
        "roughness": int(props.get("roughness", base_defaults["roughness"])),
        "opacity": int(props.get("opacity", base_defaults["opacity"])),
        "groupIds": resolve_groups(props.get("group")),
        "frameId": el["parent"],
        "roundness": _roundness(props.get("roundness", True)),
        "seed": rand(),
        "version": 1,
        "versionNonce": rand(),
        "isDeleted": False,
        "boundElements": [],
        "updated": _now_ms(),
        "link": props.get("link"),
        "locked": False,
        "points": rel,
        "lastCommittedPoint": None,
        "startBinding": start_binding,
        "endBinding": end_binding,
        "startArrowhead": start_head,
        "endArrowhead": end_head,
    }


def _edge_point(geom_from, geom_to):
    """Point on `geom_from`'s boundary in the direction of `geom_to`'s center."""
    fx, fy, fw, fh = geom_from
    tx, ty, tw, th = geom_to
    cx, cy = fx + fw / 2, fy + fh / 2
    dx, dy = (tx + tw / 2) - cx, (ty + th / 2) - cy
    if dx == 0 and dy == 0:
        return cx, cy
    hw, hh = fw / 2, fh / 2
    sx = hw / abs(dx) if dx else math.inf
    sy = hh / abs(dy) if dy else math.inf
    s = min(sx, sy)
    return cx + dx * s, cy + dy * s


# --------------------------------------------------------------------------- #
# from-excalidraw  (lossy / minimal by design)
# --------------------------------------------------------------------------- #

# Native defaults that we drop when emitting the simple format.
_DROP_DEFAULTS = {
    "strokeColor": "#1e1e1e",
    "backgroundColor": "transparent",
    "fillStyle": "solid",
    "strokeWidth": 2,
    "strokeStyle": "solid",
    "roughness": 1,
    "opacity": 100,
}


def from_excalidraw(native: dict) -> dict:
    elements = native.get("elements") or []
    alive = [e for e in elements if not e.get("isDeleted")]
    by_id = {e["id"]: e for e in alive}

    # text folded into containers
    folded: set[str] = set()
    bound_text: dict[str, str] = {}
    for e in alive:
        if e.get("type") == "text" and e.get("containerId"):
            bound_text[e["containerId"]] = e.get("text", "")
            folded.add(e["id"])

    simple_elements = []
    for e in alive:
        if e["id"] in folded:
            continue
        simple_elements.append(_simplify_element(e, by_id, bound_text))

    out = {"version": SPEC_VERSION, "elements": simple_elements}
    app = native.get("appState") or {}
    bg = app.get("viewBackgroundColor")
    canvas = {}
    if bg and bg != "#ffffff":
        canvas["background"] = bg
    if app.get("theme") and app["theme"] != "light":
        canvas["theme"] = app["theme"]
    if canvas:
        out["canvas"] = canvas
    return out


def _simplify_element(e: dict, by_id: dict, bound_text: dict) -> dict:
    native_type = e.get("type")
    alias = {"rectangle": "rect", "freedraw": "draw"}.get(native_type, native_type)
    # text elements carry their content in the type-key value, not the id.
    item: dict = {"text": e.get("text", "")} if native_type == "text" else {alias: e.get("id")}

    if native_type in CONNECTOR_TYPES:
        sb = e.get("startBinding") or {}
        eb = e.get("endBinding") or {}
        if sb.get("elementId") in by_id and eb.get("elementId") in by_id:
            item["from"] = sb["elementId"]
            item["to"] = eb["elementId"]
        else:
            pts = e.get("points") or [[0, 0]]
            ox, oy = e.get("x", 0), e.get("y", 0)
            item["points"] = [[round(ox + p[0], 2), round(oy + p[1], 2)] for p in pts]
        if e.get("endArrowhead") not in (None, "arrow"):
            item["endArrowhead"] = e["endArrowhead"]
        if e.get("startArrowhead"):
            item["startArrowhead"] = e["startArrowhead"]
    else:
        item["at"] = [round(e.get("x", 0), 2), round(e.get("y", 0), 2)]
        item["size"] = [round(e.get("width", 0), 2), round(e.get("height", 0), 2)]

    if native_type == "text":
        if e.get("fontSize") not in (None, 20):
            item["fontSize"] = e["fontSize"]
        fam = e.get("fontFamily")
        if fam and fam in FONT_FAMILY_NAMES and FONT_FAMILY_NAMES[fam] != "normal":
            item["fontFamily"] = FONT_FAMILY_NAMES[fam]
    elif e["id"] in bound_text:
        item["text"] = bound_text[e["id"]]

    if native_type == "frame" and e.get("name"):
        item["name"] = e["name"]

    # styling that differs from defaults
    if _norm_color(e.get("strokeColor")) != _DROP_DEFAULTS["strokeColor"]:
        item["stroke"] = e.get("strokeColor")
    if _norm_color(e.get("backgroundColor")) != _DROP_DEFAULTS["backgroundColor"]:
        item["bg"] = e.get("backgroundColor")
    if e.get("fillStyle") and e["fillStyle"] != _DROP_DEFAULTS["fillStyle"]:
        item["fill"] = e["fillStyle"]
    if e.get("strokeWidth") not in (None, _DROP_DEFAULTS["strokeWidth"]):
        item["strokeWidth"] = _stroke_width_name(e["strokeWidth"])
    if e.get("strokeStyle") and e["strokeStyle"] != _DROP_DEFAULTS["strokeStyle"]:
        item["strokeStyle"] = e["strokeStyle"]
    if e.get("roughness") not in (None, _DROP_DEFAULTS["roughness"]):
        item["roughness"] = e["roughness"]
    if e.get("opacity") not in (None, _DROP_DEFAULTS["opacity"]):
        item["opacity"] = e["opacity"]
    if e.get("roundness"):
        item["roundness"] = True
    if e.get("angle"):
        item["angle"] = round(_rad2deg(e["angle"]), 2)
    if e.get("frameId"):
        item["frame"] = e["frameId"]
    if e.get("groupIds"):
        item["group"] = e["groupIds"][0]
    if e.get("link"):
        item["link"] = e["link"]
    return item


# --------------------------------------------------------------------------- #
# validate
# --------------------------------------------------------------------------- #


def validate(doc) -> list[str]:
    errors: list[str] = []
    if not isinstance(doc, dict):
        return ["top-level document must be a mapping"]
    elements = doc.get("elements")
    if not isinstance(elements, list):
        return ["document must contain an `elements:` list"]

    ids: set[str] = set()
    norm: list[dict] = []
    for i, entry in enumerate(elements):
        try:
            el = normalize_element(entry, i)
        except ConversionError as exc:
            errors.append(f"element[{i}]: {exc}")
            continue
        norm.append(el)
        if el["id"]:
            if el["id"] in ids:
                errors.append(f"element[{i}] ({el['id']}): duplicate id")
            ids.add(el["id"])

    def loc(el):
        return f"element[{el['n']}] ({el['id'] or '?'})"

    for el in norm:
        p = el["props"]
        if p.get("fill") and p["fill"] not in FILL_STYLES:
            errors.append(f"{loc(el)}: invalid fill {p['fill']!r} (use {sorted(FILL_STYLES)})")
        if p.get("strokeStyle") and p["strokeStyle"] not in STROKE_STYLES:
            errors.append(f"{loc(el)}: invalid strokeStyle {p['strokeStyle']!r}")
        if p.get("align") and p["align"] not in TEXT_ALIGN:
            errors.append(f"{loc(el)}: invalid align {p['align']!r}")
        if p.get("roughness") is not None and p["roughness"] not in (0, 1, 2):
            errors.append(f"{loc(el)}: roughness must be 0, 1 or 2")
        for head in ("arrowhead", "startArrowhead", "endArrowhead"):
            if p.get(head) and p[head] not in ARROWHEADS:
                errors.append(f"{loc(el)}: invalid {head} {p[head]!r}")
        if el["parent"] and el["parent"] not in ids:
            errors.append(f"{loc(el)}: frame parent {el['parent']!r} is not a defined element id")
        if el["type"] in CONNECTOR_TYPES:
            for ref in ("from", "to"):
                target = p.get(ref)
                if target is not None and target not in ids:
                    errors.append(f"{loc(el)}: {ref} {target!r} is not a defined element id")
            if not (p.get("from") and p.get("to")) and not p.get("points") and not (p.get("start") and p.get("end")):
                errors.append(f"{loc(el)}: connector needs from/to, start/end, or points")
    return errors


# --------------------------------------------------------------------------- #
# to-image  (native SVG render; PNG via cairosvg)
# --------------------------------------------------------------------------- #


def render_svg(native: dict, *, padding: float = 20.0) -> str:
    elements = [e for e in native.get("elements", []) if not e.get("isDeleted")]
    if not elements:
        return '<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"></svg>'

    minx = min(e["x"] for e in elements)
    miny = min(e["y"] for e in elements)
    maxx = max(e["x"] + e.get("width", 0) for e in elements)
    maxy = max(e["y"] + e.get("height", 0) for e in elements)
    w = (maxx - minx) + 2 * padding
    h = (maxy - miny) + 2 * padding
    ox, oy = padding - minx, padding - miny

    bg = (native.get("appState") or {}).get("viewBackgroundColor", "#ffffff")
    parts = [
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{w:.0f}" height="{h:.0f}" '
        f'viewBox="0 0 {w:.0f} {h:.0f}">',
        f'<rect x="0" y="0" width="{w:.0f}" height="{h:.0f}" fill="{_svg_color(bg, "#ffffff")}"/>',
        '<defs><marker id="arrow" viewBox="0 0 10 10" refX="9" refY="5" '
        'markerWidth="7" markerHeight="7" orient="auto-start-reverse">'
        '<path d="M0,0 L10,5 L0,10 z" fill="context-stroke"/></marker></defs>',
    ]

    # frames and shapes first, then connectors, then text on top
    order = {"frame": 0, "rectangle": 1, "ellipse": 1, "diamond": 1,
             "line": 2, "arrow": 2, "text": 3}
    for e in sorted(elements, key=lambda el: order.get(el["type"], 1)):
        parts.append(_svg_element(e, ox, oy))
    parts.append("</svg>")
    return "\n".join(p for p in parts if p)


def _svg_color(value, fallback="none"):
    c = _norm_color(value)
    return fallback if c == "transparent" else c


def _dash(style):
    return {"dashed": ' stroke-dasharray="10 6"', "dotted": ' stroke-dasharray="2 6"'}.get(style, "")


def _svg_element(e, ox, oy) -> str:
    t = e["type"]
    x, y = e["x"] + ox, e["y"] + oy
    w, h = e.get("width", 0), e.get("height", 0)
    stroke = _svg_color(e.get("strokeColor"), "#1e1e1e")
    fill = _svg_color(e.get("backgroundColor"), "none")
    sw = e.get("strokeWidth", 2)
    dash = _dash(e.get("strokeStyle"))
    opacity = e.get("opacity", 100) / 100.0
    common = f'stroke="{stroke}" stroke-width="{sw}" fill="{fill}" opacity="{opacity}"{dash}'

    if t == "rectangle":
        rx = 12 if e.get("roundness") else 0
        return f'<rect x="{x:.1f}" y="{y:.1f}" width="{w:.1f}" height="{h:.1f}" rx="{rx}" {common}/>'
    if t == "frame":
        return (f'<rect x="{x:.1f}" y="{y:.1f}" width="{w:.1f}" height="{h:.1f}" rx="6" '
                f'stroke="{stroke}" stroke-width="1" fill="none" stroke-dasharray="6 4"/>'
                + (f'<text x="{x:.1f}" y="{y - 4:.1f}" font-family="sans-serif" '
                   f'font-size="14" fill="{stroke}">{_xml(e.get("name") or "")}</text>'
                   if e.get("name") else ""))
    if t == "ellipse":
        return (f'<ellipse cx="{x + w/2:.1f}" cy="{y + h/2:.1f}" rx="{w/2:.1f}" ry="{h/2:.1f}" {common}/>')
    if t == "diamond":
        pts = f"{x + w/2:.1f},{y:.1f} {x + w:.1f},{y + h/2:.1f} {x + w/2:.1f},{y + h:.1f} {x:.1f},{y + h/2:.1f}"
        return f'<polygon points="{pts}" {common}/>'
    if t in ("arrow", "line"):
        pts = e.get("points") or [[0, 0]]
        coords = " ".join(f"{x + p[0]:.1f},{y + p[1]:.1f}" for p in pts)
        marker = ' marker-end="url(#arrow)"' if t == "arrow" and e.get("endArrowhead") else ""
        sm = ' marker-start="url(#arrow)"' if e.get("startArrowhead") else ""
        return (f'<polyline points="{coords}" fill="none" stroke="{stroke}" '
                f'stroke-width="{sw}" opacity="{opacity}"{dash}{marker}{sm}/>')
    if t == "text":
        fs = e.get("fontSize", 20)
        family = {1: "Comic Sans MS, cursive", 2: "Helvetica, sans-serif",
                  3: "Cascadia Code, monospace"}.get(e.get("fontFamily", 2), "sans-serif")
        anchor = {"left": "start", "center": "middle", "right": "end"}.get(e.get("textAlign", "left"), "start")
        ax = {"start": x, "middle": x + w / 2, "end": x + w}[anchor]
        lines = str(e.get("text", "")).split("\n")
        lh = fs * 1.25
        spans = "".join(
            f'<tspan x="{ax:.1f}" y="{y + lh * (i + 0.8):.1f}">{_xml(line)}</tspan>'
            for i, line in enumerate(lines)
        )
        return (f'<text font-family="{family}" font-size="{fs}" fill="{stroke}" '
                f'text-anchor="{anchor}" opacity="{opacity}">{spans}</text>')
    return ""


def _xml(s: str) -> str:
    return (str(s).replace("&", "&amp;").replace("<", "&lt;")
            .replace(">", "&gt;").replace('"', "&quot;"))


def _bounds(elements, padding):
    minx = min(e["x"] for e in elements)
    miny = min(e["y"] for e in elements)
    maxx = max(e["x"] + e.get("width", 0) for e in elements)
    maxy = max(e["y"] + e.get("height", 0) for e in elements)
    return minx, miny, (maxx - minx) + 2 * padding, (maxy - miny) + 2 * padding


def _get_font(size: int):
    from PIL import ImageFont

    for name in ("Helvetica.ttc", "Arial.ttf", "DejaVuSans.ttf",
                 "/System/Library/Fonts/Helvetica.ttc",
                 "/System/Library/Fonts/Supplemental/Arial.ttf"):
        try:
            return ImageFont.truetype(name, size)
        except OSError:
            continue
    try:
        return ImageFont.load_default(size=size)  # Pillow >= 10
    except TypeError:  # pragma: no cover - very old Pillow
        return ImageFont.load_default()


def _dashed_line(draw, p0, p1, fill, width, dash=12, space=8):
    import math as _m

    x0, y0 = p0
    x1, y1 = p1
    total = _m.hypot(x1 - x0, y1 - y0) or 1.0
    ux, uy = (x1 - x0) / total, (y1 - y0) / total
    pos = 0.0
    while pos < total:
        seg = min(dash, total - pos)
        a = (x0 + ux * pos, y0 + uy * pos)
        b = (x0 + ux * (pos + seg), y0 + uy * (pos + seg))
        draw.line([a, b], fill=fill, width=width)
        pos += dash + space


def render_png(native: dict, path: str | None, scale: float, padding: float) -> None:
    """Render directly to PNG with Pillow (clean vector look, no system deps)."""
    from PIL import Image, ImageDraw

    elements = [e for e in native.get("elements", []) if not e.get("isDeleted")]
    if not elements:
        Image.new("RGB", (1, 1), "white").save(_png_target(path), "PNG")
        return

    minx, miny, w, h = _bounds(elements, padding)
    ox, oy = padding - minx, padding - miny
    s = scale

    def X(v):
        return (v + ox) * s

    def Y(v):
        return (v + oy) * s

    bg = (native.get("appState") or {}).get("viewBackgroundColor", "#ffffff")
    img = Image.new("RGB", (max(1, int(w * s)), max(1, int(h * s))),
                    _svg_color(bg, "#ffffff"))
    draw = ImageDraw.Draw(img)

    order = {"frame": 0, "rectangle": 1, "ellipse": 1, "diamond": 1,
             "line": 2, "arrow": 2, "text": 3}
    for e in sorted(elements, key=lambda el: order.get(el["type"], 1)):
        _png_element(draw, e, X, Y, s)

    img.save(_png_target(path), "PNG")


def _png_target(path):
    return path if path else sys.stdout.buffer


def _png_element(draw, e, X, Y, s):
    t = e["type"]
    x0, y0 = X(e["x"]), Y(e["y"])
    x1, y1 = X(e["x"] + e.get("width", 0)), Y(e["y"] + e.get("height", 0))
    stroke = _svg_color(e.get("strokeColor"), "#1e1e1e")
    fill = _svg_color(e.get("backgroundColor"), None)
    fill = None if fill == "none" else fill
    sw = max(1, int(e.get("strokeWidth", 2) * s))
    dashed = e.get("strokeStyle") in ("dashed", "dotted")

    if t == "rectangle":
        if e.get("roundness"):
            draw.rounded_rectangle([x0, y0, x1, y1], radius=12 * s, outline=stroke, width=sw, fill=fill)
        else:
            draw.rectangle([x0, y0, x1, y1], outline=stroke, width=sw, fill=fill)
    elif t == "frame":
        for a, b in (((x0, y0), (x1, y0)), ((x1, y0), (x1, y1)),
                     ((x1, y1), (x0, y1)), ((x0, y1), (x0, y0))):
            _dashed_line(draw, a, b, stroke, max(1, int(s)))
        if e.get("name"):
            draw.text((x0, y0 - 16 * s), str(e["name"]), fill=stroke, font=_get_font(int(14 * s)))
    elif t == "ellipse":
        draw.ellipse([x0, y0, x1, y1], outline=stroke, width=sw, fill=fill)
    elif t == "diamond":
        mx, my = (x0 + x1) / 2, (y0 + y1) / 2
        draw.polygon([(mx, y0), (x1, my), (mx, y1), (x0, my)], outline=stroke, width=sw, fill=fill)
    elif t in ("arrow", "line"):
        pts = [(X(e["x"] + p[0]), Y(e["y"] + p[1])) for p in (e.get("points") or [[0, 0]])]
        for a, b in zip(pts, pts[1:]):
            if dashed:
                _dashed_line(draw, a, b, stroke, sw)
            else:
                draw.line([a, b], fill=stroke, width=sw)
        if t == "arrow" and e.get("endArrowhead") and len(pts) >= 2:
            _arrowhead(draw, pts[-2], pts[-1], stroke, 10 * s)
        if e.get("startArrowhead") and len(pts) >= 2:
            _arrowhead(draw, pts[1], pts[0], stroke, 10 * s)
    elif t == "text":
        font = _get_font(max(1, int(e.get("fontSize", 20) * s)))
        anchor = {"left": "la", "center": "ma", "right": "ra"}.get(e.get("textAlign", "left"), "la")
        ax = {"la": x0, "ma": (x0 + x1) / 2, "ra": x1}[anchor]
        draw.multiline_text((ax, y0), str(e.get("text", "")), fill=stroke, font=font,
                            anchor=anchor, align={"la": "left", "ma": "center", "ra": "right"}[anchor])


def _arrowhead(draw, p_from, p_to, color, size):
    import math as _m

    ang = _m.atan2(p_to[1] - p_from[1], p_to[0] - p_from[0])
    for da in (_m.radians(150), _m.radians(-150)):
        draw.line([p_to, (p_to[0] + size * _m.cos(ang + da), p_to[1] + size * _m.sin(ang + da))],
                  fill=color, width=max(1, int(size / 5)))


# --------------------------------------------------------------------------- #
# JSON Schema for the simple format
# --------------------------------------------------------------------------- #

SIMPLE_SCHEMA = {
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "$id": "https://github.com/ubiq/excalidraw-tool/simple-excalidraw.schema.json",
    "title": "Simple Excalidraw Document",
    "type": "object",
    "required": ["elements"],
    "properties": {
        "version": {"type": "integer", "description": "Simple-format version."},
        "canvas": {
            "type": "object",
            "properties": {
                "background": {"type": "string"},
                "theme": {"enum": ["light", "dark"]},
                "gridSize": {"type": ["integer", "null"]},
            },
            "additionalProperties": False,
        },
        "layout": {
            "type": "object",
            "properties": {
                "mode": {"enum": ["row", "col", "grid", "none"]},
                "gap": {"type": "number"},
                "start": {"type": "array", "items": {"type": "number"}, "minItems": 2, "maxItems": 2},
                "cols": {"type": "integer", "minimum": 1},
            },
            "additionalProperties": False,
        },
        "defaults": {"type": "object", "description": "Style defaults merged into every element."},
        "elements": {
            "type": "array",
            "items": {
                "type": "object",
                "description": "Exactly one type key (rect/ellipse/diamond/text/arrow/line/frame/draw) "
                               "whose value is the element id (string) or an inline props map. "
                               "Common props: at:[x,y], size:[w,h], text, stroke, bg, fill, "
                               "strokeWidth(thin|bold|extra), strokeStyle(solid|dashed|dotted), "
                               "roughness(0|1|2), roundness(bool), opacity, angle(deg), fontSize, "
                               "fontFamily(hand|normal|code), align, valign, group, frame, link. "
                               "Connectors: from/to (bind), or start/end, or points; "
                               "startArrowhead/endArrowhead/arrowhead.",
                "properties": {
                    "rect": {}, "rectangle": {}, "ellipse": {}, "circle": {}, "diamond": {},
                    "text": {}, "arrow": {}, "line": {}, "frame": {}, "draw": {}, "freedraw": {},
                    "id": {"type": "string"},
                    "at": {"type": "array", "items": {"type": "number"}, "minItems": 2, "maxItems": 2},
                    "size": {"type": "array", "items": {"type": "number"}, "minItems": 2, "maxItems": 2},
                    "x": {"type": "number"}, "y": {"type": "number"},
                    "w": {"type": "number"}, "h": {"type": "number"},
                    "width": {"type": "number"}, "height": {"type": "number"},
                    "stroke": {"type": "string"}, "bg": {"type": "string"},
                    "textColor": {"type": "string"},
                    "fill": {"enum": ["solid", "hachure", "cross-hatch"]},
                    "strokeWidth": {"oneOf": [{"type": "number"}, {"enum": ["thin", "bold", "extra"]}]},
                    "strokeStyle": {"enum": ["solid", "dashed", "dotted"]},
                    "roughness": {"enum": [0, 1, 2]},
                    "roundness": {"type": "boolean"},
                    "opacity": {"type": "number", "minimum": 0, "maximum": 100},
                    "angle": {"type": "number"},
                    "fontSize": {"type": "number"},
                    "fontFamily": {"oneOf": [{"type": "integer"}, {"enum": ["hand", "normal", "code"]}]},
                    "align": {"enum": ["left", "center", "right"]},
                    "valign": {"enum": ["top", "middle", "bottom"]},
                    "group": {"type": ["string", "array"]},
                    "frame": {"type": "string"},
                    "name": {"type": "string"},
                    "link": {"type": "string"},
                    "text": {"type": "string"},
                    "from": {"type": "string"}, "to": {"type": "string"},
                    "start": {"type": "array"}, "end": {"type": "array"},
                    "points": {"type": "array"},
                    "gap": {"type": "number"},
                    "arrowhead": {"enum": ["arrow", "triangle", "dot", "bar", "none"]},
                    "startArrowhead": {"enum": ["arrow", "triangle", "dot", "bar", "none"]},
                    "endArrowhead": {"enum": ["arrow", "triangle", "dot", "bar", "none"]},
                },
            },
        },
    },
    "additionalProperties": False,
}


# --------------------------------------------------------------------------- #
# Loaders / dumpers
# --------------------------------------------------------------------------- #


def _load_simple(text: str):
    try:
        return yaml.safe_load(text)
    except yaml.YAMLError as exc:
        raise ConversionError(f"could not parse YAML/JSON input: {exc}")


def _dump_simple(obj) -> str:
    return yaml.safe_dump(obj, sort_keys=False, allow_unicode=True, default_flow_style=False, width=100)


def _load_native(data) -> dict:
    if isinstance(data, (bytes, bytearray)):
        if data[:2] == b"\x1f\x8b":  # gzip magic
            data = gzip.decompress(data)
        data = data.decode("utf-8")
    return json.loads(data)


# --------------------------------------------------------------------------- #
# Command handlers
# --------------------------------------------------------------------------- #


def cmd_schema(args) -> int:
    if args.format == "yaml":
        write_output(args.output, _dump_simple(SIMPLE_SCHEMA))
    else:
        write_output(args.output, json.dumps(SIMPLE_SCHEMA, indent=2))
    return 0


def cmd_to_excalidraw(args) -> int:
    doc = _load_simple(read_input(args.input))
    rng = Random(args.seed) if args.seed is not None else None
    native = to_excalidraw(doc, rng=rng)
    text = json.dumps(native, indent=2)
    if args.compressed:
        write_output(args.output, gzip.compress(text.encode("utf-8")), binary=True)
    else:
        write_output(args.output, text)
    return 0


def cmd_from_excalidraw(args) -> int:
    native = _load_native(read_input(args.input, binary=True))
    simple = from_excalidraw(native)
    write_output(args.output, _dump_simple(simple))
    return 0


def cmd_to_image(args) -> int:
    raw = read_input(args.input, binary=True)
    if raw[:2] == b"\x1f\x8b":  # gzip-compressed native file
        native = _load_native(raw)
    else:
        text = raw.decode("utf-8") if isinstance(raw, (bytes, bytearray)) else raw
        stripped = text.lstrip()
        if stripped.startswith("{") and '"type"' in stripped and "excalidraw" in stripped:
            native = _load_native(raw)
        else:
            rng = Random(args.seed) if args.seed is not None else None
            native = to_excalidraw(_load_simple(text), rng=rng)

    fmt = args.format
    if fmt is None:
        fmt = "png" if (args.output or "").lower().endswith(".png") else "svg"

    if fmt == "svg":
        write_output(args.output, render_svg(native, padding=args.padding))
    else:
        render_png(native, args.output, args.scale, args.padding)
    return 0


def cmd_validate(args) -> int:
    doc = _load_simple(read_input(args.input))
    errors = validate(doc)
    if errors:
        sys.stderr.write("INVALID — {} problem(s):\n".format(len(errors)))
        for err in errors:
            sys.stderr.write(f"  - {err}\n")
        return 1
    n = len(doc.get("elements", [])) if isinstance(doc, dict) else 0
    sys.stderr.write(f"OK — {n} element(s), no problems found.\n")
    return 0


# --------------------------------------------------------------------------- #
# Argument parsing
# --------------------------------------------------------------------------- #


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="excalidraw.py",
        description="Convert a simple YAML diagram format to/from Excalidraw JSON, validate, and render.",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog="If --input is omitted, reads stdin when piped. If --output is omitted, writes stdout.",
    )
    sub = parser.add_subparsers(dest="command", metavar="<command>")

    def add_io(p, *, output=True):
        p.add_argument("--input", "-i", help="input file (default: stdin)")
        if output:
            p.add_argument("--output", "-o", help="output file (default: stdout)")

    p_schema = sub.add_parser("schema", help="print the JSON Schema for the simple format")
    p_schema.add_argument("--format", "-f", choices=["json", "yaml"], default="json")
    p_schema.add_argument("--output", "-o", help="output file (default: stdout)")
    p_schema.set_defaults(func=cmd_schema)

    p_to = sub.add_parser("to-excalidraw", help="simple YAML -> .excalidraw JSON")
    add_io(p_to)
    p_to.add_argument("--compressed", action="store_true", help="gzip the JSON output")
    p_to.add_argument("--seed", type=int, help="deterministic seed for ids/nonces")
    p_to.set_defaults(func=cmd_to_excalidraw)

    p_from = sub.add_parser("from-excalidraw", help=".excalidraw JSON -> simple YAML (lossy)")
    add_io(p_from)
    p_from.set_defaults(func=cmd_from_excalidraw)

    p_img = sub.add_parser("to-image", help="render simple/native input to SVG or PNG")
    add_io(p_img)
    p_img.add_argument("--format", "-f", choices=["svg", "png"], help="output format (default: by extension)")
    p_img.add_argument("--scale", type=float, default=1.0, help="PNG scale factor (default: 1.0)")
    p_img.add_argument("--padding", type=float, default=20.0, help="canvas padding in px (default: 20)")
    p_img.add_argument("--seed", type=int, help="deterministic seed when input is simple format")
    p_img.set_defaults(func=cmd_to_image)

    p_val = sub.add_parser("validate", help="validate a simple-format document")
    p_val.add_argument("--input", "-i", help="input file (default: stdin)")
    p_val.set_defaults(func=cmd_validate)

    return parser


def main(argv=None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    if not getattr(args, "command", None):
        parser.print_help()
        return 0
    try:
        return args.func(args)
    except ConversionError as exc:
        sys.stderr.write(f"error: {exc}\n")
        return 2
    except FileNotFoundError as exc:
        sys.stderr.write(f"error: {exc}\n")
        return 2
    except BrokenPipeError:
        return 0


if __name__ == "__main__":
    raise SystemExit(main())
