#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.12"
# dependencies = ["httpx", "rich"]
# ///
"""Web-panel vendor bundle snapshotter — mirrors Excalidraw from jsDelivr and emits the manifest.

Run it through `just`: `just web-assets`, `just web-assets-verify`.

`snapshot` walks two sources with one generating rule each — Excalidraw's own `dist/prod`, enumerated
from the jsDelivr package API, and the transitive `+esm` closure of the bare specifiers its chunks
import — downloads both into a scratch directory outside the repo, and writes the expected SHA-256 of
every file as generated Rust at `crates/ubiq-host/src/web_assets/manifest.rs`. Nothing is downloaded
into the repo and no downloaded byte is ever rewritten: the cache path mirrors the URL path exactly,
which is what lets the closure's own `/npm/…` imports resolve through one import-map prefix rule.

`verify` re-fetches every URL in the committed manifest and reports drift — cheap to run when
jsDelivr regenerates a `+esm` bundle, which is the event worth failing on.
"""

from __future__ import annotations

import argparse
import base64
import hashlib
import json
import os
import re
import sys
import tempfile
import threading
from concurrent.futures import ThreadPoolExecutor
from dataclasses import dataclass
from pathlib import Path
from urllib.parse import urljoin

import httpx
from rich.console import Console
from rich.progress import BarColumn, Progress, TextColumn, TimeElapsedColumn
from rich.table import Table

REPO = Path(__file__).resolve().parents[1]
MANIFEST = REPO / "crates/ubiq-host/src/web_assets/manifest.rs"

APP = "excalidraw"
PACKAGE = "@excalidraw/excalidraw"
DEFAULT_VERSION = "0.18.0"

DATA_API = "https://data.jsdelivr.com/v1/packages/npm"
CDN = "https://cdn.jsdelivr.net"

# The chrome serves the mirror under §4.1's `vendor/` route, beside its own `index.html`. Import-map
# values are document-relative because the map is inlined in that page.
VENDOR_PREFIX = "./vendor/"

# React is a peer dependency (`^17 || ^18 || ^19`), so its version is not pinned by Excalidraw and is
# resolved at snapshot time. Every other seed takes Excalidraw's own pin.
PEER_ROOTS = ("react", "react-dom")
# The four entry points the closure and Excalidraw between them ask for by name.
REACT_ENTRIES = ("react", "react/jsx-runtime", "react/jsx-dev-runtime", "react-dom/client")

SCRIPT_SUFFIXES = {".js", ".mjs", ".cjs"}
FONT_SUFFIXES = (".woff2", ".woff", ".ttf", ".otf")

CONCURRENCY = 12
USER_AGENT = "ubiq-webassets/1 (+https://github.com/ubiq)"

console = Console()


# ── fetching ───────────────────────────────────────────────────────────────────────────────────


def scratch_root() -> Path:
    """Where downloads live. Outside the repo, always — a re-run is fast, a commit is clean.

    The cache directory if it is writable, the temp directory if it is not: a confined run still
    works, it just loses the cache between machines rebooting.
    """
    env = os.environ.get("UBIQ_WEBASSETS_CACHE")
    if env:
        return Path(env).expanduser()
    base = os.environ.get("XDG_CACHE_HOME")
    cache = (Path(base) if base else Path.home() / ".cache") / "ubiq-webassets"
    try:
        cache.mkdir(parents=True, exist_ok=True)
    except OSError:
        return Path(tempfile.gettempdir()) / "ubiq-webassets"
    return cache


@dataclass
class Asset:
    """One mirrored file: the URL it came from, the cache-relative path, its bytes' digest."""

    path: str
    url: str
    sha256: str
    len: int
    published: str | None = None  # jsDelivr's stable hash, for `dist/prod` only


@dataclass
class Fetcher:
    scratch: Path
    client: httpx.Client
    refetch: bool = False
    hits: int = 0
    misses: int = 0

    def get(self, url_path: str) -> bytes:
        """Fetch `CDN + url_path`, caching under the scratch directory at the very same path."""
        local = self.scratch / url_path.lstrip("/")
        if not self.refetch and local.is_file():
            self.hits += 1
            return local.read_bytes()
        response = self.client.get(CDN + url_path, follow_redirects=True)
        response.raise_for_status()
        body = response.content
        local.parent.mkdir(parents=True, exist_ok=True)
        tmp = local.with_name(f"{local.name}.{os.getpid()}.{threading.get_ident()}.part")
        tmp.write_bytes(body)
        tmp.replace(local)
        self.misses += 1
        return body


