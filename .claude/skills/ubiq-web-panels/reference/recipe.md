# Adding a web panel tenant, file by file

The order matters: each step compiles and is testable on its own, and the tenant is reachable
from the interface only at step 7.

Decide first which shape you are building.

- **No vendor bytes** — the component is a page you write, or a library small enough to live in
  `assets/web/<app>/`. This is the `demo` shape: steps 1–4, then 7. No manifest, no `Bundle`, no
  `EnsureWebBundle`.
- **A mirrored npm package** — the `excalidraw` shape: every step, and read
  `chrome-and-bundle.md` before step 5.

## 1. Name the tenant

`crates/ubiq/src/web_export/bridge.rs` — a `pub const <TENANT>_APP: &str = "<tenant>";` beside
`DEMO_APP`, `EXCALIDRAW_APP` and `DRAWIO_APP`. Lower case, one word, no version. This string is the route
segment, the `app` the host is asked for a bundle under, and the `<app>` directory of the cache
path; it is never composed from anything.

Reuse `ToWeb`/`FromWeb` unless the protocol genuinely differs. They are already generic —
`Open { document, palette }`, `Reload`, `Palette`, `SaveRequested` out; `Ready`, `Changed`,
`Dirty`, `Save`, `Error`, `Unknown` back — and a second pair is a second `to_value` and a second
`decode_from_web` to keep in step. Both enums keep a catch-all: an unknown variant from a newer
page is logged and dropped, never an error.

## 2. Write the chrome page

`crates/ubiq/assets/web/<tenant>/index.html` and `app.js`. `reference/chrome-and-bundle.md` is
the contract. Copy `demo/` for a static page, `excalidraw/` for one that mounts a component through
an import map and a nonce, or `drawio/` for one that frames a whole mirrored webapp in an `<iframe>`
instead and translates its own postMessage protocol to and from the bridge.

## 3. Embed it

`crates/ubiq/build.rs` — add `"web/<tenant>/app.js"` to `ASSETS`. If `index.html` carries the
nonce placeholder, add it to `TEMPLATES` instead of `ASSETS`: templates are substituted per
response and so are never gzipped, assets are served byte for byte and are gzip-precompressed in
release. The `cargo:rerun-if-changed` loop covers both.

`crates/ubiq/src/web_export/assets.rs` — a `pub static <TENANT>_INDEX_HTML` via `include_str!`
for a template, and a `<tenant>_app_js()` following the `#[cfg(debug_assertions)]` / release pair
beside it, so a debug build reads the source file and a release build serves the gzipped bytes
`build.rs` wrote into `OUT_DIR`.

## 4. Route it

`crates/ubiq/src/web_export/routes.rs`, two arms and nothing else — `serve_web` already
dispatches `index.html`, `bridge.js`, `app.js`, `vendor/…` and the bridge queue generically.

- `serve_chrome` — a static page mirrors the `DEMO_APP` arm and serves under `CHROME_CSP`
  unchanged. A templated page mirrors the `EXCALIDRAW_APP` arm: mint a nonce with
  `super::server::mint_token()`, `.replace("__UBIQ_NONCE__", &nonce)`, and build its policy from the
  narrow one, widening only what the page actually needs and saying why on the line that widens
  it.
- `chrome_module` — `<TENANT>_APP => Some(assets::<tenant>_app_js())`.

At this point the tenant is reachable: `web_export::open_session(bridge::<TENANT>_APP)` from a
test gives a URL that serves the page.

If the vendor mirror itself holds a *document* the chrome frames rather than a component it
imports — draw.io's own `index.html` — `serve_vendor` already attaches `VENDOR_DOCUMENT_CSP` to
any vendor file whose MIME comes back HTML, and `vendor_mime` already recognises `.html`, `.xml`
and `.json`. Widen `VENDOR_DOCUMENT_CSP` itself, with a reason beside the line, only if the framed
document needs something draw.io's does not.

## 5. Mirror the vendor bytes (only if it needs them)

`_tools/webassets.py snapshot --tenant <name>` walks a mirror, hashes every file it keeps and
writes a `@generated` manifest; `just web-assets-drawio` and `just web-assets` are the two `just`
faces of it today, one per `--tenant`. **Two generating rules exist, one per tenant shape** —
`snapshot_excalidraw` walks jsDelivr's package-file API for an npm package's `dist/prod` plus its
`+esm` dependency closure; `snapshot_drawio` walks the GitHub tree API for a repository tag
instead, because jsDelivr refuses to list a package over its size ceiling. **Reach for GitHub's
tree API whenever your source is a large, non-npm site** rather than trying to force it through
jsDelivr. A tenant whose shape fits an existing rule passes its own `--version` and `--out`; one
that does not gets a third `snapshot_<tenant>` function beside the other two, and its manifest goes
beside `manifest.rs`/`manifest_drawio.rs` as `manifest_<tenant>.rs`. `just web-assets-verify` and
`just web-assets-verify-drawio` re-fetch every URL in their own committed manifest and report CDN
or upstream drift; a new tenant needs its own `verify` invocation and its own `just` recipe.

