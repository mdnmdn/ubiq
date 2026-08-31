---
id: inbox-viewers
title: Proposal — file viewers and the editor
kind: proposal
status: proposal
summary: One panel per open file instead of one shared buffer, and a viewer for each kind of file that is not plain text — Markdown, Mermaid rendered by a webview in the host and cached, Excalidraw drawn natively, and diffs the host computed.
read_when: you are deciding how a file is opened, what draws it when it is not plain text, or where a diagram is rendered
updated: 2026-08-31
depends_on: [inbox-panels, feat-workbench, tech-architecture]
---

# Proposal — file viewers and the editor

Ubiq opens a file into one shared editor and draws it as text, whatever it is. This proposes two
changes: **one panel per open file, each with its own buffer**, and **a viewer per kind of file that
is not plain text** — Markdown with a live preview, Mermaid, Excalidraw and diffs.

It is the companion to [`movable-panels-proposal.md`](./movable-panels-proposal.md), which turns
every area of the workbench into a movable panel. Neither blocks the other, and each is worth less
alone: viewers without the dock are tabs the user switches between, and a dock whose centre holds
only plain text is a smaller idea than it should be. The payoff is the two together — a Markdown
preview dragged beside its source, a Mermaid diagram beside the agent drawing it, a diff above the
terminal that produced it.

## 1. Where it stands

**The editor is one buffer wearing several tabs.** There is a single `Entity<EditorState>` on
`AppState`; `activate_editor_tab` reads the outgoing file's text back into its `OpenFile`, then
re-seeds the shared editor with the incoming one. It works, and it is a copy on every tab click.

**Nothing that is not plain text is drawn as anything else.** `FileLanguage::Markdown` selects a
syntax highlighter, not a renderer: no preview, no diagram, no diff, and no seam where one would
attach. `_docs/design/` is full of Excalidraw scenes this application cannot show.

## 2. The rule

**A viewer is a pure function of bytes and a kind.** It opens no file, resolves no path and spawns
nothing; it is handed content and draws it. Usually that content is the file. Sometimes — a diff, a
Mermaid diagram — it is something the host made from the file, which changes where the work happens
and not the rule. That is
[`../tech/architecture.md`](../tech/architecture.md)'s rule 2 — the UI never assumes the
pseudo-terminal is local — extended to the filesystem, and the line
[`project-handling-proposal.md`](./project-handling-proposal.md) already draws for the explorer and
the editor. The two compose: that one says where a file's bytes come from, this one what is drawn
with them.

## 3. One panel per file

Today `EditorPaneState` is a `Vec<OpenFile>` and an index, drawn by a hand-rolled tab strip over one
shared `EditorState`. Under the dock, **each open file is its own panel**, its tab belongs to the
group it sits in, and it carries its own `EditorState` or its own viewer.

That is a simplification rather than a port. One `EditorState` per file means **the buffer copy on
every tab click disappears** — there is no outgoing buffer to write back, because nothing was shared
— and `OpenFile` stops being a row in a `Vec` that something else indexes and becomes what a panel
holds. The cost is N editor entities where there was one, which is what every editor in this class
carries.

It is also what makes a viewer possible at all. A preview cannot share a buffer with a source, and a
diagram has no buffer to share; the shared `EditorState` is the reason there is nowhere for a
renderer to attach today.

## 4. Choosing a viewer

The path's extension selects a viewer kind, and anything unrecognised falls through to the editor —
the general case rather than a fallback. `FileLanguage` on `OpenFile` half-exists already, and grows
into a `ViewerKind` that selects a renderer with the highlighter as one arm.

| Kind | Draws | Built from |
|---|---|---|
| `Editor` | The text, highlighted | `gpui-component`'s `Editor`, as today |
| `Markdown` | Source, preview, or both | `gpui-component`'s `markdown()` text view |
| `Mermaid` | The diagram | SVG the host rendered offscreen and cached |
| `Excalidraw` | The scene | A canvas painted from the scene's JSON |
| `Diff` | Hunks, unified or side by side | Rows the host computed |
| `Image` | The image | GPUI's image element; PNG, JPEG, GIF and WebP decode already |

## 5. Markdown

The pattern is the one in `refs/gpui-playground/src/examples/markdown.rs`: an `Editor` bound to the
source on one side, `markdown(source)` on the other, and a three-way toggle — source, preview, split
— in the panel's header. The component library's Markdown view handles GFM including tables, task
lists and inline images, and renders an unknown code fence as plain monospace rather than failing.
Which of the three layouts the panel is in is the one piece of per-panel state this proposal
introduces, and it is what the panel persists, so a document reopens as it was left.

## 6. Mermaid

