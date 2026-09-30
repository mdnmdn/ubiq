#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.12"
# dependencies = ["rich"]
# ///
"""Read a Ubiq tape of a Claude Code conversation: when each subagent ran, and what it cost.

Run it through `just`: `just tape-subagents` for the whole report, `just tape-subagents -- <tape>`
for a specific capture, `just tape-subagents-json` for the same analysis as JSON.

A tape is one JSON object per line — `seq`, `at_ms`, `dir`, `kind`, `agent`, the `ConvUpdate` under
`message`, and the harness's own `raw` line where one travelled (`crates/ubiq-proto/src/bus.rs`). It
is far too big for a context window and its shape is in the joins, so this walks it line by line and
keeps only aggregates. Nothing here loads the tape whole.

**The three moments a subagent has**, which is what the report is built around:

| moment | the line that says it |
|---|---|
| *spawned* | a `ToolCall` of kind `Delegate` — the spawning call's own id **is** the subagent's id, and it is the join key back to every line that subagent produced |
| *stopped working* | the last line carrying that id as `parent_tool_use_id` or `subagent_id` — after this the harness has nothing more to say about it |
| *marked ended* | a `ToolCallUpdate` naming that id with a terminal status — the conversation acknowledging the delegate |

The gap between the last two is the number worth having, and it is not zero. A delegate runs in the
background: its work streams for minutes, and the update that closes it arrives with whatever turn
result comes next — so a subagent that stopped working at 19:04 is often marked ended at 19:06, and
a few are not marked at all until the session winds up. That lag is the delay between an agent
finishing and the main agent being able to act on it.

**Where the numbers come from.** Token counts are summed per assistant message out of
`raw.message.usage` and attributed by `parent_tool_use_id` — that is what a subagent cost, and it is
the only place the split per subagent exists. The last `result`'s `modelUsage` is the conversation's
*own* accounting: a delegate's spend is attributed to the delegate rather than folded into the turn,
which is why `UsageRecord` carries `subagent` and `subagent_id` at all. The two therefore measure
different scopes and their cache-read totals do not reconcile — the per-subagent one is the larger,
because it counts work the conversation's own books never saw. Output tokens are the figure both
scopes count alike. Cost and thinking tokens exist only in that snapshot, never in a message usage.

Two hazards, both handled here rather than left to the reader. A `result` line is delivered once per
field it fills, so one turn arrives three or more times; the last of them wins and the rest are
counted. And a delegate's mark can arrive in the same breath as its last line, which would read as a
lag when it is one moment seen twice — `--tolerance` folds those together and says it did.

Nothing here writes, and nothing here is imported by the crates.
"""

from __future__ import annotations

import argparse
import json
import os
import sys
from collections import Counter, defaultdict
from dataclasses import dataclass, field
from datetime import datetime
from itertools import pairwise
from pathlib import Path
from typing import Self

from rich import box
from rich.console import Console
from rich.table import Table

REPO = Path(__file__).resolve().parents[1]
WORKSPACE = REPO.parent

#: The conversation itself, as distinct from every subagent it spawned.
MAIN = "<conversation>"

#: The `ToolCall.kind` that means "spawn", where the tool call's id is the subagent's.
SPAWN_KIND = "Delegate"

#: `ToolStatus`'s terminal values.
TERMINAL = ("Completed", "Failed")

#: A tape carries epoch milliseconds and is read where it was written, so a local zone is the
#: right reading of it — stated once rather than left to a bare `fromtimestamp`.
LOCAL = datetime.now().astimezone().tzinfo

console = Console(width=int(os.environ.get("COLUMNS") or 190))


# ── formatting ───────────────────────────────────────────────────────────────


def dur(ms: int | None) -> str:
    """A duration, at the precision the number deserves."""
    if ms is None:
        return "—"
    if ms < 1000:
        return f"{ms}ms"
    s = ms / 1000
    if s < 60:
        return f"{s:.1f}s"
    m, sec = divmod(s, 60)
    if m < 60:
        return f"{int(m)}m{sec:04.1f}s"
    h, m = divmod(m, 60)
    return f"{int(h)}h{int(m):02d}m"


def clock(at_ms: int | None) -> str:
    """The wall time of an event, in this machine's own zone."""
    if at_ms is None or at_ms <= 0:
        return "—"
    return datetime.fromtimestamp(at_ms / 1000, tz=LOCAL).strftime("%H:%M:%S")


def tok(n: int) -> str:
    """A token count, abbreviated once it stops being countable."""
    if n >= 1_000_000:
        return f"{n / 1_000_000:.1f}M"
    if n >= 100_000:
        return f"{n // 1000}k"
    return f"{n:,}"


def money(n: float | None) -> str:
    return "—" if n is None else f"${n:,.2f}"


def pctf(part: float, whole: float) -> str:
    return "—" if not whole else f"{100 * part / whole:.1f}%"


# ── the model ────────────────────────────────────────────────────────────────


@dataclass
class Tokens:
    """What a run of assistant messages cost, from `raw.message.usage`."""

    input: int = 0
    output: int = 0
    cache_read: int = 0
    cache_creation: int = 0

    def add(self, usage: dict) -> None:
        self.input += usage.get("input_tokens") or 0
        self.output += usage.get("output_tokens") or 0
        self.cache_read += usage.get("cache_read_input_tokens") or 0
        self.cache_creation += usage.get("cache_creation_input_tokens") or 0

    def __iadd__(self, other: Tokens) -> Self:
        self.input += other.input
        self.output += other.output
        self.cache_read += other.cache_read
        self.cache_creation += other.cache_creation
        return self

    @property
    def total(self) -> int:
        return self.input + self.output + self.cache_read + self.cache_creation


