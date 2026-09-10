---
id: inbox-block-caret
title: Proposal — a block caret for vim's Normal mode, from upstream
kind: proposal
status: proposal
summary: The Normal-mode caret is the component library's fixed thin bar, and no downstream trick makes it a block without hijacking the selection or drawing a second cursor over the buffer — so this asks for the shape upstream, as a `CursorShape` on the input element's builder, with the paint change behind it and what Ubiq passes once it exists.
read_when: you are deciding how the vim caret is drawn, or whether a missing input capability is worked around in Ubiq or asked for in gpui-component
updated: 2026-09-07
depends_on: [feat-workbench, tech-ui]
---

# Proposal — a block caret for vim's Normal mode, from upstream

**Vim mode reports its mode in the status bar because the caret cannot report it.** Normal, Insert
and both visual modes drive the file editor and every multi-line box, and all four draw the same
thin blinking bar. The caret shape is the one signal a vim user reads without looking away from the
text, and Ubiq is the only surface in the window that cannot give it — a pane already draws a solid
full-cell block, in `TerminalRenderer::paint` (`vendor/gpui-terminal/src/render.rs`).

This proposes closing that in `gpui-component` rather than around it: **one shape option on the
input element, a width and a height that follow it, and nothing else.** Ubiq then passes
`CursorShape::Block` while the focused input is in Normal mode, and deletes nothing, because there
is no workaround in the tree to delete.

## 1. Where it stands

The input engine sits in the library's own `gpui-base` package and reaches Ubiq through
`gpui_component::input`. Three facts decide this proposal, all of them upstream:

**The caret is a quad with a constant width.** `TextElement::paint`, in `gpui-base`'s
`input::base::element`, fills `cursor_bounds` with `editor_style.caret`; `layout_cursor` sizes it
`0.85 * line_height` tall and `CURSOR_WIDTH` wide, and `CURSOR_WIDTH`, in `input::base::blink_cursor`,
is a `const` — `px(1.5)` on macOS, `px(2.)` elsewhere, split that way by the fix for the library's
own issue #1850. There is no cursor-shape concept anywhere in the repository.

**`InputEditorStyle` carries colour and nothing else,** and is unreachable from here.
`input::editor::highlighting` defines it with a `caret` colour, a `selection` colour and the
highlight resolvers; `gpui_component::input` does not re-export it, and `Input::render` rebuilds the
whole struct from the theme on every frame, so a downstream setter would be overwritten before it
drew. The one caret knob Ubiq has today is the global `caret` theme token.

**Per-range decoration cannot fake it.** `TextDecoration` reaches downstream and takes a
`HighlightStyle`, but a decoration becomes a `TextRun`, and a run's `background_color` is never
painted — the library calls no background paint anywhere. A block drawn as a highlighted range is
not available at any price.

That leaves the two workarounds this rejects, in §5.

## 2. What this decides

Whether the shape is asked for once, upstream, or worked around forever in Ubiq:

- what the API is, and why it is not `InputEditorStyle` — §3;
- what the paint does, and where the glyph goes — §4;
- what Ubiq does with it, and what it does until then — §5;
- what a block does at the edges — §6.

## 3. The API

**A `CursorShape` on the element builder — `Editor`, `Textarea` and `Input` — not on the style
struct.** `Bar` is the default and is what every existing caller keeps getting; `Block` and
`Underline` are the two additions.

```rust
Editor::new(state).cursor_shape(CursorShape::Block)
```

The builder is the shape's home for one reason: `Input::render` owns `InputEditorStyle` and
rewrites it each frame from the theme, so a field added there needs a second, render-proof way in,
and the setter it would need — `AnyInputState::set_editor_style` — is `pub(crate)`. A builder
option flows through the render that already runs, needs no new public state type, and cannot be
clobbered by the frame after it. The shape reaches `layout_cursor` the way the rest of the editor
style does.

`CursorShape` is public from `gpui_component::input`. `Editor` and `Textarea` render through
`Input`, so the option is one pass-through on each.

**Colour stays the `caret` token.** A block and a bar are the same cursor in two shapes, and a
second colour would be a second thing to theme for no gain. Ubiq's `crates/ubiq/src/theme.rs`
already carries the token in both palettes.

## 4. What the paint does

