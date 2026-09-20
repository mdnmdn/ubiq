#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.12"
# dependencies = ["rich"]
# ///
"""Read many files at once — the whole of them, a slice, or just their shape.

Run it through `just`: `just dump <target>…`, `just dump-list <target>…`.

One call prints every file a set of paths, globs or directories names, each under a header saying
where it came from, so exploring a corner of the tree costs one command instead of one per file.
That is the whole point: the caller is usually an agent with a context budget, and twenty reads of
twenty files cost twenty round trips and twenty sets of overhead.

Three things keep it from being `cat` with extra steps.

**A target may carry a line range.** `path:120-180`, `path:120-` and `path:120` are all slices, so a
long file is read where it matters without a second pass to find out where that is.

**It refuses to flood.** Every run has a total-line ceiling (`--max-total`, 6000 by default) and
stops at it, naming what it did not print rather than pretending it printed everything. `--list`
answers what a target would cost before it is paid, and `--outline` answers what is in a file for a
fraction of the lines.

**It skips what is never worth reading.** Build output, the vendor tree, `.git`, the local config
root and anything with a NUL byte in its first few kilobytes are out unless `--all` says otherwise.

Nothing here writes, and nothing here is imported by the crates.
"""

from __future__ import annotations

import argparse
import glob as globlib
import os
import re
import sys
from dataclasses import dataclass
from pathlib import Path

from rich.console import Console
from rich.table import Table

ROOT = Path(__file__).resolve().parent.parent

# Directories never worth reading: build output, the vendored engine, git's own store, the
# repository's local config root, and the Python caches. `--all` turns the list off.
SKIP_DIRS = {
    ".git",
    ".venv",
    "__pycache__",
    "_data",
    "node_modules",
    "target",
    "vendor",
}

# Extensions that are never text, checked before the NUL-byte sniff so a big binary is skipped
# without being opened.
SKIP_SUFFIXES = {
    ".bundle",
    ".icns",
    ".ico",
    ".jpeg",
    ".jpg",
    ".lock",
    ".mp4",
    ".pdf",
    ".png",
    ".so",
    ".ttf",
    ".webp",
    ".woff",
    ".woff2",
    ".zip",
}

# What `--outline` keeps, per language. A pattern matches the whole line, and the line is printed as
# it was written — an outline is a filter over the file, never a rendering of it.
OUTLINE: dict[str, re.Pattern[str]] = {
    # `//!` is kept deliberately: a Ubiq module states what it is in its own header, so the header
    # is the summary an outline would otherwise have to invent. `--max-lines` is the lever for a
    # file whose header is longer than its signatures.
    ".rs": re.compile(
        r"^\s*(?://!|(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?(?:fn|struct|enum|trait|impl|mod|type|const|static|union)\s|macro_rules!)"
    ),
    ".py": re.compile(r"^\s*(?:async\s+def|def|class)\s"),
    ".js": re.compile(r"^\s*(?:export\s+)?(?:async\s+)?(?:function|class|const|let)\s"),
    ".ts": re.compile(
        r"^\s*(?:export\s+)?(?:async\s+)?(?:function|class|const|let|interface|type|enum)\s"
    ),
    ".toml": re.compile(r"^\s*\["),
    ".md": re.compile(r"^#{1,6}\s"),
}
OUTLINE[".tsx"] = OUTLINE[".ts"]
OUTLINE[".jsx"] = OUTLINE[".js"]

RANGE = re.compile(r"^(?P<path>.+?):(?P<start>\d+)(?:-(?P<end>\d*))?$")

console = Console(stderr=True)


@dataclass
class Slice:
    """One file, and which of its lines were asked for. `end` of `None` means to the end."""

    path: Path
    start: int = 1
    end: int | None = None

    @property
    def whole(self) -> bool:
        return self.start == 1 and self.end is None


def parse_target(text: str) -> tuple[str, int, int | None]:
    """Split `path:120-180` into its three parts. A path with no range reads whole.

    The suffix is a range only when it is digits — a colon inside a real filename survives.
    """
    match = RANGE.match(text)
    if not match:
        return text, 1, None
    start = int(match["start"])
    raw_end = match["end"]
    if raw_end is None:
        # `path:120` — one line, which is what a citation of a single line means.
        return match["path"], start, start
    if raw_end == "":
        return match["path"], start, None
    return match["path"], start, int(raw_end)


