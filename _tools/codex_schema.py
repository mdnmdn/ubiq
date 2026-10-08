#!/usr/bin/env python3
"""The Codex app-server's method set, against the snapshot the bridge was written to.

`codex app-server generate-json-schema` emits the protocol of whichever `codex` is installed. This
regenerates it into a temporary directory, lists every method by direction, and diffs the list
against `crates/agent-manager/tests/fixtures/codex-app-server/methods.txt`. A difference is not a
failure of the tree — it is the list of things `src/io/codex.rs` may now be missing or reading
under a dead name. `--write` replaces the snapshot after the bridge has been checked against it.

Run it through `just codex-schema-diff [--write]`.
"""

from __future__ import annotations

import argparse
import difflib
import json
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SNAPSHOT = ROOT / "crates/agent-manager/tests/fixtures/codex-app-server/methods.txt"
FILES = {
    "client-request": "ClientRequest.json",
    "client-notification": "ClientNotification.json",
    "server-notification": "ServerNotification.json",
    "server-request": "ServerRequest.json",
}


def methods(schema_dir: Path) -> list[str]:
    lines: list[str] = []
    for direction, name in FILES.items():
        schema = json.loads((schema_dir / name).read_text())
        for variant in schema.get("oneOf", schema.get("anyOf", [])):
            method = variant.get("properties", {}).get("method", {})
            for value in method.get("enum", []) or ([method["const"]] if "const" in method else []):
                lines.append(f"{direction} {value}")
    return sorted(set(lines))


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--write", action="store_true", help="replace the committed snapshot")
    args = parser.parse_args()

    version = subprocess.run(
        ["codex", "--version"], capture_output=True, text=True, check=True
    ).stdout.strip().splitlines()[-1]
    with tempfile.TemporaryDirectory() as out:
        subprocess.run(
            ["codex", "app-server", "generate-json-schema", "--out", out],
            capture_output=True,
            check=True,
        )
        current = methods(Path(out))

    header = f"# {version} — regenerate with `just codex-schema-diff --write`"
    if args.write:
        SNAPSHOT.write_text("\n".join([header, *current]) + "\n")
        print(f"wrote {len(current)} methods for {version} to {SNAPSHOT.relative_to(ROOT)}")
        return 0

    committed = [
        line for line in SNAPSHOT.read_text().splitlines() if line and not line.startswith("#")
    ]
    diff = list(difflib.unified_diff(committed, current, "snapshot", version, lineterm="", n=0))
    if not diff:
        print(f"{version}: {len(current)} methods, same as the snapshot")
        return 0
    print("\n".join(diff))
    return 1


if __name__ == "__main__":
    sys.exit(main())
