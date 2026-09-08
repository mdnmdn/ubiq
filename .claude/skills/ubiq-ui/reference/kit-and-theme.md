# Tokens, constants and kit primitives

Everything here is `crates/ubiq/src/theme.rs` and `crates/ubiq/src/ui/kit/`.

## Theme tokens (`theme.rs`)

A token names what a colour is *for*, so a palette swap changes every surface consistently.
Two complete palettes, `dark()` and `light()`; a token in one exists in the other. The active
theme is thread-local — a call site never learns which palette answered.

| Group | Accessors | For |
|---|---|---|
| Surface | `app_bg`, `pane_bg`, `surface`, `surface_raised`, `hover`, `selected`, `selected_focus`, `scrim` | The stack from window down to a selected row, deepening once the list holding it has the keyboard; `scrim` is what a modal lays over the window |
| Text | `text`, `text_muted`, `text_faint`, `on_accent` | Primary, secondary, faintest (ignored rows, timestamps, hints), and copy on a filled surface |
| Accent | `accent`, `accent_muted`, `accent_soft` | The interactive colour, its subdued form, the fill behind a selected row |
| Terminal | `selection_background`, `link_underline`, `link_underline_hover` | Selected cells in a pane; the OSC 8 / detected-URL underline |
| Border | `border`, `border_focus` | Ordinary separation; the focused edge |
| Status | `danger`, `success`, `warning`, `info` + a `_soft` each | Agent and process states, and the fills behind them |
| Ribbon | `ribbon_alpha`, `ribbon_beta`, `ribbon_ink` | The build-channel ribbon — same value in both palettes |
| Project | `project_colour(n)`, `project_colour_count()`, `project_temporary()`, `project_tint(temporary, colour, custom)`, `mark_dark(colour)` | One project's identity wherever it appears |

Helpers: `set_mode(id, cx)`, `palette_for(id)`, `fade(colour, alpha)`, `rgba_of(rgb)`,
`ThemeId::toggled`.

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

| Constant | Value | Is |
|---|---|---|
| `MONO_FONT` | `Menlo` / `Cascadia Mono` / `DejaVu Sans Mono` | The OS mono, so the text system resolves it rather than falling back to a proportional face |
| `ACCENT_EDGE` | 2.0 | The identifying left border's width |
| `TERMINAL_FONT_SIZE` / `TERMINAL_PADDING` / `TERMINAL_SCROLLBACK` | 13.0 / 8.0 / 10 000 | The terminal body |
| `EDITOR_FONT_SIZE` / `EDITOR_FONT_MIN` / `EDITOR_FONT_MAX` | 13.0 / 8.0 / 36.0 | Editor base size and the range a project's zoom lives in — the same size the terminal panes and explorer tree follow |
| `TITLEBAR_HEIGHT` / `STATUS_BAR_HEIGHT` / `RAIL_WIDTH` | 34 / 30 / 56 | Fixed chrome, does not resize |
| `EXPLORER_WIDTH` / `CHAT_WIDTH` / `DOCK_HEIGHT` | 300 / 420 / 300 | What each dock edge region *opens at*; what a drag lands on is remembered per project |
| `INSPECTOR_WIDTH` / `TASKS_HEIGHT` / `GRAPH_DOT_PITCH` | 420 / 220 / 28 | The orchestration screen |
| `AGENT_SIDEBAR_WIDTH` / `NEW_COLUMN_STRIP` | 300 / 28 | The agents screen |
| `COLUMN_WIDTH` / `COLUMN_SHUT` / `TASK_PANEL_WIDTH` | 320 / 44 / 420 | Columns and the task panel |
| `MODAL_WIDTH` / `MODAL_MAX_HEIGHT` | 460 / 0.8 | One question; the fraction of window height its body scrolls inside |
| `LOGIN_MODAL_WIDTH` / `LOGIN_MODAL_HEIGHT` | 960 / 720 | The one modal that is not one question — a full-screen harness login TUI |
| `SETTINGS_WIDTH` / `SETTINGS_HEIGHT` | 820 / 560 | The settings page overlay |

