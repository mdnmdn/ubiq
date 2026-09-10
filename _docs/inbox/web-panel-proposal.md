---
id: inbox-web-panel
title: Proposal — web panels, and Excalidraw editing as the first one
kind: proposal
status: proposal
summary: A reusable panel whose body is a webview over the interface's own loopback origin, with a typed JSON bridge in place of an ad-hoc `postMessage`, a versioned asset cache that downloads a vendor bundle from a CDN once and works offline afterwards, and Excalidraw editing as the first and only tenant it is proposed for — the native scene painter keeping the read path and gaining the source view it never had.
read_when: you are deciding whether HTML may appear inside a panel, how a JavaScript component talks to the interface, where a downloaded vendor bundle is cached, or whether an Excalidraw document becomes editable
updated: 2026-09-07
depends_on: [feat-workbench, tech-architecture, tech-structure, tech-decisions, inbox-viewers, inbox-capture]
---

# Proposal — web panels, and Excalidraw editing as the first one

Ubiq draws Excalidraw scenes and cannot edit them. This proposes the mechanism that would change
that: **a web panel** — a dock panel whose body is a webview pointed at the interface's own loopback
origin, talking to Rust over a typed bridge — and **Excalidraw as its first tenant**, loading the
file's JSON, reporting changes back, and saving through the file family that already exists.

The mechanism is the proposal. Excalidraw is the reason to build it and the only tenant proposed
here, because a panel that can host arbitrary HTML is a door, and a door is worth opening once, for
a named reason, with the hinges written down.

## 1. Where it stands

**The scene viewer is read-only by design, and says so twice.**
`crates/ubiq/src/ui/viewer/scene.rs` paints a `state::scene::Scene` parsed from the file's JSON, and
its module doc states: *"The viewer is read-only. It draws the scene and nothing else; editing is
not proposed and is not built."*
[`completed/file-viewers-proposal.md`](./completed/file-viewers-proposal.md) §7 is where that came
from, and is more emphatic: *"Editing is not proposed and should not be."* Nothing turns a `Scene`
back into JSON — there is no writer and no mutation path.

**The bytes are already in a buffer, and the save path already exists.** An `.excalidraw` file is
not binary to the interface: `draws_bytes()` is true for `Image` alone, so it lands in
`FileBody::Text` with a real `EditorState` that `viewer/mod.rs` reads the scene source out of and
never draws. The dirty dot, `⌘S`, save-as and `D37`'s version check are already wired to it. And
`WriteProjectFile { project_id, rel_path, bytes, expected }` with `ProjectFileWritten { version }`
are in `crates/ubiq-proto/src/messages.rs` today. **This proposal adds no transport message.**

**There is no webview in the workspace.** No `wry`, `tao`, `tauri` or `objc2-web-kit` in
`Cargo.lock`, and neither `gpui` nor `gpui-component` declares a `webview` feature at the pinned
revisions — there is no `WebView` element to reach for. This is the one place the proposal adds a
dependency, and §3 is the argument for it.

**But an HTML surface already exists, in the right crate.** `crates/ubiq/src/web_export/` runs one
process-wide `tiny_http` listener on `127.0.0.1:0`, registering roots under a slug and returning
`http://127.0.0.1:<port>/<slug>/`. `D55` put it in the interface deliberately. It is the origin a
webview needs, already built and already sanctioned.

**And so does a disposable content-addressed cache.** `crates/ubiq/src/state/diagrams.rs` keys
SHA-256 over a cache-format marker, the renderer's version, the palette and the source, writing
through a sibling temp file and a rename. That is §5's shape — including the part where a new
version of Ubiq re-fetches what it needs: put the bundle's version in the key and staleness stops
being a policy and becomes arithmetic.

## 2. The rule

**A web panel is a document, a bridge and an origin — never a browser.**

It renders one named component over one subject the interface already holds, exchanges typed
messages with Rust and nothing else, and may not navigate: no address bar, no link following, no
second origin, no arbitrary URL. That difference is enforced rather than intended — the chrome
page's content security policy names the loopback origin and `'self'`, and nothing else.

Three consequences follow, and they are the whole of what makes this safe to add.

**The webview never touches the filesystem.** It is handed a document and hands one back; it
resolves no path, opens no file, and does not know a project exists. That is rule 2 of
[`../tech/architecture.md`](../tech/architecture.md) restated for a second runtime, and it means the
bridge carries values rather than capabilities.

