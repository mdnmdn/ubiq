#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.12"
# dependencies = ["httpx", "rich"]
# ///
"""Web-panel vendor bundle snapshotter — mirrors a tenant's vendor tree from jsDelivr and emits its manifest.

Run it through `just`: `just web-assets` / `web-assets-drawio` (snapshot), `web-assets-verify` /
`web-assets-verify-drawio` (drift check). `--tenant` picks which one `snapshot` walks; `verify` reads
whatever manifest `--out` points at and needs no tenant of its own.

Two tenants, two generating rules. `excalidraw` walks Excalidraw's own `dist/prod`, enumerated from
the jsDelivr npm package API, plus the transitive `+esm` closure of the bare specifiers its chunks
import, and folds the stray React copies the closure drags in onto one via the import map. `drawio`
walks `src/main/webapp/` off the GitHub tree API — draw.io's webapp is not on npm — and fetches every
kept file from jsDelivr's GitHub CDN mirror; it has no import map to build and no font check to run,
because it loads through classic `<script>` tags rather than ES modules.

Both write into a scratch directory outside the repo and never rewrite a downloaded byte: the cache
path mirrors the URL path (or, for `drawio`, the webapp-relative path) exactly, so nothing here has
to rewrite an import for the mirror to work.
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
import time
import concurrent.futures as cf
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

# draw.io's webapp is not published to npm, so it is a second tenant with a second generating rule
# below (`snapshot_drawio`), not a second call into the npm walk above.
DRAWIO_APP = "drawio"
DRAWIO_REPO = "jgraph/drawio"
DRAWIO_DEFAULT_VERSION = "31.4.5"
DRAWIO_MANIFEST = REPO / "crates/ubiq-host/src/web_assets/manifest_drawio.rs"
DRAWIO_WEBAPP_PREFIX = "src/main/webapp/"

DATA_API = "https://data.jsdelivr.com/v1/packages/npm"
GITHUB_API = "https://api.github.com"
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


# A status the CDN hands out for a file it has not warmed yet rather than one that does not exist.
# jsDelivr's GitHub mirror serves a hard 403 ("package size exceeded") for any file of an
# over-50-MB ref it has not already cached at that edge, and that cache fills in as *other*
# requests for the same file land — including our own retries — so it is a warm-up cost, not a
# permanent refusal, and `fetch_all`'s round retry below is what pays it.
TRANSIENT_STATUS = {403, 429, 500, 502, 503, 504}


class Transient(Exception):
    """A fetch that failed for a reason `fetch_all`'s retry rounds may resolve."""


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
        if response.status_code in TRANSIENT_STATUS:
            raise Transient(f"{response.status_code}: {response.text[:200]}")
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


# Wait between retry rounds, one round per entry. A file that is still cold after all of these has
# had roughly three minutes of the CDN's own traffic — ours included — to warm it.
RETRY_WAITS = (3, 6, 12, 24, 45, 60)


def fetch_all(fetcher: Fetcher, url_paths: list[str], description: str) -> dict[str, bytes]:
    """Download a batch concurrently but politely, showing progress.

    A file that comes back [`Transient`] rejoins the next round rather than failing the batch: one
    slow file must not cost the 577 that landed fine, and a round gives the CDN time to catch up
    rather than hammering the same cold file back to back.
    """
    bodies: dict[str, bytes] = {}
    pending = list(dict.fromkeys(url_paths))
    with progress_bar() as progress:
        task = progress.add_task(description, total=len(pending))
        for round_number in range(len(RETRY_WAITS) + 1):
            if not pending:
                break
            if round_number > 0:
                wait = RETRY_WAITS[round_number - 1]
                console.print(
                    f"  [yellow]{len(pending)} file(s) not ready yet — retrying in {wait}s "
                    f"(round {round_number}/{len(RETRY_WAITS)})[/yellow]"
                )
                time.sleep(wait)
            batch, pending = pending, []
            with ThreadPoolExecutor(max_workers=CONCURRENCY) as pool:
                futures = {pool.submit(fetcher.get, url_path): url_path for url_path in batch}
                for future in cf.as_completed(futures):
                    url_path = futures[future]
                    try:
                        bodies[url_path] = future.result()
                        progress.advance(task)
                    except Transient:
                        pending.append(url_path)
        if pending:
            die(f"{len(pending)} file(s) never became available, e.g. {pending[0]!r}")
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