def skipped(path: Path, keep_all: bool) -> bool:
    if keep_all:
        return False
    if path.suffix.lower() in SKIP_SUFFIXES:
        return True
    return any(part in SKIP_DIRS for part in path.parts)


def is_binary(path: Path) -> bool:
    """A NUL byte in the first 8 KiB. The same sniff `git` uses, and good enough for a dev tool."""
    try:
        with path.open("rb") as handle:
            return b"\0" in handle.read(8192)
    except OSError:
        return True


def resolve(pattern: str) -> list[Path]:
    """Every file one target names, tried against the working directory and then the repository.

    Trying `ROOT` second is what lets a caller write `crates/ubiq/src/ui/*.rs` from anywhere in the
    tree, which is how these paths are written down in `_docs/` and in the code.
    """
    found: list[Path] = []
    for base in (Path.cwd(), ROOT):
        candidate = Path(pattern)
        if not candidate.is_absolute():
            candidate = base / candidate
        if candidate.is_dir():
            found = sorted(p for p in candidate.rglob("*") if p.is_file())
        else:
            found = [Path(p) for p in sorted(globlib.glob(str(candidate), recursive=True))]
            found = [p for p in found if p.is_file()]
        if found:
            break
        if Path(pattern).is_absolute():
            break
    return found


def expand(targets: list[str], excludes: list[str], keep_all: bool) -> tuple[list[Slice], list[str]]:
    """Turn the targets into an ordered, de-duplicated list of slices, plus the ones that matched
    nothing. A file named twice keeps its first range: the second mention is a duplicate, not a
    second reading."""
    slices: list[Slice] = []
    seen: set[Path] = set()
    missed: list[str] = []
    for target in targets:
        pattern, start, end = parse_target(target)
        paths = resolve(pattern)
        if not paths:
            missed.append(target)
            continue
        kept = 0
        for path in paths:
            if skipped(path, keep_all):
                continue
            if any(path.match(rule) or rule in str(path) for rule in excludes):
                continue
            if path in seen:
                continue
            seen.add(path)
            slices.append(Slice(path, start, end))
            kept += 1
        if kept == 0:
            missed.append(target)
    return slices, missed


def read(item: Slice) -> tuple[list[str], int] | None:
    """The asked-for lines and the file's real length, or nothing when it cannot be read as text."""
    if is_binary(item.path):
        return None
    try:
        text = item.path.read_text(encoding="utf-8", errors="replace")
    except OSError as err:
        console.print(f"[red]{shown(item.path)}: {err}[/]")
        return None
    lines = text.splitlines()
    total = len(lines)
    start = max(1, item.start)
    end = total if item.end is None else min(item.end, total)
    return lines[start - 1 : end], total


def plural(count: int, noun: str) -> str:
    return f"{count} {noun}" if count == 1 else f"{count} {noun}s"


def shown(path: Path) -> str:
    """The path as a reader would write it — relative to the repository when it is inside it."""
    try:
        return str(path.resolve().relative_to(ROOT))
    except ValueError:
        return str(path)


def header(item: Slice, kept: int, total: int, outlined: bool) -> str:
    where = shown(item.path)
    if outlined:
        return f"==> {where} (outline, {kept} of {total} lines) <=="
    if item.whole:
        return f"==> {where} ({total} lines) <=="
    end = item.end if item.end is not None else total
    return f"==> {where} (lines {item.start}-{min(end, total)} of {total}) <=="


def emit(
    body: list[tuple[int, str]],
    numbers: bool,
    width: int,
) -> None:
    """Write the lines straight to stdout. Never through `rich`: a console would wrap a long line
    and interpret square brackets as markup, and what this prints is source."""
    out = sys.stdout
    for number, line in body:
        if numbers:
            out.write(f"{number:>{width}}  {line}\n")
        else:
            out.write(line + "\n")


def listing(slices: list[Slice], keep_all: bool) -> int:
    table = Table(box=None, pad_edge=False)
    table.add_column("file")
    table.add_column("lines", justify="right")
    table.add_column("KiB", justify="right")
    total_lines = 0
    total_bytes = 0
    count = 0
    for item in slices:
        result = read(item)
        if result is None:
            if keep_all:
                table.add_row(shown(item.path), "—", "binary")
            continue
        _, total = result
        size = item.path.stat().st_size
        total_lines += total
        total_bytes += size
        count += 1
        table.add_row(shown(item.path), str(total), f"{size / 1024:.1f}")
    console.print(table)
    console.print(
        f"[dim]{plural(count, 'file')} · {plural(total_lines, 'line')} · "
        f"{total_bytes / 1024:.0f} KiB[/]"
    )
    return 0 if count else 1