**The bridge is typed on the Rust side** — two enums per app, JSON on the wire, unknown variants
logged and dropped. Not a string channel that grows a dialect.

**A web panel is opened, never fallen into.** No extension routes to one; the user asks. The native
path stays the default, which for Excalidraw means `scene.rs` keeps the read path forever (§7).

## 3. Why a webview now, and what `D7` and `D44` actually decided

This tree has rejected a webview twice, so the reversal has to be argued rather than assumed.

**`D7` replaced a Tauri and `xterm.js` frontend with GPUI**, for a specific reason: *"several
full-refresh terminals under a stream of escape sequences is the load the UI has to survive, and a
GPU-drawn native tree carries it without a serialisation boundary in the render path."* That objects
to a serialisation boundary **in the per-frame render path of the whole application**. A web panel
puts none anywhere near a frame: every other surface stays as it is, and one panel — opened
deliberately, closed when done — hosts a component that draws itself. `D7` chose what draws Ubiq; it
did not decide no part of Ubiq may ever be HTML, and `D55` proved that by shipping an HTML surface.

**`D44` is the closer call, and it went the other way for a reason that does not hold here.** It
rejected an offscreen webview for Mermaid in favour of `merman`, reasoning that *"a Mermaid document
is text, and the bus carries a file's bytes… drawing is the whole of what the interface is for"*,
and that *"the renderer bakes colours in, so a theme switch is a re-render"*. Every word of that is
about **producing a picture**, and it is right: a picture is a pure function of source and palette,
so a Rust implementation beats a browser.

**Editing is not a picture.** What no amount of painting reproduces is an *interaction model* —
multi-select, eight-handle transform with rotation, arrow binding that survives moving the shape it
points at, containers with reflowing text, grouping, alignment, snapping, the library, undo across
all of it, and a format that keeps changing underneath. `.claude/excalidraw.py` and `scene.rs`
reproduce the *read* subset and are honest about their limits. Reproducing the *write* half is not a
bigger subset of the same job; it is reimplementing an application against a moving target.

So the line drawn here is `D44`'s, extended rather than contradicted:

> **Rendering belongs in the interface. Authoring in somebody else's format belongs to somebody
> else's editor.**

The two platforms Ubiq ships are the two where this is cheapest: macOS gets `WKWebView` from the
system, Windows gets WebView2 from a runtime Windows 10 and 11 carry. Linux — WebKitGTK, a real
install burden — has no release recipe in this tree, so the decision it would force is not due yet.

## 4. The pattern — what a web panel is made of

Four parts. Excalidraw uses all four; a second tenant reuses three and writes only its chrome.

### 4.1 The origin

The `web_export` server grows a second kind of route beside its project slugs:

```
http://127.0.0.1:<port>/_web/<app>/<token>/index.html   the chrome, from baked-in assets
http://127.0.0.1:<port>/_web/<app>/<token>/vendor/…     the cached bundle, from the shared workarea
```

`Registry` gains a second map, app id to a `WebApp` entry, and `routes.rs` one branch; the traversal
refusal, the dotfile refusal and the byte serving are reused unchanged.

**The token is not decoration.** The server serves whatever asks on loopback; today the worst that
costs is a local process reading project files, which `D55` accepted. A bridge that carries a *save*
raises that materially, so every panel mints a random token at open, it appears in the URL and in
every frame, and a frame without it is dropped. Loopback is not an authentication boundary and this
proposal does not treat it as one.

### 4.2 The container

`wry`, building a child webview over the GPUI window's raw handle, sized to the panel's rectangle.

**This must be spiked before anything else is written**, because a native child view is not a GPUI
element: it composites *above* the GPUI surface, so it does not clip to a scroll container, ignores
the dock's z-order, and will cover a tab strip, popover, modal or drag preview that overlaps it.
Everyone who embeds one solves this the same tedious way — hide or detach the webview when the panel
is not the visible tab, when a modal is up, and while a dock drag is in flight. The spike must
establish that GPUI exposes a usable window handle at the pinned revision and that
show/hide/move/clip behave well enough for the dock. **If it fails, the fallback is already built**:
same chrome, same origin, same bridge, opened in the external browser as `D55` opens an export.

Focus is the second container question. A webview takes keystrokes natively, which the workbench's
*exactly one thing holds focus* model knows nothing about. A focused web panel is, for the keyboard,
what a terminal pane is: everything goes in except a named escape chord, on `D45`'s model.