def progress_bar() -> Progress:
    return Progress(
        TextColumn("[bold]{task.description}"),
        BarColumn(),
        TextColumn("{task.completed}/{task.total}"),
        TimeElapsedColumn(),
        console=console,
    )


def fetch_all(fetcher: Fetcher, url_paths: list[str], description: str) -> dict[str, bytes]:
    """Download a batch concurrently but politely, showing progress."""
    bodies: dict[str, bytes] = {}
    with progress_bar() as progress:
        task = progress.add_task(description, total=len(url_paths))
        with ThreadPoolExecutor(max_workers=CONCURRENCY) as pool:
            for url_path, body in zip(url_paths, pool.map(fetcher.get, url_paths), strict=True):
                bodies[url_path] = body
                progress.advance(task)
    return bodies


# ── part one: Excalidraw's own dist/prod ───────────────────────────────────────────────────────


def package_manifest(client: httpx.Client, package: str, version: str) -> dict:
    response = client.get(f"{DATA_API}/{package}@{version}", follow_redirects=True)
    response.raise_for_status()
    return response.json()


def latest_version(client: httpx.Client, package: str) -> str:
    response = client.get(f"{DATA_API}/{package}", follow_redirects=True)
    response.raise_for_status()
    return response.json()["tags"]["latest"]


def flatten(nodes: list[dict], prefix: str = "") -> list[tuple[str, str, int]]:
    """The package API's nested tree as (path, base64 sha-256, size) triples."""
    out: list[tuple[str, str, int]] = []
    for node in nodes:
        path = f"{prefix}/{node['name']}"
        if node["type"] == "directory":
            out.extend(flatten(node["files"], path))
        else:
            out.append((path, node.get("hash", ""), node.get("size", 0)))
    return out


def snapshot_excalidraw(fetcher: Fetcher, version: str) -> tuple[list[Asset], dict[str, bytes]]:
    """Every file under `dist/prod`, verified against jsDelivr's published per-file SHA-256."""
    data = package_manifest(fetcher.client, PACKAGE, version)
    files = [f for f in flatten(data["files"]) if f[0].startswith("/dist/prod/")]
    if not files:
        die(f"no dist/prod files in the package manifest for {PACKAGE}@{version}")

    base = f"/npm/{PACKAGE}@{version}"
    url_paths = [base + path for path, _, _ in files]
    bodies = fetch_all(fetcher, url_paths, "excalidraw dist/prod")

    assets: list[Asset] = []
    for (path, published_b64, _size), url_path in zip(files, url_paths, strict=True):
        body = bodies[url_path]
        digest = hashlib.sha256(body).hexdigest()
        published = base64.b64decode(published_b64).hex() if published_b64 else None
        if published and published != digest:
            die(f"published hash mismatch for {url_path}\n  jsDelivr {published}\n  fetched  {digest}")
        assets.append(
            Asset(
                path=url_path.lstrip("/"),
                url=CDN + url_path,
                sha256=digest,
                len=len(body),
                published=published,
            )
        )
    return assets, bodies


# ── part two: the dependency closure ───────────────────────────────────────────────────────────

# Minified ESM leaves a module specifier in exactly three shapes. `from` immediately followed by a
# quote is never anything else — `Array.from("x")` has a paren in the way.
IMPORT_RE = re.compile(
    r"""\bfrom\s*['"]([^'"]+)['"]"""
    r"""|\bimport\s*\(\s*['"]([^'"]+)['"]\s*\)"""
    r"""|\bimport\s*['"]([^'"]+)['"]"""
)
NPM_PATH_RE = re.compile(r"""['"](/npm/[^'"\s]+)['"]""")
SPEC_RE = re.compile(r"^(@[^/]+/[^/]+|[^@/][^/]*)(/.*)?$")


def bare_specifiers(bodies: dict[str, bytes]) -> set[str]:
    """Every bare specifier a browser cannot resolve, scanned out of the bytes. Never hardcoded."""
    found: set[str] = set()
    for url_path, body in bodies.items():
        if Path(url_path).suffix not in SCRIPT_SUFFIXES:
            continue
        text = body.decode("utf8", "replace")
        for match in IMPORT_RE.finditer(text):
            spec = match.group(1) or match.group(2) or match.group(3)
            if spec and not spec.startswith((".", "/", "http:", "https:", "data:", "node:")):
                found.add(spec)
    return found


