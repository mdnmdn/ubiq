---
id: inbox-markdown-preview
title: Proposal — the markdown preview, its structure and its readability
kind: proposal
status: proposal
summary: One proposal in two halves that turn out to be the same one — the preview reads badly because every typographic choice is applied globally, and it cannot be fixed because it is a single `TextView` over the whole file that reports a height and nothing else. Splitting it into one row per root block inside GPUI's variable-height `list()` replaces the byte-offset ratios the minimap and split-scroll sync guess from with measured block bounds, and simultaneously makes per-element spacing, per-heading line height and a structural minimap reachable at all; proposed as a three-day proof with explicit kill criteria, then three phases, positioned as the prerequisite that de-risks a full native markdown element rather than an alternative to it.
read_when: you are changing how markdown is rendered, why its minimap and scroll sync are wrong, what a native markdown renderer would cost, or where block editing, annotation decorators, typography controls and markdown export should attach
updated: 2026-09-25
depends_on: [feat-workbench-ide, tech-ui, tech-components, tech-diagrams, tech-architecture]
---

# Proposal — the markdown preview, its structure and its readability

The preview renders correctly and reads worse than it should. Body and headings share one
line-height multiplier, the column is too wide for prose with no rule for where it sits, and
whitespace is spread evenly instead of placed where it carries meaning. About 14 lines fit on a
screen, on lines long enough to lose your place.

Those are the symptoms that are visible. Underneath them is one that is not: the preview is a single
`TextView` covering the whole file, and that component reports a height and nothing else. So the
minimap guesses positions from byte offsets, the split-scroll sync guesses which side moved,
anchors do not resolve, and the typography controls that already shipped reach nothing.

**These are one proposal, not two.** The typographic scale in Part III is unreachable while the
document is one opaque block — per-element spacing requires per-element addressing. Part II is what
makes Part III possible, and it is also what fixes the minimap, the sync, the anchors and the frame
budget on the way past.

**What is proposed for three days is a proof, not the feature set.** §6 is the three-day work and it
ends in a go/no-go. §7 is the roughly three weeks that follow if it goes.

---

# Part I — Diagnosis

## 1. What is wrong today

Measured from the current rendering:

| Issue | Observation | Effect |
|---|---|---|
| Uniform leading | Body and headings both at roughly 1.8–1.9× font size | A two-line H1 takes as much height as four body lines; ~14 lines visible per screen |
| Long measure | Body lines run ~90–100 characters | The eye struggles to find the start of the next line; high leading was compensating |
| Oversized H1 | ~2.3× body size with loose leading | Title wraps with a stranded last word and a large gap between lines |
| Heading attachment | Space above and below H2 is nearly equal | Headings float between sections instead of belonging to the one they introduce |
| Inline code | Same sans font, faint background with no padding or radius | Reads as a highlight, not as code or a file reference |
| Edges | ~60px dead space above H1; last line clipped at the bottom; git gutter letters clipped on the left | Wasted height at the top, broken-looking frame elsewhere |
| Horizontal placement | Text column pinned left; large empty area on the right | Content looks unfinished on wide windows |

## 2. Why it cannot be fixed in place

The preview is one `gpui_component::text::TextView` over the entire file
([`markdown.rs:376`](../../crates/ubiq/src/ui/viewer/markdown.rs)). Its `scroll_offset` is
`pub(super)`; it exposes no per-block layout, no heading id, no rendered-to-source mapping. Every
defect below is that same missing fact wearing a different hat.