`layout_cursor` picks the bounds from the shape:

| Shape | Width | Height |
|---|---|---|
| `Bar` | `CURSOR_WIDTH` | `0.85 * line_height` |
| `Block` | the advance of the character under the cursor | the full `line_height` |
| `Underline` | the advance of the character under the cursor | a hairline at the baseline |

The advance is the one new measurement, and the shaped line already holds it — the same value
`range_to_bounds` reads to size a one-character selection.

**The block paints before the text, where the selection paints.** `TextElement::paint` already
fills the selection and then draws the glyphs over it, which is why a selected character stays
readable; the block joins that order rather than the one the pane uses, where an opaque quad hides
the cell it marks. A code buffer is read at the cursor more than a terminal is, and reverse video —
re-shading the glyph under the block — is a larger change to the run builder than this asks for. It
stays out of the first version.

Blink is untouched: `show_cursor` already gates the quad on focus, blink phase and window activity,
and a block blinks on the same clock a bar does.

## 5. What Ubiq does with it

`crates/ubiq/src/app/vim.rs` holds the mode, `crates/ubiq/src/state/vim/` holds the command set, and
neither changes. The mode is the window's — exactly one input holds focus — so the shape is a
reading of `app.vim.mode` passed at the call sites that build an input: the buffer in
`crates/ubiq/src/ui/viewer/mod.rs`, and the multi-line boxes in the chat composer, the agent input,
the task description, the project about and the commit message. `Block` in Normal, `Block` in both
visual modes, `Bar` in Insert, and `Bar` everywhere when vim mode is off.

**The two workarounds this rejects**, either of which would ship before the upstream change lands
and would then be removed:

- **A one-character selection in Normal mode.** `set_selected_range(cursor..next)` draws the right
  cell in the right place, on both editors and textareas, for about thirty lines. It also makes the
  selection a lie — copy, cut and IME all take the character the caret sits on — makes Normal and
  Visual look identical, draws nothing past the end of a line, and leaves the thin bar painted on
  the block's trailing edge unless the global `caret` token is made transparent.
- **A quad Ubiq paints itself,** sized from the state's `range_to_bounds`. It keeps the selection
  honest, but it is a second cursor with its own scroll, clipping and blink to keep in step with the
  one underneath it, and it needs the same transparent-token trick to hide the bar it duplicates.

Both trade a permanent cost in Ubiq for a temporary one upstream. The upstream change is a shape
field, a `match` in `layout_cursor` and three builder pass-throughs.

## 6. Behaviour

| Situation | What happens |
|---|---|
| Normal mode, cursor on a character | A block the width of that character, the glyph drawn over it |
| Normal mode, cursor at the end of a line | A block one space wide, past the last character |
| Normal mode, empty line | A block one space wide at the line's start |
| Insert mode | The bar, at the width it has today |
| Visual mode | The block, over a selection that is drawn as it is today |
| Vim mode off | The bar, in every input, as before this change |
| A single-line field | The bar always; vim mode does not claim single-line inputs |
| The window loses focus | No cursor, exactly as today — the shape does not change the blink gate |

## 7. What this asks to be decided

- The caret shape is an upstream capability, not a Ubiq workaround. Neither the one-character
  selection nor a self-drawn quad is written in the meantime.
- The shape is a builder option on `Editor`, `Textarea` and `Input`, with `CursorShape` public from
  `gpui_component::input`. `InputEditorStyle` is not extended and is not re-exported.
- The block carries the `caret` token's colour, and paints before the glyph rather than over it.
  Reverse video is out of scope.
- Until it lands, the status bar's mode chip stays the only mode signal, which is what it is today.
- Ubiq's side is a reading of `app.vim.mode` at the call sites that build an input, and no change to
  the command set or the driver.

The library has no vim support and no open work towards one; its issue #2617 asks for the editing
primitives to be made public for exactly this kind of layer, which is where a shape option belongs
alongside.

## Related docs

- [`../features/workbench.md`](../../features/workbench.md) — vim mode, its engine and driver, and the status bar chip that reports the mode today
- [`../tech/ui-and-design.md`](../../tech/ui-and-design.md) — the theme tokens, and the rule that keeps every colour in one file
- [`../backlog.md`](../../backlog.md) — `G100`, where the fixed thin caret is recorded as a gap