# ── the drawio tenant: GitHub tree, not npm ────────────────────────────────────────────────────

# js/** is kept except source maps and the handful of scripts the panel never needs: the export
# family talks to draw.io's own conversion service, the two viewers and the embed variant are for a
# read-only page this tenant never opens, and open.js/clear.js/vsdxImporter.js are desktop-app glue.
DRAWIO_JS_EXCLUDE_NAMES = {
    "integrate.min.js",
    "viewer.min.js",
    "viewer-static.min.js",
    "embed.dev.js",
    "open.js",
    "clear.js",
    "vsdxImporter.js",
}

# Whole `js/` subtrees dropped because production never fetches them, verified against the real
# bytes rather than assumed: `diagramly/` and `grapheditor/` are both bundled straight into
# `app.min.js` (the only entry point `index.html` loads), and `desktop/` is Electron-only glue this
# panel, running in an embedded browser, never reaches.
DRAWIO_JS_EXCLUDE_DIRS = ("js/diagramly/", "js/grapheditor/", "js/desktop/")

# Everything under these directories is kept in full.
DRAWIO_KEEP_DIRS = ("styles/", "images/")

# `app.min.js` sets `mxImageBasePath` under `mxgraph/images/` and links `mxgraph/css/common.css`;
# every other `mxgraph/` file — `mxgraph/src/**`, `mxgraph/mxClient.js` — is the unminified source
# `bootstrap.js` only loads under `?dev=1`, which this panel never sets.
DRAWIO_MXGRAPH_KEEP_DIRS = ("mxgraph/images/", "mxgraph/css/")

# `resources/` is otherwise one `.txt` per locale; only English is worth the weight.
DRAWIO_KEEP_RESOURCES = {"resources/dia.txt", "resources/dia_en.txt"}


def keep_drawio_path(rel: str) -> bool:
    """Whether a `src/main/webapp/`-relative path belongs in the mirror.

    An allowlist, not a set of skips: everything not named here — `stencils/`, `templates/`, `img/`,
    `math4/`, `plugins/`, `WEB-INF/`, `META-INF/`, `connect/`, every stray top-level HTML file, every
    other `resources/*.txt`, and all of `shapes/` (bundled into `js/shapes-14-6-5.min.js`, so the
    unbundled source is dead weight) — falls out on its own rather than needing its own exclusion.
    """
    if rel == "index.html":
        return True
    if rel.startswith("js/"):
        if rel.startswith(DRAWIO_JS_EXCLUDE_DIRS):
            return False
        name = rel.rsplit("/", 1)[-1]
        if name.endswith(".map") or name in DRAWIO_JS_EXCLUDE_NAMES:
            return False
        if name.startswith("export") and name.endswith(".js"):
            return False
        return True
    if rel.startswith("mxgraph/"):
        return rel.startswith(DRAWIO_MXGRAPH_KEEP_DIRS)
    if rel.startswith(DRAWIO_KEEP_DIRS):
        return True
    return rel in DRAWIO_KEEP_RESOURCES


def drawio_tree(gh_client: httpx.Client, version: str) -> list[dict]:
    """The full, non-truncated file tree at tag `v<version>`, from the GitHub tree API — jsDelivr's
    own data API refuses to list this package (it is over its 50 MB cap)."""
    response = gh_client.get(
        f"{GITHUB_API}/repos/{DRAWIO_REPO}/git/trees/v{version}",
        params={"recursive": "1"},
        follow_redirects=True,
    )
    response.raise_for_status()
    data = response.json()
    if data.get("truncated"):
        die(f"GitHub truncated the tree for drawio@{version} — this walk needs the untruncated one")
    return data["tree"]


