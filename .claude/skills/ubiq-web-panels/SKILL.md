---
name: ubiq-web-panels
description: Reference for web panels in Ubiq — adding a new web-driven editor or viewer that runs somebody else's component over the interface's own loopback origin. The tenant id, the bridge frames, the chrome page and its CSP, the hash-verified vendor mirror, and how a tenant is wired into a ViewerKind. Use when adding a web panel tenant, touching web_export, the bridge, assets/web/, the web_assets bundle fetch, or the Edit layout of a viewer.
---

# Web panels in Ubiq

A **web panel** is somebody else's component, drawn in an embedded browser over the interface's
own loopback origin, exchanging typed JSON frames with Rust. It is how Ubiq edits a format it has
no business reimplementing — `D104`. Ubiq still *draws* the file natively; the panel is only ever
opened, never fallen into.

```
 crates/ubiq (UI)                          crates/ubiq-host
 ────────────────                          ────────────────
 state/editor.rs   ViewerKind picks it     web_assets/mod.rs   fetch, verify, archive
 app/web_panel.rs  session + bundle        web_assets/manifest.rs  @generated hashes
 web_export/       server, routes, bridge      <shared>/web/<app>/<version>.bundle
 assets/web/<app>/ index.html + app.js         rename over `.part` is the only proof of done
 ui/viewer/web.rs  the loader / the view
 ui/web_view.rs    the native child view

 Edit selected ─ EnsureWebBundle ─▶ host ─ WebBundlePending{done,total,file} ─▶ loader counts
              ◀─ WebBundleReady{path} ── set_vendor_root ── webview at /_web/<app>/<token>/
              ◀─ ready ── the loader lifts ── Open{document,palette} ─▶ … ◀─ changed / save
```