### 4.3 The bridge

A duplex of JSON frames, with **two transports behind one JavaScript surface** because the container
may be either:

| Container | Web → Rust | Rust → Web |
|---|---|---|
| Embedded webview | `wry`'s IPC handler | `evaluate_script` |
| External browser | `POST /_web/<app>/<token>/bridge` | long-poll on the same route |

`assets/web/bridge.js` picks whichever exists and exposes `ubiq.post(msg)` and `ubiq.on(handler)`. A
chrome page is written once and never learns which container it is in.

On the Rust side a web app is a descriptor — id, chrome entry point, the bundle it requires, and its
two message types:

```
enum ToWeb   { Open { document, palette }, Reload { document }, Palette { .. }, SaveRequested }
enum FromWeb { Ready, Changed { document }, Dirty, Error { message } }
```

Excalidraw's are those; a second tenant declares its own and the plumbing is generic over them.

**Frames are documents, not commands.** The web side may not ask Rust to read a file, list a
directory or run anything — it receives content and returns content. That is what keeps the bridge
from becoming an RPC surface with the interface's whole authority behind it.

### 4.4 The chrome

One HTML file and one module per app in `assets/web/<app>/`, baked in the way `web_export`'s CSS and
JS already are: `build.rs` gzips, `assets.rs` `include_bytes!`es. Mount the component, wire `ubiq.on`
to it, wire its change callback to `ubiq.post`. It is *not* the vendor bundle, which is §5.

## 5. The asset cache

**The vendor bundle is downloaded, never bundled.** Excalidraw with React is 25 MiB (§6); baking
that into the executable inflates every build and every release for a feature most sessions never
open. So: fetch once from a CDN, keep on disk, work offline afterwards.

**Where it goes.** Not the project workarea — the same bundle in five projects is the same bytes and
is a property of no project. It belongs in a **shared workarea**: one directory the host reserves for
the interface, alongside the per-project one rule 6 already defines. That is the one structural
addition outside the panel itself, and it is a field on `HostInfo` rather than a path the interface
composes, for exactly rule 6's reason — *"the interface never composes the path out of
`HostInfo.config_root`; it uses the string it was handed, which is what makes a host on another
machine a change of value rather than a change of code."* The rest of rule 6 applies unchanged: the
host makes it and never looks inside, and everything in it is disposable.

```
<shared workarea>/web/<app>/<bundle version>/…
```

**Versioned by directory, not invalidated by policy.** The version is pinned in Rust source, so a
build that pins a new one looks in a directory that does not exist, fetches, and leaves the old one
for the next sweep. No staleness question and no TTL — the same property `diagrams.rs` gets from
putting `RENDERER` in its key and `FileHarnessCache` gets from keying on the binary's version.

**What integrity means here.** A manifest in Rust source lists every file with its expected SHA-256;
a file whose hash does not match is discarded rather than cached. A downloader that trusts a CDN to
have served the right bytes is a supply-chain hole with a nice interface. Generating the manifest is
a `_tools/` script and a `just` recipe, run when the pinned version changes — never at runtime.

§6's two halves earn that manifest differently. Excalidraw's own files come from jsDelivr's package
API, which publishes a **stable** SHA-256 per file; all 300 were fetched and verified against it. The
dependency closure is `+esm` output, which jsDelivr generates on demand and explicitly says not to
pin with subresource integrity — so the manifest records **our own** hashes, taken once at snapshot,
and a later mismatch means the CDN regenerated the bundle, which is the event worth failing on.

**Fetching is the host's, not the interface's.** Downloading is network plus disk, and the
interface's sanctioned exceptions (`D44`, `D54`, `D55`) are each narrow and argued. The host already
owns `ureq`. The interface asks; the host fetches, verifies, unpacks into the shared workarea and
answers when it is there; the interface serves those bytes off its own origin. This is the one place
the proposal is genuinely unsure which side wins, so §11 records it as a question — the alternative,
the interface fetching into its own workarea, is shorter by a message family and wider by a rule.

**Failure is a downgrade, not an error.** No network and nothing cached means Edit is disabled with a
reason and the native viewer is untouched — so a first run offline loses editing and nothing else.


## 6. What the bundle actually is