def snapshot_drawio(fetcher: Fetcher, gh_client: httpx.Client, version: str) -> list[Asset]:
    """Every kept file under `src/main/webapp/`, fetched off jsDelivr's GitHub CDN mirror.

    jsDelivr publishes no per-file digest for a GitHub source, so every hash here is ours, taken
    once at snapshot — exactly how the npm tenant hashes its own `+esm` closure.
    """
    kept: list[tuple[str, str, int]] = []  # (webapp-relative path, url path off the CDN, GitHub size)
    for node in drawio_tree(gh_client, version):
        if node.get("type") != "blob" or not node["path"].startswith(DRAWIO_WEBAPP_PREFIX):
            continue
        rel = node["path"][len(DRAWIO_WEBAPP_PREFIX) :]
        if keep_drawio_path(rel):
            url_path = f"/gh/{DRAWIO_REPO}@{version}/{DRAWIO_WEBAPP_PREFIX}{rel}"
            kept.append((rel, url_path, node.get("size", 0)))
    if not kept:
        die(f"no files kept from drawio@{version} — check the tag and the allowlist")

    url_paths = [url_path for _, url_path, _ in kept]
    bodies = fetch_all(fetcher, url_paths, "drawio webapp")

    assets: list[Asset] = []
    for rel, url_path, expected_size in kept:
        body = bodies[url_path]
        if expected_size and len(body) != expected_size:
            die(f"{rel}: GitHub's tree says {expected_size} bytes, jsDelivr served {len(body)}")
        assets.append(
            Asset(path=rel, url=CDN + url_path, sha256=hashlib.sha256(body).hexdigest(), len=len(body))
        )
    return assets


def render_manifest_drawio(version: str, bundle_version: str, assets: list[Asset]) -> str:
    total = sum(a.len for a in assets)
    lines = [
        "//! The draw.io offline mirror — every file the web panel's vendor cache must hold, with",
        "//! the SHA-256 the host verifies each download against.",
        "//!",
        "//! draw.io's webapp is not published to npm, so this snapshot is walked and hashed",
        "//! differently from Excalidraw's: the file list comes from the GitHub tree API at",
        f"//! `{GITHUB_API}/repos/{DRAWIO_REPO}/git/trees/v<version>`, filtered to",
        "//! `src/main/webapp/`, and every file is fetched off jsDelivr's GitHub CDN mirror rather",
        "//! than its npm one. jsDelivr publishes no per-file digest for this source, so every hash",
        "//! here is ours, taken once at snapshot — a later mismatch means upstream changed a file",
        "//! at the same tag, which is the event worth failing on.",
        "//!",
        "//! [`VERSION`] is the cache directory name. It changes whenever the snapshot changes, so a",
        "//! build that pins a new one looks in a directory that does not exist, fetches, and leaves",
        "//! the old one behind: versioned by directory, not invalidated by policy.",
        "//!",
        "//! [`Entry::path`] is the file's path under `src/main/webapp/`, which is also where it lands",
        "//! under the mirror's `vendor/` root. draw.io loads through classic `<script>` tags rather",
        "//! than ES module specifiers, so there is no import map to route an absolute import through.",
        "//!",
        "//! @generated by `_tools/webassets.py` — do not edit by hand.",
        "//! Regenerate with `just web-assets-drawio`; check for CDN drift with",
        "//! `just web-assets-verify-drawio`.",
        "",
        "// A `Bundle` needs one file type across every tenant, not a second one that merely looks",
        "// the same — [`super::Bundle::files`] is `&'static [manifest::Entry]` regardless of which",
        "// tenant it holds.",
        "use super::manifest::Entry;",
        "",
        "/// The web app this bundle belongs to — the `<app>` segment of the cache path and the route.",
        f'pub const APP: &str = "{DRAWIO_APP}";',
        "",
        "/// The bundle version: the draw.io release plus a digest over the whole snapshot.",
        f'pub const VERSION: &str = "{bundle_version}";',
        "",
        "/// The upstream draw.io release (the `v<PACKAGE_VERSION>` tag this was walked at) this",
        "/// snapshot was taken from.",
        f'pub const PACKAGE_VERSION: &str = "{version}";',
        "",
        "/// Every byte of the mirror, for a progress report and a disk-space check.",
        f"pub const TOTAL_BYTES: u64 = {total};",
        "",
        f"/// The {len(assets)} files of the mirror, sorted by cache path.",
        "///",
        "/// One line per entry, and `rustfmt` is told to leave them alone: this table would otherwise",
        "/// fight every `just fmt`.",
        "#[rustfmt::skip]",
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
        "/// draw.io loads through classic `<script>` tags, not ES module specifiers, so there is",
        "/// nothing for an import map to route. The chrome still fetches `importmap.json` like every",
        "/// tenant does, and gets this.",
        'pub const IMPORT_MAP: &str = "{}";',
        "",
    ]
    return "\n".join(lines)


