---
name: ubiq-icons
description: Reference for Ubiq's icon set — the registry in assets/icons/icons.yaml, the SVG spec every icon obeys, how GPUI draws and tints one, and the draw-render-review loop for adding icons without the set drifting. Use when adding, redrawing or renaming an icon, when a concept needs a glyph it does not have, or when running just icons-check / icons-sheet / icons-audit.
---

# Ubiq icons

An icon is a **monochrome 24×24 SVG** in `assets/icons/`, listed in `assets/icons/icons.yaml`.
`just icons-check` is what a change has to pass. The owning document is
`_docs/tech/ui-and-design.md`; the theme tokens live in `crates/ubiq/src/theme.rs`.

## Before you draw anything

**Most icons are not drawn.** `gpui-component` embeds the Lucide set and exposes it as `IconName`,
which is what `kit::icon_button`, `Icon::new` and the menus already take. Climb this in order:

1. **Does the concept need an icon?** A label a reader can read is not improved by a glyph beside
   it. Status that is already carried by colour (`status_dot`) or by a word does not need one.
2. **Is it in the set gpui-component ships?** That is **101 Lucide icons, not all of Lucide** —
   read the directory, do not go by what Lucide's website has. If it fits, record the mapping as
   `source: lucide:<Variant>` and stop: no file to draw, nothing to lint. `I11` fails a borrowed
   row the dependency does not actually ship.
   Where the icon belongs to a set whose look we own and expect to tune — the rail's badges, the
   titlebar's controls — write `source: adopted:<Variant>` instead and run `just icons-adopt`,
   which rewrites the shipped file on our root and leaves the provenance in the row.
3. **Is it already here under another name?** `grep` the registry's `keywords`. Then
   `just icons-dupes` after drawing, which compares the rendered masks.
4. **Only then draw it**, and only if you can state its goal in one line.

A harness logo (Claude, Codex, Gemini, opencode, Copilot) is **not** an icon in this sense — it is a
mark that obeys its owner's shape, not our grid. Keep those out of the loop and out of the spec.

## How GPUI draws one — and why the spec is what it is

`Window::paint_svg` rasterises through resvg into an **alpha mask**, caches it in the sprite atlas
under `RenderSvgParams { path, size }`, and tints it with an `Hsla` at composite time. Three
consequences that decide every rule below:

- **Colour in the file is discarded.** No gradients, no two-tone, no opacity. An icon that needs two
  colours is two `Icon` elements in Rust with two tokens, never one clever file.
- **Theming is free.** Dark, light and any future accent are one `Hsla` at the call site; the colour
  is not in the cache key, so the same tile serves every theme and no re-rasterisation happens.
  Never encode a theme decision in an SVG — pass `theme::fg()`, `theme::accent()`, `theme::muted()`.
- **Transformation is free too, but centred.** `Transformation::rotate/scale/translate` is a GPU
  matrix on the cached tile, always about the element's own centre. Nothing here rotates today
  (see Motion), so this only matters if that changes.

## The spec

| Rule | Value |
|---|---|
| viewBox | exactly `0 0 24 24`, no `width`/`height` on the root |
| Stroke | `stroke="currentColor"`, `stroke-width="2"`, round caps and joins |
| Fill | `fill="none"` on the root; `currentColor` only for a deliberately solid shape |
| Live area | all ink inside `1.5 … 22.5` — 2px of optical padding |
| Shapes | at most 4 top-level shapes |
| Forbidden | `<text> <image> <use> <defs> <style> <mask> <clipPath> <filter>`, gradients, `<animate*>`, and `style=` `class=` `opacity=` anywhere |
| Precision | two decimals |
| Name | kebab-case, `<category>-<thing>`, one spelling — file name = registry key = Rust variant |

Optical, not arithmetic: a circle fills the 20px live square, a rectangle sits at ~18px, a triangle
needs its centroid nudged. Balance by eye at 16px, which is where icons are actually read.

`just icons-check` enforces the mechanical half (`I01`–`I11`); a registry row with no file yet is a
worklist entry, not a failure, so a category can be filled in one pass at a time. The rest is what
a sheet is for.

The live area is measured off the files gpui-component ships, not chosen: their paths keep to 2–22
and a 2px stroke puts the ink at 1–23. An icon inside that envelope but well short of it reads as
smaller and lighter than its neighbours, which is the most common fault a sheet exposes.

