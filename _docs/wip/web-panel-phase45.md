---
id: wip-web-panel-phase45
title: Web panels — the Excalidraw chrome, and editing
kind: wip
status: current
summary: Phases 4 and 5 of the web-panel proposal as built — a chrome page that fetches the host's generated import map, injects it under a per-response CSP nonce and mounts Excalidraw off the local mirror; an Edit action on the viewer header that is an axis of its own rather than a fourth ViewLayout; and a debounced `Changed` frame written into the existing `EditorState` buffer, so the dirty dot, `⌘S`, save-as and `D37`'s version check keep working with no new save path and no new message.
read_when: you are touching the Excalidraw chrome, the Edit action, the bridge drain loop, or how a web panel's edits reach a file buffer
updated: 2026-09-11
verified: 2026-09-11
code_anchors: [crates/ubiq/assets/web/excalidraw/index.html, crates/ubiq/assets/web/excalidraw/app.js, crates/ubiq/src/app/web_panel.rs, crates/ubiq/src/state/web_panel.rs, crates/ubiq/src/web_export/routes.rs, crates/ubiq/src/web_export/assets.rs, crates/ubiq/src/ui/viewer/mod.rs]
depends_on: [wip-web-panel-phase2, wip-web-panel-phase3, feat-workbench, tech-decisions]
---

# Web panels — the Excalidraw chrome, and editing

Phases 4 and 5 of [`../inbox/web-panel-proposal.md`](../inbox/web-panel-proposal.md), and the last
unit of it. Phase 2 built the origin, the token and the typed bridge; phase 3 built the fetch that
puts 554 verified files in the shared workarea. This mounts a real component on those bytes and
lets it write back. Everything here is in `crates/ubiq` and `crates/ubiq/assets/` — no transport
message was added, and neither `crates/ubiq-proto` nor `crates/ubiq-host` was touched.

## `Edit` is a separate axis, not a fourth `ViewLayout`

§11 asks *"whether `Edit` belongs on `ViewLayout` with the other three or is a separate axis, being
a container rather than a view of the same bytes."* **With the external browser as the container it
is plainly a separate axis, and `ViewLayout` keeps its three variants.**

The reasoning is short. `ViewLayout` says what the panel draws. The shipped container is a browser
window beside this one, so there is *nothing* for the panel to draw differently when editing is on
— a fourth variant would name a layout that renders the same pixels as the third. Worse, it
persists: the dock writes the layout into its saved arrangement, so an `Edit` variant would reopen
a document by launching a browser, which is exactly the "opened, never fallen into" rule in §2.

So `Edit` is a control on the viewer header beside the three-way toggle, and a session recorded in
`state/web_panel.rs`. The panel underneath keeps whichever of Source, Preview and Split it was in,
and the native painter draws it the whole time the browser is open.

## The chrome's boot sequence

`assets/web/excalidraw/index.html` is a template — the route substitutes a nonce into it per
response — and `app.js` beside it is bytes, gzipped in release like every other web asset.
`index.html` loads `bridge.js` and then `app.js`, **both as classic scripts**, because an import
map must be in the document before any module loads and a `<script type="module">` in the markup
would start a module graph before the map existed.

`app.js` then, in this order and no other:

1. **Registers the bridge handler and queues** whatever arrives before the mount. The long-poll in
   `bridge.js` starts as soon as it loads, and the component takes seconds to download.
2. **Fetches `vendor/importmap.json`** — the name `web_assets::IMPORT_MAP_FILE` writes it under,
   inside the bundle directory the host answered with.
3. **Injects it as an inline `<script type="importmap">`** carrying the page's nonce. Without the
   map, `dist/prod/index.js`'s 32 bare specifiers resolve to nothing and the page is blank; the
   map is also what collapses the five React and four react-dom copies `+esm` hard-pins into one.
4. **Sets `window.EXCALIDRAW_ASSET_PATH`** to `new URL(DIST, document.baseURI).href` — *absolute*,
   because a `/`-relative value resolves against `location.origin` and throws.
5. **Links `index.css`** from inside the mirror, whose four Assistant faces are relative
   `url("./fonts/…")` and only resolve from there.
6. **Dynamically imports** `react`, `react-dom/client` and the entry point, and mounts.
7. Waits for the component to hand over `excalidrawAPI`, drains the queue, and posts `Ready`.

