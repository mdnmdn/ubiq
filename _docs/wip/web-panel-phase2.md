---
id: wip-web-panel-phase2
title: Web panels — the origin and the bridge
kind: wip
status: current
summary: Phase 2 of the web-panel proposal as built — the `_web/<app>/<token>/` routes on the interface's existing loopback server, a per-panel token from the platform's CSPRNG, two frame queues per session with a long-poll that answers on its own thread, the two-transport `bridge.js` shim, and a demo tenant that proves the loop. The container is the external browser; the embedded `wry` webview was not built and the shim carries its half anyway.
read_when: you are opening a web panel, adding a tenant to the bridge, or changing the `_web` routes or the session token
updated: 2026-09-11
verified: 2026-09-11
code_anchors: [crates/ubiq/src/web_export/server.rs, crates/ubiq/src/web_export/routes.rs, crates/ubiq/src/web_export/bridge.rs, crates/ubiq/src/web_export/assets.rs, crates/ubiq/assets/web/bridge.js, crates/ubiq/build.rs]
depends_on: [tech-architecture, tech-structure, tech-decisions, feat-workbench]
---

# Web panels — the origin and the bridge

Phase 2 of [`../inbox/web-panel-proposal.md`](../inbox/web-panel-proposal.md): the origin, the
token, the bridge and a demo tenant to drive them. It adds no crate dependency, no transport
message and no host involvement — everything here is inside `crates/ubiq/src/web_export/`, the
loopback server `D55` already put in the interface.

## The container is the external browser

The proposal's §4.2 container — an embedded `wry` webview over the GPUI window handle — was not
built and is not spiked here. The shipped container is the one §4.2 names as its own fallback:
*"same chrome, same origin, same bridge, opened in the external browser as `D55` opens an export."*
So only the external-browser row of §4.3's transport table has a Rust side.

`bridge.js` is still written as the two-transport shim, because that is the whole point of it: it
picks `window.ipc.postMessage` and `window.__ubiq_receive` when a `wry` container provides them,
and falls back to `POST`/long-poll otherwise. A future embedded container needs no change to any
chrome page.

**An outbound `POST` is retried, not swallowed.** Up to five tries with backoff, because a lost
`ready` frame is a loader that never lifts — the chrome believes it spoke and the panel never hears
it. A frame that still fails after every try is logged to the browser console rather than thrown
away silently.

## What a session is

One open panel. `Registry` gains a third map, token to `WebChannel`, beside the two project maps.
A channel holds the app id, an optional vendor root, and two frame queues with their own lock and
condition variable — so a parked long-poll never holds the registry.

