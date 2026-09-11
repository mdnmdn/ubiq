---
id: ref-servo-engine
title: Servo — the embeddable web engine
kind: reference
status: current
summary: Servo as an external dependency Ubiq could embed — the libservo embedding API and its stability, the window and offscreen rendering contexts and what a Metal/GPUI bridge would take, platform and build cost, web-platform completeness measured against WPT, the multiprocess and sandbox model, licensing, and the reference embedders that exist.
read_when: you are weighing an embedded web engine for a panel or a plugin surface, or you need Servo's actual capabilities before assuming them
updated: 2026-09-10
verified: 2026-09-10
depends_on: [tech-decisions]
---

# Servo — the embeddable web engine

**Servo** is a web engine written in Rust — layout, style, script bindings, and the WebRender
GPU compositor — published as a library (`libservo`) for other applications to embed. It lives at
[servo/servo](https://github.com/servo/servo), with the project site at
[servo.org](https://servo.org/) and design documentation at [book.servo.org](https://book.servo.org/).

It is the only production-track web engine that a Rust application can link directly, which is what
makes it interesting to Ubiq: the alternatives are the platform's own view (WKWebView on macOS,
WebView2 on Windows, WebKitGTK on Linux) reached through a C or Objective-C boundary, or a
Chromium embedding.

**Checked:** 2026-09-10, against the project's published releases and monthly reports.

## What this is not

Servo is not a browser Ubiq would ship, and not a replacement for the interface. Ubiq draws its own
tree with GPUI; the question a reader comes here with is narrower — whether a *region* of that tree
can be a web engine's output, and at what cost.

## 1. The embedding API and its stability

- Servo published `servo` **0.1.0 to crates.io on 2026-04-13**, the first release installable as an
  ordinary Rust dependency. Before that, embedding meant vendoring the monorepo.
- The API is **explicitly pre-1.0**. Monthly releases may carry breaking changes. An **LTS track**
  exists alongside them — half-yearly upgrades with security backports — for embedders unwilling to
  chase the monthly cadence.
- The public surface is small and shaped for an embedder that owns its own event loop: `Servo` and
  `ServoBuilder` for the engine instance, `WebView` with its own builder and a `Delegate` trait for
  each view, `RenderingContext` for where pixels go, and a delegate for event-loop integration. A
  `WebView` handle's lifetime is the view's lifetime, so create and destroy are the embedder's call.
- A `servo_capi` crate exposing a **C ABI** over `libservo` is in progress, intended as a
  stable-ABI shared-library entry point for non-Rust embedders. Its scope is minimal — webview
  creation and the event loop.
- Governance sits with **Linux Foundation Europe**, with published governance documents, rather
  than with Mozilla Research as it did originally.

## 2. Rendering — how pixels reach an embedder

This is the section that decides whether Servo can live inside a GPUI window, and it is where the
gap between the design and the shipped code is widest.

- Two rendering contexts exist: `WindowRenderingContext`, which paints to a native window, and
  `OffscreenRenderingContext`, which does not. `WindowRenderingContext::offscreen_context()`
  produces the second from the first, so a view can paint to a sub-region rather than demand the
  whole window.
- Internally, canvas and WebGPU output reaches WebRender as **ExternalImages**, which is the seam a
  GPU-import bridge hooks into.
- **There is no upstream wgpu, Metal or GL interop path.** The zero-copy "render into another
  engine's texture" work that exists — a `ServoWgpuRenderingContext` demonstrated against iced and
  Bevy — is a third-party project ([mark-ik/wgpu-graft](https://github.com/mark-ik/wgpu-graft)), not
  a blessed API, and carries no compatibility promise.
- The path that needs no new work is a **CPU readback** of the offscreen frame, re-uploaded to the
  host renderer's texture — a GPU→CPU→GPU round trip per frame.
  *Unverified: the exact readback entry point was not confirmed against `doc.servo.org`.*
- **No Servo/GPUI integration exists.** For Ubiq's stack the realistic options are the readback
  path or a custom Metal texture-sharing bridge written and maintained in-tree. Both are
  unprecedented work, and GPUI is itself pre-1.0.

The one structural advantage this buys, and the reason the section matters: an offscreen context
yields *pixels*, so the result clips to a scroll container and obeys z-order like any other element.
A platform child view composites above the window's own surface and does neither.

## 3. Platforms and build cost

- Officially developed and supported on 64-bit **macOS, Linux, Windows, OpenHarmony and Android**.
- `Servo.app` builds are distributed **unsigned**, so macOS Gatekeeper blocks them for end users.
  This is the project's own distribution, not a constraint on an embedder that signs its own
  binary. *Unverified against the current download page.*
- Binary size is an acknowledged and only partly mitigated concern: the project has begun making
  components optional at build time (gamepad support among them). **No published figure.**
- **No published build-time figure.** A full engine — WebRender, script, style, layout, media — on
  top of the Rust toolchain should be assumed substantial. Publication to crates.io reduces but does
  not remove the native toolchain and system library requirements.

## 4. Web-platform completeness

- **Web Platform Tests: roughly 62% passing**, up from about 30% over two and a half years. The
  project's own dashboard is at [servo.org/wpt](https://servo.org/wpt/).
- 2026 landings show real velocity: the CSS attr, image and calc functions, font feature settings and
  media queries in the June release (558 commits); incremental accessibility-tree updates, a
  multi-threaded canvas and up to tenfold faster text rendering in July. DuckDuckGo rendering
  correctly was itself release-note material at 0.5, which sets the expectation for arbitrary sites.
- 141 tracked features have **zero velocity** — they need architectural work before they move, and
  they gate any pass rate much above the current one.
- Of 593 Baseline Widely Available features, 154 are not WPT-mapped at all (78 of those are
  JavaScript built-ins tracked through test262), so the single percentage both understates and
  overstates the gap depending on what is being counted.
- One outside analysis ([webtransitions.org](https://webtransitions.org/servo-readiness/)) puts the
  project at roughly 22 new Baseline features a year against the web platform's ~52, and projects a
  plateau near 80% at current staffing. *Third-party projection, not a project statement.*

What this means for content: Servo is credible for a **known, controlled document** whose markup and
scripting an embedder can constrain and test. It is not credible for **arbitrary third-party
HTML/CSS/JS**, because the author writes and tests against Chrome and every gap arrives as a bug
report against the embedder.

## 5. Process and security model

- The intended architecture mirrors Chromium and WebKit2: a trusted embedder process plus multiple
  less-trusted content processes. Multiprocess is the expected mode for serious use; single-process
  exists mainly for testing and simple embedding.
- In multiprocess mode each script thread runs in its own content process, launched with
  `--content-process`, coordinated by one **Constellation** per Servo instance which owns every
  webview's content processes and routes input events.
- Content processes are meant to use OS sandboxing to restrict system-resource access, and
  `components/constellation/sandboxing.rs` exists in tree. Rust's memory safety is a second
  mitigation layer. *Coverage and maturity on macOS specifically are unverified.*
- Against WKWebView or a Chromium embedding, the architecture is directionally the same and the
  hardening is not comparable: those sandboxes have years of adversarial exposure. **Servo should be
  treated as materially less proven for containing hostile script**, which is the relevant bar
  whenever the content is third-party.

## 6. Licensing

**MPL-2.0**, which is file-level copyleft. Linking and embedding it in a proprietary binary
carries no obligation to release the embedding application. Any Servo *file* that is modified stays
MPL-2.0 and its source must be made available, so an embedder that patches the engine tracks which
files it patched. There is no whole-binary copyleft effect.

## 7. Reference embedders

- **[Verso](https://github.com/versotile-org/verso)** (versotile-org) is the closest thing to a real
  embedder: a browser built to explore the embedding patterns, with the stated ambition of becoming
  a webview library. An experimental `tauri-runtime-verso` substitutes it for Tauri's default
  runtime. Both are pre-stable — Verso's own notes say it is not accepting feature requests and its
  navigation workflow is unpolished.
- The iced and Bevy integrations mentioned in §2 are the other public examples, and are
  proof-of-concept.

## 8. Where this bears on Ubiq

Three existing positions frame any use of Servo here, and this document changes none of them.

- **D7** put a GPU-drawn native tree in place of a web frontend because a serialization boundary in
  the per-frame render path cannot survive a full-refresh terminal stream. A CPU readback per frame
  (§2) is that boundary, which rules Servo out of anything in the per-frame path and leaves only
  deliberately-opened, self-contained surfaces.
- **D44** rejected an offscreen webview for diagrams, holding that drawing is what the interface is
  for. Servo does not reopen that; it is an engine choice, not an argument about what to draw
  natively.
- The **web panel** proposal's compositing risk — a platform child view ignoring scroll clipping,
  dock z-order, tab strips, modals and drag previews — is the one problem an offscreen context
  addresses structurally rather than by manual hide-and-detach. That is the case where Servo is
  worth measuring, and the measurement is a small spike: offscreen context to GPUI texture, at a
  realistic panel size, against the child-view approach.
- For **plugin UI**, the completeness gap (§4) and the sandbox maturity gap (§5) both fall on the
  wrong side, because plugin markup is third-party by definition. A declarative contribution set is
  a contract Ubiq can honour completely; a web engine is one it cannot.

## Related docs

- [Decision register](../tech/decisions.md) — D7 and D44, the two positions §8 measures against
- [A2UI protocol](./a2ui-protocol.md) — the declarative alternative to shipping markup to a surface
- [Backlog](../backlog.md) — where the offscreen-context spike is filed
