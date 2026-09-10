---
id: inbox-theming
title: Problem — the palette is an enum with two variants, the accent is a literal, and 363 call sites pick their own type size
kind: proposal
status: proposal
summary: Ubiq's theme is `ThemeId::{Dark, Light}` and two functions that spell out every token by hand, so a third palette is a third function and a different accent is not expressible at all. Type size is worse — one knob (`ui_font_size`) scales the editor, the panes and the tree, and every other surface carries a hand-picked `text_size(px(N))`, 363 of them across ten distinct values. The proposal keeps the layout and the design rules untouched and splits the theme into four independent axes — a palette registry keyed by slug, an accent derived from one seed, a text scale of named roles over three independently sized surface families, and one density factor — so N palettes × M accents cost N + M declarations instead of N × M.
read_when: you are adding a palette, an accent, a colour token or a type size, wiring an Appearance control, or asking why a surface cannot be resized on its own
updated: 2026-09-09
depends_on: [tech-ui, tech-decisions, feat-workbench]
---

# Problem — the palette is an enum with two variants, the accent is a literal, and 363 call sites pick their own type size

Three facts about the tree today, all of them in one file and its call sites.

**A theme is an enum variant, and its colours are seventy hex literals.** `ThemeId` has exactly two
members (`crates/ubiq/src/theme.rs:185`); `dark()` and `light()` (lines 434 and 508) each write out
every token by hand; `palette_for` (582) is a two-arm match; `ThemeId::toggled` (217) is what the
titlebar's one control needs. `InterfacePrefs.theme` (`state/prefs.rs:88`) stores the variant. A
third palette is a third seventy-line function, and nothing in the shape says which of those seventy
values are the palette's own choices and which are consequences of one.

**The accent is not an axis.** `AccentColors { primary, muted, soft }` is three unrelated literals
per palette, and `border.focus` and `terminal.link_underline`/`link_underline_hover` are three more
that happen to carry the same hue — six literals expressing one decision, in each palette. A user
who wants the same dark ground with a green accent cannot have it, and adding that as a palette
would duplicate the other sixty-four values.

**Type size has one knob, and it reaches three surfaces.** `ViewPrefs.ui_font_size`
(`state/prefs.rs:163`) is per project, moves the editor, the terminal panes and the explorer tree
together (`app/shell.rs:538–608`), and is offered as an eleven-entry ladder in the status bar
(`ui/status_bar.rs:260`). Every other surface is frozen: `363` `text_size(px(N))` call sites across
`crates/ubiq/src/ui/`, clustered on ten values — `11.0` at 114 sites, `12.5` at 70, `11.5` at 54,
`10.5` at 31, `13.0` at 23, then a long tail. `ui/conversation/mod.rs` alone holds 38 of them and
`ui/settings.rs` 29. `kit::mono` (`ui/kit/controls.rs:48`) is the only shared starting point and half
its callers override the size on the next line. So the chat cannot be read at a comfortable size
without dragging the code up with it, and the chrome cannot be left alone while either moves.

The rule that keeps colours honest — `D10`, no literal outside `theme.rs` — has no counterpart for
sizes, which is the whole of the third problem.

**The layout is not the problem and this proposal does not touch it.** `D18` (square surfaces, one
coloured left edge), `D19` (a project is a swatch), no radii, the shape of every screen, the token
groups and their names all stand. What changes is how many values can fill them, and who chooses.

---

## 1. Four axes, not one enum

| Axis | Is | Scope | Costs |
|---|---|---|---|
| **Palette** | The neutral ground: surface, text, border, status, terminal, scrim | Interface | One declaration per palette |
| **Accent** | One seed colour, from which the six accent-hued tokens derive | Interface | One hex per accent |
| **Text scale** | A base size per surface family, and named roles over it | Interface, except the content family which stays per project | Three numbers |
| **Density** | One factor over the sizes that are grids rather than dragged regions | Interface | One number |

Each is independent, and a `Theme` is the resolved product of all four — still one `Copy` value in
the thread-local, still read through the accessors, so **no call site learns that any of this
happened**. `theme::set_mode` stays the single point that switches, which is what keeps Ubiq's tokens
and the component library's own theme from drifting apart.

## 2. A palette is a registry entry keyed by a slug