A failure anywhere in that chain posts `Error { message }` and draws the message over the page. The
buffer is never touched by a chrome that did not start.

### The three §6 runtime constraints, and what each cost

**The import map is mandatory, and inline, and therefore needs a nonce.** An import map has no
external form, and `script-src 'self'` blocks an inline `<script>` of any type. The map is
generated beside the mirror so it cannot be hashed at build time either. The chrome route
therefore mints a nonce per response from the same CSPRNG as the session token, substitutes it into
the page as `data-nonce`, and names it in that response's policy. Excalidraw's policy is phase 2's
widened in exactly two places and no more:

```text
default-src 'self'; script-src 'self' 'nonce-…'; style-src 'self' 'unsafe-inline';
img-src 'self' data: blob:; frame-ancestors 'none'; base-uri 'none'; object-src 'none'
```

`img-src` takes `data:` and `blob:` because that is how the editor holds an embedded image and
previews an export; neither is a network fetch. **No remote origin is permitted, deliberately** —
Excalidraw always appends its own `esm.sh` path to the font candidate list, so a hole in the mirror
has to fail loudly here rather than quietly reach the network. `crates/ubiq/src/web_export/mod.rs`
asserts both the nonce and the absence of any remote origin.

**`worker-src` was not widened.** There is one module worker — font subsetting, built from
`import.meta.url`, wrapped upstream in a try/catch with a main-thread fallback — and being built
from a URL rather than a blob makes it same-origin, which `default-src 'self'` already permits.
There is no service worker.

**Nothing is rewritten.** The mirror is served byte-identical off the `vendor/` route, React comes
from the map as the peer dependency it is, and the chrome is the only file that knows the mirror's
shape.

## The edit cycle

```text
Edit pressed ── EnsureWebBundle ──▶ host        (idempotent; a second ask joins the first)
             ◀── WebBundleReady { path }
  open_session ▸ set_vendor_root(path) ▸ open_url
             ◀── Ready
  ToWeb::Open { document, palette } ──▶         the buffer's text, the window's palette
             ◀── Dirty                          the dot lights at once
             ◀── Changed { document }           written into the existing buffer
  ⌘S ── WriteProjectFile { .. expected } ──▶    unchanged, and the only save path
```

`app/web_panel.rs` owns all of it. The drain loop is the shape `app/stats.rs`'s `poll_stats`
already uses — one `cx.spawn` per window, guarded by a flag, a 250 ms
`background_executor().timer`, broken by the first failed `update` or by there being no session
left. 250 ms rather than frame rate because the other direction is the bridge's long-poll and this
side only has a queue to read.

**`Changed` reaches the buffer through `EditorState::set_value`, and nothing else.** That call
needs a `Window` and a bridge frame does not come with one, so the text is parked in
`WebPanels::incoming` and written by `apply_web_documents` in `render`, immediately after
`attach_arrived_files` — the same deferral, for the same reason. From there everything downstream
is the machinery that was already attached to that buffer: the buffer's own change subscription
calls `OpenFile::refresh_dirty`, the dot lights, `⌘S` sends `WriteProjectFile` with the version the
host gave it, `ProjectFileWritten` clears the dot. **No new save path, no new transport message.**

`Dirty` is the one addition: `OpenFile::mark_dirty` sets the same flag `refresh_dirty` sets and
nothing else — no baseline, no version, no save state — so the dot does not lag every stroke by the
debounce. It is safe only because the web side has already decided the change is real (below).

## The churn mitigations

Excalidraw reserialises the whole document: key order, a `version`/`versionNonce` pair bumped on
every element it touches, `appState` fields the file may never have carried. Both mitigations §7
asks for are implemented, in `app.js`:

- **(a) The buffer is written only when the document actually differs.** `onChange` fires
  continuously; the chrome posts `Dirty` on the first *real* change and serialises on a 600 ms
  idle, never per event. "Real" is a stable stringification of the serialised document with
  `version`, `versionNonce` and `updated` stripped from every element, compared against a baseline
  taken *after* the scene lands — so `restore`'s own normalisation does not read as an edit, and a
  document merely opened and looked at never marks the tab dirty.
- **(b) The file's own `source` and `type` survive the round trip**, rather than the editor's being
  emitted in their place.

**Neither makes the diff small.** A save after a session of edits is still a large diff where
little moved, and a round-tripped file is still not byte-identical to the one that was read. A user
who wants a tidy diff should not round-trip a file they did not change.