| Symptom | Actual cause |
|---|---|
| Minimap marks sit at the wrong height | `structure_marks` computes `fraction` as **byte offset ÷ body length** ([`markdown.rs:767-783`](../../crates/ubiq/src/ui/viewer/markdown.rs)). A code block and a heading of equal byte length claim equal screen height. Nothing measures anything. |
| Split scroll sync drifts | `sync_markdown_split_scroll` ([`viewer/mod.rs:482`](../../crates/ubiq/src/ui/viewer/mod.rs)) re-reads both sides' scroll fractions each frame and infers who moved by which moved further, remembering last frame in `md_split_scroll` ([`state/editor.rs:542`](../../crates/ubiq/src/state/editor.rs)). It is a proportional guess between two differently-shaped documents. |
| `G138`, `G296` — anchors dead in preview | No heading id or block index to scroll to. Loci resolve against the *source* via `state/nav.rs::line_of_slug`, so a preview-only layout drops them. |
| `G91` — preview does not scroll | The document grows to natural height inside an external `ScrollHandle`, which exists only because the component's own offset is unreachable (`markdown.rs:301-310`). |
| Typography controls do nothing | One `TextViewStyle` for the whole document, so per-heading-level line height is unreachable and `MdReading::text_shade` never reaches the prose (`markdown.rs:190-217`). **Part III is blocked entirely on this row.** |
| 38ms per frame on a 150KB file, twice | No virtualization: the whole document parses and lays out every frame. `SCAN_CACHE` and `WALK_CACHE` (`markdown.rs:137,797`) are the band-aid. |

No amount of further work on top of a component that will not say where anything is produces a
correct minimap. Measurement requires that the document be laid out in addressable pieces.

---

# Part II — The structural change

## 3. The one idea

**Render one `TextView` per root block, as rows in GPUI's `list()`.**

`list()` / `ListState` (`gpui/src/elements/list.rs`) is a variable-height virtualized list backed by
a sum-tree of measured heights. It gives us, for free, the three primitives the preview lacks:

- `bounds_for_item(ix) -> Option<Bounds<Pixels>>` — a **measured** rectangle per block.
- `logical_scroll_top() -> ListOffset { item_ix, offset_in_item }` — a scroll position expressed as
  a block index plus an offset, not a pixel fraction. Immune to unmeasured heights below.
- `scroll_to_reveal_item(ix)`, `splice(old_range, count)`, `remeasure_items(range)`.

We write no text renderer. The block boundaries already exist: `walks()`
([`markdown.rs:824`](../../crates/ubiq/src/ui/viewer/markdown.rs)) already walks root-level mdast
children to build the minimap and navigator marks. Today those walks produce approximate fractions;
under the block list they produce row identities and source ranges.

The invariant the whole proposal rests on:

> **Every row knows its source byte range, and the list knows every row's measured bounds.**

Given that pair, position-to-source and source-to-position are both a binary search.

## 4. What the block list gives, against the requirements