@dataclass
class Run:
    """One subagent, or the conversation itself, as this tape saw it."""

    id: str
    kind: str = "—"
    model: str = "—"
    title: str | None = None
    task: str | None = None

    # the three moments
    spawned_at: int | None = None
    spawned_seq: int | None = None
    announced: bool = False
    first_at: int | None = None
    first_seq: int | None = None
    last_at: int | None = None
    last_seq: int | None = None
    marked_at: int | None = None
    marked_seq: int | None = None
    marked_status: str | None = None

    # what it did
    events: int = 0
    messages: int = 0
    chunks: int = 0
    tool_calls: int = 0
    tool_failed: int = 0
    tools: Counter = field(default_factory=Counter)
    context_peak: int = 0
    context_size: int = 0
    last_text: str = ""

    tokens: Tokens = field(default_factory=Tokens)

    def describe(self, kind=None, model=None, task=None, thinking=None) -> None:
        """Fill in identity from whatever line carried it — no single line has all of it."""
        if kind:
            self.kind = kind
        if task:
            self.task = task
        if model and self.model == "—":
            self.model = model
        if thinking and thinking not in self.model:
            self.model = f"{self.model} +{thinking}" if self.model != "—" else f"+{thinking}"

    @property
    def is_main(self) -> bool:
        return self.id == MAIN

    @property
    def observed(self) -> int | None:
        """First line this tape holds for the run to its last — the window actually witnessed."""
        if self.first_at is None or self.last_at is None:
            return None
        return self.last_at - self.first_at

    @property
    def worked(self) -> int | None:
        """How long it was working: spawn to last line, or the observed window where the spawn
        predates the tape."""
        if self.spawned_at is None:
            return self.observed
        if self.last_at is None:
            return None
        return self.last_at - self.spawned_at

    @property
    def lifetime(self) -> int | None:
        """Spawn to the mark — how long the conversation still thought it was running."""
        if self.spawned_at is None or self.marked_at is None:
            return None
        return self.marked_at - self.spawned_at

    @property
    def lag(self) -> int | None:
        """The gap: the work stopped, but nothing had said so yet."""
        if self.last_at is None or self.marked_at is None:
            return None
        return self.marked_at - self.last_at

    @property
    def context_pct(self) -> float | None:
        return 100 * self.context_peak / self.context_size if self.context_size else None

    def state(self) -> str:
        if self.is_main:
            return "conversation"
        if self.marked_status:
            base = self.marked_status.rstrip("*").lower()
            return f"{base} (folded)" if self.marked_status.endswith("*") else base
        if self.first_at is None:
            return "closed, work unseen"
        if not self.announced:
            return "unmarked, pre-tape"
        return "never marked"

    def label(self) -> str:
        return MAIN if self.is_main else self.id.removeprefix("toolu_")[-8:]


@dataclass
class Snapshot:
    """A `result` line: the harness's own account of a finished turn."""

    at: int
    seq: int
    duration_ms: int | None
    num_turns: int | None
    stop_reason: str | None
    total_cost_usd: float | None
    model_usage: dict
    usage: dict
    stats: dict
    ttft_ms: int | None


# ── reading the tape ─────────────────────────────────────────────────────────


def find_tape(arg: str | None) -> Path:
    """The capture to read: the one named, else the newest tape-shaped file there is."""
    if arg:
        path = Path(arg).expanduser()
        if not path.exists():
            sys.exit(f"no such tape: {path}")
        return path

    roots: list[Path] = [Path.cwd(), WORKSPACE, REPO]
    if os.environ.get("UBIQ_TAPE_DIR"):
        roots.insert(0, Path(os.environ["UBIQ_TAPE_DIR"]))
    for root in roots:
        for pattern in ("_data/claude-subagents*.jsonl", "ubiq-tape-*.jsonl", "_data/ubiq-tape-*.jsonl"):
            found = [p for p in root.glob(pattern) if p.is_file()]
            if found:
                return max(found, key=lambda p: p.stat().st_mtime)
    sys.exit("no tape found — pass one as an argument, or set UBIQ_TAPE_DIR, or drop a capture in _data/")


def parse_raw(text) -> dict:
    if not isinstance(text, str) or not text:
        return {}
    try:
        raw = json.loads(text)
    except json.JSONDecodeError:
        return {}
    return raw if isinstance(raw, dict) else {}


def content_text(content) -> str:
    """`ConvContent` is a tagged union; only its text arm is worth keeping."""
    if isinstance(content, str):
        return content
    if isinstance(content, dict):
        for key in ("Text", "text", "Thinking", "Markdown"):
            if isinstance(content.get(key), str):
                return content[key]
    return ""


