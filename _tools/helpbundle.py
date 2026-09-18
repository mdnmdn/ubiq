#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.12"
# dependencies = ["rich"]
# ///
"""Help-content packer — validates `help/` and writes `target/help/help.bundle`.

Run it through `just`: `just help-check` (validate only), `just help-bundle` (validate, then write),
`just help-clean` (remove `target/help/`). See `_docs/wip/help.md` for the format this content and
this bundle obey; where the two disagree, that document is the source of truth and this script is
what gets fixed.

Three things this script owns and nothing else: reading `help/` (a human-authored markdown tree),
validating it end to end, and writing one `UBIQBND1` archive the same reader in
`crates/ubiq/src/web_export/archive.rs` already opens — deflated entries, a JSON index, a `u64` LE
index offset, the magic. Nothing here talks to a network and nothing here is imported by a crate.

Frontmatter is parsed by a small hand-rolled subset of YAML — strings, `[a, b]` inline lists and
`- a` block lists — rather than pulling in a YAML dependency for ten pages. It is deliberately
strict: anything it does not recognise is a validation error, not a best-effort guess.
"""

from __future__ import annotations

import argparse
import json
import re
import shutil
import struct
import sys
import tomllib
import zlib
from dataclasses import dataclass, field
from pathlib import Path

from rich.console import Console

REPO = Path(__file__).resolve().parents[1]
HELP_DIR = REPO / "help"
MANIFEST_PATH = HELP_DIR / "manifest.toml"
# The default output. `--out-dir` overrides it, because the bundle has to land in the `target/` of
# the workspace that built the binary: Ubiq Studio compiles the base through its own workspace, so
# its `target/help/` is the one the executable walks up to.
OUT_DIR = REPO / "target" / "help"

MAGIC = b"UBIQBND1"

IMAGE_SUFFIXES = {".png", ".jpg", ".jpeg", ".gif", ".svg", ".webp"}

FRONTMATTER_FIELDS = {
    "id",
    "title",
    "summary",
    "keywords",
    "context",
    "order",
    "status",
    "related",
    "redirects",
    "since",
}
LIST_FIELDS = {"keywords", "context", "related", "redirects"}
STATUSES = {"current", "draft", "planned"}

# The four context-key shapes the interface computes: a panel's stable name, a view, a rail mode, a
# modal. This checks the *shape* only — TODO: read `PanelKind::name()`, `View` and `RailMode` out of
# the Rust source to check the name after the dot is a real one, the way the proposal describes; that
# is a known deferred check, not an oversight.
CONTEXT_KEY_RE = re.compile(r"^(panel|view|rail|modal)\.[A-Za-z0-9_.\-]+$")

# A markdown link, `[text](target)`, excluding an image's `![text](target)` — the lookbehind is what
# tells the two apart.
LINK_RE = re.compile(r"(?<!!)\[[^\]]*\]\(([^)]+)\)")
IMAGE_RE = re.compile(r"!\[[^\]]*\]\(([^)]+)\)")

IGNORED_SCHEMES = ("http:", "https:", "mailto:", "ubiq://")

console = Console()


# ── frontmatter ────────────────────────────────────────────────────────────────────────────────


class PageError(Exception):
    """A page-level problem, carrying which page it came from."""


def split_frontmatter(text: str, rel: str) -> tuple[str, str]:
    lines = text.splitlines()
    if not lines or lines[0].strip() != "---":
        raise PageError(f"{rel}: no frontmatter block (must open with '---')")
    for i in range(1, len(lines)):
        if lines[i].strip() == "---":
            front = "\n".join(lines[1:i])
            body = "\n".join(lines[i + 1 :])
            return front, body
    raise PageError(f"{rel}: frontmatter opened with '---' but never closed")


def parse_scalar(raw: str) -> str:
    raw = raw.strip()
    if len(raw) >= 2 and raw[0] == raw[-1] and raw[0] in "\"'":
        return raw[1:-1]
    return raw


