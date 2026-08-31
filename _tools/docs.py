#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.12"
# dependencies = ["pyyaml", "rich"]
# ///
"""Documentation maintenance for Ubiq's _docs/ — lint, index, drift, touched, graph.

Run it through `just`: `just docs-lint`, `docs-index`, `docs-drift`, `docs-touched`, `docs-graph`.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
from dataclasses import dataclass, field
from datetime import date, datetime, timezone
from pathlib import Path

import yaml
from rich.console import Console
from rich.markup import escape
from rich.table import Table

REPO = Path(__file__).resolve().parents[1]
DOCS = REPO / "_docs"

# The Cargo workspace. `crates/agent-manager/` carries its own `_docs/`, so its files are
# scanned for symbols and named in prose, but never queued as unanchored here.
SOURCE_ROOTS = ("crates/ubiq/src", "crates/agent-manager/src")
ANCHORABLE_ROOTS = ("crates/ubiq/src",)
# The tree `tech/code-map.md` draws. agent-manager draws its own in `crates/agent-manager/AGENTS.md`.
TREE_ROOTS = ("crates/ubiq/src",)
SOURCE_SUFFIXES = {".rs"}
IGNORED_DIRS = {".git", "target", "refs", "node_modules", ".venv", "__pycache__", ".serena"}

# `_docs/design/` holds wireframes, prototypes and captured artifacts, not documents.
# `tech/ui-and-design.md` is the library's pointer into it.
DOC_EXCLUDE_DIRS = ("design",)

LENGTH_TARGET = (150, 400)
LENGTH_CEILING = 500
LENGTH_FLOOR_WARN = 80
FENCE_DENSITY_MAX = 0.15
FENCE_LINES_MAX = 20

# Read by lookup rather than read through: neither the length band nor the fence caps apply.
# `tech-code-map`'s membership also exempts its generated tree fence from the per-fence cap.
L4_EXEMPT_IDS = {
    "index",
    "backlog",
    "prod-glossary",
    "tech-decisions",
    "tech-code-map",
    "tech-transport",
}

# Append-only ledgers: they grow one row at a time and have no parent to fold into.
LENGTH_EXEMPT_IDS = L4_EXEMPT_IDS | {"meta-feedback", "meta-review-log"}

# Captured artifacts whose value is the verbatim sample: fence caps only, length band still applies.
FENCE_EXEMPT_IDS = L4_EXEMPT_IDS | {"tech-diagrams"}

# Only these prefixes make a backticked token a claim about a file in this repository.
REPO_PATH_PREFIXES = ("crates/", "_tools/")

# `refs/` holds read-only reference checkouts of other projects; their paths are not claims
# about this tree.
EXTERNAL_PATH_MARKERS = ("refs/",)

BANNED_PHRASES = (
    "now",
    "already",
    "no longer",
    "used to",
    "previously",
    "currently",
    "not yet",
    "will be",
    "has been added",
    "was changed",
)
BANNED_PHRASE_EXEMPTIONS = {"tech-decisions": {"used to", "previously", "no longer"}}

# Allowed to discuss time: `wip/` and `inbox/` are dated by nature, and `_meta/` describes a
# process that happens in time — and quotes the banned list verbatim as instruction.
TIMELESSNESS_EXEMPT_DIRS = ("wip", "inbox", "_meta")

WIP_STALE_DAYS = 30
INBOX_STALE_DAYS = 14
MAX_LINK_REPEATS = 3
MAX_INBOUND_EDGES = 5

FENCE_RE = re.compile(r"^(`{3,}|~{3,})(.*)$")
LINK_RE = re.compile(r"(?<!!)\[([^\]]*)\]\(([^)\s]+)(?:\s+\"[^\"]*\")?\)")
HEADING_RE = re.compile(r"^(#{1,6})\s+(.*)$")
CODE_SPAN_RE = re.compile(r"`([^`\n]+)`")
FILE_SPAN_RE = re.compile(r"^[\w./-]+\.(?:rs|toml|md|json|yaml|yml|py|sh|css|html|js|png)$")
SYMBOL_SPAN_RE = re.compile(r"^([A-Za-z_][\w]*)\(\)$")
IDENTIFIER_RE = re.compile(r"[A-Za-z_][\w]*")
MARKER_RE = "<!-- generated:{kind} {name} -->"

console = Console()


# ---------------------------------------------------------------- shared core


@dataclass
class Fence:
    start: int
    lines: int
    info: str


@dataclass
class Doc:
    path: Path
    rel: str
    meta: dict
    lines: list[str]
    prose: list[tuple[int, str]]
    fences: list[Fence]
    headings: list[tuple[int, int, str]]
    links: list[tuple[int, str]]
    file_spans: list[tuple[int, str]]
    symbol_spans: list[tuple[int, str]]
    meta_error: str | None = None

    @property
    def id(self) -> str | None:
        value = self.meta.get("id")
        return str(value) if value else None

    @property
    def folder(self) -> str:
        parts = Path(self.rel).relative_to("_docs").parts
        return parts[0] if len(parts) > 1 else ""

    @property
    def title(self) -> str:
        if self.meta.get("title"):
            return str(self.meta["title"])
        for _, level, text in self.headings:
            if level == 1:
                return text
        return self.path.name

    @property
    def total_lines(self) -> int:
        return len(self.lines)

    @property
    def fence_lines(self) -> int:
        return sum(f.lines for f in self.fences)

    @property
    def anchors(self) -> list[str]:
        raw = self.meta.get("code_anchors") or []
        return [str(a) for a in raw] if isinstance(raw, list) else [str(raw)]

    @property
    def depends_on(self) -> list[str]:
        raw = self.meta.get("depends_on") or []
        return [str(d) for d in raw] if isinstance(raw, list) else [str(raw)]


def split_frontmatter(lines: list[str]) -> tuple[dict, int, str | None]:
    if not lines or lines[0].rstrip() != "---":
        return {}, 0, "no frontmatter block"
    for i in range(1, len(lines)):
        if lines[i].rstrip() == "---":
            raw = "\n".join(lines[1:i])
            try:
                meta = yaml.safe_load(raw) or {}
            except yaml.YAMLError as exc:
                return {}, i + 1, f"frontmatter YAML does not parse: {exc}"
            if not isinstance(meta, dict):
                return {}, i + 1, "frontmatter is not a mapping"
            return meta, i + 1, None
    return {}, 0, "frontmatter block is never closed"


def scan_structure(lines: list[str], skip_to: int) -> tuple[list[tuple[int, str]], list[Fence]]:
    """Split a document into prose lines and fenced blocks, tracking fence state line by line."""
    prose: list[tuple[int, str]] = []
    fences: list[Fence] = []
    open_marker: tuple[str, int] | None = None
    start = 0
    count = 0
    info = ""
    for lineno, raw in enumerate(lines, start=1):
        if lineno <= skip_to:
            continue
        match = FENCE_RE.match(raw.lstrip())
        if open_marker is None:
            if match:
                open_marker = (match.group(1)[0], len(match.group(1)))
                start, count, info = lineno, 0, match.group(2).strip()
            else:
                prose.append((lineno, raw))
            continue
        char, width = open_marker
        closing = (
            match
            and match.group(1)[0] == char
            and len(match.group(1)) >= width
            and not match.group(2).strip()
        )
        if closing:
            fences.append(Fence(start, count, info))
            open_marker = None
        else:
            count += 1
    if open_marker is not None:
        fences.append(Fence(start, count, info))
    return prose, fences


def load_doc(path: Path) -> Doc:
    text = path.read_text(encoding="utf-8")
    lines = text.splitlines()
    meta, fm_end, meta_error = split_frontmatter(lines)
    prose, fences = scan_structure(lines, fm_end)

    headings: list[tuple[int, int, str]] = []
    links: list[tuple[int, str]] = []
    file_spans: list[tuple[int, str]] = []
    symbol_spans: list[tuple[int, str]] = []
    for lineno, raw in prose:
        heading = HEADING_RE.match(raw)
        if heading:
            headings.append((lineno, len(heading.group(1)), heading.group(2).strip()))
        for _, target in LINK_RE.findall(raw):
            links.append((lineno, target))
        for span in CODE_SPAN_RE.findall(raw):
            span = span.strip()
            if FILE_SPAN_RE.match(span):
                file_spans.append((lineno, span))
            symbol = SYMBOL_SPAN_RE.match(span)
            if symbol:
                symbol_spans.append((lineno, symbol.group(1)))

    return Doc(
        path=path,
        rel=str(path.relative_to(REPO)),
        meta=meta,
        lines=lines,
        prose=prose,
        fences=fences,
        headings=headings,
        links=links,
        file_spans=file_spans,
        symbol_spans=symbol_spans,
        meta_error=meta_error,
    )


def load_docs() -> list[Doc]:
    """Every Markdown file under `_docs/` is a document, except the asset subtrees."""
    paths = sorted(
        p
        for p in DOCS.rglob("*.md")
        if not any(d in IGNORED_DIRS for d in p.parts)
        and p.relative_to(DOCS).parts[0] not in DOC_EXCLUDE_DIRS
    )
    return [load_doc(p) for p in paths]


def walk_files(root: Path) -> list[Path]:
    if not root.exists():
        return []
    out = []
    for p in root.rglob("*"):
        if p.is_file() and not any(d in IGNORED_DIRS for d in p.parts):
            out.append(p)
    return sorted(out)


def source_files(roots: tuple[str, ...] = SOURCE_ROOTS) -> list[str]:
    out: list[str] = []
    for root in roots:
        for p in walk_files(REPO / root):
            if p.suffix in SOURCE_SUFFIXES:
                out.append(str(p.relative_to(REPO)))
    return sorted(out)


_tracked: list[str] | None = None


def all_files() -> list[str]:
    global _tracked
    if _tracked is None:
        _tracked = [str(p.relative_to(REPO)) for p in walk_files(REPO)]
    return _tracked


_symbols: set[str] | None = None


def symbol_index() -> set[str]:
    global _symbols
    if _symbols is None:
        found: set[str] = set()
        for rel in source_files():
            text = (REPO / rel).read_text(encoding="utf-8", errors="ignore")
            found.update(IDENTIFIER_RE.findall(text))
        _symbols = found
    return _symbols


def is_repo_path_claim(ref: str) -> bool:
    """A backticked token claims a repo file only when it is rooted in a source folder."""
    if any(marker in ref for marker in EXTERNAL_PATH_MARKERS):
        return False
    return ref.startswith(REPO_PATH_PREFIXES)


def resolve_repo_path(ref: str) -> str | None:
    candidate = (REPO / ref).resolve()
    return ref if candidate.exists() else None


DOC_REFERENCE_ROOTS = ("_docs/",)

# The ledgers record what past passes did, which means naming documents that have since been
# deleted. Resolving those names would force the history to be falsified to stay green.
DOC_HISTORY_DIRS = ("_meta",)

# The link cap stops prose from repeating a link the reader has been given. These documents ARE
# routing tables — every row is a distinct lookup, so repetition is the point.
LINK_REPEAT_EXEMPT_IDS = {"index", "backlog", "tech-code-map"}


def resolve_doc_reference(ref: str) -> str | None:
    """A bare `something.md` names a document, not a repo path — resolve it inside `_docs/`."""
    tail = ref.lstrip("./")
    while tail.startswith("../"):
        tail = tail.removeprefix("../")
    if (REPO / tail).exists():
        return tail
    for rel in all_files():
        if not rel.startswith(DOC_REFERENCE_ROOTS):
            continue
        if rel.endswith(f"/{tail}"):
            return rel
    return None


def resolve_symbol(name: str) -> bool:
    return name in symbol_index()


def git(*args: str) -> str:
    result = subprocess.run(
        ["git", *args], cwd=REPO, capture_output=True, text=True, check=False
    )
    return result.stdout.strip() if result.returncode == 0 else ""


def last_commit_date(paths: list[str]) -> date | None:
    if not paths:
        return None
    out = git("log", "-1", "--format=%cI", "--", *paths)
    if not out:
        return None
    return datetime.fromisoformat(out).date()


def as_date(value) -> date | None:
    if isinstance(value, date):
        return value
    if isinstance(value, datetime):
        return value.date()
    if isinstance(value, str):
        try:
            return date.fromisoformat(value.strip())
        except ValueError:
            return None
    return None


def file_age_days(path: Path) -> int:
    committed = last_commit_date([str(path.relative_to(REPO))])
    if committed:
        return (date.today() - committed).days
    mtime = datetime.fromtimestamp(path.stat().st_mtime, tz=timezone.utc).date()
    return (date.today() - mtime).days


# ---------------------------------------------------------------------- lint


@dataclass
class Finding:
    check: str
    doc: str
    message: str
    line: int | None = None
    severity: str = "fail"
    markup: str | None = None


@dataclass
class LintReport:
    findings: list[Finding] = field(default_factory=list)

    def add(self, *args, **kwargs) -> None:
        self.findings.append(Finding(*args, **kwargs))

    @property
    def failures(self) -> list[Finding]:
        return [f for f in self.findings if f.severity == "fail"]


def check_l1(docs: list[Doc], report: LintReport) -> None:
    seen: dict[str, str] = {}
    known = {d.id for d in docs if d.id}
    for doc in docs:
        if doc.meta_error:
            report.add("L1", doc.rel, doc.meta_error)
            continue
        if not doc.id:
            report.add("L1", doc.rel, "frontmatter has no `id`")
            continue
        if doc.id in seen:
            report.add("L1", doc.rel, f"`id: {doc.id}` is already used by {seen[doc.id]}")
        else:
            seen[doc.id] = doc.rel
        for dep in doc.depends_on:
            if dep not in known:
                report.add("L1", doc.rel, f"`depends_on: {dep}` resolves to no known document")


def check_l2(docs: list[Doc], report: LintReport) -> None:
    for doc in docs:
        here = doc.path.parent
        for anchor in doc.anchors:
            if not (REPO / anchor).exists():
                report.add("L2", doc.rel, f"`code_anchors` names a missing file: {anchor}")
        external = bool(doc.meta.get("external_paths"))
        seen_paths: set[str] = set()
        for lineno, ref in doc.file_spans:
            if ref in seen_paths or external:
                continue
            seen_paths.add(ref)
            if is_repo_path_claim(ref):
                if resolve_repo_path(ref) is None:
                    report.add("L2", doc.rel, f"referenced file does not exist: `{ref}`", lineno)
            elif (
                ref.endswith(".md")
                and doc.folder not in DOC_HISTORY_DIRS
                and not any(m in ref for m in EXTERNAL_PATH_MARKERS)
            ):
                if resolve_doc_reference(ref) is None:
                    report.add("L2", doc.rel, f"referenced document does not exist: `{ref}`", lineno)
        seen_symbols: set[str] = set()
        for lineno, name in doc.symbol_spans:
            if name in seen_symbols:
                continue
            seen_symbols.add(name)
            if not resolve_symbol(name):
                report.add("L2", doc.rel, f"symbol not found in the tree: `{name}()`", lineno)
        seen_links: set[str] = set()
        for lineno, target in doc.links:
            if target.startswith(("http://", "https://", "#", "mailto:")):
                continue
            clean = target.split("#", 1)[0]
            if not clean or clean in seen_links:
                continue
            seen_links.add(clean)
            if not (here / clean).resolve().exists():
                report.add("L2", doc.rel, f"link target does not resolve: `{target}`", lineno)


def check_l4(docs: list[Doc], report: LintReport) -> None:
    for doc in docs:
        total = doc.total_lines
        if doc.id in LENGTH_EXEMPT_IDS:
            pass
        elif total > LENGTH_CEILING:
            report.add("L4", doc.rel, f"{total} lines, over the {LENGTH_CEILING}-line ceiling")
        elif total > LENGTH_TARGET[1]:
            report.add(
                "L4",
                doc.rel,
                f"{total} lines, over the {LENGTH_TARGET[1]}-line target",
                severity="warn",
            )
        if total < LENGTH_FLOOR_WARN and doc.id not in LENGTH_EXEMPT_IDS:
            report.add(
                "L4",
                doc.rel,
                f"{total} lines, under {LENGTH_FLOOR_WARN} — fold it into its parent?",
                severity="warn",
            )
        if doc.id in FENCE_EXEMPT_IDS:
            continue
        if total and doc.fence_lines / total > FENCE_DENSITY_MAX:
            pct = 100 * doc.fence_lines / total
            report.add(
                "L4",
                doc.rel,
                f"fenced code is {pct:.0f}% of the document ({doc.fence_lines}/{total} lines), "
                f"over {FENCE_DENSITY_MAX:.0%}",
            )
        for fence in doc.fences:
            if fence.lines > FENCE_LINES_MAX:
                label = f" ({fence.info})" if fence.info else ""
                report.add(
                    "L4",
                    doc.rel,
                    f"fence{label} is {fence.lines} lines, over {FENCE_LINES_MAX}",
                    fence.start,
                )


def banned_pattern(phrase: str) -> re.Pattern:
    return re.compile(r"\b" + r"\s+".join(re.escape(w) for w in phrase.split()) + r"\b", re.I)


_BANNED = [(p, banned_pattern(p)) for p in BANNED_PHRASES]


def check_l5(docs: list[Doc], report: LintReport) -> None:
    for doc in docs:
        if doc.meta.get("status") != "current":
            continue
        if doc.folder in TIMELESSNESS_EXEMPT_DIRS:
            continue
        exempt = BANNED_PHRASE_EXEMPTIONS.get(doc.id or "", set())
        for lineno, raw in doc.prose:
            # An inline code span may legitimately hold `Date.now()`; judge prose only.
            text = CODE_SPAN_RE.sub(lambda m: " " * (len(m.group(0))), raw)
            for phrase, pattern in _BANNED:
                if phrase in exempt:
                    continue
                match = pattern.search(text)
                if not match:
                    continue
                start, end = match.span()
                highlighted = (
                    escape(raw[:start].strip())
                    + f" [bold red]{escape(raw[start:end])}[/bold red] "
                    + escape(raw[end:].strip())
                )
                report.add(
                    "L5",
                    doc.rel,
                    f'banned phrasing "{phrase}"',
                    lineno,
                    markup=highlighted.strip(),
                )


def check_l7(docs: list[Doc], report: LintReport) -> None:
    index = DOCS / "INDEX.md"
    linked: set[str] = set()
    if index.exists():
        index_doc = next((d for d in docs if d.path == index), None)
        if index_doc:
            for _, target in index_doc.links:
                clean = target.split("#", 1)[0]
                if clean.endswith(".md"):
                    linked.add(str((index.parent / clean).resolve()))
    for doc in docs:
        if doc.path == index:
            continue
        if str(doc.path.resolve()) not in linked:
            report.add("L7", doc.rel, "orphan: not linked from `INDEX.md`")
        if doc.folder == "wip":
            updated = as_date(doc.meta.get("updated"))
            if updated is None:
                report.add("L7", doc.rel, "`wip/` document has no usable `updated` date")
            elif (age := (date.today() - updated).days) > WIP_STALE_DAYS:
                report.add("L7", doc.rel, f"`wip/` document last updated {age} days ago")
    inbox = DOCS / "inbox"
    for path in walk_files(inbox):
        if path.name == ".gitkeep":
            continue
        age = file_age_days(path)
        if age > INBOX_STALE_DAYS:
            report.add(
                "L7", str(path.relative_to(REPO)), f"`inbox/` file is {age} days old — file it"
            )


def check_l9(docs: list[Doc], report: LintReport) -> None:
    for doc in docs:
        if doc.meta.get("id") in LINK_REPEAT_EXEMPT_IDS:
            continue
        counts: dict[str, int] = {}
        for _, target in doc.links:
            key = target.split("#", 1)[0] or target
            counts[key] = counts.get(key, 0) + 1
        for target, count in sorted(counts.items()):
            if count > MAX_LINK_REPEATS:
                report.add("L9", doc.rel, f"link `{target}` appears {count} times (max 3)")


def check_l10(report: LintReport) -> None:
    agents = REPO / "AGENTS.md"
    index = DOCS / "INDEX.md"
    agents_text = agents.read_text(encoding="utf-8") if agents.exists() else ""
    index_text = index.read_text(encoding="utf-8") if index.exists() else ""
    duty = re.compile(r"same\s+commit", re.I)
    if "INDEX.md" not in agents_text:
        report.add("L10", "AGENTS.md", "does not mention `INDEX.md`")
    if "_docs/_meta/authoring.md" not in agents_text:
        report.add("L10", "AGENTS.md", "does not point at `_docs/_meta/authoring.md`")
    if not duty.search(agents_text):
        report.add("L10", "AGENTS.md", "does not state the same-commit update duty")
    if "authoring.md" not in index_text:
        report.add("L10", "_docs/INDEX.md", "does not point at `authoring.md`")


def run_lint(paths: list[str], as_json: bool) -> int:
    docs = load_docs()
    report = LintReport()
    check_l10(report)
    check_l1(docs, report)

    selected = docs
    if paths:
        wanted = {str((Path.cwd() / p).resolve()) for p in paths}
        selected = [d for d in docs if str(d.path.resolve()) in wanted]
        if not selected:
            console.print("[yellow]no document matched the given paths; linting all[/yellow]")
            selected = docs
    check_l2(selected, report)
    check_l4(selected, report)
    check_l5(selected, report)
    check_l7(selected, report)
    check_l9(selected, report)

    if as_json:
        print(
            json.dumps(
                {
                    "failures": len(report.failures),
                    "findings": [
                        {
                            "check": f.check,
                            "doc": f.doc,
                            "line": f.line,
                            "severity": f.severity,
                            "message": f.message,
                        }
                        for f in report.findings
                    ],
                },
                indent=2,
            )
        )
        return 1 if report.failures else 0

    l10 = [f for f in report.findings if f.check == "L10"]
    if l10:
        console.rule("[bold red]L10 — the update duty is not discoverable")
        console.print(
            "[bold red]The library has no upkeep channel. Restore these before anything else.[/bold red]"
        )
        for f in l10:
            console.print(f"  [red]{escape(f.doc)}[/red] — {escape(f.message)}")
        console.print()
    else:
        console.print("[green]L10 ok[/green] — the update duty is discoverable\n")

    by_doc: dict[str, list[Finding]] = {}
    for f in report.findings:
        if f.check == "L10":
            continue
        by_doc.setdefault(f.doc, []).append(f)

    for doc in sorted(by_doc):
        console.rule(f"[bold]{escape(doc)}", align="left")
        for f in sorted(by_doc[doc], key=lambda f: (f.check, f.line or 0)):
            colour = "red" if f.severity == "fail" else "yellow"
            where = f":{f.line}" if f.line else ""
            console.print(
                f"  [{colour}]{f.check}[/{colour}] {escape(doc)}{where} — {escape(f.message)}"
            )
            if f.markup:
                console.print(f"        {f.markup}", highlight=False)
        console.print()

    counts: dict[str, list[int]] = {}
    for f in report.findings:
        slot = counts.setdefault(f.check, [0, 0])
        slot[0 if f.severity == "fail" else 1] += 1
    table = Table(title="Summary", title_justify="left")
    table.add_column("Check")
    table.add_column("Failures", justify="right")
    table.add_column("Warnings", justify="right")
    for check in sorted(counts, key=lambda c: int(c[1:])):
        table.add_row(check, str(counts[check][0]), str(counts[check][1]))
    table.add_section()
    table.add_row(
        "total",
        str(sum(c[0] for c in counts.values())),
        str(sum(c[1] for c in counts.values())),
    )
    console.print(table)
    console.print(f"{len(docs)} documents scanned")
    return 1 if report.failures else 0


# --------------------------------------------------------------------- index

GROUPS = (
    ("product", "Product"),
    ("features", "Features"),
    ("tech", "Tech"),
    ("_meta", "Meta"),
    ("wip", "Work in progress"),
    ("", "Unclassified"),
)
GROUP_BY_KIND = {"product": "product", "feature": "features", "tech": "tech", "meta": "_meta"}


def read_marker(text: str, name: str) -> tuple[int, int] | None:
    begin = MARKER_RE.format(kind="begin", name=name)
    end = MARKER_RE.format(kind="end", name=name)
    i, j = text.find(begin), text.find(end)
    if i < 0 or j < 0 or j < i:
        return None
    return i + len(begin), j


def replace_marker(text: str, name: str, body: str) -> str | None:
    span = read_marker(text, name)
    if span is None:
        return None
    start, end = span
    return text[:start] + "\n\n" + body.strip() + "\n\n" + text[end:]


def doc_group(doc: Doc) -> str:
    if doc.folder:
        return doc.folder
    return GROUP_BY_KIND.get(str(doc.meta.get("kind") or ""), "")


def link_from(base: Path, target: Path) -> str:
    out = os.path.relpath(target, base.parent)
    return out if out.startswith(".") else f"./{out}"


def doc_label(base: Path, target: Path) -> str:
    if target.parent == base.parent:
        return target.name
    return str(target.relative_to(DOCS))


def render_catalogue(docs: list[Doc]) -> str:
    index = DOCS / "INDEX.md"
    out: list[str] = []
    for folder, heading in GROUPS:
        group = [
            d
            for d in docs
            if doc_group(d) == folder and d.path != index and d.folder != "inbox"
        ]
        if not group:
            continue
        out.append(f"### {heading}\n")
        out.append("| Document | What it is | Verified |")
        out.append("|---|---|---|")
        for doc in sorted(group, key=lambda d: d.rel):
            href = link_from(index, doc.path)
            summary = str(doc.meta.get("summary") or "").strip().replace("\n", " ") or "—"
            verified = as_date(doc.meta.get("verified"))
            out.append(
                f"| [{doc.title}]({href}) | {summary} | {verified.isoformat() if verified else '—'} |"
            )
        out.append("")
    return "\n".join(out)


def tree_entries(roots: tuple[str, ...]) -> dict[str, list[tuple[str, bool]]]:
    children: dict[str, list[tuple[str, bool]]] = {}
    for root in roots:
        base = REPO / root
        if not base.exists():
            continue
        for path in sorted(base.rglob("*")):
            if any(d in IGNORED_DIRS for d in path.parts):
                continue
            rel = str(path.relative_to(REPO))
            parent = str(Path(rel).parent)
            children.setdefault(parent, []).append((rel, path.is_dir()))
    for entries in children.values():
        entries.sort(key=lambda e: (not e[1], e[0]))
    return children


TREE_LINE_RE = re.compile(r"^((?:(?:│   |    ))*)(├── |└── )(.+)$")


def parse_tree(block: str) -> tuple[dict[str, str], dict[str, int]]:
    """Read the existing tree for its hand-written descriptions and its entry order.

    Both are unrecoverable from the filesystem — the descriptions are written by hand, and the
    order encodes the layering (`core` before `routes`) rather than the alphabet. Regeneration
    reproduces them so an unchanged tree yields no diff.
    """
    notes: dict[str, str] = {}
    order: dict[str, int] = {}
    stack: list[str] = []
    for raw in block.splitlines():
        line = raw.rstrip()
        if not line or line.startswith("```"):
            continue
        match = TREE_LINE_RE.match(line)
        if match:
            depth = len(match.group(1)) // 4
            rest = match.group(3)
        elif line.endswith("/") or re.match(r"^[\w./-]+/?\s*$", line):
            depth, rest = -1, line
        else:
            continue
        parts = re.split(r"\s{2,}", rest.strip(), maxsplit=1)
        name = parts[0].rstrip("/")
        note = parts[1].strip() if len(parts) > 1 else ""
        stack = stack[: depth + 1]
        while len(stack) < depth + 1:
            stack.append("")
        stack.append(name)
        key = "/".join(p for p in stack if p)
        order.setdefault(key, len(order))
        if note:
            notes[key] = note
    return notes, order


def render_tree(existing: str) -> str:
    notes, order = parse_tree(existing)
    children = tree_entries(TREE_ROOTS)
    rows: list[tuple[str, str]] = []

    def walk(parent: str, prefix: str) -> None:
        entries = sorted(
            children.get(parent, []),
            key=lambda e: (order.get(e[0], len(order)), not e[1], e[0]),
        )
        for i, (rel, is_dir) in enumerate(entries):
            last = i == len(entries) - 1
            glyph = "└── " if last else "├── "
            name = Path(rel).name + ("/" if is_dir else "")
            rows.append((prefix + glyph + name, notes.get(rel, "")))
            if is_dir:
                walk(rel, prefix + ("    " if last else "│   "))

    for root in TREE_ROOTS:
        if not (REPO / root).exists():
            continue
        if rows:
            rows.append(("", ""))
        rows.append((f"{root}/", notes.get(root, "")))
        walk(root, "")

    width = max((len(label) for label, note in rows if note), default=0) + 2
    body = ["```"]
    for label, note in rows:
        body.append(f"{label.ljust(width)}{note}".rstrip() if note else label)
    body.append("```")
    return "\n".join(body)


def render_anchors(docs: list[Doc]) -> str:
    code_map = DOCS / "tech" / "code-map.md"
    inverted: dict[str, list[Doc]] = {}
    for doc in docs:
        for anchor in doc.anchors:
            inverted.setdefault(anchor, []).append(doc)
    out = [
        "## File to document",
        "",
        "Every file a document names in its `code_anchors` frontmatter, inverted: change this file, and check",
        "the documents in its row.",
        "",
        "| File | Documents |",
        "|---|---|",
    ]
    for anchor in sorted(inverted):
        cited = ", ".join(
            f"[`{doc_label(code_map, d.path)}`]({link_from(code_map, d.path)})"
            for d in sorted(inverted[anchor], key=lambda d: d.rel)
        )
        out.append(f"| `{anchor}` | {cited} |")
    anchored = set(inverted)
    unanchored = [f for f in source_files(ANCHORABLE_ROOTS) if f not in anchored]
    out += [
        "",
        "## Unanchored",
        "",
        "No document's `code_anchors` names these. Restricted to `crates/ubiq/src/`.",
        "",
        "| File |",
        "|---|",
    ]
    out += [f"| `{f}` |" for f in unanchored]
    return "\n".join(out)


def run_index(check_only: bool) -> int:
    docs = load_docs()
    index = DOCS / "INDEX.md"
    code_map = DOCS / "tech" / "code-map.md"
    changed = False

    plan: list[tuple[Path, str, str]] = []
    if index.exists():
        plan.append((index, "catalogue", render_catalogue(docs)))
    if code_map.exists():
        current = code_map.read_text(encoding="utf-8")
        span = read_marker(current, "tree")
        existing_tree = current[span[0] : span[1]] if span else ""
        plan.append((code_map, "tree", render_tree(existing_tree)))
        plan.append((code_map, "anchors", render_anchors(docs)))

    for path in {p for p, _, _ in plan}:
        text = path.read_text(encoding="utf-8")
        updated = text
        missing = []
        for target, name, body in plan:
            if target != path:
                continue
            result = replace_marker(updated, name, body)
            if result is None:
                missing.append(name)
            else:
                updated = result
        rel = str(path.relative_to(REPO))
        for name in missing:
            console.print(f"[yellow]skipped[/yellow] {rel}: no `{name}` markers")
        if updated == text:
            console.print(f"[green]unchanged[/green] {rel}")
            continue
        changed = True
        before, after = text.splitlines(), updated.splitlines()
        console.print(
            f"[cyan]{'would rewrite' if check_only else 'rewrote'}[/cyan] {rel} "
            f"({len(before)} → {len(after)} lines)"
        )
        if not check_only:
            path.write_text(updated, encoding="utf-8")
            console.print(f"  diffstat: {git('diff', '--stat', '--', rel) or 'no tracked diff'}")
    if check_only and changed:
        console.print("[red]generated blocks are out of date[/red]")
        return 1
    return 0


# --------------------------------------------------------------------- drift


def run_drift() -> int:
    docs = [d for d in load_docs() if d.anchors]
    rows = []
    for doc in docs:
        verified = as_date(doc.meta.get("verified"))
        moved = []
        for anchor in doc.anchors:
            committed = last_commit_date([anchor])
            if committed and verified and committed > verified:
                moved.append((anchor, committed))
        if moved:
            rows.append((doc, verified, sorted(moved, key=lambda m: m[1], reverse=True)))
    rows.sort(key=lambda r: (r[1] or date.min))

    table = Table(title="L3 drift queue — most stale first", title_justify="left")
    table.add_column("Document")
    table.add_column("Verified")
    table.add_column("Anchored files that moved after it")
    for doc, verified, moved in rows:
        table.add_row(
            doc.rel,
            verified.isoformat() if verified else "—",
            "\n".join(f"{a} ({d.isoformat()})" for a, d in moved),
        )
    if rows:
        console.print(table)
    else:
        console.print("[green]no anchored file has moved since its document was verified[/green]")
    console.print(
        "\n[yellow]This detects the possibility of drift only[/yellow] — judging whether the document "
        "is still true needs knowing what the change meant."
    )
    console.print(f"{len(rows)} of {len(docs)} anchored documents queued")
    return 0


# ------------------------------------------------------------------- touched


def run_touched(paths: list[str]) -> int:
    if not paths:
        paths = [
            p
            for p in git("diff", "--name-only", "HEAD").splitlines()
            if p and not p.startswith("_docs/")
        ]
    if not paths:
        console.print("[yellow]no changed files[/yellow]")
        return 0
    docs = load_docs()
    normalized = []
    for p in paths:
        candidate = Path(p)
        if not candidate.is_absolute():
            candidate = Path.cwd() / p
        try:
            normalized.append(str(candidate.resolve().relative_to(REPO)))
        except ValueError:
            normalized.append(p)

    table = Table(title="Documents to verify", title_justify="left")
    table.add_column("Changed file")
    table.add_column("Anchored by")
    unanchored = []
    for rel in sorted(set(normalized)):
        owners = [d for d in docs if rel in d.anchors]
        if owners:
            table.add_row(rel, "\n".join(d.rel for d in sorted(owners, key=lambda d: d.rel)))
        else:
            unanchored.append(rel)
    if table.row_count:
        console.print(table)
        console.print(
            "\nVerify each of these in the same commit, and bump its `verified` date.",
        )
    if unanchored:
        console.print("\n[yellow]Anchored by no document[/yellow] — the missing-document case:")
        for rel in unanchored:
            console.print(f"  {rel}")
    return 0


# --------------------------------------------------------------------- graph


def run_graph() -> int:
    docs = [d for d in load_docs() if d.id]
    by_id = {d.id: d for d in docs}
    outbound = {d.id: [dep for dep in d.depends_on if dep in by_id] for d in docs}
    inbound: dict[str, list[str]] = {d.id: [] for d in docs}
    for src, deps in outbound.items():
        for dep in deps:
            inbound[dep].append(src)

    console.print("[bold]depends_on graph[/bold]\n")
    roots = [d.id for d in docs if not inbound[d.id] and outbound[d.id]]

    def render(node: str, depth: int, seen: tuple[str, ...]) -> None:
        marker = "  " * depth + ("└─ " if depth else "")
        console.print(f"{marker}{node} [dim]{escape(by_id[node].rel)}[/dim]")
        if node in seen:
            console.print("  " * (depth + 1) + "[red]… cycle[/red]")
            return
        for dep in sorted(outbound[node]):
            render(dep, depth + 1, seen + (node,))

    for root in sorted(roots):
        render(root, 0, ())
        console.print()

    isolated = [d.id for d in docs if not inbound[d.id] and not outbound[d.id]]
    hubs = [(i, len(v)) for i, v in inbound.items() if len(v) > MAX_INBOUND_EDGES]
    if isolated:
        console.print("[yellow]Isolated[/yellow] — no inbound or outbound edge:")
        for node in sorted(isolated):
            console.print(f"  {node} [dim]{escape(by_id[node].rel)}[/dim]")
    if hubs:
        console.print(
            f"\n[yellow]Over-connected[/yellow] — more than {MAX_INBOUND_EDGES} inbound edges:"
        )
        for node, count in sorted(hubs, key=lambda h: -h[1]):
            console.print(f"  {node} — {count} inbound")
    if not isolated and not hubs:
        console.print("[green]no isolated or over-connected documents[/green]")
    return 0


# ----------------------------------------------------------------------- cli


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=["lint", "index", "drift", "touched", "graph"])
    parser.add_argument("paths", nargs="*")
    parser.add_argument("--json", action="store_true", help="lint: machine-readable output")
    parser.add_argument("--check", action="store_true", help="index: fail instead of writing")
    args = parser.parse_args()

    if not DOCS.exists():
        console.print(f"[red]no _docs/ directory at {DOCS}[/red]")
        return 2

    match args.action:
        case "lint":
            return run_lint(args.paths, args.json)
        case "index":
            return run_index(args.check)
        case "drift":
            return run_drift()
        case "touched":
            return run_touched(args.paths)
        case "graph":
            return run_graph()
    return 2


if __name__ == "__main__":
    sys.exit(main())