| # | Requirement | Today | Under the block list | Phase |
|---|---|---|---|---|
| 1 | Custom block rendering (mermaid, excalidraw, images) | Works, via `extensions()` block parser/renderer hooks (`markdown.rs:618`) | Same hooks, scoped to one row; measurement stops being frame-global (see **K3**) | 1 |
| 2 | Replace a block with an editor | Not possible; the annotation surface edits a whole section in a separate `TextareaState` | A row is a container we own — swap it for `gpui_component::input::Editor` on focus, seeded from the row's source range | 2 |
| 3 | Decorators (annotation badges, gutter marks, thread counts) | Exist, but only on the *other* rendering path (`ui/kit/blocks.rs`, host-supplied `PlanBlock`s) | One row wrapper serves both; markdown tab and annotation surface stop being two renderers | 2 |
| 4 | Selectable blocks | — | `selection_adapter.rs` already keys endpoints by `EntityId` + `content_key` with a `block_ix`, and `TextSelectionCoverage::{Full,FromStart,ToEnd,Bounded}` exists precisely for selections spanning entities | 2 |
| 5 | Index / TOC | Three marks producers and a **second independent tree-sitter parse** in `ui/outline.rs:27-31` | The row list *is* the outline; navigator, minimap and outline collapse to queries over one block table | 3 |
| 6 | Virtual rendering | None — whole document every frame | Native to `list()`; only visible rows measure and paint | 1 |
| 7 | Reflow | Whatever the component does, uncontrolled | `remeasure_items(range)` on width change; unmeasured rows keep estimates | 1 |
| 8 | Search (text → position) | — | source offset → binary search block table → `scroll_to_reveal_item`. **Within-block** highlight depends on the row renderer exposing highlight ranges — partial here, complete in the native phase | 3 |
| 9 | Minimap | Byte-offset ratios | `bounds_for_item` — real positions. Unblocks §17 | 1 |
| 10 | Scroll sync | Proportional guess | `ListOffset` in block indices, matched against source ranges | 1 |
| 11 | Anchors (`G138`, `G296`) | Dead in preview | `scroll_to_reveal_item(block_ix)` | 1 |
| 12 | Personalisation / per-element spacing | One style for the document | **Per-row style** — the whole of Part III becomes reachable, except letter-spacing (§8) | 2 |
| 13 | Diff-friendly | `viewer/diff.rs` is line-based and markdown-blind | Two block tables aligned by source range and content hash; render changed blocks, collapse unchanged. Also makes **streaming** cheap — `splice` the tail block instead of reparsing the document | 3 |
| 14 | Justification | Not available | Per-block `Left`/`Center`/`Right` if the row renderer exposes it. **True justify is not reachable** — see §8 | 2 (partial) |
| 15 | Render to image / PDF | — | Not delivered here, but the block table is the precondition. See §9 | later |

## 5. Phase 0 — the three-day proof

The purpose is to answer one question before any of §7 is committed:

> **Does a per-block `TextView` lay out identically to a single one, and does selection still drag
> across rows?**

**Day 1 — the splitter.** Source → `Vec<BlockRange>` from the existing `walks()` mdast root-children
pass. Render N `TextView` rows inside `list()` behind a feature flag, beside the current path.

**Day 2 — the payoff.** Point the minimap at `bounds_for_item` instead of `structure_marks`
fractions. Drive split-scroll sync from `logical_scroll_top()` instead of
`sync_markdown_split_scroll`'s fraction comparison.

**Day 3 — adversarial.** Drag-select across three rows. Drag the window width and watch reflow. Open
a 150KB document. Put a mermaid fence in a row, scroll it out of view and back.

### 5.1 Acceptance criteria

- A heading's minimap mark sits within **2px** of that heading's actual painted top.
- Dragging either side of the split keeps the corresponding headings within **one line** of each
  other, in both directions.
- 150KB document: frame time under **8ms** (today: 38ms, twice per frame before caching).
- Visual diff against the current renderer shows no spacing or wrapping change that per-row style
  cannot correct.

### 5.2 Kill criteria — any one ends this route and makes the native element the only one

- **K1** Block boundaries introduce spacing or wrapping artefacts that per-row style cannot control.
- **K2** Selection will not drag across rows despite `selection_adapter`'s cross-entity design.
- **K3** Virtualization breaks the custom block renderers. This is the **most likely failure**:
  `diagram.rs` hands measurements over through `publish_measure` / `current_measure`, a
  **frame-local** protocol that assumes every block is laid out every frame. Virtualization voids
  that assumption directly. Budget a contingency for giving diagram rows a cached intrinsic size.
- **K4** Reflow during a width drag exceeds the frame budget on a 150KB document.

Three days ends in go/no-go with these answered. If it stops here, the cost was three days and the
native estimate is grounded instead of guessed.

## 6. If it goes — the phases after

| Phase | Work | Rough |
|---|---|---|
| 1 | Block list replaces the preview. Minimap, split sync and anchors on measured positions. Virtualization and reflow. Retire `md_split_scroll` and the byte-ratio marks. | ~1 week |
| 2 | Per-row style — **all of Part III**. Decorators and selectable blocks. Block-to-editor swap. Unify the annotation surface onto the same list. | ~1 week |
| 3 | Search-to-position. Collapse navigator + outline + minimap onto one block table, deleting the second tree-sitter parse. Streaming `splice`. Diff alignment. | ~3–4 days |

