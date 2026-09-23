# Proposal — readability of the native Markdown preview

The preview renders correctly, but it reads worse than it should. The problems are not in any single element. They come from a few global choices applied everywhere: one line-height multiplier for every style, a column that is too wide for prose but has no rule for where it sits, and whitespace spread evenly instead of placed where it carries meaning. The result is a page that looks airy but shows only about 14 lines per screen, with lines long enough to lose your place.

This proposal replaces those global choices with per-element spacing, a measured text column, a clear horizontal layout rule, and a side minimap that fits into that layout instead of competing with it. The goal is a preview that is comfortable to read *and* dense enough to scan a long spec.

## 1. What is wrong today

Measured from the current rendering:

| Issue | Observation | Effect |
|---|---|---|
| Uniform leading | Body and headings both at roughly 1.8–1.9× font size | A two-line H1 takes as much height as four body lines; ~14 lines visible per screen |
| Long measure | Body lines run ~90–100 characters | The eye struggles to find the start of the next line; high leading was compensating |
| Oversized H1 | ~2.3× body size with loose leading | Title wraps with a stranded last word ("them") and a large gap between lines |
| Heading attachment | Space above and below H2 is nearly equal | Headings float between sections instead of belonging to the one they introduce |
| Inline code | Same sans font, faint background with no padding or radius | Reads as a highlight, not as code or a file reference |
| Edges | ~60px dead space above H1; last line clipped at the bottom; git gutter letters (M, U) clipped on the left | Wasted height at the top, broken-looking frame elsewhere |
| Horizontal placement | Text column pinned left; large empty area on the right | Content looks unfinished on wide windows |

## 2. Principles

1. **Leading shrinks as size grows.** Body text needs generous line height; headings need tight line height.
2. **Block spacing does the separating.** Space between paragraphs and around headings communicates structure. Line height does not.
3. **Headings belong to what follows them.** Space above a heading is always larger than space below it.
4. **Measure and leading are linked.** Longer lines need more leading. Neither is chosen in isolation.
5. **Density is a budget.** Every point of vertical space should earn its place. The target is more visible lines, not fewer, compared with today.
6. **Everything is relative to body size.** All values are expressed in `em` (multiples of body point size), so user font-size changes scale the whole page consistently.

## 3. Typographic scale and vertical rhythm

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

- The H1 gets slight negative tracking (about −0.02em) to look tighter at display size.
- Heading lines are balanced where the text engine allows it, so a two-line title splits evenly instead of leaving one word behind. If balancing is not available, the smaller H1 size mostly removes the problem.
- Paragraph spacing stays well under one full body line, so paragraphs read as a continuous flow.

**Expected result.** In the same window as the current screenshot, visible lines go from about 14 to roughly 19–21, and the title shrinks from ~160px to ~100px of height. The page should not feel cramped, because whitespace moves to section boundaries where it carries meaning.

## 4. Text measure

The text column is sized in **characters**, not points: target columns × the body font's average character width. This keeps "80 columns" true when the user changes font size or typeface.

Line height follows the measure, per principle 4:

| Measure | Body line height |
|---|---|
| ≤ 65 ch | 1.45 |
| 75–80 ch | 1.5 |
| 90–100 ch | 1.55–1.6 |
| Full pane width | 1.6+ |

Interpolate between these rather than switching in steps, so resizing the window never causes a visible jump in leading.

### 4.1 Width presets

A width selector offers three presets:

| Preset | Text measure | Line height | Intended for |
|---|---|---|---|
| Readable (default) | ~75 ch | 1.5 | Long-form prose, proposals |
| Wide | ~95 ch | 1.55 | Dense technical docs with lots of inline code and file names |
| Full | Pane width | 1.6+ | Table- and code-heavy files |

The preset is remembered per window or per file type, not globally. A table-heavy spec and a prose proposal should not have to share one setting.

### 4.2 Breakout elements

Prose stays capped at the chosen measure, but wide elements may grow beyond it up to the available pane width:

- tables
- code blocks
- images and diagrams

Breakout elements align to the text column's left edge and extend right. With breakouts in place, most reasons to choose Full go away: prose stays readable while wide content still gets room.

## 5. Horizontal layout

### 5.1 Centered or left

The text column is **centered** in the available space when there is room, and **left-aligned with a fixed inset** when there is not:

```
available = pane width − minimap width − minimap gap   (minimap terms are 0 when hidden)

if available ≥ column width + 2 × min margin:
    center the column in `available`
else:
    left-align the column with inset = min margin (2–3em)
```

Centering on wide windows frames the content. Left alignment on narrow windows avoids thin, awkward margins on both sides and stops the text from shifting on every small resize.

### 5.2 Insets

- **Top:** about one body line above the first block. This removes the current ~60px of dead space.
- **Bottom:** enough padding for the last line to scroll fully clear of the frame. At least a third of the viewport height is recommended, so the final section can be read at eye level instead of at the bottom edge.
- **Left gutter:** git status markers are either hidden in preview mode or given a dedicated gutter wide enough not to clip. In the long term they move to the minimap (§8.4).

## 6. Inline elements

### 6.1 Inline code

- Monospaced font at about 0.9× body size, with a small baseline offset so it sits optically on the body baseline.
- Background as a rounded chip with 2–3pt horizontal padding and a 3–4pt corner radius, in a tone slightly more contrasted than today.
- The chip is drawn in a custom pass. A plain background-color attribute hugs the glyph box with no padding or radius, which is what produces today's highlight look.

