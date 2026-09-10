---
id: inbox-component-reuse
title: Problem — the interface rebuilds widgets the library it embeds already ships
kind: proposal
status: proposal
summary: Ubiq depends on gpui-component (gpui-kit) and uses seven of its parts — the editor, the input, the dock, the text view, the tooltip, the scrollbar and the keycap. Everything else in `ui/kit` is a raw `div()` — a dropdown with no keyboard navigation across fifty-three call sites, two context-menu bodies, a bespoke filterable list beside the picker's, a hand-rolled grid, and a modal stack with its own z-priority contract — while the window is wrapped in the library's `Root` and its dialogs, popovers, menus, toasts, table, list and tree sit unused. Most of the kit is house *skin* and should stay; the proposal is to adopt the library where it brings behaviour Ubiq lacks and would otherwise have to write, and to give it Ubiq's palette first so anything adopted looks like Ubiq.
read_when: you are adding a widget to `ui/kit`, reaching for `deferred` and `anchored` to build a popup, hand-rolling a table or a filterable list, or asking which parts of gpui-kit Ubiq should be using
updated: 2026-09-09
depends_on: [tech-ui, feat-workbench, tech-architecture]
---

# Problem — the interface rebuilds widgets the library it embeds already ships

Ubiq embeds `gpui-component`, branded gpui-kit, at revision `df1d07b`. It uses seven things from
it: the editor over a rope (`input::EditorState`), the text input and textarea, the docking layout
(`dock::{DockArea, DockAreaState, NodeId}`), the markdown text view, the tooltip, the scrollbar and
the keycap. Fifty-eight files import one of those.

Everything else the interface draws is a raw `div()` composition in `crates/ubiq/src/ui/kit/`, or a
one-off in the screen that needed it. That is the right call for most of it — the kit is where the
house style lives, and a library button cannot express "a coloured left edge means identity". It is
the wrong call in a specific, enumerable set of places: **where the hand-rolled version is missing
behaviour the library has, and the only way to get that behaviour is to write it.**

Two facts make the cost of adoption much lower than it looks:

- **The library is initialised and its overlay layers are live.** `gpui_component::init(cx)`
  (`crates/ubiq-app/src/lib.rs:206`), the asset provider (line 186), and — the one that matters —
  every window's view is wrapped in `gpui_component::Root` (`crates/ubiq/src/app/mod.rs:1097`).
  `Root` is what owns the dialog, sheet, notification and tooltip layers with their z-priority
  stacking. Dialogs, popovers, menus and toasts would work the day they are called.
- **The palette is not wired, and that is the one blocking task.** `theme::set_mode`
  (`crates/ubiq/src/theme.rs:233`) calls `gpui_component::Theme::change(mode, None, cx)` — it
  switches the library between its own light and dark defaults and never hands it Ubiq's colours.
  So every library widget draws in gpui-kit's palette, not Ubiq's, which is why the ones in use
  read as almost-but-not-quite. The library keeps `Theme::global_mut(cx).light_theme` and
  `.dark_theme` as public `Rc<ThemeConfig>` and offers `apply_semantic_tokens`,
  `apply_semantic_config_str` and `ThemeRegistry::load_themes_from_str`; building one config per
  palette from the tokens `theme.rs` already holds is a single function in the one file allowed to
  name a colour. **Nothing else in this proposal should land before it.**

## 1. Where the library brings behaviour, ranked