**≈ 3 weeks**, on top of the 3-day proof.

Phase 2 touches `G336`/`G337` (two buffers over one file with nothing reconciling them) and
`G332`/`G333` (decoration layers and the slash menu painted into a buffer nobody draws). Block
editing lands directly on top of that unreconciled pair, so it wants settling first or it compounds.

## 7. What this still does not fix

Honest limits, all arguments *for* the native element and none against doing this first:

- **Letter-spacing / H1 negative tracking (§12).** `gpui::TextStyle` has **no letter-spacing field at
  all**. This is a GPUI limitation, not a component one; the native element does not fix it either
  without patching GPUI upstream. §12's tracking rule is currently unimplementable — the smaller H1
  size is the mitigation.
- **True justification.** `gpui::TextAlign` is `Left | Center | Right` — there is no `Justify`
  variant. Real justification means distributing slack across word gaps and painting glyphs
  individually via `Window::paint_glyph`, possible only once we own the paint. Native phase, ~+1 week.
- **Within-block search highlight, inline decorations, fine typographic control.** These need
  `TextRun`-level authority, which means owning the renderer.

## 8. Render to image, and the PDF question

GPUI has **no offscreen surface and no draw-to-image path** — `Window::capture` is pointer capture,
not screen capture. Two routes, not competitors:

**Now — the HTML route.** `web_export/routes.rs` already renders markdown to HTML with a table of
contents using `pulldown-cmark`, independently of the viewer. Print CSS plus a headless renderer is
the cheap PDF, and it half exists. This is the recommendation for any near-term need.

**Later — the second painter.** Once the native element owns layout, the block table *is* a
pagination engine: measured block heights make page breaking "fill until height, then break". A PDF
backend becomes a second painter over the same event stream rather than a second markdown
implementation. Single-block render-to-image (exporting one mermaid diagram) falls out of the same
mechanism.

Worth stating plainly: the tree currently parses markdown **four times** — mdast in the viewer's
fence scan, mdast again inside `TextView`, `pulldown-cmark` in `web_export`, and tree-sitter in the
outline. A shared source-ranged block table is what collapses that, and export is a beneficiary.

## 9. Why this is not throwaway work

The native element's architecture is **source-ranged block table → measured index → custom element
per block**. This proposal builds the first two and leaves the third as `TextView`.

Going native later means swapping what renders *inside* a row. The list, the minimap, the sync
layer, the block table, search, the TOC and the decorators all survive unchanged. And because the
swap is per-block-type, the native element can land incrementally — headings first, paragraphs
last — instead of as one 4,000-line cutover.

Going straight to native is roughly 4–6 weeks before the minimap is correct, and it is the same
minimap either way.

## 10. Licensing note

Zed's `markdown` crate is on disk at our pinned `gpui` revision and is the best prior art for the
native phase. It is **GPL-3.0**. Ubiq Studio is proprietary. The design may be read and
reimplemented — a flat `Vec<(Range<usize>, Event)>` event stream with a bidirectional pixel↔source
index is an idea, and reconstructing it from `pulldown-cmark`'s own API is clean-room. The code may
not be copied. Anyone working the native phase needs to know this before opening the file.

---

# Part III — The readability design

Everything in this part is a phase-2 deliverable and is unreachable before §3 lands.

## 11. Principles

1. **Leading shrinks as size grows.** Body text needs generous line height; headings need tight.
2. **Block spacing does the separating.** Space between paragraphs and around headings communicates
   structure. Line height does not.
3. **Headings belong to what follows them.** Space above a heading is always larger than below.
4. **Measure and leading are linked.** Longer lines need more leading. Neither is chosen alone.
5. **Density is a budget.** Every point of vertical space earns its place. The target is *more*
   visible lines than today, not fewer.
