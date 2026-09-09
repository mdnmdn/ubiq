#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.12"
# dependencies = ["pyyaml", "pillow", "resvg-py", "rich"]
# ///
"""Ubiq's icon set — the mechanical checks and the review sheets.

Run it through `just`: `just icons-check`, `icons-sheet`, `icons-audit`, `icons-dupes`.

The registry is `assets/icons/icons.yaml`; the files are `assets/icons/<name>.svg`. Rendering goes
through resvg — the same rasteriser GPUI uses — and only the alpha channel is kept, because GPUI
draws an icon as a monochrome mask tinted with a theme token. An icon that carries a colour in the
file renders here exactly as wrongly as it will in the app.
"""

from __future__ import annotations

import argparse
import os
import re
import sys
import xml.etree.ElementTree as ET
from dataclasses import dataclass
from io import BytesIO
from pathlib import Path

import resvg_py
import yaml
from PIL import Image, ImageDraw, ImageFont
from rich.console import Console

ROOT = Path(__file__).resolve().parent.parent
ICONS = ROOT / "assets" / "icons"
REGISTRY = ICONS / "icons.yaml"
# Competing takes on an icon that is not settled yet. They live beside the set rather than in it,
# so `check` never sees them; one of them is promoted to `assets/icons/<name>.svg` and the rest
# are deleted the moment a choice is made.
VARIANTS = ICONS / "variants"
# The sheets are written beside the icons rather than under `target/`, so they survive a
# `cargo clean` and are one click away from the file being drawn. Git ignores the directory.
OUT = ICONS / "preview"

SVG_NS = "{http://www.w3.org/2000/svg}"
# The spec, in one place. Lucide's grid, because half the set is Lucide by way of gpui-component.
BOX = "0 0 24 24"
STROKE = "2"
# The envelope every icon's ink has to stay inside — measured off the 101 Lucide files
# gpui-component ships, which is the family a new icon has to sit beside. Their paths keep to
# 2-22 and a 2px stroke puts the ink at 1-23; `github` and `triangle-alert` are the two that
# fill the box, and neither is a shape we would draw.
LIVE = (1.0, 23.0)
SHAPES_MAX = 4
BANNED_TAGS = {
    "text", "image", "use", "defs", "style", "mask", "clipPath", "filter",
    "linearGradient", "radialGradient", "pattern", "script", "animate",
    "animateTransform", "animateMotion", "set", "switch", "foreignObject",
}
BANNED_ATTRS = {"style", "class", "opacity", "fill-opacity", "stroke-opacity"}

# The two palettes, from crates/ubiq/src/theme.rs. Only what a sheet needs: what an icon sits on
# and the three tokens it is ever tinted with.
THEMES = {
    "dark": {"bg": "#121216", "fg": "#e8e8ed", "muted": "#8f8f9a", "accent": "#5b8def"},
    "light": {"bg": "#f8f8fa", "fg": "#1a1a2e", "muted": "#6b6b80", "accent": "#3b6fd4"},
}
SIZES = (16, 24, 64)

# The Lucide files gpui-component ships, so a sheet can stand a new icon beside the borrowed ones
# it has to match — half the set is Lucide, so that is what coherence is judged against. It is a
# cargo checkout, hence the glob; sheets simply leave the borrowed rows out if it is not there.
LUCIDE = next(
    iter(sorted(Path(os.environ.get("CARGO_HOME", Path.home() / ".cargo")).glob(
        "git/checkouts/gpui-component-*/*/crates/assets/assets/icons"))),
    None,
)

console = Console()


@dataclass
class Icon:
    name: str
    goal: str
    category: str
    source: str = "custom"
    keywords: tuple[str, ...] = ()

    @property
    def path(self) -> Path:
        if self.borrowed and LUCIDE:
            return LUCIDE / f"{kebab(self.variant)}.svg"
        return ICONS / f"{self.name}.svg"

    @property
    def borrowed(self) -> bool:
        """Drawn by gpui-component, not by us: there is no file here and nothing to lint."""
        return self.source.startswith("lucide:")

    @property
    def variant(self) -> str:
        """The Lucide variant this row borrows, or the one it was adopted from."""
        return self.source.split(":", 1)[-1] if ":" in self.source else ""


def foreign(icon: Icon | None) -> bool:
    """Geometry we did not choose: a harness's own mark, or an icon adopted from the shipped set."""
    return bool(icon) and (icon.category == "harness" or icon.source.startswith("adopted:"))


