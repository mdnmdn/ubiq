# Tokens, constants and kit primitives

Everything here is `crates/ubiq/src/theme.rs` and `crates/ubiq/src/ui/kit/`.

## Theme tokens (`theme.rs`)

A token names what a colour is *for*, so a palette swap changes every surface consistently.
`PALETTES` is the registry: ten palettes in five families (`dark`/`light`, `ember-*`,
`contrast-*`, `navy-*`, `violet-*`). A token in one exists in every other. The active theme is
thread-local — a call site never learns which palette answered.

| Group | Accessors | For |
|---|---|---|
| Surface | `app_bg`, `pane_bg`, `surface`, `surface_raised`, `hover`, `selected`, `selected_focus`, `scrim` | The stack from window down to a selected row, deepening once the list holding it has the keyboard; `scrim` is what a modal lays over the window |
| Text | `text`, `text_muted`, `text_faint`, `on_accent` | Primary, secondary, faintest (ignored rows, timestamps, hints), and copy on a filled surface |
| Accent | `accent`, `accent_muted`, `accent_soft` | The interactive colour, its subdued form, the fill behind a selected row |
| Terminal | `selection_background`, `link_underline`, `link_underline_hover` | Selected cells in a pane; the OSC 8 / detected-URL underline |
| Border | `border`, `border_focus` | Ordinary separation; the focused edge |
| Status | `danger`, `success`, `warning`, `info` + a `_soft` each | Agent and process states, and the fills behind them |
| Ribbon | `ribbon_alpha`, `ribbon_beta`, `ribbon_ink`, `ribbon_experimental`, `ribbon_experimental_ink` | The build-channel ribbon, and Git mode's experimental ribbon — same values in every palette |
| Project | `project_colour(n)`, `project_colour_count()`, `project_temporary()`, `project_tint(temporary, colour, custom)`, `mark_dark(colour)` | One project's identity wherever it appears |

Helpers: `set_mode(id, cx)`, `palette_for(id)`, `fade(colour, alpha)`, `rgba_of(rgb)`,
`ThemeId::counterpart`, `contrast(a, b)`.

### Custom themes (`D152`)

A user's theme is a **fork of a built-in plus a sparse override map**, never a full palette:
`CustomTheme { id: "custom-…", name, base: ThemeId, overrides: BTreeMap<String, u32> }` in
`InterfacePrefs.custom_themes` (`serde(default)`, no schema bump). `resolve` is
`palette_for(base)` → overrides → `with_accent` → the two derived ink/border overrides put back.

- **The editable set is grounds and ink**: `theme::EDITABLE_TOKENS`, fifteen rows — eight surfaces,
  four text colours, two borders, the accent seed — each with a `TokenGroup` and, for ink, the
  token it is read `against`. Status, ribbon, terminal and project tokens are *not* editable.
- **An override is a hue, not a transparency**: `set_token` keeps the palette's alpha.
- **`ThemeId` stays `Copy` over a `&'static str`.** A `custom-…` slug is interned (leaked once);
  `is_custom`, `is_known_custom` and `base()` are how a call site asks. `name()` returns a
  `SharedString`, because a custom name is runtime.
- `theme::set_custom_themes(list)` is the push from `AppState`; `custom_theme(id)` is the lookup.
  A slug naming nothing resolves through its base, and deleting the worn theme falls back to it.
- The editor is `ui/themes.rs` over `app/themes.rs`, with `kit::colour_picker`,
  `kit::prompt_modal`, and `ui::sink::style::tokens()` as its live specimen. It warns under
  `TEXT_MIN_CONTRAST` (4.5) and never blocks.

## The size axis (`theme::Metrics`, `D151`)

Two axes and three trims, all in `InterfacePrefs`, all clamped in the setters:

| | Range | Moves |
|---|---|---|
| `ui_scale` | 0.80–1.40 | Every dimension: the window's rem size, every constant below, icons, the terminal inset, and the type bases |
| `text_ratio` | 0.85–1.20 | Type only, on top of `ui_scale` |
| `chrome_trim` / `content_trim` / `conversation_trim` | 0.70–1.60 | One family against the other two |

```rust
pub fn scaled(base: f32) -> f32 { (base * ui_scale()).round().max(1.0) }
pub fn font(family: Family, role: Role) -> Pixels {
    let base = TEXT_BASE * ui_scale() * text_ratio() * family.ratio() * trim(family) * role.ratio();
    px((base * 2.0).round() / 2.0)
}
```