class Analysis:
    """Everything the report prints, built from one streaming pass."""

    def __init__(self, path: Path, tolerance_ms: int = 0) -> None:
        self.path = path
        self.tolerance_ms = tolerance_ms
        self.runs: dict[str, Run] = {}
        self.order: list[str] = []
        self.turns: list[tuple[int | None, int, str, str | None]] = []
        self.snapshots: list[Snapshot] = []
        self.rate_limits: list[tuple[int, dict]] = []
        self.dangling_marks: list[tuple[str, str, int | None, int]] = []
        self.folded_marks: list[str] = []
        self.notes: list[str] = []

        self.lines = 0
        self.malformed = 0
        self.start: int | None = None
        self.end: int | None = None
        self.dump_at: int | None = None
        self.dirs: Counter = Counter()
        self.kinds: Counter = Counter()
        self.updates: Counter = Counter()
        self.raw_types: Counter = Counter()
        self.tool_status: Counter = Counter()
        self.tool_kinds: Counter = Counter()
        self.agents: set[str] = set()
        self.sessions: set[str] = set()
        self.duplicates = 0

        self._marks: dict[str, tuple[int | None, int, str]] = {}
        self._subagent_ids: set[str] = set()
        self._announced: dict[str, str] = {}
        self._results_seen: set[str] = set()
        self._own_activity: list[int] = []

    # ── one pass ──

    def feed(self, rec: dict) -> None:
        self.lines += 1
        at = rec.get("at_ms")
        seq = rec.get("seq")
        self.dirs[rec.get("dir") or "—"] += 1
        kind = rec.get("kind") or "—"
        self.kinds[kind] += 1
        if rec.get("agent"):
            self.agents.add(rec["agent"])

        raw = parse_raw(rec.get("raw"))
        rtype = raw.get("type") or "—"
        self.raw_types[rtype] += 1
        if raw.get("session_id"):
            self.sessions.add(raw["session_id"])

        update = (rec.get("message") or {}).get("payload", {}).get("update")
        carried = isinstance(update, dict) and bool(update)
        if kind == "SetConversationDebugDump":
            # The request for this very file. It is not conversation activity, and letting it set
            # the end of the window would report a tape that is an hour longer than the run in it.
            self.dump_at = at if isinstance(at, int) else self.dump_at
            return
        if isinstance(at, int):
            self.start = at if self.start is None else min(self.start, at)
            self.end = at if self.end is None else max(self.end, at)

        owner = raw.get("parent_tool_use_id")
        if owner:
            self._subagent_ids.add(owner)
        if raw.get("subagent_type"):
            self.run(owner or MAIN).describe(kind=raw["subagent_type"], task=raw.get("task_description"))

        if rtype == "assistant":
            message = raw.get("message") or {}
            r = self.run(owner or MAIN)
            r.tokens.add(message.get("usage") or {})
            r.messages += 1
            # The bridge reports a delegate's resolved model on its spawn; a line of its own stream
            # agrees. Only used where nothing better is known, which is the conversation itself.
            r.describe(model=message.get("model"))
            self.touch(r, at, seq)

        if carried:
            for variant, body in update.items():
                self.on_update(variant, body if isinstance(body, dict) else {}, at, seq, raw, owner, isinstance(at, int))

        if rtype == "result":
            self.on_result(raw, at, seq)

    def on_update(self, variant: str, body: dict, at, seq, raw: dict, owner: str | None, has_at: bool) -> None:
        self.updates[variant] += 1

        if variant == "ToolCall":
            call_id = body.get("id")
            if call_id:
                self._announced[call_id] = body.get("kind") or "—"
            self.tool_kinds[body.get("kind") or "—"] += 1
            sub = body.get("subagent") or {}
            if body.get("kind") == SPAWN_KIND and not sub:
                # The spawn. The spawning call's own id is the subagent it created, and it is the
                # join key back to every line that subagent will produce.
                self._subagent_ids.add(call_id)
                r = self.run(call_id)
                r.announced = True
                r.spawned_at, r.spawned_seq = at, seq
                r.title = body.get("title")
                self.touch(r, at, seq)
                # The call is the caller's, not the callee's: it is the conversation that reached for
                # Delegate. Counting it against the subagent would read as the subagent having
                # delegated, which is the one thing about it that is certainly not true.
                caller = self.run(owner or MAIN)
                caller.tool_calls += 1
                caller.tools[SPAWN_KIND] += 1
                if isinstance(at, int):
                    self._own_activity.append(at)
                return
            r = self.run(sub.get("id") or owner or MAIN)
            r.tool_calls += 1
            r.tools[body.get("kind") or "—"] += 1
            if sub:
                self._subagent_ids.add(sub["id"])
                r.describe(kind=sub.get("kind"), model=sub.get("model"), thinking=sub.get("thinking"))
            self.touch(r, at, seq, own=not sub)
            return

        if variant == "ToolCallUpdate":
            status = body.get("status") or "—"
            self.tool_status[status] += 1
            if status in TERMINAL and body.get("id"):
                self._marks.setdefault(body["id"], (seq, at, status))
            r = self.run(owner or MAIN)
            if status == "Failed":
                r.tool_failed += 1
            # A subagent's own closing update is its mark, not work — it must not be the last line
            # that sets its working window's end.
            if not (status in TERMINAL and body.get("id") in self._subagent_ids):
                self.touch(r, at, seq)
            return

        if variant == "Usage":
            rid = body.get("subagent_id")
            if rid:
                self._subagent_ids.add(rid)
            r = self.run(rid or owner or MAIN)
            r.describe(kind=body.get("subagent") if rid else None, model=body.get("model") if rid else None)
            r.context_peak = max(r.context_peak, body.get("used") or 0)
            r.context_size = max(r.context_size, body.get("size") or 0)
            self.touch(r, at, seq, own=not rid)
            return

        if variant in ("AgentChunk", "ThoughtChunk", "UserChunk"):
            sub = body.get("subagent") or {}
            if sub.get("id"):
                self._subagent_ids.add(sub["id"])
            r = self.run(sub.get("id") or owner or MAIN)
            r.describe(kind=sub.get("kind"), model=sub.get("model"), thinking=sub.get("thinking"))
            if variant == "AgentChunk":
                r.chunks += 1
                text = content_text(body.get("content"))
                if text:
                    r.last_text = text
            self.touch(r, at, seq, own=not sub)
            return

        if variant == "RateLimit":
            self.rate_limits.append((at if isinstance(at, int) else 0, body))
            return

        if variant == "TurnEnded":
            self.turns.append((seq, at if isinstance(at, int) else 0, body.get("stop_reason") or "—", body.get("error")))

    def on_result(self, raw: dict, at, seq) -> None:
        # One result arrives once per field it fills, so the same object is delivered three or more
        # times. The uuid identifies the turn; the first delivery is the one kept.
        key = raw.get("uuid") or json.dumps(raw, sort_keys=True)
        if key in self._results_seen:
            self.duplicates += 1
            return
        self._results_seen.add(key)
        self.snapshots.append(
            Snapshot(
                at=at if isinstance(at, int) else 0,
                seq=seq if isinstance(seq, int) else 0,
                duration_ms=raw.get("duration_ms"),
                num_turns=raw.get("num_turns"),
                stop_reason=raw.get("stop_reason"),
                total_cost_usd=raw.get("total_cost_usd"),
                model_usage=raw.get("modelUsage") or {},
                usage=raw.get("usage") or {},
                stats=raw.get("subagent_stats") or {},
                ttft_ms=raw.get("ttft_ms"),
            )
        )

    def touch(self, r: Run, at, seq, own: bool = False) -> None:
        """Note an event against a run: first sight, last word, and whether it is the main agent's."""
        if not isinstance(at, int):
            return
        if r.first_at is None or at < r.first_at:
            r.first_at, r.first_seq = at, seq
        if r.last_at is None or at >= r.last_at:
            r.last_at, r.last_seq = at, seq
        r.events += 1
        if own:
            self._own_activity.append(at)

    def run(self, rid: str) -> Run:
        r = self.runs.get(rid)
        if r is None:
            r = Run(id=rid)
            self.runs[rid] = r
            self.order.append(rid)
        return r

    # ── finishing ──

    def finish(self) -> None:
        # A closing update names a subagent only if that id was ever a subagent. An id this tape
        # saw open as a plain tool call is a Bash or an Edit, and is not a run; an id it never saw
        # open at all is reported as dangling rather than guessed at.
        for rid, (seq, at, status) in self._marks.items():
            if rid in self._subagent_ids:
                r = self.run(rid)
                if r.marked_at is None or (at or 0) >= r.marked_at:
                    r.marked_seq, r.marked_at, r.marked_status = seq, at, status
            elif rid not in self._announced:
                self.dangling_marks.append((rid, status, seq, at or 0))

        for rid, r in self.runs.items():
            if r.is_main and r.announced is False:
                r.spawned_at, r.spawned_seq = r.first_at, r.first_seq

        # A mark landing within a hair of the last line is the same moment delivered twice, not a
        # lag. Fold it, and say so, rather than reporting a duration nobody should trust.
        for rid, r in self.runs.items():
            lag = r.lag
            if lag is not None and self.tolerance_ms and lag <= self.tolerance_ms:
                r.marked_at, r.marked_seq = r.last_at, r.last_seq
                r.marked_status = (r.marked_status or "Completed") + "*"
                self.folded_marks.append(rid)

        self.order.sort(key=lambda rid: (self.runs[rid].first_at or 0, self.runs[rid].last_at or 0))
        self.notes = self._notes()

    def _notes(self) -> list[str]:
        notes: list[str] = []
        sub = self.subagents
        off = [r for r in sub if not r.announced and r.first_at is None]
        if off:
            notes.append(
                f"{len(off)} subagent(s) only ever appear as a closing line — their spawn and their "
                f"work are both outside this tape: {', '.join(r.label() for r in off)}"
            )
        unseen = [r for r in sub if r.announced is False and r.first_at is not None]
        if unseen:
            notes.append(
                f"{len(unseen)} subagent(s) were already running when the tape starts, and their work "
                f"is in it: {', '.join(r.label() for r in unseen)}"
            )
        open_ended = [r for r in sub if r.marked_at is None]
        if open_ended:
            notes.append(
                f"{len(open_ended)} subagent(s) stopped working but were never marked ended in this "
                f"tape: {', '.join(r.label() for r in open_ended)}"
            )
        if self.folded_marks:
            notes.append(
                f"{len(self.folded_marks)} mark(s) landed within --tolerance of the last work and are "
                f"reported against it: {', '.join(r.label() for r in sub if r.id in set(self.folded_marks))}"
            )
        if self.dangling_marks:
            notes.append(
                f"{len(self.dangling_marks)} tool call(s) were closed with no opening line in this "
                f"tape: {', '.join(rid.removeprefix('toolu_')[-6:] for rid, _, _, _ in self.dangling_marks)}"
            )
        if self.duplicates:
            notes.append(
                f"{self.duplicates} re-delivered `result` line(s) collapsed — a result arrives once per "
                f"field it fills, so one turn is three or more lines"
            )
        if self.malformed:
            notes.append(f"{self.malformed} line(s) did not parse as JSON and were skipped")
        return notes

    # ── derived views ──

    @property
    def main(self) -> Run:
        return self.runs.get(MAIN) or self.run(MAIN)

    @property
    def subagents(self) -> list[Run]:
        return [self.runs[rid] for rid in self.order if rid != MAIN]

    @property
    def spawned(self) -> list[Run]:
        return [r for r in self.subagents if r.announced]

    @property
    def wall_ms(self) -> int | None:
        return None if self.start is None or self.end is None else self.end - self.start

    @property
    def last_snapshot(self) -> Snapshot | None:
        return self.snapshots[-1] if self.snapshots else None

    def totals(self) -> Tokens:
        t = Tokens()
        for r in self.runs.values():
            t += r.tokens
        return t

    def spans(self) -> list[tuple[int, int]]:
        """Each subagent's working window — where a subagent could be running."""
        out = []
        for r in self.subagents:
            if r.first_at is not None and r.last_at is not None and r.last_at >= r.first_at:
                out.append((r.first_at, r.last_at))
        return out

    def levels(self) -> list[tuple[int, int]]:
        """`(timestamp, subagents running)` over each interval between two boundaries.

        One sweep, and every duration below is read off it, so the histogram and the totals cannot
        disagree. Deltas are summed per timestamp first, so two subagents meeting at the same
        millisecond are one ending and one starting rather than an overlap nobody had.
        """
        deltas: dict[int, int] = defaultdict(int)
        for lo, hi in self.spans():
            deltas[lo] += 1
            deltas[hi] -= 1
        stamps = sorted(deltas)
        curve: list[tuple[int, int]] = []
        cur = 0
        for at in stamps[:-1]:
            cur += deltas[at]
            curve.append((at, cur))
        return curve

    def peak_concurrent(self) -> int:
        return max((n for _, n in self.levels()), default=0)

    def time_at(self) -> dict[int, int]:
        """How much wall time each concurrency level held."""
        curve = self.levels()
        out: dict[int, int] = defaultdict(int)
        for (a, n), (b, _) in pairwise(curve):
            out[n] += b - a
        return dict(out)

    def busy_ms(self) -> int:
        """Wall time with at least one subagent running — overlaps counted once."""
        return sum(ms for n, ms in self.time_at().items() if n >= 1)

    def parallel_ms(self) -> int:
        """Wall time with two or more — the overlap the fan-out actually bought."""
        return sum(ms for n, ms in self.time_at().items() if n >= 2)

    def blocked(self) -> tuple[int, int]:
        """How much of the main agent's own activity landed while a subagent was running."""
        spans = self.spans()
        own = self._own_activity
        if not spans:
            return 0, len(own)
        during = sum(1 for at in own if any(lo <= at <= hi for lo, hi in spans))
        return during, len(own)


