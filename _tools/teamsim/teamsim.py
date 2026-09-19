#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.12"
# dependencies = ["pillow"]
# ///
"""The teams graph's block positioning, rendered as PNGs you can look at.

Run it through `just`: `just teamsim --all-scenarios --algo all --sheet`. The algorithms in
`algos.py` are a port of `crates/ubiq/src/state/layout.rs`, so what a picture shows is what the app
does; `--selftest` is the evidence for that claim.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

from algos import ALGOS, layout_auto, load  # noqa: E402
from metrics import measure, table  # noqa: E402
from render import BG, render, sheet  # noqa: E402

SCENARIOS = HERE / "scenarios"


def _scenarios(names: list[str], every: bool) -> list[Path]:
    if every or not names:
        return sorted(SCENARIOS.glob("*.json"))
    out = []
    for name in names:
        path = Path(name)
        if not path.exists():
            path = SCENARIOS / (name if name.endswith(".json") else f"{name}.json")
        if not path.exists():
            raise SystemExit(f"no scenario {name!r} — try --list")
        out.append(path)
    return out


def _algos(names: list[str]) -> list[str]:
    if not names or "all" in names:
        return list(ALGOS)
    for name in names:
        if name not in ALGOS:
            raise SystemExit(f"unknown algorithm {name!r} — have {', '.join(ALGOS)}")
    return names


def main() -> int:
    ap = argparse.ArgumentParser(prog="teamsim", description=__doc__)
    ap.add_argument("--scenario", action="append", default=[], metavar="PATH|NAME")
    ap.add_argument("--all-scenarios", action="store_true")
    ap.add_argument("--algo", action="append", default=[], metavar="NAME")
    ap.add_argument("--out", type=Path, default=HERE / "out")
    ap.add_argument("--sheet", action="store_true", help="also compose one contact sheet")
    ap.add_argument(
        "--incremental",
        action="store_true",
        help="also render the growth frames of every arrangement that grows",
    )
    ap.add_argument("--max-width", type=int, default=1600)
    ap.add_argument("--json", action="store_true", help="the metrics as JSON, not a table")
    ap.add_argument("--selftest", action="store_true")
    ap.add_argument("--list", action="store_true")
    args = ap.parse_args()

    if args.selftest:
        import test_algos

        return 1 if test_algos.run() else 0

    if args.list:
        print("scenarios:")
        for path in sorted(SCENARIOS.glob("*.json")):
            scen = load(path)
            print(f"  {scen.name:<18} {scen.note}")
        print("algorithms:")
        for algo in ALGOS.values():
            print(f"  {algo.key:<18} {algo.hint}")
        return 0

    args.out.mkdir(parents=True, exist_ok=True)
    rows: list[dict] = []
    cells: list[list[tuple[str, object]]] = []

    for path in _scenarios(args.scenario, args.all_scenarios):
        scen = load(path)
        line: list[tuple[str, object]] = []
        for key in _algos(args.algo):
            arr = layout_auto(scen, key)
            stats = measure(arr)
            rows.append(stats)
            img = render(arr, stats, max_w=args.max_width)
            img.save(args.out / f"{scen.name}-{key}.png")
            line.append((f"{scen.name} · {arr.algo.label}", img))
            if args.incremental and arr.frames:
                for ix, frame in enumerate(arr.frames, 1):
                    shot = measure(frame)
                    img = render(frame, shot, max_w=args.max_width)
                    img.save(args.out / f"{scen.name}-{key}-frame{ix}.png")
                    line.append((f"{scen.name} · {arr.algo.label} frame {ix}", img))
        cells.append(line)

    if args.sheet and cells:
        cols = max(len(line) for line in cells)
        from PIL import Image

        flat: list[tuple[str, object]] = []
        for line in cells:
            flat += line + [("", Image.new("RGB", (8, 8), BG))] * (cols - len(line))
        sheet(flat, cols).save(args.out / "sheet.png")

    (args.out / "metrics.json").write_text(json.dumps(rows, indent=2) + "\n")
    print(json.dumps(rows, indent=2) if args.json else table(rows))
    print(f"\n{len(rows)} arrangements → {args.out}", file=sys.stderr)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