def parse_yaml_subset(text: str, rel: str) -> dict:
    """Strings, `[a, b]` inline lists, and `- a` block lists. Nothing else parses."""
    data: dict = {}
    lines = text.splitlines()
    i, n = 0, len(lines)
    while i < n:
        line = lines[i]
        if not line.strip():
            i += 1
            continue
        match = re.match(r"^([A-Za-z_][A-Za-z0-9_]*):\s*(.*)$", line)
        if not match:
            raise PageError(f"{rel}: cannot parse frontmatter line: {line!r}")
        key, rest = match.group(1), match.group(2).strip()
        if key in data:
            raise PageError(f"{rel}: field {key!r} repeated")
        if rest == "":
            items: list[str] = []
            i += 1
            while i < n and re.match(r"^\s*-\s+", lines[i]):
                items.append(parse_scalar(re.sub(r"^\s*-\s+", "", lines[i])))
                i += 1
            data[key] = items
            continue
        if rest.startswith("["):
            if not rest.endswith("]"):
                raise PageError(f"{rel}: unterminated inline list on field {key!r}: {rest!r}")
            inner = rest[1:-1].strip()
            data[key] = [] if not inner else [parse_scalar(part) for part in inner.split(",")]
        else:
            data[key] = parse_scalar(rest)
        i += 1
    return data


def validate_frontmatter_shape(data: dict, rel: str) -> None:
    unknown = set(data) - FRONTMATTER_FIELDS
    if unknown:
        raise PageError(f"{rel}: unknown frontmatter field(s): {sorted(unknown)}")
    for field_name in LIST_FIELDS:
        if field_name in data and not isinstance(data[field_name], list):
            raise PageError(f"{rel}: {field_name!r} must be a list, e.g. `[a, b]` or `- a`")
    for required in ("id", "title", "summary"):
        if not data.get(required):
            raise PageError(f"{rel}: missing required frontmatter field {required!r}")
    if "status" in data and data["status"] not in STATUSES:
        raise PageError(f"{rel}: status {data['status']!r} is not one of {sorted(STATUSES)}")
    if "order" in data:
        try:
            int(data["order"])
        except ValueError:
            raise PageError(f"{rel}: order {data['order']!r} is not an integer") from None


# ── the page model ─────────────────────────────────────────────────────────────────────────────


@dataclass
class Page:
    path: str  # help/-relative, forward-slashed
    id: str = ""
    title: str = ""
    summary: str = ""
    keywords: list[str] = field(default_factory=list)
    context: list[str] = field(default_factory=list)
    order: int | None = None
    status: str = "current"
    related: list[str] = field(default_factory=list)
    redirects: list[str] = field(default_factory=list)
    since: str = ""
    body: str = ""

    @property
    def depth(self) -> int:
        return self.path.count("/")

    def sort_key(self) -> tuple:
        return (0, self.order) if self.order is not None else (1, self.path.lower())


def load_pages(errors: list[str]) -> list[Page]:
    pages: list[Page] = []
    for file in sorted(HELP_DIR.rglob("*.md")):
        rel = file.relative_to(HELP_DIR).as_posix()
        try:
            text = file.read_text(encoding="utf-8")
            front_text, body = split_frontmatter(text, rel)
            data = parse_yaml_subset(front_text, rel)
            validate_frontmatter_shape(data, rel)
        except PageError as error:
            errors.append(str(error))
            continue
        order = int(data["order"]) if "order" in data else None
        pages.append(
            Page(
                path=rel,
                id=data["id"],
                title=data["title"],
                summary=data["summary"],
                keywords=list(data.get("keywords", [])),
                context=list(data.get("context", [])),
                order=order,
                status=data.get("status", "current"),
                related=list(data.get("related", [])),
                redirects=list(data.get("redirects", [])),
                since=data.get("since", ""),
                body=body,
            )
        )
    return pages


# ── validation ─────────────────────────────────────────────────────────────────────────────────


def resolve_link_target(target: str, from_page: Page, by_id: dict[str, Page], by_path: set[str]) -> bool:
    """Whether a bare id, a relative `.md` path, or `<id>#anchor` resolves. `#anchor` is not itself
    checked against real headings — that is the renderer's job at read time, not the packer's."""
    if target.startswith(IGNORED_SCHEMES):
        return True
    head = target.split("#", 1)[0]
    if not head:
        return True  # a same-page `#anchor` link
    if "/" in head or head.endswith(".md"):
        resolved = (Path(from_page.path).parent / head).as_posix()
        resolved = str(Path(resolved).as_posix())
        # Normalise `a/../b` and a leading `./`.
        parts: list[str] = []
        for part in resolved.split("/"):
            if part in ("", "."):
                continue
            if part == "..":
                if parts:
                    parts.pop()
                continue
            parts.append(part)
        return "/".join(parts) in by_path
    return head in by_id