def split_specifier(spec: str) -> tuple[str, str]:
    match = SPEC_RE.match(spec)
    if not match:
        die(f"unparseable bare specifier: {spec!r}")
    return match.group(1), match.group(2) or ""


def resolve_versions(
    client: httpx.Client, specs: set[str], pins: dict[str, str]
) -> dict[str, str]:
    """Excalidraw's own pin for each package; the latest release for a peer it only ranges."""
    versions: dict[str, str] = {}
    for spec in sorted(specs):
        package, _ = split_specifier(spec)
        if package in versions:
            continue
        pin = pins.get(package)
        if pin and re.fullmatch(r"\d+\.\d+\.\d+(?:[-+].*)?", pin):
            versions[package] = pin
        else:
            versions[package] = latest_version(client, package)
            reason = "peer range" if pin else "undeclared"
            console.print(f"  [yellow]{package}[/yellow] {reason} → resolved {versions[package]}")
    return versions


def esm_url_path(package: str, version: str, subpath: str) -> str:
    return f"/npm/{package}@{version}{subpath}/+esm"


def close_dependencies(
    fetcher: Fetcher, seeds: list[str]
) -> tuple[list[Asset], dict[str, bytes]]:
    """Fetch each seed's `+esm`, enqueue every `/npm/…` in the body, until nothing new appears."""
    seen: set[str] = set()
    queue = list(dict.fromkeys(seeds))
    bodies: dict[str, bytes] = {}

    with progress_bar() as progress:
        task = progress.add_task("dependency closure", total=len(queue))
        with ThreadPoolExecutor(max_workers=CONCURRENCY) as pool:
            while queue:
                batch = [u for u in dict.fromkeys(queue) if u not in seen]
                queue = []
                seen.update(batch)
                if not batch:
                    break
                for url_path, body in zip(batch, pool.map(fetcher.get, batch), strict=True):
                    bodies[url_path] = body
                    text = body.decode("utf8", "replace")
                    for found in NPM_PATH_RE.findall(text):
                        if found not in seen:
                            queue.append(found)
                            progress.update(task, total=progress.tasks[task].total + 1)
                    progress.advance(task)

    assets = [
        Asset(
            path=url_path.lstrip("/"),
            url=CDN + url_path,
            sha256=hashlib.sha256(body).hexdigest(),
            len=len(body),
        )
        for url_path, body in sorted(bodies.items())
    ]
    return assets, bodies


# ── the import map ─────────────────────────────────────────────────────────────────────────────


def build_import_map(
    specs: set[str],
    versions: dict[str, str],
    closure_bodies: dict[str, bytes],
    react_version: str,
) -> tuple[dict[str, str], list[str]]:
    """The bare specifiers, the React entry points, the prefix rule, and the collapse entries.

    `+esm` output hard-pins whichever React each transitive package was built against, so the raw
    closure carries several React and react-dom copies that Radix statically imports — "Invalid hook
    call". The collapse half is scanned out of the downloaded bytes, never hardcoded, because those
    inner pins are jsDelivr's build-time choices and move when it regenerates.
    """
    imports: dict[str, str] = {}

    # One prefix rule carries every absolute `/npm/…` import the closure's own bodies contain, which
    # is what lets the mirror stay byte-identical to what the CDN served.
    imports["/npm/"] = f"{VENDOR_PREFIX}npm/"

    for spec in sorted(specs | set(REACT_ENTRIES)):
        package, subpath = split_specifier(spec)
        version = react_version if package in PEER_ROOTS else versions.get(package)
        if version is None:
            die(f"no version resolved for {spec!r}")
        imports[spec] = VENDOR_PREFIX + esm_url_path(package, version, subpath).lstrip("/")

    collapse: list[str] = []
    pinned: set[str] = set()
    for body in closure_bodies.values():
        text = body.decode("utf8", "replace")
        for found in NPM_PATH_RE.findall(text):
            match = re.match(r"^/npm/(react|react-dom)@([^/]+)(/.*)?$", found)
            if match and match.group(2) != react_version:
                pinned.add(found)
    for found in sorted(pinned):
        match = re.match(r"^/npm/(react|react-dom)@[^/]+(/.*)?$", found)
        assert match
        target = f"/npm/{match.group(1)}@{react_version}{match.group(2) or ''}"
        imports[found] = VENDOR_PREFIX + target.lstrip("/")
        collapse.append(found)

    return imports, collapse