`crates/ubiq-host/src/web_assets/mod.rs` — a `pub const <TENANT>: Bundle` from that module's
`APP`, `VERSION`, `FILES` and `IMPORT_MAP`, and its name added to the vec in `WebAssets::new`.
That vec is the whole registration: `ensure` finds a bundle by the `app` string, answers
`WebBundleFailed` for one it does not know, answers `WebBundleReady` immediately for a `.bundle`
file that already exists, and joins a fetch already in flight rather than starting a second.
Nothing in the coordinator or in `crates/ubiq-proto` is per-tenant — every message in the family
carries a plain `app: String`.

The fetch itself needs no change: the version names the `.bundle` file, so a new snapshot fetches
into a file that does not exist and leaves the old one alone.

## 6. Give the panel its bundle

`crates/ubiq/src/app/web_panel.rs`. `WebPanels::bundles` is already keyed by app id, and
`web_editable`, `ensure_web_session`, `open_web_session` and `receive_web_assets` already take the
tenant's app id rather than naming one — a further bundle-backed tenant reaches this path with no
preparatory commit of its own, the way draw.io did as the second one.

The cycle, once it is generic: `Edit` selected → `EnsureWebBundle` (idempotent, joins a fetch in
flight) → `WebBundleReady { path }` → `set_vendor_root(&token, Some(path))` → a child webview at
the session URL → `ready` → `Open { document, palette }`.

## 7. Wire it into the editor

- `crates/ubiq/src/state/editor.rs` — a `ViewerKind` variant, its extensions in `of`, and an
  honest answer from `has_preview`, `shows_buffer` and `layouts`. A tenant whose document is JSON
  or XML nobody hand-edits offers `[Edit, Preview]` as Excalidraw and draw.io do; one whose source
  is what an author writes keeps `[Source, Preview, Split]` and the panel is a fourth thing only if
  it is worth a fourth thing. A tenant with no native painter behind its Preview position draws it
  from an export instead — `state/diagrams.rs`'s `EXPORTED` tier, `resolve_exported`/
  `keep_exported`, and `FromWeb::Preview` are draw.io's version of that; reuse the tier rather than
  inventing a second cache for the same kind of picture.
- `crates/ubiq/src/ui/viewer/mod.rs` — an arm in each `match file.viewer`. `ViewLayout::Edit`
  already dispatches to `viewer/web.rs` generically.
- `crates/ubiq/src/ui/viewer/web.rs` — the loader names Excalidraw in its fallback text; name the
  tenant instead, from the kind.
- `crates/ubiq/src/ui/web_view.rs` — nothing. It is keyed by tab and gated by platform, not by
  tenant.

Anything else that names the format — the file dialog's filters, the explorer's menu rows, the
sink, the new-file templates — is the *viewer's* wiring rather than the panel's. Grep for the
existing tenant's name and answer each hit deliberately.

## 8. Tests

Web-export coverage is inline in `crates/ubiq/src/web_export/mod.rs`. Add:

- a session smoke test on `<TENANT>_APP`: open a session, serve the chrome, round-trip
  `Ready` → `Open` → `Changed`, close it and check the token stops answering;
- for a templated page, a nonce-and-policy test beside the Excalidraw one — the nonce differs per
  response and the policy names no remote origin;
- for a bundle, a fetch test in `crates/ubiq-host/src/web_assets/mod.rs` over the `Fetch` seam
  (never the network) if the tenant changes anything about how a bundle is fetched;
- `ViewerKind` cases wherever the existing kinds are asserted — `crates/ubiq/tests/explorer.rs`,
  `crates/ubiq/tests/sink.rs`.

`just verify` is what the change has to pass.

## 9. Documents

`just docs-touched` names them from the diff. Expect `_docs/features/workbench.md` for anything
the user sees, `_docs/tech/transport-contract.md` if the wire changed, and the
`_docs/wip/web-panel-*.md` document that owns the half you touched. Bump `verified`. A tenant is
an instance of `D104`–`D107`; write a new `Dnn` only if you changed the policy.