Measured, not reasoned. A full offline mirror was built and driven in a real browser with every
non-loopback request blocked at the network layer: Excalidraw booted, drew, opened its Radix
colour-picker popover, exported SVG and PNG, ran its font-subsetting module worker and rendered CJK
text — **zero blocked requests, zero page errors, zero console warnings** across 114 requests.

**It is ESM only** — no UMD build in 0.18.x or 0.17.x — and React is a peer (`^17 || ^18 || ^19`)
whose npm package is CommonJS, so an ESM React must come from a CDN transform. **An import map is
therefore mandatory**: `dist/prod/index.js` imports **28 bare specifiers** a browser cannot resolve
(`react`, `roughjs/*`, `jotai`, `pako`, `perfect-freehand`, `@radix-ui/react-popover`, …) plus four
more dynamically. Everything internal to Excalidraw is relative, so **its own files need no
rewriting at all**.

**Two sources, one generating rule each, no hand-maintained list.**

| Part | Source | Files | Bytes |
|---|---|---|---|
| Excalidraw's `dist/prod` | jsDelivr, enumerated from `data.jsdelivr.com`'s manifest | 300 | 17,692,835 |
| The 28 dependencies, transitively closed | jsDelivr `/npm/<pkg>@<ver>/+esm` | 258 | 8,623,740 |
| **Total** | | **558** | **26,316,575** (25.1 MiB) |

The closure is generated: seed with the 28 specifiers at Excalidraw's own pinned versions, fetch,
enqueue every `/npm/…` string in each body. The cache path mirrors the URL path exactly, which is
what makes zero content rewriting possible. Two alternatives were measured and rejected — `esm.sh` is
774 files and 40 MB with 140 query-string URLs that resolve only through server-side redirects, and
jsDelivr's `+esm` for Excalidraw *itself* re-bundles per entry point, inflating two subsetting chunks
from 174 and 376 bytes to 1.85 and 1.82 MB.

**One hazard, and its correct fix.** `+esm` output hard-pins whichever React each transitive package
was *built* against, so the raw closure drags in five React copies and four react-dom copies,
statically imported by Radix internals Excalidraw genuinely renders — *"Invalid hook call"*. An
import map collapses them onto one, and that is correct rather than a hack: React is a peer
dependency, so npm and every bundler already dedupe it for every real Excalidraw user. Verified —
Radix 1.0.x, authored against React 18.2, opens, positions and closes its popover cleanly on 19.2.5.
The map is ~42 entries: 33 bare specifiers, 4 React entry points, 9 collapse entries, and **the
collapse half is generated by scanning the downloaded bytes, never hardcoded**, since those inner
pins are jsDelivr's build-time choices and move when it regenerates.

**Fonts sit beside the stylesheet, and that is not negotiable.** Two independent mechanisms: 230 font
URIs resolve through `window.EXCALIDRAW_ASSET_PATH` — which must be an *absolute* URL to the
directory containing `fonts/`, since a `/`-relative value resolves against `location.origin` and
throws where there is none — while the four Assistant UI faces are referenced by `index.css` as
relative `url("./fonts/…")`. **Any missing font is a silent network attempt**: Excalidraw always
appends its own `esm.sh` path to the candidate list, verified by watching it reach for the network
with the local copy absent. A mirror must be complete or it leaks.

**The 25.1 MiB is mostly one optional thing.** Fonts are 13.1 MB, of which the Xiaolai CJK face alone
is 12.7 MB. Everything that is not a font, a locale or the Mermaid converter — React, Radix, jotai,
roughjs, pako, all 28 dependencies — is **107 files and 603 KB**. Three profiles were built and each
passes the same suite: **full** (558 files, 25.1 MiB, nothing lost); **no CJK font** (349, 13.0, CJK
glyphs fall back and 3 remote attempts when CJK is typed); **no CJK, no Mermaid, English only** (145,
3.8, Mermaid import 404s). **Take the full 25.1 MiB** — one rule, no feature gaps, and 25 MiB is
noise beside a tantivy index. The trims exist, with known consequences, if that changes.

**Three runtime facts the container must respect.** `file://` fails outright — CORS blocks the module
graph from origin `null`, and forcing past that dies on MIME — which settles §4.1's loopback origin.
The **MIME rule is a hard failure, not a degradation**: `+esm` files are extensionless, and serving
them as `application/octet-stream` yields *"Expected a JavaScript-or-Wasm module script"* and a blank
panel, so the route must answer `text/javascript` for `+esm`/`.js`/`.mjs`, plus `text/css` and
`font/woff2`. And there is **one module worker** — font subsetting, built from `import.meta.url`,
wrapped in a try/catch that falls back to the main thread; it constructed successfully from the local
origin. **There is no service worker**, which removes the usual offline-mirror complication entirely.