def kebab(variant: str) -> str:
    """`SquareTerminal` -> `square-terminal`, `Settings2` -> `settings-2`: the file's own name."""
    return re.sub(r"(?<!^)(?=[A-Z])|(?<=[a-z])(?=\d)", "-", variant).lower()


def registry() -> tuple[dict[str, Icon], list[str]]:
    """Parse `icons.yaml`. An entry's value is either the goal string or a mapping."""
    doc = yaml.safe_load(REGISTRY.read_text()) or {}
    icons: dict[str, Icon] = {}
    for category, entries in (doc.get("categories") or {}).items():
        for name, body in (entries or {}).items():
            if isinstance(body, str):
                body = {"goal": body}
            icons[name] = Icon(
                name=name,
                goal=body.get("goal", ""),
                category=category,
                source=body.get("source", "custom"),
                keywords=tuple(body.get("keywords", ())),
            )
    return icons, list(doc.get("canon") or [])


# --- rendering ------------------------------------------------------------------------------


def mask(svg: str, px: int) -> Image.Image:
    """The alpha mask resvg produces, which is all GPUI keeps of an SVG."""
    png = resvg_py.svg_to_bytes(svg_string=svg, width=px, height=px)
    return Image.open(BytesIO(bytes(png))).convert("RGBA").getchannel("A")


def tinted(svg: str, px: int, fg: str, bg: str) -> Image.Image:
    """The mask painted in `fg` over `bg` — what the app actually draws."""
    tile = Image.new("RGB", (px, px), bg)
    tile.paste(Image.new("RGB", (px, px), fg), (0, 0), mask(svg, px))
    return tile


def font(size: int) -> ImageFont.FreeTypeFont:
    return ImageFont.load_default(size=size)


def normalise(svg: str) -> str:
    """A shipped icon rewritten as one of ours: our root, its shapes, nothing else.

    Lucide's files carry `width`, `height` and a `class`, and the stroke settings live on the
    root. Adopting one keeps the geometry and drops everything the spec forbids, so the copy
    passes `check` exactly as a drawn icon does.
    """
    keep = {"d", "cx", "cy", "r", "rx", "ry", "x", "y", "x1", "x2", "y1", "y2",
            "width", "height", "points", "fill", "fill-rule"}
    shapes = []
    for el in ET.fromstring(svg):
        tag = el.tag.removeprefix(SVG_NS)
        attrs = "".join(f' {k}="{v}"' for k, v in el.items() if k in keep)
        shapes.append(f"<{tag}{attrs}/>")
    return (
        '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none"'
        ' stroke="currentColor" stroke-width="2" stroke-linecap="round"'
        f' stroke-linejoin="round">{"".join(shapes)}</svg>\n'
    )


# --- check ----------------------------------------------------------------------------------


def check(icons: dict[str, Icon]) -> tuple[list[str], list[str]]:
    """The mechanical rules. Everything catchable without looking at a picture.

    Returns violations and, separately, the registry rows still waiting to be drawn — those are
    the worklist, not a failure, or the set could never be added to one category at a time.
    """
    problems: list[str] = []
    on_disk = {p.stem for p in ICONS.glob("*.svg")}
    listed = {n for n, i in icons.items() if not i.borrowed}

    pending = sorted(listed - on_disk)
    for name in sorted(on_disk - listed):
        problems.append(f"I02 {name}.svg: a file nobody registered — add it or delete it")
    for name, icon in sorted(icons.items()):
        if not icon.goal:
            problems.append(f"I03 {name}: no goal — say what the icon has to communicate")
        # gpui-component ships 101 Lucide icons, not all of Lucide. Borrowing one it does not
        # have is the mistake this catches.
        if icon.borrowed and LUCIDE and not icon.path.exists():
            problems.append(f"I11 {name}: gpui-component ships no {icon.path.name} — draw it")

    # Every file on disk, registered or not — an unregistered file is still going to be drawn.
    for path in sorted(ICONS.glob("*.svg")):
        problems += check_one(path.stem, path.read_text(), foreign(icons.get(path.stem)))
    return problems, pending


