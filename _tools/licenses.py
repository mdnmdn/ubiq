#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.12"
# dependencies = []
# ///
"""Third-party licence collector — writes `assets/third-party.json` for the About modal's Open source tab.

Run it through `just`: `just licenses` (regenerate the file), `just licenses-check` (fail if the
committed file is stale against `Cargo.lock`). `build`, `bundle` and `bundle-win` depend on
`licenses`, so building keeps the file current.

The source of truth is `cargo metadata --locked`: every package reachable from a workspace member
through normal and build dependencies (dev-only edges are skipped), across every target platform,
minus the workspace's own crates. Vendored third-party path crates (`vendor/…`) stay — they are
somebody else's code. The interface embeds the result with `include_bytes!`; nothing here is imported
by a crate and nothing talks to a network except `cargo metadata` itself when a crate is not yet
downloaded.

Output is deterministic — sorted by name then version, no timestamp — so regenerating on an
unchanged `Cargo.lock` is a zero diff. Web-panel vendor bundles (Excalidraw, draw.io) are not
included: their manifests under `crates/ubiq-host/src/web_assets/` carry hashes only, no licence.
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from collections import Counter
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]
OUT = REPO / "assets" / "third-party.json"
SCHEMA = 1

LICENSE_FILE_RE = re.compile(r"^(LICEN[CS]E|COPYING|NOTICE|UNLICENSE)", re.IGNORECASE)
# A real notice: "Copyright (c) 2020 Foo", "Copyright © Foo". Not the Apache appendix template.
COPYRIGHT_RE = re.compile(r"(?:copyright|©)\s*(?:\(c\)|©)?\s*\S", re.IGNORECASE)
COPYRIGHT_TEMPLATE_RE = re.compile(r"\[yyyy\]|<year>|\{yyyy\}|copyright (?:owner|holder|notice|license|and)\b", re.IGNORECASE)
SPDX_TOKEN_RE = re.compile(r"[A-Za-z0-9][A-Za-z0-9.\-+]*")
SPDX_OPERATORS = {"AND", "OR", "WITH"}


def die(message: str) -> None:
    print(f"licenses: {message}", file=sys.stderr)
    sys.exit(1)


def cargo_metadata() -> dict:
    """`--offline` first (no network, no surprises), then online for crates not yet downloaded."""
    base = ["cargo", "metadata", "--format-version", "1", "--locked"]
    for extra in (["--offline"], []):
        result = subprocess.run(base + extra, cwd=REPO, capture_output=True, text=True)
        if result.returncode == 0:
            return json.loads(result.stdout)
        last = result.stderr.strip()
    die(f"cargo metadata failed:\n{last}")
    raise AssertionError  # unreachable


def reachable(meta: dict) -> set[str]:
    """Package ids reachable from a workspace member over normal and build edges."""
    nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}
    seen: set[str] = set()
    stack = list(meta["workspace_members"])
    while stack:
        pid = stack.pop()
        if pid in seen:
            continue
        seen.add(pid)
        for dep in nodes[pid]["deps"]:
            # kind None is a normal dependency, "build" a build script's; "dev" is test-only.
            if any(k["kind"] in (None, "build") for k in dep["dep_kinds"]):
                stack.append(dep["pkg"])
    return seen


def spdx_ids(expression: str) -> list[str]:
    """Licence ids in order of appearance, deduplicated. A `WITH` exception counts as its own id."""
    ids: list[str] = []
    for token in SPDX_TOKEN_RE.findall(expression):
        if token.upper() in SPDX_OPERATORS or token in ids:
            continue
        ids.append(token)
    return ids


def spdx_urls(ids: list[str]) -> list[str]:
    return [f"https://spdx.org/licenses/{i}.html" for i in ids if not i.startswith(("LicenseRef-", "DocumentRef-"))]


def source_kind(source: str | None) -> str:
    if source is None:
        return "path"
    if source.startswith("registry+"):
        return "crates.io" if "crates.io" in source else "registry"
    if source.startswith("git+"):
        return "git"
    return "path"


def licence_files(pkg_dir: Path) -> list[Path]:
    return sorted(p for p in pkg_dir.iterdir() if LICENSE_FILE_RE.match(p.name) and p.is_file())


def copyright_line(path: Path) -> str | None:
    try:
        text = path.read_text(encoding="utf-8", errors="replace")
    except OSError:
        return None
    for raw in text.splitlines():
        line = raw.strip().lstrip("/#*;> ").strip()
        # A notice starts the line; prose mentioning "Copyright License" does not.
        if COPYRIGHT_RE.match(line) and not COPYRIGHT_TEMPLATE_RE.search(line):
            return line
    return None


def git_file_url(pkg: dict, rel: str) -> str | None:
    """A github blob URL for a git dependency, from the checkout path `…/checkouts/<repo>-<h>/<rev>/…`."""
    repo = (pkg.get("repository") or "").rstrip("/").removesuffix(".git")
    source = pkg["source"]
    if not repo.startswith("https://github.com/") or "#" not in source:
        return None
    rev = source.rsplit("#", 1)[1]
    parts = Path(pkg["manifest_path"]).parts
    if "checkouts" not in parts:
        return None
    sub = parts[parts.index("checkouts") + 3 : -1]  # past <repo>-<hash>/<short rev>
    return f"{repo}/blob/{rev}/" + "/".join((*sub, rel))


def entry(pkg: dict) -> dict:
    kind = source_kind(pkg["source"])
    pkg_dir = Path(pkg["manifest_path"]).parent
    files = licence_files(pkg_dir) if pkg_dir.is_dir() else []
    # Prefer a file that actually carries a notice (LICENSE-MIT over LICENSE-APACHE).
    chosen, copyright = None, None
    for f in files:
        line = copyright_line(f)
        if line:
            chosen, copyright = f, line
            break
    if chosen is None and files:
        chosen = files[0]

    license = pkg.get("license")
    if license:
        license = license.strip()
    elif pkg.get("license_file"):
        license = "see licence file"
    else:
        license = None

    file_url = None
    if chosen is not None:
        if kind == "crates.io":
            file_url = f"https://docs.rs/crate/{pkg['name']}/{pkg['version']}/source/{chosen.name}"
        elif kind == "git":
            file_url = git_file_url(pkg, chosen.name)

    return {
        "name": pkg["name"],
        "version": pkg["version"],
        "license": license,
        "license_urls": spdx_urls(spdx_ids(license)) if pkg.get("license") else [],
        "license_file_url": file_url,
        "repository": pkg.get("repository"),
        "homepage": pkg.get("homepage"),
        "authors": pkg.get("authors") or [],
        "copyright": copyright,
        "source": kind,
    }


def collect() -> list[dict]:
    meta = cargo_metadata()
    members = set(meta["workspace_members"])
    keep = reachable(meta)
    vendor = (REPO / "vendor").as_posix() + "/"
    out = []
    for pkg in meta["packages"]:
        if pkg["id"] not in keep:
            continue
        # The workspace's own crates are excluded; a vendored third-party one is not.
        if pkg["id"] in members and not Path(pkg["manifest_path"]).as_posix().startswith(vendor):
            continue
        out.append(entry(pkg))
    out.sort(key=lambda e: (e["name"], e["version"]))
    return out


def render(packages: list[dict]) -> str:
    return json.dumps({"schema": SCHEMA, "packages": packages}, indent=1, ensure_ascii=False) + "\n"


def tally(packages: list[dict]) -> str:
    counts = Counter(p["license"] or "unknown" for p in packages)
    return ", ".join(f"{k} {v}" for k, v in counts.most_common(5))


def cmd_generate(_: argparse.Namespace) -> None:
    packages = collect()
    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_text(render(packages), encoding="utf-8")
    print(f"{OUT.relative_to(REPO)}: {len(packages)} packages; top licences: {tally(packages)}")


def cmd_check(_: argparse.Namespace) -> None:
    fresh = collect()
    if not OUT.exists():
        die(f"{OUT.relative_to(REPO)} is missing — run `just licenses`")
    if OUT.read_text(encoding="utf-8") == render(fresh):
        print(f"{OUT.relative_to(REPO)}: up to date ({len(fresh)} packages)")
        return
    try:
        committed = json.loads(OUT.read_text(encoding="utf-8")).get("packages", [])
    except json.JSONDecodeError:
        committed = []
    old = {(p["name"], p["version"]): p for p in committed}
    new = {(p["name"], p["version"]): p for p in fresh}
    fmt = lambda keys: ", ".join(f"{n} {v}" for n, v in sorted(keys)) or "none"  # noqa: E731
    added, removed = new.keys() - old.keys(), old.keys() - new.keys()
    changed = {k for k in new.keys() & old.keys() if new[k] != old[k]}
    die(
        f"{OUT.relative_to(REPO)} is stale — run `just licenses`\n"
        f"  added:   {fmt(added)}\n  removed: {fmt(removed)}\n  changed: {fmt(changed)}"
    )


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    sub = parser.add_subparsers(dest="command", required=True)
    sub.add_parser("generate", help="write assets/third-party.json").set_defaults(run=cmd_generate)
    sub.add_parser("check", help="fail if the committed file is stale").set_defaults(run=cmd_check)
    args = parser.parse_args()
    args.run(args)


if __name__ == "__main__":
    main()