6. **Everything is relative to body size.** All values in `em`, so a font-size change scales the
   page consistently.

## 12. Typographic scale and vertical rhythm

Values relative to body size (1em = body point size):

| Element | Size | Line height | Space before | Space after |
|---|---|---|---|---|
| Body paragraph | 1.0 | 1.45–1.55 | 0 | 0.7em |
| H1 | 1.6–1.75 | 1.15 | 0 | 0.6em |
| H2 | 1.3 | 1.2 | 1.4em | 0.35em |
| H3 | 1.1, semibold | 1.25 | 1.1em | 0.25em |
| List item | 1.0 | 1.45 | 0 | 0.25em |
| Code block | 0.9, monospace | 1.35 | 0.6em | 0.8em |
| Table cell | 0.95 | 1.35 | — | — |

Additional rules:

- The H1 gets slight negative tracking (about −0.02em). **Not currently implementable** — see §7.
  The reduced H1 size mostly removes the problem it was compensating for.
- Heading lines are balanced where the text engine allows, so a two-line title splits evenly.
- Paragraph spacing stays well under one full body line, so paragraphs read as continuous flow.

**Expected result.** In the reference window, visible lines go from ~14 to roughly 19–21, and the
title shrinks from ~160px to ~100px. The page should not feel cramped, because whitespace moves to
section boundaries where it carries meaning.

## 13. Text measure

The text column is sized in **characters**, not points: target columns × the body font's average
character width. This keeps "80 columns" true when the user changes font size or typeface.

Line height follows the measure, per principle 4:

| Measure | Body line height |
|---|---|
| ≤ 65 ch | 1.45 |
| 75–80 ch | 1.5 |
| 90–100 ch | 1.55–1.6 |
| Full pane width | 1.6+ |

Interpolate between these rather than switching in steps, so resizing never causes a visible jump in
leading.

### 13.1 Width presets

| Preset | Text measure | Line height | Intended for |
|---|---|---|---|
| Readable (default) | ~75 ch | 1.5 | Long-form prose, proposals |
| Wide | ~95 ch | 1.55 | Dense technical docs with inline code and file names |
| Full | Pane width | 1.6+ | Table- and code-heavy files |

The preset is remembered per window or per file type, not globally. A table-heavy spec and a prose
proposal should not share one setting.

### 13.2 Breakout elements

Prose stays capped at the chosen measure; wide elements may grow beyond it up to the pane width:
tables, code blocks, images and diagrams. Breakouts align to the text column's left edge and extend
right. With breakouts in place, most reasons to choose Full go away.

Under the block list a breakout is simply a row with a different width rule — which is why it is
cheap here and awkward today.

## 14. Horizontal layout

### 14.1 Centered or left

```
available = pane width − minimap width − minimap gap   (minimap terms are 0 when hidden)

if available ≥ column width + 2 × min margin:
    center the column in `available`
else:
    left-align the column with inset = min margin (2–3em)
```

Centering on wide windows frames the content. Left alignment on narrow windows avoids thin margins
on both sides and stops text shifting on every small resize.

### 14.2 Insets

- **Top:** about one body line above the first block, removing today's ~60px of dead space.
- **Bottom:** enough padding for the last line to scroll fully clear of the frame — at least a third
  of the viewport height, so the final section can be read at eye level.
- **Left gutter:** git status markers are hidden in preview mode or given a gutter wide enough not
  to clip. Long term they move to the minimap (§17.4).

## 15. Inline elements

### 15.1 Inline code

- Monospaced at ~0.9× body size, with a small baseline offset so it sits optically on the baseline.
- Background as a rounded chip with 2–3pt horizontal padding and a 3–4pt corner radius, in a tone
  slightly more contrasted than today.