# ── the completeness check ─────────────────────────────────────────────────────────────────────


def asset_subpath(assets: list[Asset]) -> str:
    """The manifest-relative directory that contains `fonts/` — what EXCALIDRAW_ASSET_PATH names."""
    dirs = {a.path.split("/fonts/")[0] + "/" for a in assets if "/fonts/" in a.path}
    if len(dirs) != 1:
        die(f"expected exactly one directory containing fonts/, found {sorted(dirs)}")
    return dirs.pop()


def check_fonts(assets: list[Asset], bodies: dict[str, bytes], subpath: str) -> int:
    """Assert every font the tree reaches for is mirrored.

    Any missing font is a silent network attempt — Excalidraw always appends its own `esm.sh` path to
    the candidate list — so an incomplete mirror leaks rather than fails. This is the check that
    catches it before the bundle ships.
    """
    have = {a.path for a in assets}
    referenced: set[str] = set()
    missing: list[str] = []

    css = [p for p in bodies if p.endswith("/dist/prod/index.css")]
    if not css:
        die("dist/prod/index.css not found — cannot check the four relative Assistant faces")
    css_text = bodies[css[0]].decode("utf8", "replace")
    css_base = css[0].rsplit("/", 1)[0] + "/"
    relative = re.findall(r"""url\(\s*['"]?(\.[^'")]+\.(?:woff2|woff|ttf|otf))['"]?\s*\)""", css_text)
    for ref in relative:
        resolved = urljoin("https://x" + css_base, ref)[len("https://x/") :]
        referenced.add(resolved)
        if resolved not in have:
            missing.append(f"index.css url({ref})")
    if len(relative) < 4:
        die(f"index.css names {len(relative)} relative font faces, expected the four Assistant faces")

    # Every font URI anywhere in the tree, resolved against the asset path the chrome will set.
    uri_re = re.compile(r"""['"]((?:\./)?[A-Za-z0-9_\-./]+\.(?:woff2|woff|ttf|otf))['"]""")
    for url_path, body in bodies.items():
        if Path(url_path).suffix not in SCRIPT_SUFFIXES and not url_path.endswith(".css"):
            continue
        for ref in uri_re.findall(body.decode("utf8", "replace")):
            resolved = subpath + ref.removeprefix("./").lstrip("/")
            referenced.add(resolved)
            if resolved not in have:
                missing.append(f"{url_path} → {ref}")

    if missing:
        console.print("[red]incomplete mirror — these fonts are referenced but not cached:[/red]")
        for item in sorted(set(missing))[:40]:
            console.print(f"  {item}")
        die(f"{len(set(missing))} font references are missing from the manifest")
    return len(referenced)


# ── emitting ───────────────────────────────────────────────────────────────────────────────────


def rust_escape(value: str) -> str:
    return value.replace("\\", "\\\\").replace('"', '\\"')