## 7. Excalidraw, as the first tenant

**The native painter keeps the read path.** Opening a `.excalidraw` file draws it exactly as today —
no browser, no download, no delay, and `_docs/design/` stays readable on a cold offline start. The
webview appears only when the user asks to edit, which is what makes this compatible with the
read-only rule rather than a repeal of it.

It is the shape [`capture-proposal.md`](./capture-proposal.md) already chose when it reversed
read-only for a scene Ubiq authored — *"The scene editor edits a capture, not a document"* — and it
answers the question that proposal left open, *whether `.excalidraw` files should become editable now
the machinery exists*: yes, and not through that machinery. Capture edits a scene Ubiq made with
Ubiq's own tools and never needs a browser; this edits a document somebody else's application owns,
in that application. Neither blocks the other.

**The toggle it arrives as, and the gap it closes.** Excalidraw is `has_preview() == false` today, so
its panel draws no header strip at all — which is `G71`, *"excalidraw no code view"*. Turning it on
gives the file the three-way toggle every other viewer has: **Source** is the JSON already sitting in
the buffer, already highlighted; **Preview** is the painter. `G71` closes on that alone, with no
webview and no bundle, and is worth doing whether or not the rest is taken. Editing is then a fourth
position on the same strip, and `ViewLayout` gains an `Edit` variant that persists with the others.

**The cycle.** Edit is chosen: the bundle is ensured, the panel mints a token, registers with the
origin and opens the webview. `Ready` arrives and the interface sends `Open { document, palette }` —
the buffer's current text, and the window's palette as Excalidraw's own `theme`. The user draws;
`onChange` fires continuously, so the panel marks the tab dirty at once and serializes on idle rather
than per event. `Changed { document }` writes into the **existing `EditorState` buffer**, which is
the whole trick: the dirty dot, `⌘S`, save-as, `D37`'s version check and the Source view keep working
because every one of them is already attached to that buffer. `⌘S` then sends `WriteProjectFile` with
the version it read, and `ProjectFileWritten` clears the dot. No new message, no new save path.

**The churn is the honest cost.** Excalidraw reserializes the whole document — key order, `version`
and `versionNonce` counters on every element, `appState` fields the file may never have had. A save
after a session of edits is a large git diff even where little moved, and a round-tripped file is not
byte-identical to the one that was read. Two partial mitigations: write only when the element set or
the persisted `appState` subset actually differs, and preserve the file's own `source` and `type`
fields rather than emitting the editor's. Neither makes the diff small. A user who wants a tidy diff
should not round-trip a file they did not change, and Ubiq should not save one it was only shown.


## 8. Failure

| What happens | Result |
|---|---|
| The platform has no webview, or the spike's constraints cannot be met | Edit is unavailable and says why; Source and Preview are unaffected |
| No network on a first run, nothing cached | Edit is disabled with the reason; the whole read path is unaffected |
| A downloaded file's hash does not match the manifest | It is discarded, nothing is cached, and Edit reports a failed download |
| The bridge sends a frame with a bad or missing token | Dropped and logged; the panel is not disturbed |
| An unknown message variant arrives in either direction | Logged and dropped; the panel keeps working |
| The chrome fails to load, or the component throws | The panel shows the failure and offers Source; the buffer is untouched |
| The webview is closed with unsaved changes | The buffer is dirty, exactly as if the text had been typed; the tab asks on close as any dirty tab does |
| The file changed on disk while the webview held edits | `D37` refuses the save, which is `G111`'s existing gap and not made worse |
| A modal, a popover or a dock drag overlaps the panel | The webview hides for the duration; §4.2 |
| The panel is not the visible tab | The webview detaches; nothing runs behind a tab the user cannot see |

## 9. What it costs, in one place

Five, each argued where it arises. A **dependency with a platform behind it** (§3). A **second
runtime and a pinned bundle version** somebody must keep bumping under a chrome page that mounts it.
A **native child view fighting a GPUI dock** (§4.2) — the cost most likely to be underestimated, and
why phase 0 is a spike with a stated fallback rather than a task. A **save that rewrites the file**
(§7). And **a door held open**: once a panel can host HTML, everything becomes a candidate, and the
pressure for a second and third tenant will not arrive with §3's argument attached. §2's rule is the
guard, and it is worth exactly what it is enforced with.