def main() -> int:
    parser = argparse.ArgumentParser(
        prog="dump.py",
        description="Print many files at once — paths, globs, directories, or `path:start-end`.",
        epilog=(
            "examples:\n"
            "  just dump crates/ubiq/src/ui/kit/'*'.rs\n"
            "  just dump 'crates/ubiq/src/state/**/*.rs' --outline\n"
            "  just dump crates/ubiq/src/state/ui_id.rs:225-300\n"
            "  just dump-list 'crates/ubiq/src/ui/**/*.rs'\n"
        ),
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    parser.add_argument("targets", nargs="+", help="paths, globs, directories, or path:start-end")
    parser.add_argument(
        "-l", "--list", action="store_true", help="what would be printed, and what it costs"
    )
    parser.add_argument(
        "-o", "--outline", action="store_true", help="only the declaration lines, where the suffix is known"
    )
    parser.add_argument("-p", "--plain", action="store_true", help="no line numbers")
    parser.add_argument(
        "-m", "--max-lines", type=int, default=0, metavar="N", help="per-file ceiling (0: none)"
    )
    parser.add_argument(
        "--max-total",
        type=int,
        default=6000,
        metavar="N",
        help="ceiling over the whole run (0: none). Default 6000",
    )
    parser.add_argument(
        "-x", "--exclude", action="append", default=[], metavar="PAT", help="skip paths matching it"
    )
    parser.add_argument(
        "-g", "--grep", metavar="RE", help="only files whose text matches this expression"
    )
    parser.add_argument("-i", "--ignore-case", action="store_true", help="make --grep insensitive")
    parser.add_argument(
        "--all", action="store_true", help="do not skip target/, vendor/, .git/ and the rest"
    )
    args = parser.parse_args()

    slices, missed = expand(args.targets, args.exclude, args.all)
    for target in missed:
        console.print(f"[yellow]no file matched[/] {target}")
    if not slices:
        return 1

    if args.grep:
        pattern = re.compile(args.grep, re.IGNORECASE if args.ignore_case else 0)
        kept = []
        for item in slices:
            result = read(item)
            if result is None:
                continue
            if any(pattern.search(line) for line in result[0]):
                kept.append(item)
        slices = kept
        if not slices:
            console.print(f"[yellow]no file matched[/] /{args.grep}/")
            return 1

    if args.list:
        return listing(slices, args.all)

    printed = 0
    files = 0
    stopped: list[Slice] = []
    for index, item in enumerate(slices):
        if args.max_total and printed >= args.max_total:
            stopped = slices[index:]
            break
        result = read(item)
        if result is None:
            continue
        lines, total = result
        offset = max(1, item.start)
        body = [(offset + n, line) for n, line in enumerate(lines)]

        outlined = False
        if args.outline:
            rule = OUTLINE.get(item.path.suffix.lower())
            if rule is None:
                console.print(f"[yellow]no outline for[/] {shown(item.path)} — printing it whole")
            else:
                body = [(n, line) for n, line in body if rule.match(line)]
                outlined = True

        truncated = 0
        if args.max_lines and len(body) > args.max_lines:
            truncated = len(body) - args.max_lines
            body = body[: args.max_lines]
        if args.max_total:
            room = args.max_total - printed
            if len(body) > room:
                truncated += len(body) - room
                body = body[:room]

        width = len(str(body[-1][0])) if body else 1
        sys.stdout.write(header(item, len(body), total, outlined) + "\n")
        emit(body, not args.plain, width)
        if truncated:
            sys.stdout.write(f"... {truncated} more lines in {shown(item.path)}\n")
        sys.stdout.write("\n")
        printed += len(body)
        files += 1

    if stopped:
        console.print(
            f"[yellow]stopped at --max-total {args.max_total}[/] — "
            f"{len(stopped)} files not printed, first is {shown(stopped[0].path)}. "
            "Narrow the target, or try --list, --outline or a line range"
        )
    console.print(f"[dim]{plural(files, 'file')} · {plural(printed, 'line')}[/]")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except KeyboardInterrupt:
        sys.exit(130)
    except BrokenPipeError:
        # `… | head` closes the pipe. Nothing went wrong and nothing needs saying.
        os._exit(0)