**Not here**: a *screen's* own furniture lives with its state — `state::git`'s `SIDEBAR_WIDTH`,
`CHANGES_WIDTH`, `DIFF_HEIGHT`, `LANE_PITCH`; `state::agents`' `COLUMN_MIN_WIDTH`. Only the
window's own areas belong in `theme.rs`.

## Kit primitives (`ui/kit/`)

Callback types: `Action = Rc<dyn Fn(&mut Window, &mut App)>` and
`IndexedAction = Rc<dyn Fn(usize, &mut Window, &mut App)>`. Bridge to the root view with
`ui::handler` / `ui::indexed`. `HARNESS_GLYPH = "*"` is the placeholder harness icon — used where
a harness is a small secondary tag, never where one is being *chosen*.

### `controls.rs` — the primitives the library does not give us

| Fn | Is |
|---|---|
| `slab(edge) -> Div` | The shape every surface is drawn in: square, coloured left edge |
| `card(id, edge, selected) -> Stateful<Div>` | A slab you can pick |
| `field(edge, focused) -> Div` | The container every free-text input sits in — left edge plus a focus-colour bottom underline while it holds the keyboard |
| `mono(text, color)` | A mono run |
| `section_label(text)` | A section heading |
| `status_dot(color, ring)` | The one surviving circle |
| `pill(edge) -> Div`, `badge(text, color)`, `state_chip(label, colour, scale)` | Inline tags |
| `choice_pill(...)` | One value of a set |
| `toggle_pill(...)` | An independent facet |
| `removable_tag(...)` | `×` drops it; the label does something when clicked |
| `check_box(...)` | A row chosen where several may be |
| `icon_button(...)`, `ghost_button(...)`, `primary_button(...)` | Icon-only (30px, tooltip carries the label), inline label, and the screen's single obvious action |
| `elided(...)`, `elided_with(...)` | Truncate with the system ellipsis, whole string as tooltip — takes an element id |
| `meter(fraction, colour)` | The flat meter |
| `progress_ring(pct, diameter)`, `progress_ring_in(pct, diameter, fill)` | A canvas ring; `_in` takes its own fill token |
| `stepper(...)` | Numeric stepper |
| `disclosure(...)` | The disclosure bar |

### `panel.rs` — panel chrome

`panel() -> Div`, `panel_header(title, actions)`, `Tab`, `tab_strip(...)` — the strip the editor
and the terminal dock both use.

### `overlay.rs` — the modal family

`modal(...)`, `modal_sized(...)`, `modal_note(text)`, `confirm_modal(...)` (`danger: true` draws
the `danger` edge), `prompt_modal(...)` (the caller owns the field's `InputState`, so what was
typed survives a redraw; `confirm_enabled: false` dims at `.opacity(0.5)` — there is no second
disabled style).

### `menu.rs` — one dropdown mechanism for every menu in the window

`Picker`, `PickerStyle`, `context_menu(...)`, `context_panel(...)`, `MENU_LAYER = 1`,
`MENU_ANCHOR_UP`.

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

### `files.rs` — chrome the picker and the explorer share

`ROW_FONT = 12.5`, `row_height(font_size)`, `row_indent(font_size)`, `file_row(...)`,
`filter_bar(...)`, `twisty(...)`, `kind_icon(is_dir, color)`, `view_switch(...)`.

A file row is sized from its text, so the explorer's zoom changes the tree's density. A surface
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

## `ui/mod.rs` helpers

| Fn | Is |
|---|---|
| `handler(&view, f)` | Adapts an `AppState` method into the plain `Fn(&mut Window, &mut App)` the kit expects |
| `indexed(&view, f)` | The same for index-carrying callbacks — tab strips, menu rows |
| `eid(prefix, id)`, `eid2(prefix, a, b)` | An `ElementId` for a ULID-keyed row (and one two ids deep) |
| `on_link(app, base)` | Follows a link in a rendered document: relative resolved against `base` (`None` = project root), a place is navigated to, http/https/mailto go to the OS, anything else is nothing |