- The chip is a **custom paint pass**. A plain background-colour attribute hugs the glyph box with
  no padding or radius, which is exactly today's highlight look. In GPUI this is a `paint_quad`
  under the glyphs, which requires owning the paint — so full fidelity is the native phase, and
  phase 2 gets the font and size change only.

### 15.2 File references

Inline code matching a file in the workspace renders as a link to that file, with chip styling plus
a subtle link affordance. In a tree of interlinked specs this turns references into navigation.
(Recorded as unbuilt in `features/workbench-ide.md` §6.2.)

### 15.3 Emphasis

Italics use the font's true italic cut. If the chosen face has none, pick a face that does rather
than accepting a synthesized oblique. (`workbench-ide.md` §6.3.)

## 16. Density modes

Because every value is a multiplier of body size, a density toggle swaps only the line-height and
spacing values:

| Mode | Body line height | Paragraph space after | H2 space before |
|---|---|---|---|
| Comfortable (default) | 1.5 | 0.7em | 1.4em |
| Compact | 1.35 | 0.5em | 1.1em |

Compact serves users who scan long specs and prefer lines per screen over air.

## 17. Side minimap

The minimap shows the whole document as a narrow strip on the right edge. Its job is orientation and
fast navigation: where am I, how long is this, where are the sections, what changed.

### 17.1 Placement

- Docked to the **right edge of the pane**, fixed width ~80–120pt, gap ~2em from the content.
- Its width is subtracted from available space **before** the centering rule in §14.1 runs, so it
  never overlaps content and centering stays visually honest.
- It hides automatically when `available` would drop below column width + 2 × min margin. Readable
  text always takes priority.
- Breakout elements stop at the minimap gap and never extend underneath it.

### 17.2 What it draws

The minimap renders **structure, not shrunken text**. Miniature text at 10% scale is unreadable
noise; shapes are legible at any scale.

| Document element | Minimap representation |
|---|---|
| H1, H2 | Short labeled bar; heading text shown if it fits legibly (~8–9pt), otherwise just the bar |
| H3 | Thinner, unlabeled bar |
| Paragraph | Grey lines whose lengths roughly follow the real line lengths |
| Code block | Tinted rectangle |
| Table | Small grid pattern |
| Image or diagram | Neutral filled rectangle |

Heading labels are the most valuable part; in a numbered spec the minimap doubles as a TOC.
(Recorded as unbuilt in `workbench-ide.md` §8.2.)

### 17.3 Scale and viewport

- Vertical position maps linearly from document to minimap coordinates.
- Short documents draw at a fixed maximum scale and are **not stretched** to fill the strip, so a
  one-page file looks like one page.
- Long documents scale to fit. Where scale becomes too small to be useful, the minimap itself
  scrolls in proportion to the main view.
- The visible region is a translucent viewport rectangle. **The markdown-tab minimap supplies no
  viewport rect today** — only the annotated-document path does.
- Width presets and breakouts are reflected proportionally.

### 17.4 Interaction and change markers

- Click jumps to that position; dragging the viewport rectangle scrolls.
- Hovering a heading bar shows the full heading text.
- Git change markers appear as small coloured ticks on the outer edge, replacing the clipped letters
  in today's left gutter and showing every change in the file at a glance, not only those on screen.

### 17.5 Rendering

- The minimap is built from **the main view's layout results** — that is precisely
  `ListState::bounds_for_item`, and precisely what does not exist today. §17 as a whole is
  unimplementable before §3.
- Its image is cached and updated incrementally, invalidating only affected regions.
- It updates on resize after layout settles, not on every intermediate frame.
- The strip itself is a `canvas()` element painting quads directly — no per-mark element tree.

## 18. Implementation notes — GPUI

> The previous revision of this section described AppKit and SwiftUI APIs
> (`minimumLineHeight`, `paragraphSpacingBefore`, `.lineSpacing`, TextKit 2 fragment passes). None of
> those exist in this codebase, which is Rust on GPUI. Replaced.