def run_snapshot_drawio(version: str, out: Path, refetch: bool) -> int:
    scratch = scratch_root()
    scratch.mkdir(parents=True, exist_ok=True)
    console.print(f"[bold]snapshot[/bold] drawio@{version}  [dim]scratch {scratch}[/dim]\n")

    with client() as http:
        fetcher = Fetcher(scratch=scratch, client=http, refetch=refetch)
        assets = snapshot_drawio(fetcher, http, version)

    paths = [a.path for a in assets]
    if len(set(paths)) != len(paths):
        die("two assets claim the same cache path")

    digest = hashlib.sha256(
        "\n".join(f"{a.path} {a.sha256}" for a in sorted(assets, key=lambda a: a.path)).encode()
    ).hexdigest()[:12]
    bundle_version = f"{version}-{digest}"

    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(render_manifest_drawio(version, bundle_version, assets))

    table = Table(show_header=False, box=None, pad_edge=False)
    table.add_row("files", f"{len(assets)}")
    total = sum(a.len for a in assets)
    table.add_row("bytes", f"{total:,} ({total / 2**20:.1f} MiB)")
    table.add_row("VERSION", bundle_version)
    table.add_row("written", rel_to_repo(out))
    console.print(table)
    console.print(f"\n[dim]cache {fetcher.hits} hits, {fetcher.misses} downloads[/dim]")
    return 0


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
        "///",
        "/// One line per entry, and `rustfmt` is told to leave them alone: expanded, this table is",
        "/// some 3,300 lines, and every `just fmt` would otherwise fight this generator.",
        "#[rustfmt::skip]",
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
    parser.add_argument(
        "--tenant", choices=["excalidraw", "drawio"], default="excalidraw", help="which bundle to snapshot"
    )
    parser.add_argument("--version", help="upstream release to mirror (defaults per tenant)")
    parser.add_argument("--react", help="pin React explicitly instead of resolving the latest (excalidraw only)")
    parser.add_argument("--out", type=Path, help="generated manifest path (defaults per tenant)")
    parser.add_argument("--refetch", action="store_true", help="ignore the scratch cache")
    args = parser.parse_args()

    if args.tenant == "excalidraw":
        version = args.version or DEFAULT_VERSION
        out = args.out or MANIFEST
    else:
        version = args.version or DRAWIO_DEFAULT_VERSION
        out = args.out or DRAWIO_MANIFEST

    try:
        match args.action:
            case "snapshot":
                if args.tenant == "excalidraw":
                    return run_snapshot(version, out, args.react, args.refetch)
                return run_snapshot_drawio(version, out, args.refetch)
            case "verify":
                return run_verify(out)
    except (httpx.HTTPError, Transient) as error:
        die(f"network: {error}")
    return 2


if __name__ == "__main__":
    sys.exit(main())