def render_manifest(
    version: str,
    bundle_version: str,
    assets: list[Asset],
    imports: dict[str, str],
    subpath: str,
) -> str:
    total = sum(a.len for a in assets)
    import_map = json.dumps({"imports": imports}, indent=2, sort_keys=False)
    lines = [
        "//! The Excalidraw offline mirror — every file the web panel's vendor cache must hold, with",
        "//! the SHA-256 the host verifies each download against.",
        "//!",
        "//! Two halves, earning their hashes differently. Excalidraw's own `dist/prod` files carry",
        "//! jsDelivr's published, stable per-file digest, checked at snapshot time against the bytes",
        "//! actually served. The dependency closure is `+esm` output, which jsDelivr generates on",
        "//! demand and says not to pin with subresource integrity, so these are *our* hashes taken",
        "//! once at snapshot — a later mismatch means the CDN regenerated the bundle, which is the",
        "//! event worth failing on.",
        "//!",
        "//! [`VERSION`] is the cache directory name. It changes whenever the snapshot changes, so a",
        "//! build that pins a new one looks in a directory that does not exist, fetches, and leaves",
        "//! the old one behind: versioned by directory, not invalidated by policy.",
        "//!",
        "//! [`Entry::path`] mirrors the URL path exactly, which is what makes zero content rewriting",
        "//! possible — the closure's own absolute `/npm/…` imports resolve through the single prefix",
        "//! rule at the head of [`IMPORT_MAP`]. No downloaded byte is ever rewritten.",
        "//!",
        "//! @generated by `_tools/webassets.py` — do not edit by hand.",
        "//! Regenerate with `just web-assets`; check for CDN drift with `just web-assets-verify`.",
        "",
        "/// One mirrored file: where it goes in the cache, where it came from, and what it must hash to.",
        "pub struct Entry {",
        "    pub path: &'static str,",
        "    pub url: &'static str,",
        "    pub sha256: &'static str,",
        "    pub len: u64,",
        "}",
        "",
        "/// The web app this bundle belongs to — the `<app>` segment of the cache path and the route.",
        f'pub const APP: &str = "{APP}";',
        "",
        "/// The bundle version: Excalidraw's release plus a digest over the whole snapshot.",
        f'pub const VERSION: &str = "{bundle_version}";',
        "",
        "/// The upstream package release this snapshot was taken from.",
        f'pub const PACKAGE_VERSION: &str = "{version}";',
        "",
        "/// Manifest-relative directory containing `fonts/`. The chrome makes an *absolute* URL of it",
        "/// for `window.EXCALIDRAW_ASSET_PATH`: a `/`-relative value resolves against `location.origin`",
        "/// and throws where there is none, and any font it fails to find is a silent network attempt.",
        f'pub const ASSET_SUBPATH: &str = "{subpath}";',
        "",
        "/// Every byte of the mirror, for a progress report and a disk-space check.",
        f"pub const TOTAL_BYTES: u64 = {total};",
        "",
        f"/// The {len(assets)} files of the mirror, sorted by cache path.",
        "pub const FILES: &[Entry] = &[",
    ]
    for asset in sorted(assets, key=lambda a: a.path):
        lines.append(
            f'    Entry {{ path: "{rust_escape(asset.path)}", '
            f'url: "{rust_escape(asset.url)}", '
            f'sha256: "{asset.sha256}", len: {asset.len} }},'
        )
    lines += [
        "];",
        "",
        "/// The import map the chrome inlines. A browser cannot resolve Excalidraw's bare specifiers,",
        "/// and `+esm` output hard-pins whichever React each transitive package was built against — so",
        "/// the collapse entries fold every stray React and react-dom copy onto the one chosen here.",
        "/// That is correct rather than a hack: React is a peer dependency, so npm and every bundler",
        "/// already dedupe it for every real Excalidraw user.",
        "///",
        "/// Values are document-relative to the chrome page, which serves the mirror under `vendor/`.",
        "pub const IMPORT_MAP: &str = r#\"" + import_map + "\"#;",
        "",
    ]
    return "\n".join(lines)


# ── actions ────────────────────────────────────────────────────────────────────────────────────


def rel_to_repo(path: Path) -> str:
    try:
        return str(path.resolve().relative_to(REPO))
    except ValueError:
        return str(path)


def die(message: str) -> None:
    console.print(f"[red]{message}[/red]")
    sys.exit(1)


def client() -> httpx.Client:
    return httpx.Client(
        headers={"User-Agent": USER_AGENT},
        timeout=httpx.Timeout(60.0, connect=15.0),
        limits=httpx.Limits(max_connections=CONCURRENCY, max_keepalive_connections=CONCURRENCY),
        http2=False,
    )