def walk(path: Path, tolerance_ms: int = 0) -> Analysis:
    """Read the tape once, keeping aggregates and one record per subagent."""
    an = Analysis(path=path, tolerance_ms=tolerance_ms)
    with path.open(encoding="utf-8", errors="replace") as fh:
        for line in fh:
            if not line.strip():
                continue
            try:
                rec = json.loads(line)
            except json.JSONDecodeError:
                an.malformed += 1
                continue
            if isinstance(rec, dict):
                an.feed(rec)
            else:
                an.malformed += 1
    an.finish()
    return an


# ── the report ───────────────────────────────────────────────────────────────


def rule(title: str) -> None:
    console.print()
    console.rule(f"[bold]{title}[/]", style="cyan")


def kv_table(pairs: list[tuple[str, str]]) -> Table:
    t = Table(box=None, pad_edge=False, show_header=False)
    t.add_column(style="dim")
    t.add_column()
    for k, v in pairs:
        t.add_row(k, v)
    return t


def print_header(an: Analysis) -> None:
    rule("the tape")
    rows = [
        ("file", f"{an.path}  ({an.path.stat().st_size / 1e6:.1f} MB)"),
        ("lines", f"{an.lines:,} read · {an.malformed:,} unparseable · {an.duplicates:,} duplicate result deliveries"),
        ("window", f"{clock(an.start)} → {clock(an.end)}   [bold]{dur(an.wall_ms)}[/] of conversation"),
        ("agent", ", ".join(sorted(an.agents)) or "—"),
        ("session", ", ".join(sorted(an.sessions)) or "—"),
        ("directions", ", ".join(f"{k} {v:,}" for k, v in an.dirs.most_common())),
        ("harness lines", ", ".join(f"{k} {v:,}" for k, v in an.raw_types.most_common())),
    ]
    if an.dump_at is not None:
        rows.append(("dumped", f"{clock(an.dump_at)} — the request for this file, {dur(an.dump_at - an.end)} after the last line of the conversation"))
    console.print(kv_table(rows))