`Family::ratio()` — Chrome 0.96, Content 1.00, Conversation 0.96. `Role::ratio()` — Title 1.15,
Body 1.00, Dense 0.96, Label 0.92, Meta 0.85, Micro 0.80.

- **A size is a family and a role, never a number.** `just ui` rejects a literal `text_size(px(N))`
  and a literal icon size (`with_size(px(N))`, `Size::Size(px(N))`) anywhere under `crates/ubiq/src`.
- **`Density` does not exist.** Its three steps are `ui_scale` 0.9 / 1.0 / 1.15, and its three
  names survive as built-in **size presets** — `BUILT_IN_SIZE_PRESETS`, Compact 0.90 / Regular 1.0
  / Comfortable 1.15 / Large 1.30, each at `text_ratio` 1.0. A `SizePreset` is a name and those two
  numbers and nothing else: never a palette, never an accent. The built-ins are code; what the user
  saves rides `InterfacePrefs.size_presets` (`serde(default)`, no schema bump), and a saved one
  named after a built-in takes that built-in's place rather than sitting beside it.
- **Nothing about appearance is per project**, the content size included. `AppState::set_ui_scale`,
  `set_text_ratio` and `set_trim` all move the process-wide cell, and all settle behind
  `AppState::settle_metrics` — re-dressing an emulator emits a `TerminalResize`. `⌘=` / `⌘-`
  (`nudge_content_trim`) is the only other way in; there is no setter for an outright point size.
- **A scale change calls `theme::redress(cx)`**: the library's `font_size` *is* the window's rem
  size, and GPUI's whole Tailwind spacing scale is rem-relative (`D153`).

- **`set_mode`, never `Theme::set`.** Two theme systems are live — Ubiq's tokens and the
  component library's own (which colours the editor, textarea, scrollbars, markdown view).
  `set_mode` moves both so they cannot drift.
- **A pane's emulator is *given* a palette, not reading one.** `toggle_theme()` pushes a rebuilt
  config into every emulator as well as calling `set_mode`. Any component handed a palette must
  be walked the same way.
- **`_soft` variants declare their own alpha** in `theme.rs`, never `.alpha(...)` at a call site.
- **`project_colour` wraps**, so swatches are only ever *appended to* — reordering recolours every
  project in the catalogue. `project_tint` ranks temporary grey → custom → swatch, in one place.
- **Syntax colours are not tokenised** — they come from the library's highlighter theme, which
  `set_mode` keeps in step.
- **Which token a *state* reads in is decided in `ui/work.rs`**: an activity or bucket becomes a
  status colour, a role becomes a glyph. `ubiq_proto::work` keeps the words, tokens keep the
  values — so the agents columns, the graph, the board and the status bar cannot disagree.

## Layout constants (`theme.rs`)

**Every pixel constant is a `pub const` base with a `scaled()` accessor beside it, and a call site
reads the accessor** — `theme::titlebar_height()`, `theme::explorer_width()`,
`theme::settings_width()`. The const is the size at `ui_scale = 1.0`. Two exceptions:
`theme::hairline()` is 1.0 at every scale, and `MODAL_MAX_HEIGHT` is a ratio.

| Constant | Value | Is |
|---|---|---|
| `MONO_FONT` | `Menlo` / `Cascadia Mono` / `DejaVu Sans Mono` | The OS mono, so the text system resolves it rather than falling back to a proportional face |
| `ACCENT_EDGE` | 2.0 | The identifying left border's width |
| `TERMINAL_PADDING` / `TERMINAL_SCROLLBACK` | 8.0 / 10 000 | The terminal body. Its type size is `theme::content_base()` |
| `TEXT_BASE` / `REM_BASE` | 13.0 / 16.0 | The one base every type size derives from, and the window's rem size at `ui_scale = 1.0` (`D153`) |
| `EDITOR_FONT_MIN` / `EDITOR_FONT_MAX` | 8.0 / 36.0 | A point range with no caller since the status bar's px ladder became the size popover |
| `SIZE_STEP` / `UI_SCALE_STOPS` / `TEXT_RATIO_STOPS` | 0.05 / 13 / 8 | The step both axis sliders snap to, and the stops that follow. `0.05` divides 0.80, 1.40, 0.85, 1.20 *and* 1.0, which is what makes both ends reachable by the ladder rather than the clamp |
| `ICON_SM` / `ICON_MD` / `ICON_LG` | 11 / 14 / 18 | The icon sizes, read through `theme::icon_sm/md/lg()` and fed to `Size::Size(px)` |
| `TITLEBAR_HEIGHT` / `STATUS_BAR_HEIGHT` / `RAIL_WIDTH` | 34 / 30 / 56 | The chrome the user cannot drag |
| `EXPLORER_WIDTH` / `CHAT_WIDTH` / `DOCK_HEIGHT` | 300 / 420 / 300 | What each dock edge region *opens at*. The **default** scales; the **stored** size, in the arrangement blob, does not (`D151`) |
| `INSPECTOR_WIDTH` / `TASKS_HEIGHT` / `GRAPH_DOT_PITCH` | 420 / 220 / 28 | The orchestration screen |
| `AGENT_SIDEBAR_WIDTH` / `NEW_COLUMN_STRIP` | 300 / 28 | The agents screen |
| `COLUMN_WIDTH` / `COLUMN_SHUT` / `TASK_PANEL_WIDTH` | 320 / 44 / 420 | Columns and the task panel |
| `MODAL_WIDTH` / `MODAL_MAX_HEIGHT` | 460 / 0.8 | One question; the fraction of window height its body scrolls inside |
| `LOGIN_MODAL_WIDTH` / `LOGIN_MODAL_HEIGHT` | 960 / 720 | The one modal that is not one question — a full-screen harness login TUI |
| `SETTINGS_WIDTH` / `SETTINGS_HEIGHT` | 820 / 560 | The settings page overlay |