## 10. Phases

0. **The spike.** A `wry` child webview in a GPUI window on macOS: does the pinned `gpui` expose a
   usable window handle, and do position, resize, show/hide and overlap behave for the dock? Nothing
   below is written until this answers. A negative answer selects the external-browser container and
   the rest proceeds unchanged.
1. **Source view for Excalidraw.** `has_preview()`, the toggle, `G71` closed. No dependency, no
   bundle, worth landing on its own.
2. **The origin and the bridge.** The `_web/` routes, the token, `bridge.js`, both transports, and a
   trivial demo panel on the kitchen sink page to test them against.
3. **The asset cache.** The manifest, the `_tools/` generator, the fetch, the verify, the shared
   workarea, and the downgrade when it is empty.
4. **The Excalidraw chrome, read-only.** Mount it, load the document, render, save nothing. This
   proves the entire loop with nothing at stake.
5. **Editing.** `Changed` into the buffer, the debounce, the `Edit` layout, the churn mitigations.
6. **Windows**, or the decision that it stays macOS-only for now.

## 11. What this asks to be decided

Decision rows, if this is taken:

- **The interface may host a webview in a panel, over its own origin, for authoring in a format
  somebody else owns.** `D7` governs what draws Ubiq, `D44` where a picture is produced; neither
  covers this, and §3 is the argument.
- **A web panel is a document, a bridge and an origin, never a browser** — no navigation, no second
  origin, a CSP that says so, a bridge carrying content rather than capability.
- **The bridge is authenticated by a per-panel token**, because loopback is not a boundary and the
  bridge can now cause a write.
- **A vendor bundle is downloaded and cached, never linked into the binary**, keyed by pinned
  version and verified against a manifest in source.
- **The host reserves a shared workarea for the interface**, alongside the per-project one, given as
  a value on `HostInfo` rather than composed — rule 6 widened by one directory.
- **Excalidraw documents become editable, through Excalidraw**, while the native painter keeps the
  read path — reversing `completed/file-viewers-proposal.md` §7's *"editing is not proposed and
  should not be"* for documents, and answering `capture-proposal.md`'s open question.

Open questions this does not settle, for `backlog.md` if it is taken:

- Which side fetches the bundle — the host, at the cost of a message family, or the interface into
  its own workarea, at the cost of widening a rule (§5).
- Whether the shared workarea is bounded, evicted or aged. `G66` asks this of the per-project one and
  has no answer; a second directory is a second instance of the same gap.
- What a remote host means for both workareas — `Q10` restated, and worse here, since a bundle the
  interface must serve off its own origin cannot live on another machine's disk.
- What a web panel persists across a window rebuild. A panel writes down what it is looking at, never
  what it drew, so the payload is its subject and its layout and the webview is rebuilt from those.
- Whether `Edit` belongs on `ViewLayout` with the other three or is a separate axis, being a
  container rather than a view of the same bytes.
- Whether a webview's own custom scheme handler could replace the loopback server — it would remove
  the port, the token and a class of local exposure, and §6's mirror is scheme-agnostic, but ES
  modules *and* a module worker under a custom scheme is the one thing the probe could not test and a
  wrong answer is a hard failure. Loopback is proven; this is the optimisation to try afterwards.
- What happens when jsDelivr regenerates a `+esm` bundle and the recorded hashes stop matching —
  whether the fix is re-snapshotting on a version bump, or vendoring the closure somewhere Ubiq
  controls.


## Related docs

- [`completed/file-viewers-proposal.md`](./completed/file-viewers-proposal.md) — the read-only rule this proposes to lift for documents, and the viewer model it lifts it within
- [`capture-proposal.md`](./capture-proposal.md) — the other reversal of read-only, for scenes Ubiq authored, and the open question this answers
- [`../tech/architecture.md`](../tech/architecture.md) — rule 2 and rule 6, which §2 and §5 work inside
- [`../tech/decisions.md`](../tech/decisions.md) — `D7`, `D44`, `D45`, `D55`, `D37`
- [`../features/workbench.md`](../features/workbench.md) — the dock, the viewer toggle, and the workarea cache
- [`../tech/project-structure.md`](../tech/project-structure.md) — where the module, the assets and the cache go
