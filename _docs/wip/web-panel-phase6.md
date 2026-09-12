---
id: wip-web-panel-phase6
title: Web panels — the embedded browser
kind: wip
status: current
summary: Phase 6 of the web-panel proposal as built — the container moves from an external browser into the window through `gpui-wry` on macOS and Windows, `Edit` becomes a fourth `ViewLayout` (reversing phase 5's call), a mark-and-sweep module keeps the child webview clipped to its own dock tab, sessions are settled every render rather than on a click, saving reaches the file through the existing `⌘S` path with a new `Save` bridge frame, and the explorer gains a `New Excalidraw` row. Every platform without `gpui-wry`'s finished Unix path keeps phase 5's external browser. A later addition, draw.io, is the second tenant on this same axis — it offers the same `[Edit, Preview]` layouts, the explorer gains a matching `New draw.io` row, and a new `Preview { svg }` bridge frame gives its Preview position a picture for a format the interface has no native renderer for.
read_when: you are touching the embedded webview, the mark-and-sweep in `ui/web_view.rs`, the `Editor` `ViewLayout`, the web panel's save path, or the explorer's `New Excalidraw` row
updated: 2026-09-11
verified: 2026-09-11
code_anchors: [crates/ubiq/Cargo.toml, crates/ubiq-app/Cargo.toml, crates/ubiq/src/state/editor.rs, crates/ubiq/src/ui/web_view.rs, crates/ubiq/src/ui/viewer/web.rs, crates/ubiq/src/ui/viewer/mod.rs, crates/ubiq/src/ui/shell.rs, crates/ubiq/src/app/web_panel.rs, crates/ubiq/src/app/editor.rs, crates/ubiq/src/app/shell.rs, crates/ubiq/src/web_export/bridge.rs, crates/ubiq/src/app/explorer.rs, crates/ubiq/src/state/explorer/mod.rs, crates/ubiq/src/state/explorer/menu.rs, crates/ubiq/src/ui/file_dialog.rs, crates/ubiq/src/state/workbench.rs, crates/ubiq/src/state/diagrams.rs, crates/ubiq/src/ui/viewer/diagram.rs]
depends_on: [wip-web-panel-phase2, wip-web-panel-phase3, wip-web-panel-phase45, feat-workbench, tech-decisions]
---

# Web panels — the embedded browser

Phase 6 of [`../inbox/web-panel-proposal.md`](../inbox/web-panel-proposal.md), and the unit that
moves the container itself. Phase 5 shipped Excalidraw editable in a browser window beside Ubiq's
own; this puts that browser inside the panel, on the two platforms where the binding for it is
ready.

## The container moves into the window

`crates/ubiq/Cargo.toml` gains three dependencies under
`[target.'cfg(any(target_os = "macos", target_os = "windows"))'.dependencies]`: `gpui-wry`
(gpui-component's own wry binding), `wry` (published as `lb-wry` 0.53.3) and `raw-window-handle`.
Every other platform compiles none of it and keeps the external browser phase 5 shipped —
`gpui-wry`'s Unix path is unfinished upstream.

Adding a fourth dependency from the `longbridge/gpui-component` git source forced Cargo to
re-resolve that source to its current head, where `gpui-component-assets` no longer exists. All
four dependencies drawn from that repository — `gpui-component` and `gpui-component-assets` in both
`crates/ubiq/Cargo.toml` and `crates/ubiq-app/Cargo.toml`, plus the two new ones in `crates/ubiq` —
are pinned to `rev = "df1d07b212e21217b1168e1728293b3e0c48e5a3"`, the revision `Cargo.lock` already
held. The pin is what keeps all four resolving to one source rather than drifting apart the next
time any one of them moves.

## `Edit` becomes a `ViewLayout`, reversing phase 5's call

[Phase 4/5](./web-panel-phase45.md) put `Edit` on the viewer header as a control separate from the
Source/Preview/Split toggle, and gave the reason: with the container a browser window beside Ubiq's
own, there was nothing for the panel to draw differently when editing was on — a fourth `ViewLayout`
would have named a layout that rendered the same pixels as Preview. **That premise is what changed.**
With the container embedded in the panel there is something to draw, so `Edit` moves onto the axis
it named as the alternative and phase 5 declined.

`crates/ubiq/src/state/editor.rs` adds `ViewLayout::Edit` (label `"Editor"`),
`ViewerKind::layouts() -> &'static [ViewLayout]` and `ViewerKind::offers(layout)`.
**Excalidraw's toggle is `[Edit, Preview]` and nothing else** — no Source, no Split: its document is
JSON nobody edits by hand, and reading the raw bytes is what the general-purpose editor is for.
Markdown and Mermaid keep all three. draw.io's `ViewerKind::Drawio`, added when that tenant landed,
offers the identical `[Edit, Preview]` pair — its document is XML nobody edits by hand either, for
the same reason. `OpenFile::set_layout` now refuses a layout the viewer does not
offer, which is also the coercion for an arrangement saved by a build whose toggle held a different
set of positions. `ViewerKind::shows_buffer` is `false` for Excalidraw and draw.io in every layout,
because the webview takes the keyboard itself once it is open. `ViewLayout::all()` survives as
"every variant, for a test that has to cover them all" — not what a header draws, which is
`ViewerKind::layouts()`. The `edit_action` pill that used to sit beside the three-way toggle in
`ui/viewer/mod.rs` is gone; the header draws whatever `file.viewer.layouts()` returns. The sink
fixture page keeps its own three positions (`SINK_LAYOUTS` in `ui/sink/docs.rs`), because it hosts no
session and has no fourth layout to offer.

## The mark and sweep

`crates/ubiq/src/ui/web_view.rs` is new. A child webview is a native view the platform stacks over
the whole window; it knows nothing about the dock, a hidden tab or a modal, and no GPUI element can
paint over it. The module's mechanism is a mark and a sweep, both in prepaint:

- `element(key)` places a zero-size `canvas` beside the webview that marks its slot **seen**. The
  mark only fires when the owning panel is actually drawn — a dock tab that is off screen is never
  rendered at all, so this is the one signal GPUI gives for "the panel behind this key is on
  screen this frame".
- `sweeper(suppressed)`, the last child of the window root in `ui/shell.rs`, prepaints after every
  panel and every overlay. It shows the webviews that were marked seen, hides the rest, and clears
  every mark for the next frame.

Slots live in a thread-local keyed by tab and by `WindowId`, so sweeping one window's frame never
hides another window's webviews. `overlaid(app)` in `ui/shell.rs` is the list of overlays — the
project-settings screen, the settings screen, the new-agent, clone-project, all-projects, file
dialog, remote-manager, remote-connect and file-picker states, and the two menus — that force every
webview hidden regardless of what was marked, because a modal is drawn by GPUI and a native child
view would sit on top of it. A webview shown for the first time after being hidden had never been
given screen bounds, since `gpui-wry` skips layout while it is invisible; showing it calls
`cx.notify()` once so the next frame lays it out, and showing rather than staying shown is what
keeps that from looping.

## The loader

`crates/ubiq/src/ui/viewer/web.rs` draws the `Editor` position. Three things can be on screen and
only ever one: a `gpui_component::progress::Progress` bar with a line of text until the chrome
posts `Ready`, the webview element once it has, or a sentence in the danger colour if the session's
chrome threw or the bundle failed. The webview is created hidden and the sweep is what first shows
it, so the blank white rectangle every embedded webview starts as is never seen — the loader covers
exactly that gap. A bundle still fetching shows `BundleState::unavailable()`'s sentence ("Downloading
Excalidraw… 41/554 files") over a bar that fills to that fraction — the host reports the file count
before it fetches anything, so the bar is real rather than a sweep — with a second line under it
naming the file `BundleState::fetching_file()` last landed; a browser still starting sweeps the same
bar instead, with no such line. A platform with no embedded browser at all
(`web_view::SUPPORTED == false`) says editing opens in the external browser instead, which is what
still happens there.

## Sessions are settled in `render`, not on a click

`AppState::settle_web_panels(window, cx)` runs in `app/shell.rs`'s render prologue, just before
`apply_web_documents`. It scans the open tabs for ones in `ViewLayout::Edit`, ensures a session for
each — the same `EnsureWebBundle` dance phase 4/5 built — and then `settle_web_views` builds any
webview a session is missing.

Driving this from `render` rather than from the toggle's click covers two cases with one path:
building a child webview needs a `Window`, which a click handler may not have synchronously, and a
tab can also arrive in `Edit` by being *restored* from a saved arrangement, which lands on a message
that carries no `Window` at all. Running the same idempotent pass every frame is what makes covering
both free. A session outlives a switch back to `Preview` — the component is expensive to mount and
the sweep has already taken its webview off screen — and ends only when the tab does;
`close_web_session` now also calls `web_view::close` so the native view goes with it.

## Saving goes to the file

A new bridge frame, `FromWeb::Save { document }` in `crates/ubiq/src/web_export/bridge.rs`. `⌘S`
and `Ctrl-S` never reach GPUI while the webview holds the keyboard, so
`assets/web/excalidraw/app.js` listens for the chord itself, captured, and the component renders a
`Save` button through `renderTopRightUI`, styled in `index.html` off Excalidraw's own CSS variables
rather than a literal colour. Both call the same `save()`, which flushes the 600 ms debounce, moves
the baseline and posts the document.

The window parks the text in `web_panels.incoming` and queues the tab's key in the new
`web_panels.saving`; `apply_web_documents` writes the buffer first and `flush_web_saves` then calls
the existing `AppState::save_file` — the same call the tab's own context-menu row makes. **No new
save path and no new transport message**: the version check, the dirty dot and
`ProjectFileWritten` behave exactly as they do for `⌘S`. `app/editor.rs::set_view_layout` now pushes
the layout the file *took* into the panel rather than the one asked for, since a viewer can refuse a
layout it does not offer and the panel must agree with what actually settled.

## A second tenant with no native painter draws its Preview from an export

Excalidraw's Preview position is the native scene painter, so a `.excalidraw` file costs no download
until `Edit` is chosen. draw.io has no such painter in the interface, so its Preview needs a picture
from somewhere else: a new bridge frame, `FromWeb::Preview { svg }` in
`crates/ubiq/src/web_export/bridge.rs`, carries the SVG the editor itself exports.

`crates/ubiq/src/state/diagrams.rs` files it in a second, read-only tier beside the Mermaid renderer's
— `key_for(EXPORTED, …)`, `resolve_exported`, `keep_exported` — in the same workarea directory, so a
preview survives a restart without the panel being reopened. `app/web_panel.rs` gained
`exported_preview`, `drain_exported_asks` and `keep_exported`, mirroring the diagram-cache trio; and
`ui/viewer/diagram.rs` gained `exported`, drawn beside `render` for the Preview position of a
`ViewerKind::Drawio` tab. Nothing in the interface *renders* a `.drawio` file — a document never
opened in `Edit` reads "Open the editor once to draw this." rather than a picture, since exporting
one is the editor's own act and nothing else in Ubiq can perform it.

## A `New Excalidraw` row on the explorer, and `New draw.io` beside it

`ExplorerAction::NewExcalidraw` sits beside `NewFile` and `NewFolder` in both context-menu groups in
`crates/ubiq/src/state/explorer/menu.rs`. `FileDialog::New` gains `ext: Option<String>`, and
`confirm_file_dialog` in `crates/ubiq/src/app/explorer.rs` appends that extension to the typed name
when it does not already end with it — forced rather than merely suggested, so deleting the seed's
suffix cannot turn a drawing into a plain text file. The name field is seeded `drawing.excalidraw`.
The host still creates the file empty (`PathOp::Create` always writes `b""`), which is what an empty
Excalidraw scene is.

`ExplorerAction::NewDrawio` is the same row for the second tenant, seeded `diagram.drawio`, and
`ui/file_dialog.rs`'s modal title follows suit: it is keyed off the extension being created —
`New Excalidraw`, `New draw.io`, or `New file` for any other one — rather than off a flag that only
said some extension was set, which is what lets a second forced extension add a second title with no
new branch in the dialog's own logic.

## The webcomponent evaluation

Asked for explicitly, so the answer is recorded rather than left to be re-asked. Two artefacts were
looked at, and only the second is the web component.

[`components/sample-excalidraw.html`](https://github.com/mdnmdn/wikive/blob/main/components/sample-excalidraw.html)
is not one: it is a page loading React 18 UMD, Excalidraw 0.17.3 UMD and Vue 3 from jsdelivr with
plain `<script>` tags. It is a demonstration of the UMD mounting pattern and nothing to adopt —
the line it pins was dropped after 0.17.3, and the CDN load is offline-hostile.

[`public/js/excalidraw-wc.umd.js`](https://raw.githubusercontent.com/mdnmdn/wikive/refs/heads/main/public/js/excalidraw-wc.umd.js)
is the real thing: one 7.96 MB UMD file defining `<excalidraw-component>`, with React, Excalidraw
**0.18.0** — the version this mirror already pins — and the stylesheet all inside it. Its element methods are
save, load, export-PNG, zoom-to-fit, clear and a raw-API escape hatch, beside an `initial-data`
attribute and a bubbling `change` event. `save()` is `serializeAsJSON(elements, appState, files,
"local")`, which is what this chrome already calls.

**What it would remove is most of §6.** There are no bare specifiers, so there is no import map; no
import map means no inline `<script>`, which means **no per-response CSP nonce and no template** —
`script-src 'self'` alone covers the page. The stylesheet is injected by the component, so the
`index.css` link and the rule that it must be loaded from inside the mirror both go. React is
bundled, so the `+esm` duplicate-collapsing the map does has nothing left to do. The mirror stops
being 554 files and becomes one script beside the fonts.

**What it would cost is two capabilities, one of them recoverable.** The element takes no `theme`,
so `ToWeb::Palette` would have nothing to drive — recoverable, because the raw-API escape hatch hands over the
component's own `excalidrawAPI`, and an `updateScene` carrying a themed `appState` sets it. The other is not: `load()` feeds
`JSON.parse`'d elements straight into `updateScene` and **never calls `restore`**, and `restore` is
not reachable from outside the bundle. This chrome calls it today, which is what opens a legacy or
foreign `.excalidraw` file correctly and what the churn baseline is taken after. A separate cost is
provenance: an 8 MB prebuilt artefact in another repository has no manifest and no hash behind it,
where [phase 3](./web-panel-phase3.md) verifies every file it caches against a SHA-256.

**Not adopted in this phase, and worth adopting in the next** — the trade is good once the
component gains a `theme` attribute and calls `restore` on load, and once the bundle is fetched
through the same manifest-and-hash path everything else is. Both are small changes upstream rather
than here. Phases 3 through 5 had already built and tested the ESM path this phase extends, which
is the only reason it is still the one in the tree.

The one idea taken from the reference either way is the explicit loading state, which is now
native — `ui/viewer/web.rs`'s spinner — rather than drawn inside the page.

## Unverified, and deviations

- **The embedded Excalidraw was never seen to boot.** Nothing here was driven in a real window by
  this unit: no bundle was fetched, no session was opened by hand, no drawing was made or saved.
  What is verified is `cargo test -p ubiq` (every suite), `cargo clippy -p ubiq --all-targets -- -D
  warnings`, `cargo fmt --all`, `cargo check -p ubiq --all-targets` and `cargo check -p ubiq
  --release --lib`.
- **The embedded draw.io was never seen to boot either**, and its `Preview { svg }` round trip is
  verified only at the bridge and the route, not by an editor actually exporting one: no session was
  opened by hand, no diagram was drawn, and no export landed in the disk tier a real restart would
  read back.
- **`just check` and `just verify` still cannot be run here** — the `foundation-models` Swift build
  script fails under `sandbox-exec`, as phase 5 recorded.
- **The mark-and-sweep depends on GPUI prepainting a later root sibling after every earlier
  subtree.** That is how `div` prepaints its children today; it is reasoned from the element code,
  not asserted by a test.
- **`overlaid()` is a hand-kept list.** A new overlay added to `ui/shell.rs` and not to that
  function would be covered by a webview rather than covering it.
- **Nothing shrinks the diff a round trip produces.** Phase 4/5's two mitigations — writing the
  buffer only on a real change, and preserving the file's own `source`/`type` fields — are
  unchanged and still do not make a save after a session of edits a small diff.
- **`ToWeb::Reload` is still declared and never sent.**

## Related docs

- [`./web-panel-phase2.md`](./web-panel-phase2.md) — the origin, the token and the bridge this builds on
- [`./web-panel-phase3.md`](./web-panel-phase3.md) — the shared workarea and the bundle fetch
- [`./web-panel-phase45.md`](./web-panel-phase45.md) — the chrome, the external-browser container and the edit cycle this phase moves
- [`../inbox/web-panel-proposal.md`](../inbox/web-panel-proposal.md) — the proposal this and its predecessors build
- [`../tech/decisions.md`](../tech/decisions.md) — `D108`'s answer, and the entry this phase adds beside it
- [`../features/workbench.md`](../features/workbench.md) — the viewer, its layout toggle and the file buffer