`MdWidth` (`Readable`/`Wide`/`Full`), `MdDensity` (`Comfortable`/`Compact`) and `MdMinimapSide`
(`Left`/`Right`) are the Markdown preview's own axes (T-116, T-118) — ratios over the body font
size, not `ui_scale`-scaled pixels, so they take no `scaled()` accessor. `MD_AVG_CHAR_WIDTH_EM`,
`MD_CODE_LINE_HEIGHT`, `MD_INLINE_CODE_SIZE_EM` and `md_measure_width`, `md_body_line_height`,
`md_paragraph_gap`, `md_heading_ratio`, `md_min_margin`, `md_top_inset`, `md_bottom_inset` are read
by `ui/viewer/markdown.rs`; see `_docs/inbox/markdown-improvement-proposal.md` §3–§7.

**Not here**: a *screen's* own furniture lives with its state — `state::git`'s `SIDEBAR_WIDTH`,
`CHANGES_WIDTH`, `DIFF_HEIGHT`, `LANE_PITCH`; `state::agents`' `COLUMN_MIN_WIDTH`. Only the
window's own areas belong in `theme.rs`.

## Kit primitives (`ui/kit/`)

Callback types: `Action = Rc<dyn Fn(&mut Window, &mut App)>` and
`IndexedAction = Rc<dyn Fn(usize, &mut Window, &mut App)>`. Bridge to the root view with
`ui::handler` / `ui::indexed`. `HsvAction = Rc<dyn Fn(f32, f32, f32, &mut Window, &mut App)>` is
`colour_picker`'s, bridged with `ui::hsv`. `HARNESS_GLYPH = "*"` is the placeholder harness icon — used where
a harness is a small secondary tag, never where one is being *chosen*.

### `controls.rs` — the primitives the library does not give us

| Fn | Is |
|---|---|
| `slab(edge) -> Div` | The shape every surface is drawn in: square, coloured left edge |
| `card(id, edge, selected) -> Stateful<Div>` | A slab you can pick |
| `field(edge, focused) -> Div` | The container every free-text input sits in — left edge plus a focus-colour bottom underline while it holds the keyboard |
| `mono(text, color)` | A mono run |
| `section_label(text)` | A section heading |
| `status_dot(color, ring)` | The one surviving circle. Returns `Div`, not an opaque element, so `ui::conversation::lifecycle_dot` can hang a pulse animation on it |
| `pill(edge) -> Div`, `badge(text, color)`, `state_chip(label, colour, scale)` | Inline tags |
| `choice_pill(...)` | One value of a set |
| `toggle_pill(...)` | An independent facet |
| `tag(...)` | The label does something when clicked, and nothing takes it off |
| `removable_tag(...)` | `tag` plus a `×` that drops it. Built from `tag`, so the two never drift |
| `check_box(...)` | A row chosen where several may be |
| `icon_button(...)`, `ghost_button(...)`, `primary_button(...)` | Icon-only (30px, tooltip carries the label), inline label, and the screen's single obvious action |
| `elided(...)`, `elided_with(...)` | Truncate with the system ellipsis, whole string as tooltip — takes an element id |
| `meter(fraction, colour)` | The flat meter |
| `progress_ring(pct, diameter)`, `progress_ring_in(pct, diameter, fill)` | A canvas ring; `_in` takes its own fill token |
| `stepper(...)` | Numeric stepper |
| `disclosure(...)` | The disclosure bar |