def run_snapshot(version: str, out: Path, react: str | None, refetch: bool) -> int:
    scratch = scratch_root()
    scratch.mkdir(parents=True, exist_ok=True)
    console.print(f"[bold]snapshot[/bold] {PACKAGE}@{version}  [dim]scratch {scratch}[/dim]\n")

    with client() as http:
        fetcher = Fetcher(scratch=scratch, client=http, refetch=refetch)

        own, own_bodies = snapshot_excalidraw(fetcher, version)
        console.print(
            f"  {len(own)} files, {sum(a.len for a in own):,} bytes, "
            f"all verified against jsDelivr's published hash\n"
        )

        meta = http.get(f"{CDN}/npm/{PACKAGE}@{version}/package.json", follow_redirects=True)
        meta.raise_for_status()
        package_json = meta.json()
        pins = {
            **package_json.get("peerDependencies", {}),
            **package_json.get("dependencies", {}),
        }

        specs = bare_specifiers(own_bodies)
        console.print(f"  {len(specs)} bare specifiers scanned out of dist/prod")
        versions = resolve_versions(http, specs, pins)
        react_version = react or versions.get("react") or latest_version(http, "react")
        for root in PEER_ROOTS:
            versions[root] = react_version
        console.print(f"  react pinned at {react_version}\n")

        seeds = []
        for spec in sorted(specs | set(REACT_ENTRIES)):
            package, sub = split_specifier(spec)
            seeds.append(esm_url_path(package, versions[package], sub))
        closure, closure_bodies = close_dependencies(fetcher, seeds)
        console.print(
            f"  {len(closure)} files, {sum(a.len for a in closure):,} bytes in the closure\n"
        )

    assets = own + closure
    paths = [a.path for a in assets]
    if len(set(paths)) != len(paths):
        die("two assets claim the same cache path")

    subpath = asset_subpath(own)
    fonts = check_fonts(assets, {**own_bodies, **closure_bodies}, subpath)
    console.print(f"  [green]mirror complete[/green] — {fonts} font URIs all present\n")

    imports, collapse = build_import_map(specs, versions, closure_bodies, react_version)

    digest = hashlib.sha256(
        "\n".join(f"{a.path} {a.sha256}" for a in sorted(assets, key=lambda a: a.path)).encode()
    ).hexdigest()[:12]
    bundle_version = f"{version}-{digest}"

    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(render_manifest(version, bundle_version, assets, imports, subpath))

    table = Table(show_header=False, box=None, pad_edge=False)
    table.add_row("files", f"{len(assets)}")
    table.add_row("bytes", f"{sum(a.len for a in assets):,} ({sum(a.len for a in assets) / 2**20:.1f} MiB)")
    table.add_row("import map", f"{len(imports)} entries, {len(collapse)} of them collapse")
    table.add_row("VERSION", bundle_version)
    table.add_row("ASSET_SUBPATH", subpath)
    table.add_row("written", rel_to_repo(out))
    console.print(table)
    console.print(f"\n[dim]cache {fetcher.hits} hits, {fetcher.misses} downloads[/dim]")
    return 0


ENTRY_RE = re.compile(
    r'Entry \{ path: "([^"]+)", url: "([^"]+)", sha256: "([0-9a-f]{64})", len: (\d+) \}'
)


def run_verify(out: Path) -> int:
    if not out.is_file():
        die(f"no manifest at {out} — run `just web-assets` first")
    text = out.read_text()
    entries = ENTRY_RE.findall(text)
    if not entries:
        die(f"no entries parsed out of {out}")
    console.print(f"[bold]verify[/bold] {len(entries)} files against {rel_to_repo(out)}\n")

    scratch = scratch_root() / "verify"
    scratch.mkdir(parents=True, exist_ok=True)
    drift: list[tuple[str, str]] = []
    with client() as http:
        fetcher = Fetcher(scratch=scratch, client=http, refetch=True)
        url_paths = [url[len(CDN) :] for _, url, _, _ in entries]
        bodies = fetch_all(fetcher, url_paths, "re-fetching")
    for (path, url, sha, length), url_path in zip(entries, url_paths, strict=True):
        body = bodies[url_path]
        got = hashlib.sha256(body).hexdigest()
        if got != sha:
            drift.append((path, f"sha256 {sha[:12]}… → {got[:12]}…"))
        elif len(body) != int(length):
            drift.append((path, f"len {length} → {len(body)}"))

    if drift:
        console.print(f"[red]{len(drift)} of {len(entries)} files drifted[/red]")
        for path, detail in drift[:40]:
            console.print(f"  {path} — {detail}")
        if len(drift) > 40:
            console.print(f"  … and {len(drift) - 40} more")
        console.print("\nthe CDN regenerated a bundle; re-run `just web-assets`")
        return 1
    console.print(f"[green]no drift[/green] — all {len(entries)} files match the manifest")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=["snapshot", "verify"])
    parser.add_argument("--version", default=DEFAULT_VERSION, help="Excalidraw release to mirror")
    parser.add_argument("--react", help="pin React explicitly instead of resolving the latest")
    parser.add_argument("--out", type=Path, default=MANIFEST, help="generated manifest path")
    parser.add_argument("--refetch", action="store_true", help="ignore the scratch cache")
    args = parser.parse_args()

    try:
        match args.action:
            case "snapshot":
                return run_snapshot(args.version, args.out, args.react, args.refetch)
            case "verify":
                return run_verify(args.out)
    except httpx.HTTPError as error:
        die(f"network: {error}")
    return 2


if __name__ == "__main__":
    sys.exit(main())