def check_one(name: str, svg: str, foreign: bool = False) -> list[str]:
    """`foreign` is geometry that is not ours to judge — a harness's own mark, or an icon adopted
    from the set gpui-component ships. The rules about how many shapes it took, where the ink
    sits and how many decimals it carries are dropped for those: `sun` is nine shapes and
    `network` is five, and an adopted icon is coherent by construction because it came from the
    family. Everything GPUI actually needs still applies, because it is drawn as one alpha mask
    like anything else."""
    out: list[str] = []
    try:
        root = ET.fromstring(svg)
    except ET.ParseError as err:
        return [f"I04 {name}: not parseable — {err}"]

    if root.get("viewBox") != BOX:
        out.append(f"I05 {name}: viewBox is {root.get('viewBox')!r}, must be {BOX!r}")
    if root.get("width") or root.get("height"):
        out.append(f"I05 {name}: drop width/height from the root — the element sizes the icon")

    shapes = 0
    for el in root.iter():
        tag = el.tag.removeprefix(SVG_NS)
        if tag in BANNED_TAGS:
            out.append(f"I06 {name}: <{tag}> — GPUI keeps only an alpha mask, this cannot survive")
        if tag != "svg":
            shapes += 1
        for attr in BANNED_ATTRS.intersection(el.keys()):
            out.append(f"I06 {name}: {attr}= on <{tag}> — put it on the element in Rust, not here")
    if not foreign and shapes > SHAPES_MAX:
        out.append(f"I07 {name}: {shapes} shapes, at most {SHAPES_MAX} — simplify")

    for el in root.iter():
        if (w := el.get("stroke-width")) and w != STROKE:
            out.append(f"I08 {name}: stroke-width {w}, must be {STROKE} to sit beside Lucide")
        if (f := el.get("fill")) and f not in ("none", "currentColor"):
            out.append(f"I08 {name}: fill={f!r} — the mask discards it; use none or currentColor")
        if (s := el.get("stroke")) and s != "currentColor":
            out.append(f"I08 {name}: stroke={s!r} — must be currentColor")
        for number in [] if foreign else re.findall(r"-?\d+\.\d{3,}", el.get("d", "")):
            out.append(f"I09 {name}: {number} — two decimals is plenty")
            break

    # Where the ink actually lands, measured off the mask rather than off the path data, so a
    # stroke's own width counts.
    px = 96
    scale = 24 / px
    if box := mask(svg, px).getbbox():
        x0, y0, x1, y1 = (v * scale for v in box)
        if not foreign and (x0 < LIVE[0] or y0 < LIVE[0] or x1 > LIVE[1] or y1 > LIVE[1]):
            out.append(
                f"I10 {name}: ink reaches ({x0:.1f},{y0:.1f})-({x1:.1f},{y1:.1f}), "
                f"must stay inside {LIVE[0]}-{LIVE[1]}"
            )
        if (x1 - x0) < 10 and (y1 - y0) < 10:
            out.append(f"I10 {name}: {x1 - x0:.1f}x{y1 - y0:.1f} — too small, it will read as a dot")
    else:
        out.append(f"I10 {name}: renders empty")
    return out


# --- sheets ---------------------------------------------------------------------------------