| # | Replace | With | What is gained | Size |
|---|---|---|---|---|
| 1 | `kit::menu::Picker` and `menu_panel` (`ui/kit/menu.rs:44,295`, 382 lines, **53 call sites**) | `PopupMenu` / `Select` / `Combobox` over a `SearchableListDelegate`, inside the existing `Picker` signature | Arrow-key navigation, type-ahead and focus handling, none of which the kit dropdown has anywhere. Reimplementing the body behind the same constructor means one file changes and fifty-three call sites do not | 382 → a wrapper |
| 2 | `ui/project_menu.rs:121-660` — a popover with its own rows, hover states and outside-click dismiss, built from `deferred` + `anchored` | `ContextMenu` / `PopupMenu` inside `Root`'s popup layer | Keyboard navigation, submenus, and the library's layering instead of Ubiq's hand-numbered `MENU_LAYER` / `MODAL_MENU_LAYER` priorities | ~350 lines |
| 3 | `ui/navigator.rs:89-203` + `app/nav.rs:226-470` — a filterable list with its own cursor, filter and popover positioning, parallel to the picker's | `command::{Command, CommandState}`, or a `Combobox` over the same delegate as #1 | Fuzzy match, keyboard model, and one filtering mechanism in the interface rather than two | ~230 lines |
| 4 | `ui/stats.rs:219-328` — a fixed-column grid with a `COLUMNS` array, a header row and a cell builder | `Table` + `TableState` | Sorting, column resize and **virtualized rows** for free; the interface's only from-scratch grid stops being a pattern others copy | ~110 lines |
| 5 | The long lists — the diff body, commit history, refs, the working tree, the transcript | `List` + `ListDelegate` (virtualized, with selection and keyboard navigation) or the `v_virtual_list` under it | [`render-velocity-proposal.md`](./completed/render-velocity-proposal.md)'s phase 1 virtualized four of the five with raw `uniform_list` and the transcript with `v_virtual_list`; what `List` would add on top is the selection and key handling those lists hand-roll per screen, and refs are the one still built whole (`G220`) | see that proposal |
| 6 | `kit::menu::{context_menu, context_panel}` (`ui/kit/menu.rs:473,497`, 86 lines, a duplicate of `menu_panel`'s logic) | `ContextMenu` | Deletes the duplication #2 also removes | 86 lines |
| 7 | `ui/explorer.rs` rows over `kit::files::{file_row, twisty}` | `Tree` + `TreeState` | Expansion state, stable ids and keyboard traversal as library concerns. **Evaluate, do not schedule:** `file_row`'s four-state selection-and-cursor matrix is real behaviour, and it must survive as the row renderer or this is a loss | — |

**One lead in the sweep was wrong and is worth recording so nobody chases it:** the dock's resize
strip (`ui/dock/skin.rs:203`) is not a hand-rolled resize. `Skin` implements the library's
`DockAreaRenderer`, `TabGroupRenderer` and `TilesRenderer`, and the strip captures a mouse-down and
calls the library's own `dock.resize_to(..)`. It is sixty lines of appearance over library
behaviour — exactly the pattern the rest of this proposal is asking for. What *is* hand-rolled is
the composer's height strip (`ui/conversation/mod.rs:84`, `app/agents.rs:369`) and the picker's
corner grip (`ui/file_picker.rs:180`, `app/picker.rs:264`); of those, only the composer is a
plausible `ResizablePanel`, and a floating panel's corner grip has no library analogue. Leave both.

## 2. Where the kit stays, and why

Replacing these would be a lateral move: the same line count, plus theme plumbing, minus the house
style. **Adopt one only when the file is open for another reason, and only where the library's
version brings something specific** — `Checkbox` brings focus and keyboard, `NumberInput` brings
hold-to-repeat for `stepper`, `Progress` brings animation for `meter`, `Collapsible` brings a
transition for `disclosure`.

`icon_button` (60 sites), `ghost_button` (97), `primary_button`, `toggle_pill`, `choice_pill`,
`check_box`, `stepper`, `disclosure`, `meter`, `elided`, `panel` (49), `panel_header`,
`tab_strip`, and all of `ui/kit/settings.rs` — every one is a `div()` with hover, active and
disabled states and nothing else, but the states are drawn in the house language: square corners, a
coloured left edge, no rounding. That language is what `slab`, `field`, `pill`, `card`,
`state_chip`, `badge` and `removable_tag` exist to enforce, and they are not candidates at all.

Three things must never be replaced, because there is nothing to replace them with: everything in
`ui/kit/canvas.rs` (`dot_grid`, `links`, `sand`, `dashed_box` — hand-painted vector work for the
orchestration graph), and `progress_ring_in` (`ui/kit/controls.rs:233`, an arc painted with
`PathBuilder`).

`kit::overlay::modal_sized` (`ui/kit/overlay.rs:85`) is the judgement call. The library's `Dialog`
stacks properly and `Root` manages it, which would retire Ubiq's hand-numbered layer priorities and
the `Picker::above_modal` contract that exists to work around them. Against that, `modal_sized`'s
viewport clamp and its documented split of Escape handling to the window's key context are correct
and understood. **Take it only after #1 and #2 land**, because those are what make the layering
contract removable.

## 3. What the library has that Ubiq does not

Not replacements — capabilities the interface would have to write, and does not have to:

- **`Toast` / `ToastStack`** — Ubiq has no transient notification at all; the bell opens a modal
  list. A toast is what a finished background run wants.
- **`Skeleton` / `Shimmer` / `Spinner`** — every panel that waits on the host draws muted text.
- **`Pagination`** — the git history's `Load more commits` row is a hand-rolled one.
- **`Breadcrumb`**, **`Sheet`**, **`HoverCard`**, **`form::Form`** with validation (the prompt
  modal has none), **`description_list`**, and the **chart** family, which the stats screen draws
  by hand today.

## Next steps

- Land the palette bridge in `theme.rs` first, alone, and look at the editor, textarea, markdown
  view and dock afterwards — they are the existing users and the change is visible there before any
  new component is adopted.
- Then #1 and #2, in that order: the dropdown body behind its own signature, then `project_menu`.
  Between them they retire every hand-rolled `deferred` + `anchored` popup and the layer-priority
  scheme with it.
- #5 is done with raw list elements, so what is left of it is `List`'s selection and keyboard
  handling over lists that virtualize; #4 is untouched.
- Add the rule to [`tech/ui-and-design.md`](../tech/ui-and-design.md): a widget goes in `ui/kit`
  when it carries the house style over library behaviour; a *new* interaction — a popup, a list, a
  table, a dialog — starts from gpui-kit and is skinned, not written.