## Failure, by §8's rows

| Row | What was built |
|---|---|
| No network on a first run, nothing cached | `WebBundleFailed` sets `BundleState::Unavailable`; `Edit` draws flat and unclickable with the host's sentence as its tooltip. Source, Preview, Split and the painter are untouched |
| A hash does not match the manifest | Same path — the host fails the fetch and the sentence says so |
| A bad or missing token on a bridge frame | Dropped and logged in `routes.rs`, as of phase 2 |
| An unknown variant either way | `decode_from_web` answers `None` and logs; `app.js`'s `switch` has a `default` that drops |
| The chrome fails to load, or the component throws | `Error { message }` lands on the session, the header's tooltip carries it, and the buffer is untouched |
| Closed with unsaved changes | The buffer is dirty exactly as if typed, so `close_editor_tab` raises `FileDialog::DiscardChanges` as it does for any dirty tab |
| The file changed on disk while the panel held edits | `⌘S` still carries the version the host gave, so `D37` refuses the save — `G111`'s existing gap, neither closed nor made worse |
| No webview, or the spike's constraints unmet | Settled before this unit: the container is the external browser |
| A modal or a dock drag overlaps the panel | Not applicable — there is no child view to hide |
| The panel is not the visible tab | Not applicable — the browser window is not inside the dock |

## Deviations, and what is unverified

- **`app.js` carries the mirror's package version as one constant**
  (`vendor/npm/@excalidraw/excalidraw@0.18.0/dist/prod/`). It must equal the host's
  `manifest::ASSET_SUBPATH`, and `crates/ubiq` cannot depend on `crates/ubiq-host` to read it. A
  version bump that moves that directory shows up as the chrome's explicit failure notice rather
  than as a blank page, but it is a duplicated constant and a bump has to touch both.
- **The nonce forced `index.html` to be a template.** It is `include_str!`-ed in both profiles and
  substituted per response, like `web-export/template.html`; only `app.js` is gzipped. A ~1 KB page
  over loopback loses nothing by it.
- **`ToWeb::Reload` is declared and never sent.** A file changed on disk while a session is open
  does not push the new text into the component. The buffer reload path exists
  (`OpenFile::reload`); wiring it to the bridge was not done.
- **`ToWeb::SaveRequested` is handled in the chrome and never sent by the window.** `⌘S` writes
  what the buffer holds, which is at most 600 ms behind the drawing. Sending it before a save would
  make `⌘S` asynchronous, which is a larger change than this unit.
- **`Dirty` needed a new mutator**, `OpenFile::mark_dirty`. It is not a save path, but it is the one
  place outside `refresh_dirty` that sets the flag.
- **The Excalidraw component was not seen to boot.** Nothing here was driven in a real browser by
  this unit: the vendor mirror was not fetched, no session was opened by hand and no drawing was
  made. What is verified is what a test can reach — the route serves the chrome with a per-response
  nonce its own policy names, the policy names no remote origin, `vendor/importmap.json` is
  reachable, and the module served at `app.js` is the Excalidraw one. §6's field report is the
  evidence that the *bundle* boots; that this *chrome* mounts it correctly is reasoned from that
  report and untested.
- **`just check` could not be run** — the `foundation-models` Swift build script fails under
  `sandbox-exec` in this environment. `cargo check -p ubiq --all-targets`,
  `cargo clippy -p ubiq --all-targets -- -D warnings`, `cargo test -p ubiq` and `cargo fmt --all`
  all pass, as does `cargo check -p ubiq --release --lib` — which is what exercises the
  `web-excalidraw-app.js.gz` include path the debug profile never compiles.
- `crates/ubiq/tests/new_pane.rs` gained `shared_workarea: None` — phase 3's `HostInfo` field left
  that fixture uncompilable.

## Related docs

- [`./web-panel-phase2.md`](./web-panel-phase2.md) — the origin, the token and the bridge this mounts on
- [`./web-panel-phase3.md`](./web-panel-phase3.md) — the shared workarea and the fetch that puts the mirror there
- [`../inbox/web-panel-proposal.md`](../inbox/web-panel-proposal.md) — §6, §7 and §8, which this builds
- [`../features/workbench.md`](../features/workbench.md) — the viewer, its toggle and the file buffer