The token is 192 bits from `rustls`'s `ring` provider, hex-encoded. `rustls` is already a
dependency (the remote dialer's TLS) and this adds none. A ULID would not do: the process's
generator is monotonic by design, which is the opposite of what a secret wants.

## Public API

```rust
pub struct WebSession { pub app: &'static str, pub token: String, pub url: String }

pub fn open_session(app: &'static str) -> Result<WebSession, String>;
pub fn send(token: &str, frame: serde_json::Value);
pub fn receive(token: &str) -> Vec<serde_json::Value>;
pub fn set_vendor_root(token: &str, root: Option<PathBuf>);
pub fn close_session(token: &str);
```

`open_session` starts the server if it is not up and returns
`http://127.0.0.1:<port>/_web/<app>/<token>/`. `send` queues one interface-to-web frame and wakes
a parked poll; `receive` drains the web-to-interface frames the panel has not read yet.

The typed layer is `web_export::bridge`: `ToWeb { Open, Reload, Palette, SaveRequested }` and
`FromWeb { Ready, Changed, Dirty, Error, Unknown }`, both internally tagged
(`{"type":"ready", …}`). `bridge::to_value` serialises an outbound frame; `bridge::decode_from_web`
answers `Option<FromWeb>` and logs rather than erroring, so a frame from a newer chrome page is
dropped and the panel keeps working. The queues themselves are `serde_json::Value`, so a second
tenant declares its own pair in `bridge.rs` and the plumbing is unchanged.

## The route table, as implemented

Every route lives under `/_web/<app>/<token>/`, matched in `routes::handle` beside the existing
`_assets` branch and before any project-slug lookup. A request whose app and token do not name a
live session is logged and answered 404 — never a 200.

| Route | Method | Answers |
|---|---|---|
| *(empty)* or `index.html` | GET | the app's chrome HTML, with a `Content-Security-Policy` |
| `bridge.js` | GET | the two-transport shim |
| `app.js` | GET | the app's chrome module |
| `vendor/<rest…>` | GET | bytes under the session's vendor root; 404 while it is `None` |
| `bridge` | POST | one JSON frame, `{ "token": …, "frame": … }`; a bad or missing token is dropped and 404s |
| `bridge` | GET | the long-poll — the queued frames as a JSON array, `[]` after five seconds |

The policy is `default-src 'self'; style-src 'self' 'unsafe-inline'; frame-ancestors 'none';
base-uri 'none'; object-src 'none'`. `'unsafe-inline'` is for style alone, because a chrome page
carries its own `<style>` block rather than a second request; nothing there is user-authored
markup. A tenant that genuinely needs `blob:` or a `worker-src` widens it with its reason beside
it.

**The long-poll runs on its own thread.** `serve()` is a single-threaded accept loop, so a parked
poll on that thread would stall every other request on the origin — the chrome, the vendor bytes,
another panel's bridge. `serve_bridge` spawns for the GET case and nothing else, and the park is
bounded by the channel's own five-second timeout rather than living forever.

**MIME on the vendor route is a hard failure, not a degradation.** `.js`, `.mjs` and
**extensionless** files all answer `text/javascript`, because jsDelivr's `+esm` output has no
extension and `application/octet-stream` yields *"Expected a JavaScript-or-Wasm module script"* and
a blank page. `.css` and `.woff2` are named for the same reason; anything else falls through to the
existing `mime_for_ext`. The route reuses `resolve_path` unchanged, so `..`, empty and dot-leading
segments are refused there exactly as they are for a project.

## The assets

`crates/ubiq/assets/web/` holds `bridge.js` and the demo tenant (`demo/index.html`, `demo/app.js`),
baked in the way the doc viewer's CSS and JS already are. `build.rs`'s `ASSETS` array now carries
each asset's subdirectory rather than assuming one directory, and the gzipped release copy is named
for the path with `/` replaced by `-` (`web-export-style.css.gz`, `web-demo-index.html.gz`).

## What phase 2 does not do

Each of the first two was closed by a later phase; they are left here because they say what this
phase's own boundary was.

- **No panel.** Nothing in the interface called `open_session` — no dock panel, no kitchen-sink
  entry, no `open_url` for a session. Closed by
  [`./web-panel-phase45.md`](./web-panel-phase45.md), which put an `Edit` action on the viewer
  header for an Excalidraw file.
- **No vendor bytes.** `set_vendor_root` existed as phase 3's hook and nothing set it, so the
  `vendor/` route 404'd in every shipped path. Closed by
  [`./web-panel-phase3.md`](./web-panel-phase3.md) and the phase above, which sets the root to the
  bundle directory the host answered with.
- **No embedded container**, per the decision above. Still true, and now settled: the container is
  the external browser.

Two things here changed under a real tenant, and the phase-4 note carries the reasoning:
`chrome_html` became `serve_chrome`, because a tenant answers its own policy rather than sharing
one constant; and the Excalidraw page's policy widens `script-src` with a per-response nonce (an
import map has no external form, and `'self'` alone blocks an inline one) and `img-src` with
`data:` and `blob:`. Neither widening names a remote origin.

## Deviations from the phase's brief

- `WebChannel::vendor_root` is a `Mutex<Option<PathBuf>>` with a public `set_vendor_root`, rather
  than a plain field fixed at open. The cache is ensured after a panel is already open, so phase 3
  needs a later call rather than an argument to `open_session`; it is also what makes the vendor
  route testable now.

## Related docs

- [`../inbox/web-panel-proposal.md`](../inbox/web-panel-proposal.md) — the proposal this builds §4.1, §4.3 and §4.4 of
- [`../tech/decisions.md`](../tech/decisions.md) — `D55`, the loopback server this extends
- [`../tech/architecture.md`](../tech/architecture.md) — rule 2, restated for a second runtime