### `slider.rs` — the one continuous control

`Slider::new(id, &Entity<SliderState>, tooltip)`, `.leading(icon)`, `.trailing(icon)`,
`.disabled(bool)` — a skin over `gpui_component::slider`, not a control of our own: the drag, the
pointer capture, the keyboard and the accessibility role come with the library. The palette reaches
it through the library's own reading of `background` (the track's fill) and `text` (the thumb). The
tooltip is a constructor argument because a slider carries no number and no unit and is therefore
always unlabelled; the icon slots are the scale (`size-interface-small` / `size-interface-large`),
drawn at `theme::icon_sm()`.

`slider_state(min, max, stops, value) -> SliderState` quantises the axis to `stops` evenly spaced
values, so a drag lands on a ladder rather than on 1.0374. The library rounds to multiples of the
step measured from zero rather than from `min` — pick the three numbers so the step divides them.

The `Entity<SliderState>` lives on `AppState` with its subscription on `_subscriptions` (the stated
exception: the widget's state *is* the model). Listen to `SliderEvent::Change`, not `Release`.
Nothing pushes the state back for you: a value moved by something else calls `set_value` —
`AppState::sync_sink_slider` is the pattern, and `sync_size_sliders` is the same thing over the two
size axes, called from every gesture that opens a surface drawing them.

The three sliders in the tree are `sink_slider`, `ui_scale_slider` and `text_ratio_slider`. The
last two are drawn by `ui/size.rs` and appear in **two** places — the status bar's size popover and
the Size settings section — from one builder, which is what keeps the control single. A drag routes
through `AppState::set_ui_scale` / `set_text_ratio` into `settle_metrics`, so a drag costs one
terminal re-dress and one `SetPreferences` write rather than one per frame.

### `colour.rs` — the HSV picker

`colour_picker(prefix, hue, sat, val, current, hex, hex_focused, on_pick)` — a 16×10
saturation/value plane over a painted wash, a 24-step hue strip, a preview block, and the caller's
`#RRGGBB` field (the caller owns the `Entity<InputState>`, as `prompt_modal` does). It reports the
three axes back and has no idea what they colour: project settings is one caller, the theme editor
the other. The generated swatches are **content**, the one place the no-literal-colour rule bends;
the chrome — cursor box, preview border, hex field — is tokens, sized through `theme::scaled()`.
The HSV↔RGB maths is `theme::hsv_to_rgb` / `theme::rgb_to_hsv`, in `theme.rs` because `state/` and
`ui/kit/` may both name it and not each other. `gpui-component`'s `ColorPicker` is a different
control (featured swatches in a popover) and stays on the image editor's stroke.

### `panel.rs` — panel chrome

`panel() -> Div`, `panel_header(title, actions)`, `Tab`, `tab_strip(...)` — the strip the editor
and the terminal dock both use.

### `overlay.rs` — the modal family

`modal(...)`, `modal_sized(...)`, `modal_note(text)`, `confirm_modal(...)` (`danger: true` draws
the `danger` edge), `prompt_modal(...)` (the caller owns the field's `InputState`, so what was
typed survives a redraw; `confirm_enabled: false` dims at `.opacity(0.5)` — there is no second
disabled style).

### `menu.rs` — one dropdown mechanism for every menu in the window

`Picker`, `MultiPicker`, `PickerStyle`, `multi_label(...)`, `multi_order(...)`,
`context_menu(...)`, `context_panel(...)`, `MENU_LAYER = 1`, `MENU_ANCHOR_UP`, `MULTI_WIDTH = 170`.

- **The picker never filters.** The caller narrows `items` and keeps a parallel values list in
  lockstep, so `on_pick(index)` stays correct by construction. An empty result draws one muted
  "No matches" row.
- `.search(&state, focused)` adds a filter field in the same `field(...)` shape.
- `.disabled(indices)` draws a row not pickable, in the same faint style — never dropped from
  `items`, because a vanished row reads as *gone* rather than *taken*. A picker's own selected
  row stays pickable even if also listed disabled.
- `.separators(indices)` draws a hairline instead of text — a separator still takes an index,
  because rows and the actions behind them are matched by position.
- A context menu is the same panel, opened at the pointer rather than under a chip.
- **`MultiPicker` is the same dropdown for a set**, sharing the trigger and the panel rather than
  forking them: a row toggles, the list stays down, and the closed trigger says every ticked value
  comma-separated through `kit::elided` (truncation and tooltip are one string). `.selected(ixs)`
  is the preselection *and* the value — it holds nothing — `.dots(colours)` puts a status dot on
  each row, and `multi_order(len, selected, query)` draws the ticked rows first under an empty
  query, nothing pinned under a typed one, and never reorders a list with no search field. First
  caller: the Teams toolbar's states filter.

### `md_navigator.rs` — a markdown document's headings, hierarchically, with thread counts

`md_navigator(id, trigger, open, entries: &[MdNavEntry], on_toggle, on_select, on_dismiss)`,
`MdNavEntry { level, label, open, resolved }`. The same anchored-list device as `menu.rs`'s
dropdown, built directly rather than through `Picker`: a row needs an indent by heading depth and
two independent counts a plain-label row has no place for. `on_select` is handed the row's own
index into `entries`. First caller: `crate::ui::plan`'s chrome, over
`state::document::heading_sections`.

### `minimap.rs` — a strip of positioned marks, generic over what they mean

`minimap(id, width, marks: &[MinimapMark], on_select)`, `MinimapMark { fraction, colour }` —
`fraction` is `0.0`–`1.0` down the strip, `colour` is an `Rgba` the caller already resolved from a
token. Fills whatever height its parent gives it; a mark is a short absolutely-positioned tick at
`top(relative(fraction))`, clicked to hand `on_select` its own index into `marks`. Nothing plan- or
document-shaped lives here — the caller positions and colours every mark. First caller: `crate::ui::
plan`'s thread minimap, over `state::document::thread_marks`'s pure data, with `ScrollHandle::
bounds_for_item` turning a block index into a real pixel offset once the preview has painted a
frame (a proportional spread across the blocks otherwise).

### `popover.rs` — the anchored panel that is not a list

`popover(id, min_width, debug, on_dismiss, children)` — the same chrome `context_panel` draws
(`surface_raised`, a coloured left edge, `shadow_lg`, `deferred` + `anchored` at `MENU_ANCHOR_UP`)
with the rows left to the caller, for a panel that holds controls rather than menu items. The size
popover is the second caller after the conversation screen's delegate panel. `on_dismiss` is the
outside-click handler — `Some(Rc::new(handler(...)))` for a panel painted over the window,
`None` for one drawn inside a trigger that already owns the click that closes it.

### `files.rs` — chrome the picker and the explorer share

`ROW_FONT = 12.5`, `row_height(font_size)`, `row_indent(font_size)`, `file_row(...)`,
`filter_bar(...)`, `twisty(...)`, `kind_icon(is_dir, color)`, `view_switch(...)`.

A file row is sized from its text over `theme::ui_scale()`, so the content zoom changes how tight the tree is. A surface
no project zoom reaches passes `ROW_FONT`.

**A filter field and the list under it are two focuses** — the default for every tree and
collapsible list. Three keys cross the boundary: `down`/`tab` step from field onto list, `escape`
clears the query from either, `enter` opens what the query landed on without leaving the field.
`tab`/`shift-tab` step back. Clicking a row puts the keyboard on the list.

### `settings.rs` — settings page furniture

`heading(title, note)`, `setting_row(label, note, control)`, `label_block(label, note)`,
`nav_item(...)`, `column(children)`.

### `canvas.rs` — what is painted rather than laid out

`dot_grid(spacing, offset)`, `Link` + `links(...)`, `Grain` + `sand(grains, colour)`,
`dashed_box(rect, colour, active)`. Each is one layer filling its parent absolutely, taking no
click, knowing nothing about what it draws — a caller stacks them in reading order.

### `ribbon.rs` — a word across a corner

`ribbon(word, corner, band, ink)`, `RibbonCorner`, `RIBBON_SIZE`. An SVG band pinned to one
corner of a `.relative()` parent. `.size(px)` scales the box; `.on_click(id, …)` is taken only
on the band. The build-channel ribbon and Git's experimental ribbon are this function.

## `ui/mod.rs` helpers

| Fn | Is |
|---|---|
| `handler(&view, f)` | Adapts an `AppState` method into the plain `Fn(&mut Window, &mut App)` the kit expects |
| `indexed(&view, f)` | The same for index-carrying callbacks — tab strips, menu rows |
| `eid(prefix, id)`, `eid2(prefix, a, b)` | An `ElementId` for a ULID-keyed row (and one two ids deep) |
| `on_link(app, base)` | Follows a link in a rendered document: relative resolved against `base` (`None` = project root), a place is navigated to, http/https/mailto go to the OS, anything else is nothing |