**Three rules apply only to what we draw** — the shape count, the ink envelope and the decimals.
A harness's mark obeys its owner's shape, and an adopted icon is coherent by construction because
it came from the family: `sun` is nine shapes, `network` is five, `github` fills the box. `check`
drops those three for both, and nothing else. Do not widen the exemption to reach a drawing of
your own that will not fit — that is the drawing telling you something.

## The registry

`assets/icons/icons.yaml`. A key is the icon's one name; its value is the **goal** — what a reader
has to understand from it, in one line. Write the goal first: a goal you cannot state is a concept
that is not settled, and drawing it will not settle it.

```yaml
canon: [pane-thinking, pane-awaiting]   # frozen reference — approved, never redrawn

categories:
  pane:
    pane-thinking: the harness is working, with no measurable progress to report
    pane-awaiting:
      goal: the harness has stopped and is waiting for the reader to answer
      source: custom            # or lucide:<name> — then there is no file here
      keywords: [permission, prompt, blocked, question]
```

## The loop

Work **one category at a time, 6–8 icons per pass**. Fewer and you cannot see the family trend;
more, or mixed categories, and you cannot see anything.

1. `just icons-check` — find the registry rows with no file. Pick one category's worth.
2. Draw the SVGs. Obey the spec by construction; do not draw first and lint after.
   When a metaphor is not obvious, draw **four takes** of each into `assets/icons/variants/` as
   `<name>-1.svg` … `-4.svg` and run `just icons-variants --category <category>`: one row per
   concept with the takes across it, the settled siblings above, plus a large per-concept sheet.
   Promote the winner to `assets/icons/<name>.svg` and delete the rest — takes are not a set, and
   their sheets are deleted with them on the next run. Only `sheet.png` and the `audit-*.png`
   describe the set itself, so only those are kept.
3. `just icons-check` — mechanical rules. Fix everything before looking at a picture.
4. `just icons-sheet --category pane`. Read it. Every sheet carries the **canon strip** at the top:
   judge coherence against that, never against the previous batch.
5. Redraw what fails and go back to 3. Two passes and no convergence means the goal is wrong, not
   the drawing — fix the registry line.
6. `just icons-dupes` before you finish.

Every sheet is written to `assets/icons/preview/`, beside the file being drawn and ignored by git.
A `lucide:` row resolves to the real file `gpui-component` ships, so the borrowed icons appear on
the sheets too — which is the point, since they are the family a new icon has to match.

**What to look for in a sheet**, in this order: (a) the 16px column — is it still readable, or has
detail collapsed to mud; (b) weight — does it sit at the same visual density as the canon; (c) size
— does the ink fill the same optical area as its siblings; (d) metaphor — is it the same *kind* of
drawing (outline, same corner radius, same level of abstraction).

Every ~20 accepted icons, and once at the end, run `just icons-audit`. That sheet answers a
different question — **which of these breaks the family?** — and it is paginated per category
because the answer is unfindable in one 200-icon grid.

Judging is better done by a subagent that sees the sheet and this spec but not the prompt that
produced the icons. Self-review of your own SVG under-detects.

## Motion

Micro-animation is a property of **state**, not of an icon, and it never touches a file. It is
`with_animation` on the element: opacity, position, size, or an `Svg` `Transformation`. The most
complex thing the app draws is the writing mark in
`crates/ubiq/src/ui/conversation/mod.rs` — five divs whose opacity is driven by a triangle wave.
No SVG is involved, and no icon needs a variant, a keyframe or a second file.

If you add motion: `.repeat_synced()` so several instances stay in phase, `.with_max_fps(30.)`
because a live animation keeps the whole window repainting, and nothing for reduced motion —
`with_animation` already honours `cx.reduce_motion()`.

## Using an icon in Rust

`IconName` comes from `gpui-component` and is what the kit takes. A custom icon reaches an `Icon`
through `IconNamed` (`fn path(self) -> SharedString`, with a blanket `From<T> for Icon`), so a
Ubiq-side enum slots into every existing call site with no fork. `crates/ubiq-app/src/lib.rs`
registers only `gpui_component_assets::Assets`, so a custom path needs either Ubiq's own
`AssetSource` or `Svg::data(&[u8])`, which hashes the bytes into the cache key and needs no asset
source at all.

Never name a colour: `Icon::new(…).text_color(theme::fg())`. See `ubiq-ui` for the token rules.