Mermaid is a language rather than a data format, and its grammar keeps growing. Nothing in Rust
implements it, and no headless DOM does either: Mermaid measures its text against a real browser,
which is why its own command-line tool is a browser driving a page. Laying out a subset in Rust would
therefore be a treadmill — flowcharts and sequence diagrams, and never upstream.

So **the host renders a diagram once in an offscreen webview and caches the result**, and the viewer
draws what comes back.

**The render is not a screenshot.** `mermaid.render()` returns an SVG *string*, so the browser is
needed only for the DOM Mermaid measures against, and what is read back is text from one script
evaluation — no framebuffer, no snapshot, no device pixel ratio, no scale factor, and sharp at any
zoom. GPUI draws it directly: its SVG element takes raw bytes and rasterises them through `resvg`,
against a font database loaded from the system.

**Every moving part is the host's.** The webview, the vendored Mermaid bundle and the cache sit
behind the bus; the UI spawns nothing and opens no cache file, which is rule 2 unchanged. This is
also **the one thing in this proposal that adds a transport family** — `RenderDiagram { source,
palette }` out, `DiagramRendered { svg }` or `DiagramFailed { reason }` back. The dock needs no
message; a diagram that is rendered elsewhere does.

**The cache is content-addressed and disposable.** Its key is the source's hash, the bundle's version
and the palette; its value is SVG bytes. It belongs in a shared cache root rather than under a
project, because the same diagram in two projects is the same bytes, and it may be deleted at any
time — [`config-persistence-proposal.md`](./config-persistence-proposal.md)'s cache class is exactly
this. The palette is in the key because Mermaid bakes its colours in at render time: two entries per
diagram, and the theme toggle chooses between them.

**What it costs, and it is not free.** A webview is a heavy dependency — the system one on macOS,
WebKitGTK on Linux, which is a real install burden. `D7` removed a web view from Ubiq, but its
objection was a serialisation boundary in the per-frame render path under a stream of escape
sequences; a once-per-diagram render off the frame path and behind a cache is a different trade, and
this proposal reads `D7` as not covering it. The bundle is vendored and never fetched. A cold render
pays the browser's start, so the viewer has a pending state and the host owns how long a warm webview
is kept. And an SVG carries the coordinates the browser measured, so a font `resvg` resolves
differently overflows its box — the one thing a spike has to look at rather than assume.

The last cost is permanent: **a picture has no hit-testing and no text selection**, and never will.
For Mermaid there is nothing there to lose, which is what makes the trade good. For Excalidraw there
is, which is why the next section does not take this route.

## 7. Excalidraw

Excalidraw is the opposite case: **data with a closed vocabulary**, not a language. A file is JSON —
a flat list of elements, rectangle, ellipse, diamond, arrow, line, text, freedraw, frame and image,
each with geometry, stroke, fill and binding. It is drawn natively, with GPUI's `canvas()` and
`PathBuilder` as `refs/gpui-playground/src/examples/drawing.rs` demonstrates. The viewer is
read-only: it draws the scene, and pans and zooms it. Editing is not proposed and should not be.

The vocabulary is already established here: `_tools/excalidraw.py` renders the same subset to clean
vector SVG for the wireframes under `_docs/design/`, and
[`../tech/diagram-format.md`](../tech/diagram-format.md) documents what it does and does not
reproduce. The in-app viewer draws that subset and inherits the same stated limit — **the hand-drawn
`roughness` style renders as clean vector**, and hachure and cross-hatch fills render solid. Embedded
images arrive as data URIs in the file's own `files` map and go through GPUI's image element. This
makes `_docs/design/` readable inside Ubiq, which is a second reason to build it and a free corpus to
test against.

Sending this through the webview instead would be heavier and worse: it costs a browser to produce a
flat picture, when a few hundred lines of painting give theme-aware output, sharpness at any zoom,
and later the hit-testing that lets a click on a shape follow its link.

## 8. Diffs

A diff is not a file, which is what makes it different from the other three. Its content is a
comparison — working tree against index, a commit against its parent, a file before and after an
agent's edit — and computing it means reading version control, which belongs to the host. So the
**host computes hunks and sends them; the viewer draws rows**, and no diff library enters the UI.

The row rendering is a shape the tree already has: the chat's `EDIT` tool block draws a unified diff
as one styled row per line, in status tokens at low alpha. The viewer is that at file scale, with a
side-by-side layout as its second mode and the same header toggle Markdown uses. Two things follow. A
diff is what the explorer's git badges should open onto — a modified file clicked with a modifier
gets its diff rather than its text. And an agent's edit in the chat transcript becomes something that
can be **opened as a panel**, which is where the dock and the viewers stop being two features.