Three tenants exist: `demo` (static chrome, no vendor bytes), `excalidraw` (templated chrome, an
npm package mirrored off jsDelivr) and `drawio` (static chrome framing a whole mirrored webapp in
an iframe, off GitHub's tree API instead — jsDelivr will not list a package that large). The recipe
for a further one is in `reference/recipe.md`.

## Read first

| You are | Read |
|---|---|
| Adding a tenant, or changing what a panel may do | `_docs/tech/decisions.md` — `D104`–`D109` |
| Touching the bundle fetch or the messages around it | `_docs/tech/transport-contract.md`, the web asset family |
| Touching the server, the routes or the CSP | `_docs/wip/web-panel-phase2.md`, `_docs/wip/web-panel-phase45.md` |
| Touching the bundle mirror or the manifest | `_docs/wip/web-panel-phase3.md` |
| Touching the loader, the header or the `Edit` layout | `_docs/wip/web-panel-phase6.md`, `_docs/features/workbench.md` |
| Restyling anything on screen | `_docs/tech/ui-and-design.md`, plus the `ubiq-ui` skill |

Your change updates the documents it touched, in the same commit. `just docs-touched` names them.
A tenant is an instance of `D104`–`D107`, not a new decision — write a `Dnn` only if you are
changing the policy itself, as `D108` and `D109` did for editing and for the layout toggle.

## The hard rules

- **The tenant id is one string, used three times.** The `<app>` in `/_web/<app>/<token>/`, the
  `app` on `Message::EnsureWebBundle`, and the `<app>` directory of the cache path are the same
  `pub const` in `web_export/bridge.rs`. A tenant whose origin and whose vendor bytes can drift
  apart is a tenant whose CSP means nothing.
- **Frames are documents, not commands.** Nothing a chrome page sends may ask the interface to
  read a file, list a directory or run anything. The web side receives content and returns
  content — that is what keeps the bridge from becoming an RPC surface with the interface's
  authority behind it.
- **A vendor bundle is downloaded and hash-verified, never linked into the binary** (`D106`).
  Every file is checked against the manifest *before* it is written, a mismatch fails the whole
  fetch and caches nothing, and no downloaded byte is ever rewritten — the cache path mirrors the
  URL path exactly so imports resolve through one import-map prefix rule.
- **The CSP names no remote origin, ever.** Everything the page loads comes off the loopback
  origin. Where a page needs an inline import map it gets a per-response nonce, minted per
  request, substituted into a template; a tenant that widens the policy says why beside the line
  that widens it.
- **The loader lifts on `ready` and on nothing else.** The browser is a blank white rectangle
  until the component inside it has mounted, so the panel covers it and the sweep in
  `ui/web_view.rs` keeps the native view off screen. A chrome page that can fail without posting
  either `ready` or `error` is a panel that loads for ever.
- **Failure is a downgrade, never an error the user has to act on.** No network on a first run, a
  hash that did not match, a platform with no embedded browser: the `Edit` position says why and
  every native read path is exactly as it was.
- **The native read path keeps the file.** A tenant is opened deliberately; the viewer that draws
  the format without a browser, with no download and no delay, stays and stays first.
- **Nothing in `crates/ubiq` names a bundle path or a version.** The host says where the bundle
  landed, on a message. The interface opens that `.bundle` archive and composes nothing.

## What is per-tenant, and what is already generic

| Already generic | Per-tenant |
|---|---|
| `web_export::{open_session, send, receive, close_session, set_vendor_root}` | the `<TENANT>_APP` const in `bridge.rs` |
| `serve_web`'s dispatch, the `bridge.js` route, the `vendor/` route, the bridge queue | a `serve_chrome` arm and a `chrome_module` arm in `routes.rs` |
| `Message::{EnsureWebBundle, WebBundlePending, WebBundleReady, WebBundleFailed}` — every one carries a plain `app: String` | a `Bundle` const in `web_assets/mod.rs`, and its entry in `WebAssets::new`'s vec |
| the coordinator's `EnsureWebBundle` arm — it dispatches by string inside `ensure` | `assets/web/<app>/{index.html,app.js}` and their `build.rs` rows |
| `ui/web_view.rs` — keyed by tab, mark-and-sweep, platform-gated | a `ViewerKind` variant and its extension |
| `ToWeb`/`FromWeb` — document, palette, save, error, shared by every tenant | a new frame, *only* if the protocol genuinely differs — draw.io's `Preview { svg }` is the one so far, for the Preview position of a format with no native painter |

**A bundle-backed tenant reaches an already-generalised path.** `WebPanels::bundles` is a
`HashMap<String, BundleState>` keyed by app id, and `web_editable`, `ensure_web_session`,
`open_web_session` and `receive_web_assets` in `app/web_panel.rs` take the tenant's app id rather
than naming one directly — draw.io landed as a second bundle-backed tenant on that path with no
preparatory commit of its own. A tenant with no vendor bytes (the `demo` shape) needs none of it —
it goes straight to `open_session`.

## The recipe

`reference/recipe.md` is the ordered file-by-file pass. `reference/chrome-and-bundle.md` is the
chrome page's boot contract, the CSP, and the mirror tooling.

In short: name the tenant (`bridge.rs`) → write the chrome page (`assets/web/<app>/`) → embed it
(`build.rs`, `web_export/assets.rs`) → route it (`routes.rs`) → if it needs vendor bytes, snapshot
a manifest (`just web-assets`) and add a `Bundle` → wire it to a `ViewerKind` and the `Edit`
layout → tests → documents.

## When it does not work

Filter the log console to the **Web** subsystem: it carries both halves — the bundle ask, the
per-file fetch progress, the bundle landing or failing, the session URL, the embedded browser
being built or refused, and the chrome mounting. A panel stuck on the loader is one of three
things, in this order of likelihood: the chrome never posted `ready` (look in the webview's own
devtools — `wry` gets them in a debug build); the bundle is still downloading (the loader counts
files and names the one it last landed); or the fetch failed and said so.