def print_lifecycle(an: Analysis) -> None:
    rule("the three moments: spawn, work ends, marked ended")
    console.print(
        "[dim]A delegate runs in the background: its lines stream for minutes, and the update that "
        "closes it arrives with a later turn's result. The lag column is that gap — how long the main "
        "agent held a finished subagent it could not yet see.[/]\n"
    )
    t = Table(show_edge=False, box=box.SIMPLE)
    t.add_column("subagent", no_wrap=True)
    t.add_column("kind", no_wrap=True)
    t.add_column("task", no_wrap=True, overflow="ellipsis", max_width=34)
    t.add_column("spawned", no_wrap=True)
    t.add_column("worked", justify="right", no_wrap=True)
    t.add_column("work ends", no_wrap=True)
    t.add_column("marked", no_wrap=True)
    t.add_column("lag", justify="right", no_wrap=True)
    t.add_column("state", no_wrap=True)
    t.add_column("msgs", justify="right")
    t.add_column("out", justify="right")
    t.add_column("ctx", justify="right")

    for rid in an.order:
        r = an.runs[rid]
        lag = r.lag
        if lag is None:
            lag_cell = "—"
        elif lag > 60_000:
            lag_cell = f"[yellow]{dur(lag)}[/]"
        else:
            lag_cell = dur(lag)
        if r.is_main:
            spawn, worked, ends, marked = "[dim]whole tape[/]", "—", clock(r.last_at), "—"
        else:
            spawn = clock(r.spawned_at) if r.announced else "[dim]pre-tape[/]"
            worked, ends, marked = dur(r.worked), clock(r.last_at), clock(r.marked_at)
        t.add_row(
            r.label(),
            r.kind,
            r.title or r.task or "—",
            spawn,
            worked,
            ends,
            marked,
            lag_cell,
            r.state(),
            f"{r.messages:,}",
            tok(r.tokens.output),
            f"{r.context_pct:.0f}%" if r.context_pct is not None else "—",
        )
    console.print(t)

    lags = sorted(r.lag for r in an.subagents if r.lag is not None)
    if lags:
        console.print(
            f"\n[dim]lag from last work to the mark over {len(lags)} subagent(s): min {dur(lags[0])}, "
            f"median {dur(lags[len(lags) // 2])}, max {dur(lags[-1])}[/]"
        )
    worked = sorted(r.worked for r in an.spawned if r.worked is not None)
    if worked:
        console.print(
            f"[dim]time actually working over {len(worked)} spawn(s) in this tape: median "
            f"{dur(worked[len(worked) // 2])}, max {dur(worked[-1])}[/]"
        )
    for note in an.notes:
        console.print(f"[yellow]note[/] {note}")


def print_gantt(an: Analysis, width: int) -> None:
    rule("timeline")
    if an.wall_ms is None or an.wall_ms <= 0 or not an.subagents:
        console.print("[dim]nothing to lay out[/]")
        return
    lo, hi = an.start, an.end
    span = max(1, hi - lo)
    width = max(20, width)

    def col(at: int | None) -> int | None:
        if at is None:
            return None
        return max(0, min(width - 1, int(width * (at - lo) / span)))

    label_w = max(len(r.label()) for r in an.subagents) + 1
    left, right = clock(lo), clock(hi)
    gap = max(1, width - len(left) - len(right))
    console.print(f"[dim]{'':<{label_w}}  {left}{' ' * gap}{right}[/]\n")

    for r in an.subagents:
        row = [" "] * width
        priority = [0] * width
        a, b = col(r.first_at), col(r.last_at)
        if a is not None and b is not None:
            for c in range(a, b + 1):
                row[c], priority[c] = "█", 1
        # A spawn and a mark can land in a cell the bar already owns, so each is written only over
        # something of lower priority — the mark wins, because it is the thing being looked for.
        for at, glyph, level in ((r.spawned_at, "+", 2), (r.marked_at, "x", 3)):
            c = col(at)
            if c is not None and level > priority[c]:
                row[c], priority[c] = glyph, level
        colour = "green" if r.marked_status and not r.marked_status.endswith("*") else "yellow"
        console.print(
            f"{r.label():<{label_w}}  [{colour}]{''.join(row)}[/{colour}]  [dim]{r.kind}, "
            f"{dur(r.worked)} working, {tok(r.tokens.output)} out[/]"
        )

    console.print(
        "\n[dim]  [green]█[/] the subagent's working window   [green]+[/] spawned   "
        "[red]x[/] marked ended   [yellow]█[/] no terminal update in this tape[/]"
    )