def rows(icons: list[Icon], accent: bool) -> Image.Image:
    """One row per icon: every size, in both themes. The 16px column is where icons fail."""
    label_w, pad, gap = 190, 12, 18
    cell = max(SIZES)
    row_h = cell + gap
    block = sum(s + gap for s in SIZES) + gap
    width = label_w + block * 2 + pad * 2
    height = pad * 2 + 26 + row_h * len(icons)

    sheet = Image.new("RGB", (width, height), "#2a2a33")
    draw = ImageDraw.Draw(sheet)
    tint_key = "accent" if accent else "fg"
    for n, (theme, cols) in enumerate(THEMES.items()):
        x = label_w + pad + block * n
        draw.rectangle([x - gap // 2, pad + 22, x + block - gap, height - pad], fill=cols["bg"])
        draw.text((x, pad), f"{theme} / {tint_key}", font=font(12), fill="#8f8f9a")

    for r, icon in enumerate(icons):
        y = pad + 26 + r * row_h
        draw.text((pad, y + cell // 2 - 12), icon.name, font=font(13), fill="#e8e8ed")
        # The default font has no em dash, and a goal line is full of them.
        draw.text((pad, y + cell // 2 + 4), icon.goal.replace("—", "-")[:34],
                  font=font(10), fill="#8f8f9a")
        svg = icon.path.read_text()
        for n, cols in enumerate(THEMES.values()):
            x = label_w + pad + block * n
            for size in SIZES:
                sheet.paste(tinted(svg, size, cols[tint_key], cols["bg"]), (x, y + (cell - size) // 2))
                x += size + gap
    return sheet


def takes(concepts: dict[str, list[Path]], siblings: list[Icon]) -> Image.Image:
    """One row per concept, the takes across it, each at 48 and 16px.

    The row above is the concept's already-settled siblings, so a take is chosen against the
    family rather than against the other takes.
    """
    colours = THEMES["dark"]
    label_w, pad, cw, ch = 150, 14, 96, 82
    columns = max(len(v) for v in concepts.values())
    width = label_w + max(columns, len(siblings)) * cw + pad * 2
    height = pad * 2 + ch * (len(concepts) + bool(siblings))

    sheet = Image.new("RGB", (width, height), colours["bg"])
    draw = ImageDraw.Draw(sheet)
    y = pad
    if siblings:
        draw.text((pad, y + 26), "already settled", font=font(12), fill=colours["accent"])
        for n, icon in enumerate(siblings):
            x = label_w + n * cw
            sheet.paste(tinted(icon.path.read_text(), 48, colours["muted"], colours["bg"]), (x, y))
            draw.text((x, y + 54), icon.name, font=font(10), fill=colours["muted"])
        y += ch
        draw.line([pad, y - 8, width - pad, y - 8], fill="#2a2a33")

    for name, files in concepts.items():
        draw.text((pad, y + 26), name, font=font(13), fill=colours["fg"])
        for n, file in enumerate(sorted(files)):
            x = label_w + n * cw
            svg = file.read_text()
            sheet.paste(tinted(svg, 48, colours["fg"], colours["bg"]), (x, y))
            sheet.paste(tinted(svg, 16, colours["fg"], colours["bg"]), (x + 54, y + 16))
            draw.text((x, y + 54), file.stem.rsplit("-", 1)[-1], font=font(11),
                      fill=colours["accent"])
        y += ch
    return sheet


def zoom(files: list[Path]) -> Image.Image:
    """One concept's takes, large enough to see the joins, with the true sizes under each.

    The row-per-concept sheet is for choosing between takes; this is for looking at one.
    """
    colours = THEMES["dark"]
    big, pad, cw = 128, 16, 156
    sheet = Image.new("RGB", (pad * 2 + cw * len(files), big + 96), colours["bg"])
    draw = ImageDraw.Draw(sheet)
    for n, file in enumerate(sorted(files)):
        x = pad + n * cw
        svg = file.read_text()
        sheet.paste(tinted(svg, big, colours["fg"], colours["bg"]), (x, pad))
        for m, size in enumerate(SIZES):
            sheet.paste(tinted(svg, size, colours["fg"], colours["bg"]),
                        (x + m * 30, pad + big + 14))
        draw.text((x, pad + big + 66), file.stem, font=font(12), fill=colours["accent"])
    return sheet


def grid(icons: list[Icon], theme: str, px: int = 48, cols: int = 6) -> Image.Image:
    """A contact sheet. One question only: which of these breaks the family?"""
    colours = THEMES[theme]
    cw, ch = px + 46, px + 34
    sheet = Image.new("RGB", (cols * cw, -(-len(icons) // cols) * ch), colours["bg"])
    draw = ImageDraw.Draw(sheet)
    for n, icon in enumerate(icons):
        x, y = (n % cols) * cw, (n // cols) * ch
        sheet.paste(tinted(icon.path.read_text(), px, colours["fg"], colours["bg"]),
                    (x + 23, y + 8))
        draw.text((x + cw // 2, y + px + 16), icon.name, font=font(10),
                  fill=colours["muted"], anchor="ma")
    return sheet


def dhash(icon: Icon) -> int:
    """A perceptual hash of the mask, for spotting an icon you have already drawn."""
    small = mask(icon.path.read_text(), 32).resize((9, 8))
    px = small.load()
    bits = 0
    for y in range(8):
        for x in range(8):
            bits = bits << 1 | (px[x, y] > px[x + 1, y])
    return bits


# --- commands -------------------------------------------------------------------------------


def save(image: Image.Image, name: str) -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    out = OUT / name
    image.save(out)
    console.print(f"[green]{out.relative_to(ROOT)}[/] {image.width}x{image.height}")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="cmd", required=True)
    sub.add_parser("check", help="the mechanical rules — registry, spec, ink")
    p = sub.add_parser("sheet", help="a review sheet for a few icons, every size, both themes")
    p.add_argument("names", nargs="*", help="icon names; default is the canon strip")
    p.add_argument("--category", help="the whole category, siblings and all")
    p.add_argument("--accent", action="store_true", help="tint with the accent token instead")
    p = sub.add_parser("adopt", help="copy a shipped icon in-tree as one of ours")
    p.add_argument("names", nargs="*", help="registry names; default is every unfilled adopted: row")
    p = sub.add_parser("variants", help="the competing takes in assets/icons/variants/")
    p.add_argument("--category", help="whose settled siblings to show above them")
    p = sub.add_parser("audit", help="the whole set, one contact sheet per category")
    p.add_argument("--theme", default="dark", choices=list(THEMES))
    sub.add_parser("dupes", help="pairs that look alike")
    args = parser.parse_args()

    icons, canon = registry()

    # Take sheets are scaffolding: once the winner is promoted and the takes deleted, the pictures
    # of them go too. Only the sheets that describe the set itself are kept.
    if not any(VARIANTS.glob("*.svg")):
        for stale in OUT.glob("variants*.png"):
            stale.unlink()

    if args.cmd == "check":
        problems, pending = check(icons)
        for line in problems:
            console.print(line)
        if pending:
            console.print(f"[yellow]to draw ({len(pending)})[/]: {', '.join(pending)}")
        console.print(f"{len(icons)} registered, {len(pending)} to draw, "
                      f"[{'red' if problems else 'green'}]{len(problems)} problem(s)[/]")
        return 1 if problems else 0

    if args.cmd == "adopt":
        if not LUCIDE:
            console.print("[red]no gpui-component checkout found under CARGO_HOME[/]")
            return 1
        names = args.names or [n for n, i in icons.items()
                               if i.source.startswith("adopted:") and not i.path.exists()]
        for name in names:
            icon = icons[name]
            source = LUCIDE / f"{kebab(icon.variant)}.svg"
            if not source.exists():
                console.print(f"[red]{name}: gpui-component ships no {source.name}[/]")
                return 1
            icon.path.write_text(normalise(source.read_text()))
            console.print(f"[green]{icon.path.name}[/] from {source.name}")
        return 0

    drawn = {n: i for n, i in icons.items() if i.path.exists()}

    if args.cmd == "sheet":
        names = (args.names
                 or [n for n, i in drawn.items() if i.category == args.category]
                 or canon or list(drawn))
        chosen = [drawn[n] for n in names if n in drawn]
        if not chosen:
            console.print("[red]nothing to draw[/]")
            return 1
        # The canon strip rides along on every sheet — coherence is judged against a fixed point,
        # never against the previous batch.
        reference = [drawn[n] for n in canon if n in drawn and n not in names]
        save(rows(reference + chosen, args.accent), "sheet.png")
        return 0

    if args.cmd == "variants":
        concepts: dict[str, list[Path]] = {}
        for file in sorted(VARIANTS.glob("*.svg")):
            concepts.setdefault(file.stem.rsplit("-", 1)[0], []).append(file)
        if not concepts:
            console.print(f"[red]no takes in {VARIANTS.relative_to(ROOT)}[/]")
            return 1
        # The spec applies to a take too — no point choosing one that cannot ship.
        problems = [
            p
            for name, files in concepts.items()
            for f in files
            for p in check_one(f.stem, f.read_text(), foreign(icons.get(name)))
        ]
        for line in problems:
            console.print(line)
        siblings = [i for i in drawn.values()
                    if i.category == args.category and i.name not in concepts]
        save(takes(concepts, sorted(siblings, key=lambda i: i.name)), "variants.png")
        for name, files in concepts.items():
            save(zoom(files), f"variants-{name}.png")
        return 1 if problems else 0

    if args.cmd == "audit":
        for category in sorted({i.category for i in drawn.values()}):
            members = sorted((i for i in drawn.values() if i.category == category),
                             key=lambda i: i.name)
            save(grid(members, args.theme), f"audit-{category}.png")
        return 0

    hashes = {n: dhash(i) for n, i in drawn.items()}
    names = sorted(hashes)
    found = False
    for a in range(len(names)):
        for b in range(a + 1, len(names)):
            # Several rows may point at the same shipped icon on purpose — borrowed, adopted, or
            # one of each. That is one icon serving several concepts, not two that look alike.
            first, second = drawn[names[a]], drawn[names[b]]
            if first.path == second.path or (first.variant and first.variant == second.variant):
                continue
            distance = bin(hashes[names[a]] ^ hashes[names[b]]).count("1")
            if distance <= 6:
                found = True
                console.print(f"[yellow]{names[a]} ~ {names[b]}[/] (distance {distance})")
    if not found:
        console.print("[green]no near-duplicates[/]")
    return 0


if __name__ == "__main__":
    sys.exit(main())