`ThemeId` stops being a closed enum and becomes a slug — `ThemeId(&'static str)` over a
`&'static [PaletteDef]` registry in `theme.rs`, where a `PaletteDef` carries the slug, a display
name, a `Mode { Dark, Light }`, the slug of its counterpart, and the tokens.

- **`Mode` is what the component library and the highlighter are told.** `set_mode` today calls
  `gpui_component::Theme::change(mode, None, cx)`, which can only say dark or light. A third palette
  makes that insufficient rather than merely approximate, so the semantic-token handoff that
  [`component-reuse-proposal.md`](../component-reuse-proposal.md) and `G211` already name as their
  blocking task becomes this proposal's prerequisite too. One function, in the one file allowed to
  name a colour.
- **`counterpart` is what the titlebar's toggle follows**, replacing `ThemeId::toggled`. One click
  still flips ground, but within the family the user chose — a warm dark goes to the warm light, not
  to the built-in one.
- **Prefs need no schema bump.** `theme: ThemeId` serialises today as `"Dark"` or `"Light"`; a slug
  newtype whose `Deserialize` accepts those two spellings as aliases for `dark` and `light` reads
  every blob already written. An unknown slug falls back to the default rather than discarding the
  blob, which is the posture the rest of `prefs.rs` already takes.
- **Ship three families, six palettes.** The current pair unchanged under the slugs `dark`/`light`;
  one warm low-contrast pair; one high-contrast pair for the accessibility case the current
  `text_faint` at `0x5c5c68` does not serve. Three is enough to prove the registry and small enough
  that each one can be looked at on the style reference page.

User themes read from a file are **out of scope** and belong behind this: the registry is the
extension point, and a loader would have to reject a definition missing any token — the completeness
`ui-and-design.md` guarantees today by having both palettes in one function.

## 3. An accent is one seed, and the six hued tokens derive from it

An `AccentDef` is a slug, a name and one `Rgba`. Resolution against the palette in hand:

| Token | Derivation |
|---|---|
| `accent` | The seed, luminance-corrected toward the palette's ground so a pale accent still reads on `light` and a dark one on `dark` |
| `accent_muted` | The seed mixed toward `surface.base` at a fixed ratio |
| `accent_soft` | The seed at `0.16` alpha — the ratio every `_soft` token already uses |
| `border.focus` | The seed, unmodified |
| `link_underline` / `_hover` | The seed, and the seed lifted |
| `text.on_accent` | Black or white, from `mark_dark(accent)` |

The machinery exists: `relative_luminance` and `mark_dark` (`theme.rs:420–430`) already answer the
readability question for project swatches, and `fade` is already the sanctioned alpha transform. The
derivation lives in `theme.rs`, so this is not a `.alpha()` at a call site and does not weaken the
`_soft` rule — the alphas are still declared in the one file, just once instead of per palette.

`InterfacePrefs` gains `accent: Option<AccentId>`, where `None` means the palette's own seed. Ship
six accents plus each palette's default.

**The project swatches stay outside this axis.** `D19` says a swatch is identity, not role; sixteen
of them are a fixed wheel, and recolouring them with the accent would make two projects look the
same. They keep their per-palette literals.

## 4. A text scale: five roles over three surface families

Replace every `text_size(px(N))` with `theme::text(Family, Role)`.

**Five roles**, mapped onto the clusters already in the tree so the migration is a rename, not a
redesign:

| Role | Ratio | Is |
|---|---|---|
| `Title` | 1.15 | A page or section heading — today's `15.0`, `17.0` |
| `Body` | 1.00 | The family's base — today's `13.0`, `12.5` |
| `Label` | 0.92 | A control's label, a row, a tab — today's `12.0`, `11.5` |
| `Meta` | 0.85 | A timestamp, a count, a hint — today's `11.0` |
| `Micro` | 0.80 | A badge, a chip, a superscript — today's `10.0`, `10.5` |

**Three families**, each with its own base size, each independently set:

- **Chrome** — titlebar, status bar, rail, tabs, menus, modals, settings, pickers. Base `12.5`.
  Furniture; growing it reflows the window, so it moves on its own and rarely.
- **Content** — editor, viewer, explorer tree, search results, terminal panes. Base
  `EDITOR_FONT_SIZE` = `13.0`. **This is `ui_font_size` renamed, and it stays per project** — a zoom
  travels with the project it was chosen for, the panes must be sized in whole points because a
  terminal is a cell grid, and the existing ladder and `EDITOR_FONT_MIN/MAX` clamp keep working.