def validate(pages: list[Page], manifest: dict, errors: list[str]) -> dict:
    """Runs every check; returns the resolved context map (key -> page id) for the bundle step."""
    by_id: dict[str, Page] = {}
    for page in pages:
        if page.id in by_id:
            errors.append(f"id {page.id!r} used by both {by_id[page.id].path} and {page.path}")
        else:
            by_id[page.id] = page
    by_path = {page.path for page in pages}

    for page in pages:
        for match in LINK_RE.finditer(page.body):
            target = match.group(1)
            if not resolve_link_target(target, page, by_id, by_path):
                errors.append(f"{page.path}: link target does not resolve: {target!r}")
        for match in IMAGE_RE.finditer(page.body):
            target = match.group(1)
            if target.startswith(IGNORED_SCHEMES):
                continue
            image_path = (Path(page.path).parent / target).as_posix()
            if not (HELP_DIR / image_path).is_file():
                errors.append(f"{page.path}: image does not exist: {target!r} ({image_path})")
        for related_id in page.related:
            if related_id not in by_id:
                errors.append(f"{page.path}: related id does not resolve: {related_id!r}")

    live_ids = set(by_id)
    live_paths = by_path
    for page in pages:
        for redirect in page.redirects:
            if redirect in live_ids or redirect in live_paths:
                errors.append(
                    f"{page.path}: redirect {redirect!r} collides with a live page id or path"
                )

    context_claims: dict[str, list[str]] = {}
    for page in pages:
        for key in page.context:
            if not CONTEXT_KEY_RE.match(key):
                errors.append(
                    f"{page.path}: context key {key!r} does not match "
                    "panel.<name> / view.<name> / rail.<name> / modal.<name>"
                )
            context_claims.setdefault(key, []).append(page.id)

    overrides = manifest.get("context", {})
    for key in overrides:
        if not CONTEXT_KEY_RE.match(key):
            errors.append(f"manifest.toml [context]: key {key!r} does not match the required shape")
        if overrides[key] not in live_ids:
            errors.append(f"manifest.toml [context]: override {key!r} names an unknown id {overrides[key]!r}")

    resolved_context: dict[str, str] = {}
    for key, claimants in context_claims.items():
        if len(claimants) == 1:
            resolved_context[key] = claimants[0]
        elif key in overrides:
            resolved_context[key] = overrides[key]
        else:
            errors.append(f"context key {key!r} claimed by more than one page: {sorted(claimants)}")
    for key, page_id in overrides.items():
        resolved_context.setdefault(key, page_id)

    nav_order = manifest.get("nav", {}).get("order", [])
    reached = expand_nav(nav_order, pages, errors)
    missing = live_ids - {page.id for page in reached}
    for page_id in sorted(missing):
        errors.append(f"page {page_id!r} ({by_id[page_id].path}) is not reachable from [nav] order")

    return resolved_context


def expand_nav(nav_order: list[str], pages: list[Page], errors: list[str]) -> list[Page]:
    """A flat entry names one page by path; a `folder/` entry names every page under it, siblings
    sorted by `order` then alphabetically."""
    by_path = {page.path: page for page in pages}
    out: list[Page] = []
    for entry in nav_order:
        if entry.endswith("/"):
            under = sorted((p for p in pages if p.path.startswith(entry)), key=Page.sort_key)
            if not under:
                errors.append(f"[nav] order names folder {entry!r}, which has no pages")
            out.extend(under)
        elif entry in by_path:
            out.append(by_path[entry])
        else:
            errors.append(f"[nav] order names {entry!r}, which is not a page in help/")
    return out


# ── the manifest ───────────────────────────────────────────────────────────────────────────────


def load_manifest(errors: list[str]) -> dict:
    if not MANIFEST_PATH.is_file():
        errors.append("help/manifest.toml is missing")
        return {}
    try:
        return tomllib.loads(MANIFEST_PATH.read_text(encoding="utf-8"))
    except tomllib.TOMLDecodeError as error:
        errors.append(f"help/manifest.toml: {error}")
        return {}


# ── the bundle ─────────────────────────────────────────────────────────────────────────────────


def deflate_raw(data: bytes) -> bytes:
    compressor = zlib.compressobj(level=9, wbits=-15)
    return compressor.compress(data) + compressor.flush()


def write_bundle(entries: list[tuple[str, bytes]], out_path: Path) -> None:
    """`[ deflate(entry0) ]…[ index JSON ][ u64 LE index offset ][ magic ]` — see the module doc and
    `crates/ubiq/src/web_export/archive.rs`, which reads this format unchanged."""
    out_path.parent.mkdir(parents=True, exist_ok=True)
    index: list[list] = []
    chunks: list[bytes] = []
    offset = 0
    for path, data in entries:
        compressed = deflate_raw(data)
        index.append([path, offset, len(compressed), len(data)])
        chunks.append(compressed)
        offset += len(compressed)
    index_bytes = json.dumps(index).encode("utf-8")
    index_offset = offset
    with out_path.open("wb") as handle:
        for chunk in chunks:
            handle.write(chunk)
        handle.write(index_bytes)
        handle.write(struct.pack("<Q", index_offset))
        handle.write(MAGIC)


