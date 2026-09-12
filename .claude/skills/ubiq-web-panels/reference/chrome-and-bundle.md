# The chrome page, its policy, and the vendor mirror

## `window.ubiq`, the whole API a page gets

`bridge.js` is served to every tenant and is not per-tenant. It exposes exactly two functions:

```js
window.ubiq.post(frame)   // one FromWeb frame, as a plain object with a `type`
window.ubiq.on(handler)   // called with each ToWeb frame, in order
```

Both transports are its business, not the page's. The `wry` transport is used only where the host
injected `window.__ubiq_ipc = true` — `window.ipc` exists in every `wry` page and proves nothing —
and otherwise
frames go out as `POST bridge` and come back on a long-poll `GET bridge` that backs off on error
and stops on `pagehide`. An outbound POST is retried with backoff and complained about in the
console if it never lands — a dropped `ready` used to mean a panel that loaded for ever.

The token is read out of `location.pathname` (`_web/<app>/<token>/…`) and repeated on every
frame. A frame that does not carry it is dropped by the server.

## The boot contract

A chrome page must, in this order:

1. register its handler **before anything can arrive** and queue what does, because frames are
   sent as soon as the session exists and the component mounts later;
2. do whatever it needs to mount — for a mirrored package, fetch `vendor/importmap.json`, inject
   it as an inline `<script type="importmap">` carrying the page's nonce *before* the first
   module import, set any absolute asset-path global the package needs, then `import()` the
   entry point;
3. wait for the component to actually hand over its API rather than assuming it has, with a
   ceiling that fails loudly rather than waiting for ever;
4. drain the queue through the real handler;
5. `post({ type: "ready" })` — **once, and only when an `Open` can be accepted**. This is what
   lifts the panel's loader;
6. handle `open`, `reload`, `palette` and `save_requested`, and ignore an unknown `type` rather
   than throwing — a frame from a newer interface must not break the page;
7. answer any failure with `post({ type: "error", message })` *and* show it in the page. Every
   path out of `boot()` ends in `ready` or in `error`; a third way out is a bug, and the one bug
   this design is most prone to.

What it sends back: `dirty` the moment the document differs, `changed { document }` when it has
been still long enough to be worth writing (Excalidraw debounces 600 ms), and `save { document }`
when the user asks. `⌘S` never reaches GPUI while the webview holds the keyboard, so the page
listens for the chord itself, captured, and sends `save`.

## The policy

The page is served from the interface's own loopback origin under a CSP built in `routes.rs`.
`CHROME_CSP` is the narrow one: `default-src 'self'`, `style-src` inline for the page's own
`<style>` block, and `frame-ancestors`, `base-uri` and `object-src` at `'none'`. A page that needs
more gets its own function beside it, and every widening carries its reason — `excalidraw_csp`
widens exactly two things, a per-response `'nonce-…'` on `script-src` because an import map has no
external form, and `data:`/`blob:` on `img-src` for embedded images and export previews. It
deliberately does *not* widen `worker-src`: the one module worker is built from `import.meta.url`
and `default-src 'self'` already covers it.

The nonce is a fresh `server::mint_token()` per response substituted into `__UBIQ_NONCE__` in the
template. That is why a page with an inline import map is a `TEMPLATE` in `build.rs` rather than
an `ASSET`: it is never sent verbatim, so it is never gzipped either.

A *mirrored document* served under `vendor/` — rather than the chrome page itself — runs under a
second, separate policy: `VENDOR_DOCUMENT_CSP`, attached by `serve_vendor` whenever `vendor_mime`
answers HTML. draw.io's own `index.html` is what this is for: `'unsafe-inline'` and `'unsafe-eval'`
on `script-src` for the webapp's inline bootstrap and mxGraph's runtime-built shape and formula
functions, `data:`/`blob:` where an embedded image, font or export travels, and
`frame-ancestors 'self'` because the chrome frames it. It still names no remote origin.

No remote origin ever appears in a policy. If the page fails to find a font or a chunk, the
failure must be loud — a CSP that permits the CDN turns a missing mirror file into a silent
network fetch, which is the supply-chain hole the whole mirror exists to close.

## The mirror

```
<shared workarea>/web/<app>/<version>.bundle       one file: every entry, deflated, then a JSON
                                                    index and a footer — see `archive.rs`
<shared workarea>/web/<app>/<version>.bundle.part  where a fetch in progress writes
```