def print_fanout(an: Analysis) -> None:
    peak = an.peak_concurrent()
    if not an.spans():
        return
    rule("fan-out")
    wall = an.wall_ms or 1
    during, own = an.blocked()
    marked = sum(1 for r in an.subagents if r.marked_status and not r.marked_status.endswith("*"))
    console.print(
        kv_table(
            [
                ("subagents", f"{len(an.subagents)} in this tape · {len(an.spawned)} spawned here · {marked} marked ended"),
                ("peak concurrent", f"[bold]{peak}[/]"),
                ("wall time", dur(an.wall_ms)),
                ("with ≥1 running", f"{dur(an.busy_ms())}  ({pctf(an.busy_ms(), wall)} of the tape)"),
                ("with ≥2 running", f"{dur(an.parallel_ms())}  ({pctf(an.parallel_ms(), wall)})"),
                ("conversation lines inside a subagent window", f"{during} of {own}  ({pctf(during, own)})"),
            ]
        )
    )
    hist = an.time_at()
    t = Table(box=None, show_header=False, pad_edge=False)
    t.add_column(style="dim", no_wrap=True)
    t.add_column(justify="right", no_wrap=True)
    t.add_column(justify="right", no_wrap=True)
    t.add_column()
    widest = max(hist.values()) if hist else 1
    for n in sorted(hist):
        t.add_row(f"{n} running", dur(hist[n]), pctf(hist[n], wall), "[dim]" + "▇" * max(1, hist[n] * 40 // widest) + "[/dim]")
    console.print()
    console.print(t)


def print_statistics(an: Analysis) -> None:
    rule("statistics")
    worked = sorted(r.worked for r in an.spawned if r.worked is not None)
    lags = sorted(r.lag for r in an.subagents if r.lag is not None)
    sub_tokens = sum(r.tokens.total for r in an.subagents)
    stops = Counter(r for _, _, r, _ in an.turns)

    left = kv_table(
        [
            ("lines", f"{an.lines:,}"),
            ("conv updates", ", ".join(f"{k} {v:,}" for k, v in an.updates.most_common(6))),
            ("subagents", f"{len(an.subagents)}  ({len(an.spawned)} spawned here)"),
            ("marked ended", f"{sum(1 for r in an.subagents if r.marked_status and not r.marked_status.endswith('*'))}"),
            ("turns ended", f"{len(an.turns)}" + (f"  ({', '.join(f'{k} {v}' for k, v in stops.most_common())})" if stops else "")),
            ("result snapshots", f"{len(an.snapshots)}"),
            ("tool calls", f"{an.tool_kinds.total():,}"),
            ("tool status", ", ".join(f"{k} {v}" for k, v in an.tool_status.most_common())),
            (
                "failed tools",
                (
                    f"{sum(r.tool_failed for r in an.runs.values())}  "
                    f"({sum(r.tool_failed for r in an.subagents)} inside a subagent, "
                    f"{an.main.tool_failed} in the conversation)"
                ),
            ),
        ]
    )
    right = kv_table(
        [
            ("median time working", dur(worked[len(worked) // 2]) if worked else "—"),
            ("longest", dur(worked[-1]) if worked else "—"),
            ("median mark lag", dur(lags[len(lags) // 2]) if lags else "—"),
            ("longest lag", dur(lags[-1]) if lags else "—"),
            ("main agent messages", f"{an.main.messages:,}"),
            ("subagent messages", f"{sum(r.messages for r in an.subagents):,}"),
            ("main agent output", tok(an.main.tokens.output)),
            ("subagent output", tok(sum(r.tokens.output for r in an.subagents))),
            ("subagent share of tokens", pctf(sub_tokens, sub_tokens + an.main.tokens.total)),
        ]
    )
    grid = Table.grid(padding=(0, 3))
    grid.add_row(left, right)
    console.print(grid)

    top = an.tool_kinds.most_common()
    if top:
        t = Table(title="tool calls by kind", title_justify="left", show_edge=False)
        t.add_column("kind")
        t.add_column("count", justify="right")
        t.add_column("share", justify="right")
        t.add_column("bar", no_wrap=True)
        for kind, n in top:
            t.add_row(kind, f"{n:,}", pctf(n, an.tool_kinds.total()), "[dim]" + "▇" * max(1, n * 40 // top[0][1]) + "[/dim]")
        console.print()
        console.print(t)

    if an.subagents:
        by_kind: dict[str, list[Run]] = defaultdict(list)
        for r in an.subagents:
            by_kind[r.kind].append(r)
        grand = sub_tokens or 1
        t = Table(title="subagents by kind", title_justify="left", show_edge=False)
        t.add_column("kind")
        t.add_column("n", justify="right")
        t.add_column("median work", justify="right")
        t.add_column("median lag", justify="right")
        t.add_column("out tokens", justify="right")
        t.add_column("token share", justify="right")
        for kind, runs in sorted(by_kind.items(), key=lambda kv: -len(kv[1])):
            w = sorted(r.worked for r in runs if r.worked is not None)
            lg = sorted(r.lag for r in runs if r.lag is not None)
            t.add_row(
                kind,
                str(len(runs)),
                dur(w[len(w) // 2]) if w else "—",
                dur(lg[len(lg) // 2]) if lg else "—",
                tok(sum(r.tokens.output for r in runs)),
                pctf(sum(r.tokens.total for r in runs), grand),
            )
        console.print(t)

    if an.rate_limits:
        lo = an.rate_limits[0][1].get("five_hour_pct")
        last = an.rate_limits[-1][1]
        t = Table(title="rate limit windows", title_justify="left", show_edge=False)
        t.add_column("at", no_wrap=True)
        t.add_column("5h", justify="right")
        t.add_column("7d", justify="right")
        t.add_column("status")
        t.add_column("overage")
        step = max(1, len(an.rate_limits) // 8)
        for at, b in an.rate_limits[::step] + an.rate_limits[-1:]:
            t.add_row(
                clock(at),
                f"{b['five_hour_pct']}%" if b.get("five_hour_pct") is not None else "—",
                f"{b['seven_day_pct']}%" if b.get("seven_day_pct") is not None else "—",
                b.get("status") or "—",
                (b.get("overage_status") or "—") + (f" ({b['overage_reason']})" if b.get("overage_reason") else ""),
            )
        console.print(t)
        if lo is not None and last.get("five_hour_pct") is not None:
            console.print(
                f"\n[dim]five-hour window {lo}% → {last['five_hour_pct']}% across the tape, "
                f"{len(an.rate_limits)} readings, last status {last.get('status')}[/]"
            )


def print_tokens(an: Analysis) -> None:
    rule("token usage")
    total = an.totals()

    t = Table(title="summed from every assistant message in the tape, by who spent it", title_justify="left", show_edge=False)
    t.add_column("who")
    t.add_column("msgs", justify="right")
    t.add_column("input", justify="right")
    t.add_column("output", justify="right")
    t.add_column("cache read", justify="right")
    t.add_column("cache write", justify="right")
    t.add_column("total", justify="right")
    t.add_column("share", justify="right")

    def row(who: str, r: Run) -> None:
        t.add_row(
            who,
            f"{r.messages:,}",
            tok(r.tokens.input),
            tok(r.tokens.output),
            tok(r.tokens.cache_read),
            tok(r.tokens.cache_creation),
            tok(r.tokens.total),
            pctf(r.tokens.total, total.total),
        )

    row("[bold]conversation[/]", an.main)
    for r in sorted(an.subagents, key=lambda r: -r.tokens.total):
        row(f"[cyan]{r.label()}[/]  {r.kind}", r)
    t.add_row("[bold]total[/]", f"[bold]{sum(r.messages for r in an.runs.values()):,}[/]", "", "", "", "",
              f"[bold]{tok(total.total)}[/]", "")
    console.print(t)

    cacheable = total.cache_read + total.cache_creation
    console.print(
        f"\n[dim]cache read {total.cache_read:,} · cache write {total.cache_creation:,} · "
        f"{pctf(total.cache_read, cacheable)} of cacheable input came from cache · "
        f"uncached input {total.input:,}[/]"
    )

    per_model: dict[str, Counter] = defaultdict(Counter)
    for r in an.runs.values():
        m = per_model[r.model]
        m["actors"] += 1
        m["msgs"] += r.messages
        for f in ("input", "output", "cache_read", "cache_creation"):
            m[f] += getattr(r.tokens, f)
    t = Table(title="by model, as the tape attributes it", title_justify="left", show_edge=False)
    t.add_column("model")
    t.add_column("actors", justify="right")
    t.add_column("msgs", justify="right")
    t.add_column("input", justify="right")
    t.add_column("output", justify="right")
    t.add_column("cache read", justify="right")
    t.add_column("cache write", justify="right")
    for model, m in sorted(per_model.items(), key=lambda kv: -sum(kv[1][f] for f in ("output", "cache_read"))):
        t.add_row(model, str(m["actors"]), f"{m['msgs']:,}", tok(m["input"]), tok(m["output"]), tok(m["cache_read"]), tok(m["cache_creation"]))
    console.print(t)

    snap = an.last_snapshot
    if not snap:
        return

    rule("the harness's own totals — the last result in the tape")
    t = Table(show_edge=False)
    for col, just in (("model", False), ("input", True), ("output", True), ("thinking", True),
                      ("cache read", True), ("cache write", True), ("cost", True)):
        t.add_column(col, justify="right" if just else "left")
    agg = Counter()
    for model, u in sorted(snap.model_usage.items(), key=lambda kv: -(kv[1].get("costUSD") or 0)):
        t.add_row(
            model,
            f"{u.get('inputTokens', 0):,}",
            f"{u.get('outputTokens', 0):,}",
            f"{u.get('thinkingTokens', 0):,}",
            f"{u.get('cacheReadInputTokens', 0):,}",
            f"{u.get('cacheCreationInputTokens', 0):,}",
            money(u.get("costUSD")),
        )
        for f in ("inputTokens", "outputTokens", "thinkingTokens", "cacheReadInputTokens", "cacheCreationInputTokens"):
            agg[f] += u.get(f) or 0
    t.add_row("[bold]total[/]", f"[bold]{agg['inputTokens']:,}[/]", f"[bold]{agg['outputTokens']:,}[/]",
              f"[bold]{agg['thinkingTokens']:,}[/]", f"[bold]{agg['cacheReadInputTokens']:,}[/]",
              f"[bold]{agg['cacheCreationInputTokens']:,}[/]", f"[bold]{money(snap.total_cost_usd)}[/]")
    console.print(t)

    console.print(
        f"\n[dim]thinking {agg['thinkingTokens']:,} tokens — {pctf(agg['thinkingTokens'], agg['outputTokens'])} of "
        f"output. Thinking is reported only in a result and never in a message usage, which is why "
        f"this is the only place it appears.[/]"
    )

    console.print(
        "\n[dim]The two tables above are not comparable, and must not be added. The first is every "
        "message in the tape, delegates included; the second is the conversation's own\naccounting in "
        "one turn's result, and a delegate's spend is attributed to the delegate rather than folded "
        "into the turn — which is\nwhy `UsageRecord` carries `subagent` and `subagent_id` at all. The "
        f"tape's {total.cache_read:,} cache reads against the conversation's "
        f"{agg['cacheReadInputTokens']:,} is that split, not a disagreement: the\ntape's figure is the "
        "larger one because it counts work the conversation's own books do not. Output tokens are the "
        f"figure both scopes\ncount alike — {total.output:,} here against {agg['outputTokens']:,} for the "
        "session, the difference being turns outside this capture.[/]"
    )

    if snap.stats:
        st = snap.stats
        console.print(
            "\n[dim]subagent_stats at that result: spawned "
            f"{st.get('spawned')} · completed {st.get('completed')} · failed {st.get('failed')} · killed "
            f"{sum((st.get('killed') or {}).values())} · refused {sum((st.get('refused') or {}).values())} · "
            f"max depth {st.get('max_depth')} · started in background {st.get('started_in_background')} · "
            f"requested {st.get('requested')}\nby type: "
            f"{', '.join(f'{k} {v}' for k, v in (st.get('by_type') or {}).items())}[/]"
        )

    if an.snapshots:
        t = Table(title="every result in the tape, one row per finished turn", title_justify="left", show_edge=False)
        t.add_column("at", no_wrap=True)
        t.add_column("turns", justify="right")
        t.add_column("turn ms", justify="right")
        t.add_column("ttft ms", justify="right")
        t.add_column("stop")
        t.add_column("spawned", justify="right")
        t.add_column("completed", justify="right")
        t.add_column("cost so far", justify="right")
        for s in an.snapshots:
            t.add_row(
                clock(s.at),
                f"{s.num_turns}",
                f"{s.duration_ms:,}" if s.duration_ms else "—",
                f"{s.ttft_ms:,}" if s.ttft_ms else "—",
                s.stop_reason or "—",
                f"{s.stats.get('spawned', '—')}",
                f"{s.stats.get('completed', '—')}",
                money(s.total_cost_usd),
            )
        console.print(t)


# ── json ─────────────────────────────────────────────────────────────────────


def to_json(an: Analysis) -> dict:
    during, own = an.blocked()
    total = an.totals()
    return {
        "file": str(an.path),
        "lines": an.lines,
        "malformed": an.malformed,
        "duplicate_result_lines": an.duplicates,
        "window": {"start_ms": an.start, "end_ms": an.end, "wall_ms": an.wall_ms},
        "agents": sorted(an.agents),
        "sessions": sorted(an.sessions),
        "events": {
            "directions": dict(an.dirs),
            "kinds": dict(an.kinds),
            "updates": dict(an.updates),
            "harness_lines": dict(an.raw_types),
            "tool_kinds": dict(an.tool_kinds),
            "tool_status": dict(an.tool_status),
        },
        "fan_out": {
            "subagents": len(an.subagents),
            "spawned_in_tape": len(an.spawned),
            "marked_ended": sum(1 for r in an.subagents if r.marked_status and not r.marked_status.endswith("*")),
            "peak_concurrent": an.peak_concurrent(),
            "busy_ms": an.busy_ms(),
            "parallel_ms": an.parallel_ms(),
            "time_at_concurrency": {str(k): v for k, v in sorted(an.time_at().items())},
            "conversation_lines_inside_a_subagent_window": during,
            "conversation_lines": own,
        },
        "turns": [{"seq": s, "at_ms": a, "stop_reason": r, "error": e} for s, a, r, e in an.turns],
        "results": [
            {
                "at_ms": s.at,
                "seq": s.seq,
                "duration_ms": s.duration_ms,
                "num_turns": s.num_turns,
                "stop_reason": s.stop_reason,
                "total_cost_usd": s.total_cost_usd,
                "turn_usage": s.usage,
                "model_usage": s.model_usage,
                "subagent_stats": s.stats,
                "ttft_ms": s.ttft_ms,
            }
            for s in an.snapshots
        ],
        "actors": [
            {
                "id": r.id,
                "label": r.label(),
                "is_conversation": r.is_main,
                "kind": r.kind,
                "model": r.model,
                "title": r.title,
                "task": r.task,
                "spawn": {"announced_in_tape": r.announced, "at_ms": r.spawned_at, "seq": r.spawned_seq},
                "first_work": {"at_ms": r.first_at, "seq": r.first_seq},
                "last_work": {"at_ms": r.last_at, "seq": r.last_seq},
                "marked": {"at_ms": r.marked_at, "seq": r.marked_seq, "status": r.marked_status},
                "worked_ms": r.worked,
                "lifetime_ms": r.lifetime,
                "mark_lag_ms": r.lag,
                "state": r.state(),
                "messages": r.messages,
                "chunks": r.chunks,
                "tool_calls": r.tool_calls,
                "tool_failed": r.tool_failed,
                "tools": dict(r.tools),
                "context_peak": r.context_peak,
                "context_size": r.context_size,
                "context_pct": r.context_pct,
                "tokens": {
                    "input": r.tokens.input,
                    "output": r.tokens.output,
                    "cache_read": r.tokens.cache_read,
                    "cache_creation": r.tokens.cache_creation,
                    "total": r.tokens.total,
                },
                "last_text": r.last_text[:2000],
            }
            for r in (an.runs[rid] for rid in an.order)
        ],
        "tokens_in_tape": {
            "input": total.input,
            "output": total.output,
            "cache_read": total.cache_read,
            "cache_creation": total.cache_creation,
            "total": total.total,
        },
        "rate_limits": [{"at_ms": a, **b} for a, b in an.rate_limits],
        "notes": an.notes,
    }


# ── cli ──────────────────────────────────────────────────────────────────────


def main() -> int:
    ap = argparse.ArgumentParser(
        prog="tape-subagents.py",
        description="When each subagent in a Ubiq tape spawned, stopped and was marked ended, and what it cost.",
    )
    ap.add_argument("tape", nargs="?", help="the capture to read; the newest tape in _data/ otherwise")
    ap.add_argument("--json", action="store_true", help="emit the analysis as JSON and print no report")
    ap.add_argument("--width", type=int, default=96, help="columns in the timeline (default 96)")
    ap.add_argument(
        "--tolerance",
        type=float,
        default=0.0,
        metavar="SECONDS",
        help="a mark landing within this of the last work is folded into it rather than read as a lag (0 = off)",
    )
    args = ap.parse_args()

    an = walk(find_tape(args.tape), int(args.tolerance * 1000))

    if args.json:
        print(json.dumps(to_json(an), indent=2))
        return 0

    print_header(an)
    if an.start is None:
        console.print(
            "\n[yellow]This tape holds no conversation — every line was skipped, or it is the "
            "request for a dump rather than the dump itself.[/]"
            if an.lines
            else "\n[yellow]This tape is empty.[/]"
        )
        return 0
    print_lifecycle(an)
    print_gantt(an, args.width)
    print_fanout(an)
    print_statistics(an)
    print_tokens(an)
    console.print()
    return 0


if __name__ == "__main__":
    sys.exit(main())