- **Line height** is `TextStyle::line_height: DefiniteLength`, which accepts an absolute value rather
  than only a multiple of the font's own line height — so §12's per-element values are expressible
  directly, *once each element has its own style*. That is the phase-2 dependency.
- **Block spacing** is margin on the row container, not a text property. Under the block list,
  "space before / space after" is literally the row's `mt`/`mb` — which is why per-element spacing
  becomes trivial rather than impossible.
- **Letter-spacing has no GPUI equivalent** at any layer. See §7.
- **Measure** is computed from the body font's average advance at the current size, via
  `TextSystem::layout_line` on a representative string, recomputed on font-size change. `char_advance()`
  in `viewer/diff.rs:196` already does this for the diff viewer and is the pattern to reuse.
- **Inline code chips** and the **minimap** both need direct painting (`Window::paint_quad`,
  `canvas()`). The minimap can have it now; the chips need the native element.
- **Reflow** on width change is `ListState::remeasure_items(visible_range)`, not a full relayout.

## 19. User control panel

Already implemented in `viewer/md_options.rs` — a header popover carrying width preset, density,
minimap on/side, character-scale slider, text-shade picker and "make default". The controls exist;
what this proposal gives them is **effect**. Today the text-shade picker persists per-document state
that never reaches the prose, because colour is installed once window-wide.

Phase 2 wires the existing panel to per-row style. No new controls are proposed, with one
exception: a **body typeface** option (§20).

Note `md_options.rs:11-24` — there is no per-project settings scope on the wire, so "make default"
is window-wide. A per-project scope needs a new `SetProjectSettings` message and is out of scope here.

# Part IV — criteria and questions

## 20. Acceptance criteria

Structural (phase 0, from §5.1): minimap mark within 2px; split sync within one line; 150KB under
8ms; no uncorrectable layout change.

Readability (phase 2), on the reference window:

1. Visible body lines per screen go from ~14 to at least 19.
2. Prose lines stay within the selected preset's measure (±5 characters).
3. Every heading has more space above than below.
4. The reference H1 fits on one line, or on two balanced lines.
5. Inline code is visually distinguishable from highlighted text without reading its content.
6. No element is clipped at any edge, including the last line of the document.
7. The minimap never overlaps content and hides before the text column drops below its minimum
   margins.
8. The text-shade picker visibly changes prose colour.

## 21. Open questions

Resolved in review:

- **Body typeface** — offer a serif option (Source Serif, Charter, Literata) for long-form prose,
  keeping sans for headings. Make it an option rather than a switch of default.
- **Minimap default** — user-enabled/disabled, not automatic by document length.
- **Table of contents** — the header dropdown navigator is the text list; the minimap's heading
  labels do not have to carry that job alone.

Open:

- Does per-row `TextView` carry an entity cost that virtualization does not amortise on a document
  with several hundred blocks?
- Should the annotation surface migrate onto the block list in phase 2, or after the native element
  lands? It is the largest consumer and the one with unreconciled buffers.
- Does `TextViewStyle` expose alignment per instance, or does §4 item 14 slip entirely to native?
- Is a cached intrinsic size enough for **K3**, or do diagram rows need to opt out of virtualization?
- Do the §12 values survive contact with a real 150KB spec, or are they tuned for a short document?

## Related docs

- [`features/workbench-ide.md`](../features/workbench-ide.md) — the viewer and its layouts; §6.2,
  §6.3 and §8.2 there are the unbuilt items this proposal picks up
- [`tech/components.md`](../tech/components.md) — what `gpui-component` owns
- [`tech/diagram-format.md`](../tech/diagram-format.md) — the fence renderers of **K3**
- [`tech/ui-and-design.md`](../tech/ui-and-design.md) — theme tokens and the no-literal-colour rule
- `backlog.md` — `G91`, `G138`, `G296`, `G332`, `G333`, `G336`, `G337`, `G338`