One compressed file, not a directory of hundreds of small ones: `crates/ubiq-host/src/web_assets/
archive.rs` writes it, `crates/ubiq/src/web_export/archive.rs` reads it — opened once per session,
seeking and inflating one entry per request, never extracting to disk. The import map is an entry
inside the archive at a fixed name (`web_assets::IMPORT_MAP_FILE`), not a sibling file.

Rules the host enforces, and a tenant does not get to relax:

- **Verified before written.** Length and SHA-256 against the manifest; a mismatch discards the
  bytes, fails the whole fetch and caches nothing.
- **Nothing is rewritten.** Each entry's path is the manifest's path exactly, so a package's own
  absolute `/npm/…` imports resolve through a single prefix rule in the import map.
- **The rename is the marker, and there is no resume.** Every verified entry lands in the sibling
  `.part` file; only the rename over the real name proves the bundle is done. A process killed
  mid-fetch leaves a `.part` file that is never read back — the next ask overwrites it and fetches
  every entry again.
- **Versioned by file.** `VERSION` is pinned in generated source; a build that pins a new one looks
  for a file that does not exist, fetches, and leaves the old one alone. No TTL, no staleness
  check.
- **Verified bodies travel over a bounded channel to the one thread that owns the writer** — the
  `PARALLEL` download workers only fetch and verify, so 554 bodies never race a write.

Progress reaches the window as `WebBundlePending { done, total, file }` — the count goes out
before the first byte, so the loader's bar is a real fraction and the line under it names the file
that last landed.

## The manifest generator

`_tools/webassets.py`, run through four recipes: `just web-assets` / `web-assets-drawio`
(snapshot, one per tenant) and `just web-assets-verify` / `web-assets-verify-drawio` (re-fetch
everything in that tenant's committed manifest and report drift). `--tenant {excalidraw,drawio}`
picks which `snapshot` walks; `verify` reads whatever manifest `--out` names and needs no tenant
of its own.

**Two generating rules, one per tenant shape.** `excalidraw`'s walks jsDelivr's package-file API
for the npm package's `dist/prod`, checking jsDelivr's own published per-file digest; walks the
`+esm` dependency closure by scanning the bytes for bare specifiers and hashing those itself,
because jsDelivr says not to SRI-pin `+esm` output; builds an import map of one `/npm/` prefix rule
plus per-specifier entries plus the set that collapses the stray React copies onto one; and writes
`VERSION` as the package version plus a digest over the whole snapshot, so the directory name
changes whenever any file does. `drawio`'s has no npm package to walk: it enumerates
`src/main/webapp/` from the **GitHub tree API** at a pinned tag (`jgraph/drawio`, `v31.4.5`),
because jsDelivr refuses to list a package over its size ceiling; keeps only what an offline embed
needs, dropping `stencils/`, `templates/`, `img/`, `math4/`, `plugins/` and `WEB-INF/`; fetches the
bytes off jsDelivr's GitHub CDN mirror all the same, retrying its transient `403`s in rounds; and
hashes every file itself at snapshot time, since GitHub publishes no per-file digest either — a
later mismatch means upstream changed a file at that tag, which is the event worth failing on. The
committed manifest is 243 files, 33.3 MiB, `VERSION = "31.4.5-cd257901251e"`. Its `IMPORT_MAP` is
`"{}"`: draw.io loads through classic `<script>` tags, so there is nothing to route through one.
The `Entry` type itself is shared with Excalidraw's manifest rather than redeclared.

**One deliberate exception to "the cache path mirrors the URL path exactly."** Excalidraw's entries
keep the jsDelivr path (`npm/@excalidraw/…`) because one import-map prefix rule has to resolve
against it. draw.io has no import map — it is a whole site whose every reference is relative to the
directory its own `index.html` sits in — so its mirror root **is** the webapp root: `Entry::path` is
relative to `src/main/webapp/`, and the chrome frames `vendor/index.html` naming no version at all.

Reach for the tenant's own generating rule — `snapshot_excalidraw` or `snapshot_drawio` — rather
than editing either's literals for a new tenant; add a third beside them, and its own
`manifest_<tenant>.rs`, for a shape neither fits. Both write a header saying which tool wrote the
file and how to regenerate it.