def build_catalog(pages: list[Page], manifest: dict, resolved_context: dict[str, str]) -> dict:
    nav_order = manifest.get("nav", {}).get("order", [])
    nav_errors: list[str] = []
    nav_pages = expand_nav(nav_order, pages, nav_errors)

    redirects: dict[str, str] = {}
    for page in pages:
        for target in page.redirects:
            redirects[target] = page.id

    return {
        "version": manifest.get("help", {}).get("version", ""),
        "namespace": manifest.get("help", {}).get("namespace", "ubiq"),
        "pages": [
            {
                "id": page.id,
                "path": page.path,
                "title": page.title,
                "summary": page.summary,
                "keywords": page.keywords,
                "status": page.status,
                "related": page.related,
                "depth": page.depth,
            }
            for page in pages
        ],
        "nav": [page.id for page in nav_pages],
        "context": dict(sorted(resolved_context.items())),
        "redirects": dict(sorted(redirects.items())),
    }


# ── actions ────────────────────────────────────────────────────────────────────────────────────


def run_validate() -> tuple[list[Page], dict, dict, list[str]] | None:
    """Returns `(pages, manifest, resolved_context, errors)`, or `None` when `help/` is absent."""
    if not HELP_DIR.is_dir():
        return None
    errors: list[str] = []
    manifest = load_manifest(errors)
    pages = load_pages(errors)
    resolved_context = validate(pages, manifest, errors)
    return pages, manifest, resolved_context, errors


def report_errors(errors: list[str]) -> None:
    console.print(f"[red]{len(errors)} problem(s):[/red]")
    for error in errors:
        console.print(f"  [red]•[/red] {error}")


def run_check() -> int:
    result = run_validate()
    if result is None:
        console.print("[dim]no help/ tree — nothing to check[/dim]")
        return 0
    _, _, _, errors = result
    if errors:
        report_errors(errors)
        return 1
    console.print("[green]help/ is valid[/green]")
    return 0


def shown(path: Path) -> str:
    """The path as a reader recognises it — relative to the repository when it is inside it."""
    try:
        return str(path.relative_to(REPO))
    except ValueError:
        return str(path)


def run_bundle(out_dir: Path) -> int:
    result = run_validate()
    if result is None:
        console.print("[dim]no help/ tree — nothing to bundle[/dim]")
        return 0
    pages, manifest, resolved_context, errors = result
    if errors:
        report_errors(errors)
        return 1

    catalog = build_catalog(pages, manifest, resolved_context)
    entries: list[tuple[str, bytes]] = []
    for page in pages:
        entries.append((page.path, (HELP_DIR / page.path).read_bytes()))
    for file in sorted((HELP_DIR / "img").rglob("*")) if (HELP_DIR / "img").is_dir() else []:
        if file.is_file() and file.suffix.lower() in IMAGE_SUFFIXES:
            entries.append((file.relative_to(HELP_DIR).as_posix(), file.read_bytes()))
    entries.append(("catalog.json", json.dumps(catalog, indent=2, sort_keys=False).encode("utf-8")))

    bundle_path = out_dir / "help.bundle"
    write_bundle(entries, bundle_path)

    total_in = sum(len(data) for _, data in entries)
    total_out = bundle_path.stat().st_size
    console.print(f"[green]wrote[/green] {shown(bundle_path)}")
    console.print(f"  {len(pages)} pages, {len(entries)} entries")
    console.print(f"  {total_in:,} bytes uncompressed -> {total_out:,} bytes on disk")
    return 0


def run_clean(out_dir: Path) -> int:
    if out_dir.is_dir():
        shutil.rmtree(out_dir)
        console.print(f"[green]removed[/green] {shown(out_dir)}")
    else:
        console.print("[dim]nothing to clean[/dim]")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=["check", "bundle", "clean"])
    parser.add_argument(
        "--out-dir",
        type=Path,
        default=OUT_DIR,
        help="where help.bundle is written (default: the base's own target/help/)",
    )
    args = parser.parse_args()
    out_dir = args.out_dir.resolve()
    match args.action:
        case "check":
            return run_check()
        case "bundle":
            return run_bundle(out_dir)
        case "clean":
            return run_clean(out_dir)
    return 2


if __name__ == "__main__":
    sys.exit(main())