- **Conversation** — the chat transcript, tool blocks, the composer, the agents columns and their
  sidebar. Base `12.5`. This is the family the current design cannot serve: a transcript is read as
  prose, at a size that has nothing to do with the size code is read at.

Three families is the answer to "why not one factor": the three are read differently, resize for
different reasons, and today only one of them can move at all. A fourth would be arguing about where
a boundary falls; these three fall on boundaries the code already draws.

**Sizes are enforced the way colours are.** `just ui` gains a check rejecting `text_size(px(` outside
`theme.rs`, alongside the crate-boundary checks it already runs — the mechanical half of `D10`,
applied to the axis that has never had it.

## 5. Density is one factor over the grid constants

`theme.rs`'s constant table splits in two, and only one half scales:

- **Grids** — `TITLEBAR_HEIGHT`, `STATUS_BAR_HEIGHT`, `RAIL_WIDTH`, `TERMINAL_PADDING`,
  `ACCENT_EDGE`, `kit::row_height`, `kit::row_indent`. These follow a `density` factor
  (`Compact 0.9` / `Regular 1.0` / `Comfortable 1.15`). `kit::files` already derives its row height
  and indent from the size it draws at — this is that pattern named and generalised.
- **Dragged regions** — `EXPLORER_WIDTH`, `CHAT_WIDTH`, `DOCK_HEIGHT`, `INSPECTOR_WIDTH`,
  `TASKS_HEIGHT`, the modal and settings sizes. These are what a *fresh* window opens at and what
  the user then drags; the drag is remembered per project inside the arrangement blob. Scaling them
  would fight a value the user already set, so they do not move.

## 6. Where the user sets all this

**Appearance**, in application settings — the section that exists and holds three toggles today.
Palette family, ground (the counterpart toggle the titlebar already offers), accent as a row of
swatches, the three base sizes, density. Every one of them persists as it is flipped, like the rest
of the page. The status bar's font-size dropdown stays where it is and keeps meaning the content
family, because that one is the project's.

## 7. Order of work

1. **Hand the component library Ubiq's palette** (`G211`'s blocking task). Nothing else can land on
   top of a `Mode` that only says dark or light.
2. **Palette registry and the slug `ThemeId`,** with the two current palettes as its first entries
   and the prefs alias. Behaviour identical; the titlebar toggle follows `counterpart`.
3. **The accent axis,** including walking the emulator palette copies — `set_ui_font_size` already
   dresses every open `TerminalView`, and `toggle_theme` must do the same for colour, which
   `ui-and-design.md` states and is worth verifying while touching it.
4. **The text scale,** file by file, largest first — `conversation/`, `settings.rs`, `sink/`,
   `agents/`. Then the `just ui` check, which cannot land before the last literal is gone.
5. **Density,** last, because it is the only axis that moves geometry and so the only one that can
   uncover a layout that was holding a number by accident.
6. **The Appearance controls,** once there is more than one value per axis to choose.

Steps 2, 3 and 5 are independent of each other and each is shippable alone.

## Next steps

- Every step is implemented — the component library wears Ubiq's palette, the palette registry
  ships six palettes in three families, the accent derives from one seed, density scales the grid
  half of the constant table, and every type size in the interface comes from
  `theme::font(Family, Role)` with `just ui` rejecting a literal one.
  `tech/ui-and-design.md` owns those facts.
- The registry earned a decision: `D96`, which states what `ThemeId` becomes, that `Mode` is a
  property a palette has rather than the thing it is, and what a derived accent costs a palette.
- §6 is implemented: the Appearance section of application settings sets all four axes — the
  palette family, the ground, the accent as a row of swatches, the chrome and conversation base
  sizes as a ladder of pills, and density — each persisting on the click that changed it. The
  content family stays the project's, on the status bar's dropdown. `features/workbench.md` owns
  what the section offers.
- `G212` is taken by the ACP delegate heuristic, so this document's own numbering has no row.
- The four new palettes and the six accents still need looking at on the style reference page,
  the way an icon needs the review sheet — `G218`.
- Filing this document is `P12` in `_meta/feedback.md`.