## 9. Fences inside Markdown

The diagram viewers exist twice: as panels over a file, and as fenced blocks inside a Markdown
document. The second is not extra work — the component library's Markdown view exposes a block-parser
and block-renderer hook, so a fence tagged `mermaid` or `excalidraw` is intercepted before the
default code-block conversion and drawn by the renderer the panel already uses. One renderer per
format, two call sites. A Mermaid fence resolves through the same host render and cache the panel
uses, so a document holding several of them fills in as the answers arrive. It is also what makes an
Excalidraw Markdown file — a scene stored with its prose alongside, which `_docs/design/_old/`
already contains — render as a document with its drawing in it.

## 10. What a viewer persists

A panel writes one opaque payload into the dock's saved layout, and
[`movable-panels-proposal.md`](./movable-panels-proposal.md) holds that mechanism. The rule this
proposal adds is what a viewer may put in it.

**A viewer's payload is what it is looking at, not what it drew** — a path, a viewer kind, a layout
mode, a scroll position. Never a parsed scene, a computed diff or a rendered diagram, all of which
are functions of bytes the host will send again. A Mermaid panel in particular persists its source's
identity and nothing else: the SVG lives in the render cache, keyed by content, and is regenerated if
it is not there.

## 11. Failure

| What happens | Result |
|---|---|
| A viewer's bytes have not arrived | The panel draws its header and an empty body until they do |
| A Mermaid diagram does not parse, or its render fails | The panel and the fence show the source with the renderer's own message, and nothing is cached |
| The host has no webview | Mermaid shows its source and says so; every other viewer is unaffected |
| A cached render's bundle version or palette is stale | The key misses and the diagram is rendered again |
| An Excalidraw element type is not drawn | It is skipped; the rest of the scene draws |
| A diff is asked for where there is no version control | The host answers that there is none, and the panel says so rather than showing an empty diff |
| A file's extension matches no viewer | It opens in the editor, which is the general case rather than a fallback |

## 12. Phases

1. **One panel per file.** Each open file gets its own `EditorState`; the buffer copy on tab switch
   goes. Needs the dock, and nothing else here does.
2. **Markdown and diff viewers.** The two with no new drawing in them — one is the component
   library's text view, one is styled rows.
3. **Excalidraw.** Its input is data rather than a language, its subset is already specified by
   `_tools/excalidraw.py`, and `_docs/design/` is a corpus to test against.
4. **Mermaid.** The host's offscreen webview, the vendored bundle, the render family and the
   content-addressed cache. It depends on no phase above it, and is the only one that adds a
   dependency — which is a reason to take it last and on its own.
5. **Fences.** The block-renderer hook, wiring both diagram renderers into Markdown.

Every phase is worth more after the file-content family of
[`project-handling-proposal.md`](./project-handling-proposal.md) exists, and none is blocked on it —
a viewer built first reads the fixtures in `state/sample.rs`, exactly as the editor does today.

## 13. What this asks to be decided

Four decision rows, if this is taken:

- Each open file is a panel with its own buffer, replacing the single shared `EditorState` and the
  copy on every tab switch.
- A viewer is a pure function of bytes and a kind, whether those bytes are the file's own or
  something the host made from it. No viewer opens a file.
- Mermaid is rendered to SVG by an offscreen webview in the host and cached by content, rather than
  laid out in Rust over a subset. That accepts a webview dependency in the host, which this proposal
  reads `D7` as not covering, and it is the one place a viewer adds a transport family.
- Excalidraw is drawn natively rather than through the same webview, because it is data with a closed
  vocabulary and a picture can never be clicked.

Backlog rows left open: which webview the host embeds, and whether a platform without one degrades to
source or is simply unsupported; whether `resvg` resolves the fonts the browser measured with, and
what a mismatch looks like; how long a warm webview is kept; and Excalidraw's hand-drawn stroke and
hachure fills, which render clean and solid as they do in `_tools/excalidraw.py`.

## Related docs

- [`movable-panels-proposal.md`](./movable-panels-proposal.md) — the dock these panels live in, and the layout they persist into
- [`project-handling-proposal.md`](./project-handling-proposal.md) — where a viewer's bytes come from
- [`config-persistence-proposal.md`](./config-persistence-proposal.md) — the cache class the render cache belongs to
- [`../tech/architecture.md`](../tech/architecture.md) — rule 2, which §2 extends to the filesystem
- [`../tech/diagram-format.md`](../tech/diagram-format.md) — the Excalidraw subset already rendered here
- [`../features/workbench.md`](../features/workbench.md) — the editor and its tabs as they stand
- [`../tech/decisions.md`](../tech/decisions.md) — `D7`, which §6 argues does not cover a cached render