### 6.2 File references

Inline code that matches a file in the workspace (for example `agent-graph-final.md`) is rendered as a link to that file, with the same chip styling plus a subtle link affordance. In a tree of interlinked specs this turns references into navigation.

### 6.3 Emphasis

Italics use the font's true italic cut. If the chosen face has none, pick a face that does rather than accepting a synthesized oblique.

## 7. Density modes

Because every value is a multiplier of body size, a density toggle only swaps the line-height and spacing values:

| Mode | Body line height | Paragraph space after | H2 space before |
|---|---|---|---|
| Comfortable (default) | 1.5 | 0.7em | 1.4em |
| Compact | 1.35 | 0.5em | 1.1em |

Compact serves users who scan long specs and prefer more lines per screen over more air.

## 8. Side minimap

The minimap shows the whole document as a narrow strip on the right edge of the pane. Its job is orientation and fast navigation: where am I, how long is this, where are the sections, what changed.

### 8.1 Placement

- Docked to the **right edge of the pane**, at a fixed width of about 80–120pt with a gap of about 2em from the content.
- Its width is subtracted from the available space **before** the centering rule in §5.1 runs. The text column is centered in what remains, so the minimap never overlaps content and centering stays visually honest.
- It hides automatically when `available` would drop below column width + 2 × min margin. Readable text always takes priority over the minimap.
- Breakout elements (§4.2) stop at the minimap gap and never extend underneath it.

### 8.2 What it draws

The minimap renders **structure, not shrunken text**. Miniature text at 10% scale is unreadable noise; shapes are legible at any scale.

| Document element | Minimap representation |
|---|---|
| H1, H2 | Short labeled bar; heading text shown if it fits at a legible size (~8–9pt), otherwise just the bar |
| H3 | Thinner, unlabeled bar |
| Paragraph | Grey lines whose lengths roughly follow the real line lengths |
| Code block | Tinted rectangle |
| Table | Small grid pattern |
| Image or diagram | Neutral filled rectangle |

Heading labels are the most valuable part. In a numbered spec like this one, the minimap doubles as a table of contents.

### 8.3 Scale and viewport

- Vertical position maps linearly from document coordinates to minimap coordinates.
- Short documents are drawn at a fixed maximum scale and **not stretched** to fill the strip, so a one-page file looks like one page.
- Long documents are scaled to fit the strip height. For very long documents where the scale becomes too small to be useful, the minimap itself scrolls in proportion to the main view.
- The visible region is shown as a translucent viewport rectangle.
- Width presets and breakouts are reflected proportionally, so a wide table looks wide on the minimap too.

### 8.4 Interaction and change markers

- Click jumps to that position; dragging the viewport rectangle scrolls.
- Hovering a heading bar shows the full heading text.
- Git change markers (modified, untracked, added) appear as small colored ticks on the minimap's outer edge. This replaces the clipped letters in today's left gutter and shows every change in the file at a glance, not only those currently on screen.

### 8.5 Rendering

- The minimap is built from the main view's layout results (layout fragments or line rects), not from a second layout pass.
- Its image is cached and updated incrementally when the document changes or relays out, invalidating only affected regions.
- It updates on resize after layout settles, not on every intermediate frame.

## 9. Implementation notes (native text system)

- **Line height:** set `minimumLineHeight` and `maximumLineHeight` per paragraph style instead of using `lineHeightMultiple`. A multiple scales from the font's own line height, which varies widely between typefaces and makes headings in a display face come out looser than intended.
- **Block spacing:** use `paragraphSpacingBefore` and `paragraphSpacing` for the before/after values in §3.
- **SwiftUI:** `.lineSpacing` only adds space on top of the font's line height, so tight headings are hard to achieve. Render through an attributed text view backed by TextKit instead.
- **Inline code chips and minimap:** both use custom drawing on layout fragments (TextKit 2 rendering attributes or a fragment drawing pass).
- **Measure:** compute column width from average character width of the body font at the current size, recomputed on font-size change.

## 10. Acceptance criteria

The change is successful when, on the reference window used for the current screenshot:

1. Visible body lines per screen go from ~14 to at least 19.
2. Prose lines stay within the selected preset's measure (±5 characters).
3. Every heading has more space above than below.
4. The H1 in the reference document fits on one line, or on two balanced lines.
5. Inline code is visually distinguishable from highlighted text without reading its content.
6. No element is clipped at any edge, including the last line of the document.
7. The minimap never overlaps content and hides before the text column drops below its minimum margins.

## 11. Open questions

- **Body typeface:** stay with the current sans, or offer a serif option (Source Serif, Charter, Literata) for long-form prose, keeping sans for headings?  Give an option,
- **Minimap default:** on by default for all files, or only for documents longer than two or three screens? enbled/disable by the user
- **Table of contents:** does the minimap's heading labels make a separate TOC panel unnecessary, or do long documents still need a text list? header dropodown navigator


## 12. User control panel

On top of the markdown viewer add a button that open a popover config panel where the user could customize the rendering, eg: full wide/80/90 column,  compact regular,  slider for base font size selector, font (not a dropdown 2-3 choiches) , othes? add an options "make default for all document" so this decide this  is the default or only for this view (remember in memory for this file /project session  -  reset on ubiq/project close)