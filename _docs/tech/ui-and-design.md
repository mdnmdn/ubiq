---
id: tech-ui
title: UI and design
kind: tech
status: current
summary: The GPUI rendering model, the complete theme token set and the rule that no colour escapes it, how a palette is switched, the shape every surface, modal and dialog is drawn in, the page every primitive is looked at on, and the design assets screens are built against.
read_when: you are building or restyling a screen, adding a colour or a size, switching or extending a palette, raising a modal or the file picker, looking at a primitive on the style reference, or looking for the wireframe a layout came from
updated: 2026-09-25
verified: 2026-09-25
code_anchors: [crates/ubiq/src/theme.rs, assets/icons/icons.yaml, _tools/icons.py, crates/ubiq/src/app/mod.rs, crates/ubiq/src/app/shell.rs, crates/ubiq/src/app/wire.rs, crates/ubiq/src/ui/viewer/diff.rs, crates/ubiq/src/ui/viewer/mod.rs, crates/ubiq/src/ui/viewer/md_options.rs, crates/ubiq/src/ui/mod.rs, crates/ubiq/src/ui/mark.rs, crates/ubiq/src/ui/work.rs, crates/ubiq/src/ui/outline.rs, crates/ubiq/src/ui/board/mod.rs, crates/ubiq/src/state/board.rs, crates/ubiq/src/ui/kit/mod.rs, crates/ubiq/src/ui/kit/controls.rs, crates/ubiq/src/ui/kit/colour.rs, crates/ubiq/src/ui/kit/files.rs, crates/ubiq/src/ui/kit/menu.rs, crates/ubiq/src/ui/kit/md_navigator.rs, crates/ubiq/src/ui/kit/minimap.rs, crates/ubiq/src/ui/kit/canvas.rs, crates/ubiq/src/ui/kit/blocks.rs, crates/ubiq/src/ui/kit/overlay.rs, crates/ubiq/src/state/overlay.rs, crates/ubiq/src/ui/kit/ribbon.rs, crates/ubiq/src/ui/kit/settings.rs, crates/ubiq/src/ui/kit/popover.rs, crates/ubiq/src/ui/size.rs, crates/ubiq/src/app/size.rs, crates/ubiq/src/ui/explorer.rs, crates/ubiq/src/ui/file_picker.rs, crates/ubiq/src/state/file_picker.rs, crates/ubiq/src/state/prefs.rs, crates/ubiq/src/ui/sink/style.rs, crates/ubiq/src/ui/shell.rs, crates/ubiq/src/ui/ribbon.rs, crates/ubiq/src/ui/settings.rs, crates/ubiq/src/ui/terminal.rs, crates/ubiq/src/ui/dock/mod.rs, crates/ubiq/src/ui/dock/skin.rs, crates/ubiq/src/ui/conversation/mod.rs, crates/ubiq/src/ui/teams/status.rs, crates/ubiq/src/ui/titlebar.rs, crates/ubiq/src/ui/navigator.rs, crates/ubiq/src/ui/viewer/scene.rs, crates/ubiq/tests/dismiss.rs, crates/ubiq/src/ui/remote_hosts.rs]
depends_on: [tech-architecture]
review_cycle: quarterly
---

# UI and design

## The rendering model

The UI is GPUI — Zed's retained-mode, GPU-accelerated framework — with the `gpui-component` widget
set on top for the components an application expects to be given rather than to write.

Three properties shape how UI code reads:

**Views are structs that render.** A type implementing `Render` owns its state and produces an
element tree from it. `AppState` in `crates/ubiq/src/app/mod.rs` is the root: it owns the window's own
state — the dock, the console, the emulators — and one `OpenProject` per project the window holds,
carrying that project's tree, files, panes and open chat tabs. Its render delegates to
`crates/ubiq/src/ui/shell.rs`.

**`AppState` is the only owner of state, and not the only view.** The window's arrangement is a
dock of movable panels, and the component library requires each panel to be an entity that renders,
focuses and emits — `D42`, which half reverses `D17`. A panel is an adapter: `WorkbenchPanel` in
`crates/ubiq/src/ui/dock/mod.rs` holds a weak `AppState` handle and a panel kind, and its render is
a `match` that delegates to the same free functions every screen area is.

**Mutation ends in a redraw request.** Nothing repaints because a field changed; it repaints because
the code that changed it said so through its context. Every state-mutating method on `AppState` ends
that way, and one that forgets is a pane that stops updating.

**A redraw the window has no use for is the one thing that may be skipped, and only for a stream.**
The root is one entity, so any notify costs a whole window's frame — which a token arriving in a
conversation nothing on screen is showing does not earn. `Message::ConversationUpdate` in
`crates/ubiq/src/app/wire.rs` records the delta either way and asks for the frame only for a
conversation some surface shows, and only once per frame: `Conversation::notify_due()` is true for
the first delta of a burst and the frame that draws it opens the next window, so a stream is
coalesced by the drawing rather than by a timer, and nothing can be dropped. The test is the
message, not the state: every other update draws unconditionally, because a state change nobody
counted is a screen that stops agreeing with the host.

**Layout is flexbox.** Elements are composed with the same direction, grow, gap and alignment
vocabulary as CSS flexbox, in Rust builder form.

`crates/ubiq-app/src/lib.rs`'s `run` installs the component library and its assets, sets the palette
and binds the quit action, then asks for the first window. Windows themselves are opened by
`app::open_project_window`, which is the only place one is created, so the first window and "open in
a new window" go through the same code. Each window owns its own `AppState` and they share nothing
but the palette, which is process-wide. Everything drawn belongs under `crates/ubiq/src/ui/`.

## Theme tokens

**This document owns the token set.** Every colour in the UI comes from a token accessor in
`crates/ubiq/src/theme.rs`. A literal colour anywhere else is a defect, with no exceptions worth
carving out — a one-off shade is the mechanism by which a themed application stops being themeable.

**Content is not colour.** A colour read out of a document — a scene's stroke, its fill, the canvas
ground it names, the pixels of an image embedded in it — is data the way the sixteen ANSI colours in
a pane are data, and painting it is not a design decision. The rule above binds every colour the
interface *chooses*: `crates/ubiq/src/ui/viewer/scene.rs` paints a scene's own colours and an
embedded image's bytes straight through, and reaches for a token for the only two things it decides
itself — the placeholder a picture with nothing decodable in it draws as, and the text of a failure
note.

**Which token a *state* reads in is the interface's choice, and it is made in one file.**
`crates/ubiq/src/ui/work.rs` is where a work record's state becomes a token: an activity or a bucket
becomes a status colour, and a role becomes a glyph. `ubiq_proto::work` keeps the words and this
document's tokens keep the values, so the agents screen's columns, the orchestration graph, the tasks
board and the status bar cannot disagree about what running looks like.

Tokens are grouped by role, and the role is the point: a token names what a colour is *for*, so that
a palette swap changes every surface consistently.

| Group | Accessors | For |
|---|---|---|
| Surface | `app_bg`, `pane_bg`, `surface`, `surface_raised`, `hover`, `selected`, `selected_focus`, `scrim` | The stack of backgrounds, from the window down to a selected row, deepening once the list holding that row has the keyboard — and what a modal lays over the window it took the keyboard from |
| Text | `text`, `text_muted`, `text_faint`, `on_accent`, `mark` | Primary copy, secondary copy, the faintest tier — ignored rows, timestamps, hints — copy sitting on a filled surface, and the brand mark an empty page draws: white in every dark palette, `#003d6e` in every light one |
| Accent | `accent`, `accent_muted`, `accent_soft`, `accent_selection`, `accent_id` | The interactive colour, its subdued form, the fill behind a selected row, the highlight a selected run of text takes on a non-editor surface, and which accent the window is dressed in — all four colours derived from one seed, below |
| Terminal | `selection_background`, `link_underline`, `link_underline_hover` | Selected cells in a pane, and the underline on an OSC 8 or detected URL — brighter when the pointer is over it |
| Border | `border`, `border_focus` | Ordinary separation, and the focused pane's edge |
| Status | `danger`, `success`, `warning`, `info`, each with a `_soft` variant | Agent and process states, and the fills behind them — a diff line, a status chip, a state dot's ring |
| Provenance | `edit_human`, `edit_agent`, `edit_origin(is_human)` | Who last rewrote a line of an annotated document — the underline under a changed run in the plan editor, and the legend for it in that editor's footer. Two hues nothing else on that surface uses, because the annotated passages under the same text take `info_soft` and `success_soft` |
| Ribbon | `ribbon_alpha`, `ribbon_beta`, `ribbon_ink`, `ribbon_experimental`, `ribbon_experimental_ink` | The build-channel ribbon in the window's bottom-left corner, and the Git screen's experimental ribbon in its top-left — the same values in every palette, because they mark the build or the screen rather than the mood |
| Project | `project_colour(n)`, `project_colour_count()`, `project_temporary()`, `project_tint(...)`, `mark_dark(...)` | The identity of one project, wherever it appears |

The `_soft` variants are declared with their own alpha in `theme.rs` rather than computed at a call
site with `.alpha(...)`. A shade that only exists at one call site is a shade a palette swap cannot
reach. `accent_soft` is the one whose alpha is declared once for every palette — `ACCENT_SOFT_ALPHA`,
because the accent is derived rather than written out per palette — and it is in the same file, which
is what the rule asks.

`scrim` is the newest member of the surface group and the clearest case for the rule above. A modal
has to dim what is behind it, and how much a palette dims by is not the same in both — a dark ground
needs a heavier veil than a light one — so it is a token with a value in each rather than a `fade` at
the one call site that raises a modal.

`theme::fade` is the one transform allowed on a token: the same colour at another alpha, for
something that has to sit under, over or beside a surface — a dotted ground, a connector, a fading
grain of a drag trail. It is not a way to invent a shade, and it does not soften the rule above: a
fill or a text colour that a call site wants at a fixed alpha is a `_soft` token with a value in
both palettes, not a `fade` where it is drawn.

`accent_selection` and `selection_background` look like the same idea and are not the same token,
because the two surfaces that paint a selection highlight do it in the opposite order. The terminal
repaints a cell's background and then its glyph every frame, so `selection_background` can be
opaque — nothing is ever drawn over it, it is what a cell's background *is*. Every other selectable
surface — the markdown preview, the chat transcript, the A2UI tree, the code editor — is the
component library's `TextView` or `Input`, and at least one of them (`TextView`) paints the
selection quad *after* the glyphs it covers, so `accent_selection` (what `dress_component_library`
hands the library's `ThemeColor::selection`) has to stay translucent — `ACCENT_SELECTION_ALPHA` in
`theme.rs` — or the highlight blots the very text it is marking out instead of marking it. Wiring
`selection_background`'s opaque terminal colour into that same slot was exactly this defect.

The project group is the one group whose members carry no role. A swatch means *this project* and
nothing else, and a project keeps the same one everywhere it is drawn: its dot in the picker, the
fill behind its name in the titlebar, the mark above the rail, and the window's whole left edge.
`project_colour` wraps, so the number of projects is not bounded by the number of swatches, and
`project_colour_count` is what project settings offers. `project_tint` ranks the three sources — the
temporary grey, a custom colour, then the swatch — and `mark_dark` takes the *resolved* colour, so
one function answers for all three and for the border a window with no project falls back to, in one
place rather than four call sites each falling back to swatch zero. A swatch index is stored, so the
swatches are only ever **appended to**: reordering them recolours every project the catalogue holds.

**A palette is a registry row, keyed by a slug.** `PALETTES` in `theme.rs` holds one `PaletteDef`
per palette — its slug, its display name, its `Mode { Dark, Light }`, the slug of its counterpart,
and the tokens as one complete `Palette` value rather than a builder, so a palette cannot ship
missing one. `ThemeId` is a slug newtype over that registry rather than a closed enum, so a family
is an entry rather than a variant and a match arm in every file that reads one. Ten palettes in
five families ship: `dark`/`light`; `ember-dark`/`ember-light` — warm, low contrast;
`contrast-dark`/`contrast-light`, the accessibility case a faint tier at the built-in value does not
serve; `navy-dark`/`navy-light` — cool blue grounds; and `violet-dark`/`violet-light` — aubergine
grounds. A token that exists in one exists in all ten. The active theme is thread-local and read
through the accessor, so a token call site never learns which palette answered it.

`Mode` is the one thing a palette says about itself that is not a colour: it is what the component
library and the syntax highlighter are told, since those know only two grounds.
`ThemeId::counterpart` is what the titlebar's toggle follows, so one click flips ground **inside the
family the user chose** — a warm dark reaches the warm light, not the built-in one.
`InterfacePrefs.theme` stores the slug, and its `Deserialize` is case-insensitive, which is how a
blob holding `"Dark"` or `"Light"` reads as `dark` and `light`; an unknown slug falls back to the
default rather than discarding the blob.

**A user's own theme is a fork of one of those rows plus a sparse override map** (`D152`).
`CustomTheme { id, name, base, overrides }` lives in `InterfacePrefs.custom_themes` and resolves as
`palette_for(base)` → the overrides → `with_accent`, so an author supplies one colour or sixteen and
the rest stay the fork's. The editable set is **grounds and ink** — the eight surfaces, the four
text colours, the two borders and the accent seed, which is `theme::EDITABLE_TOKENS`; status,
ribbon, brand-mark, terminal and project-swatch tokens inherit, because they carry meaning rather
than taste — `text.mark` is the brand, which a fork of a palette does not get to restate. An
override is a hue and not a transparency: the alpha stays the palette's, which is what keeps the
scrim a scrim. A custom id is `custom-…` and is **interned** rather than owned, so `ThemeId` stays a
`Copy` newtype over a `&'static str` and the built-ins stay compile-time; `ThemeId::base` is the
built-in an id resolves through, and a slug naming a theme this config root does not hold resolves
to that base rather than to a dangling row. The registry itself takes no runtime entries —
`ThemeId::all` is still the ten built-ins — and `theme::set_custom_themes` is what hands the window
its list, pushed from `AppState` whenever the interface's preferences change.

**An accent is one seed, and the six hued tokens derive from it.** `ACCENTS` holds one `AccentDef`
— slug, name, one `Rgba` — per accent, and `theme::with_accent` resolves it against the palette in
hand: `accent` is the seed pushed away from `surface.base` until it clears WCAG's contrast floor for
a non-text mark, `accent_muted` is the seed mixed toward `surface.base`, `accent_soft` is the seed at
the `_soft` alpha, `border.focus` is the seed unmodified, `link_underline` and its hover are the seed
and the seed lifted, and `text.on_accent` follows `mark_dark`. So N palettes and M accents cost
N + M declarations, and every ratio and alpha behind them is a const in `theme.rs` rather than a
value repeated per palette. `InterfacePrefs.accent` is an `Option<AccentId>` where `None` is the
palette's own seed. **The project swatches stay outside this axis**: `D19` makes a swatch identity
rather than role, and recolouring sixteen of them with one accent would make two projects look the
same, so they keep their per-palette literals.

**A type size is a family and a role, never a number.** `theme::font(Family, Role)` is the one place
a size in the interface comes from: a size is owned the way a colour is, which is `D10`'s rule on a
second axis. Three families, because the three are read differently and resize for different
reasons: `Family::Chrome` is the furniture — titlebar, status bar, rail, tabs, menus, modals,
settings, pickers, notifications, dialogs; `Family::Content` is what code is read at — editor,
viewer, explorer tree, search results, terminal panes; `Family::Conversation` is prose — the
transcript, tool blocks, the composer, the agents columns and their sidebar. Each is a ratio over
one base rather than a base of its own: `TEXT_BASE` 13.0, `Family::ratio()` Chrome 0.96, Content
1.00, Conversation 0.96 — the three numbers that reproduce the hand-picked 12.5 / 13.0 / 12.5 the
scale replaced. Six roles are ratios over that — `Role::Title` 1.15, `Body` 1.00, `Dense` 0.96,
`Label` 0.92, `Meta` 0.85, `Micro` 0.80 — and the product is rounded to the nearest half point, the
grid those hand-picked sizes sat on. `Role::Dense` names the `- 0.5` / `- 1.0` the densest content
surfaces would otherwise write by hand: the explorer tree, a search result's second line, a raw
frontmatter block. A fourth family would be arguing about where a boundary falls; these three fall
on boundaries the code draws for other reasons. **The function is `font`, not `text`**, because
`theme::text()` is the primary text *colour* — the one name collision in the file worth knowing
before reading it.

**Sizing is two axes and nothing else (`D151`).** `theme::Metrics` holds them: **`ui_scale`**
(0.80–1.40) moves every dimension — the window's rem size, every constant in the table below, the
icon sizes, the terminal inset, and the type bases through `TEXT_BASE` — and **`text_ratio`**
(0.85–1.20) moves type only, on top of it. Beside them sit three per-family trims (0.70–1.60), for
a reader who wants a transcript larger than the chrome around it. The whole derivation is two
lines:

```rust
pub fn scaled(base: f32) -> f32 { (base * ui_scale()).round().max(1.0) }
pub fn font(family: Family, role: Role) -> Pixels {
    let base = TEXT_BASE * ui_scale() * text_ratio() * family.ratio() * trim(family) * role.ratio();
    px((base * 2.0).round() / 2.0)
}
```

At `text_ratio = 1` every proportion is held exactly, so the result is the same window seen at a
different distance; `text_ratio` is the only thing that changes a proportion, which is what the
second axis is for. Every axis is clamped **in the setter** — `theme::set_ui_scale`,
`set_text_ratio`, `set_trim`, or `set_metrics` for all five — so no call site learns a range
exists. `Density` is gone: its three steps are `ui_scale` 0.9 / 1.0 / 1.15, and the three names
survive as three of the Size section's four built-in presets — Compact, Regular and Comfortable —
alongside a fourth, Large, at 1.30.

**The rule is held mechanically, the way the crate boundary is.** `just ui` rejects
`text_size(px(<digit>` anywhere under `crates/ubiq/src`, and a literal icon size
(`with_size(px(<digit>`, `Size::Size(px(<digit>`) beside it, next to the check that the interface
never names the host. The digit is load-bearing: `text_size(px(font))`, where the size came out of
`theme::font`, is legitimate and a bare `text_size(px(` would reject it.

**Every appearance value belongs to the interface, and none to a project.** All five axes are
`InterfacePrefs` fields (`ui_scale`, `text_ratio`, `content_trim`, `chrome_trim`,
`conversation_trim`, each `serde(default)` at 1.0), written by `AppState::remember_interface` and
read back by `apply_preferences` through `theme::set_metrics`. `D151` moved the content family's size here too: `ViewPrefs.content_font_size` is parsed and
ignored, `ui::shell::render` pushes nothing into the scale at the top of a paint, and `⌘=` / `⌘-`
(`AppState::nudge_content_trim`) move `content_trim` by ±0.05, which works with no project open. The
status bar's control is the size popover — an icon-only trigger opening `ui::size::panel` — and it
names no point size at all; the content family's own trim pills live in the Size settings section
instead. `EDITOR_FONT_MIN` and `EDITOR_FONT_MAX` remain in `theme.rs` with no caller.

**A `SizePreset` is a name and the two numbers, nothing else.** `SizePreset { name, ui_scale,
text_ratio }` never carries a palette or an accent — those ride their own axes in `InterfacePrefs`.
Four ship as code rather than as stored rows — `BUILT_IN_SIZE_PRESETS`: Compact (`ui_scale` 0.90),
Regular (1.0), Comfortable (1.15) and Large (1.30), all at `text_ratio` 1.0; Compact sits at 0.90 so
it agrees with the `4 → 5` migration's `Density::Compact`. `InterfacePrefs.size_presets: Vec<SizePreset>`
holds what the user has saved, `#[serde(default)]` so the schema stays at 5 with no bump; `all_size_presets()`
puts a saved preset with a built-in's name in that built-in's place rather than beside it, so saving
replaces by name rather than appending. `SIZE_STEP` (0.05) is the quantisation step both sliders
share, chosen because it divides all four range ends — 0.80 and 1.40 for `ui_scale`, 0.85 and 1.20
for `text_ratio` — and divides 1.0, so both ends of both axes are reachable and the default is
itself a stop; `UI_SCALE_STOPS` (13) and `TEXT_RATIO_STOPS` (8) are the ladders each axis quantises
to, measured from zero.

**The window's rem size is the UI scale (`D153`).** `theme::dress_component_library` writes
`gpui_component::Theme::font_size = px(REM_BASE * ui_scale())` (`REM_BASE` 16.0, the value in force
before Ubiq ever wrote one) and `mono_font_size` from the content base. `gpui_component::Root` calls
`window.set_rem_size(cx.theme().font_size)` on every paint, and GPUI's whole Tailwind spacing scale
— `p_3`, `gap_2`, `w_4`, `h_8` — expands to `rems(...)`, so that one number is what makes several
hundred hand-placed spacings and every library internal measured in rems follow the scale with no
call-site change. It is why a scale change calls `theme::redress`, not just `cx.notify()`. The cost
is `D153`'s: Ubiq inherits the library's spacing judgement wholesale.

**Switching goes through `theme::set_theme`, never through `Theme::set`.** It takes the two colour
axes — palette and accent — resolves them into the one `Theme` every accessor reads, and dresses the
component library; `theme::set_mode` is that call with the accent left as it stands. The size axis
is deliberately not an argument, and not a field on `Theme` either: it is the axis with nothing to
resolve, because a scale is a number the user set rather than something derived from the palette in
hand. So it lives in a thread-local cell of its own beside the theme's, read through
`theme::metrics` and set through `theme::set_metrics`, which is what makes a palette or accent
switch structurally unable to undo a size the user chose — there is no resolution for it to be
dropped by. Two theme systems are live at once: Ubiq's tokens, and the component library's
own theme, which is what colours the editor, the textarea, the scrollbars and the markdown view.
`set_theme` moves both — `Theme::change` first, from the palette's `Mode`, then
`theme::dress_component_library` writes Ubiq's tokens into the library's `ThemeColor` through
`DerefMut` and `sync_base` pushes the result down to the layer that paints scrollbars and resize
handles. Only the fields that plainly correspond are written, and the two easy to mistake are the
library's `accent`, which is its hover ground, and `primary`, which is the brand colour Ubiq's
`accent` maps to. The theme is process-wide, so a second window opens in the palette, accent and
size axis the first is in, and switching in either switches both.

A pane's emulator is the one surface that does not read a token when it draws: it is built with a
copy of the palette, so `AppState`'s `toggle_theme`, `set_accent`, `set_ui_scale` and `set_trim`
each push a rebuilt configuration into every open emulator through `redress_terminals` as well as
switching the theme. Any component given a palette rather than reading one has to be walked the same way.

`theme.rs` also owns the constants that are not colours, for the same reason it owns the colours:
restyling the shell should be one file to visit.

| Constant | Is |
|---|---|
| `MONO_FONT` | The family for code, paths, counts and every mono label — the mono that ships with the OS (`Menlo`, `Cascadia Mono`, `DejaVu Sans Mono`), so the text system resolves it instead of falling back to a proportional face |
| `ACCENT_EDGE` | The width of the coloured left border that identifies a surface |
| `TERMINAL_PADDING`, `TERMINAL_SCROLLBACK` | The terminal body: the inset its output is drawn inside, and how many lines an emulator keeps. Its type size is the content family's, read through `theme::content_base()` |
| `TEXT_BASE` | The one number every type size derives from: what `Family::Content` draws `Role::Body` at with both axes at 1.0. Chrome and conversation are `Family::ratio()` under it |
| `REM_BASE` | The window's rem size at `ui_scale = 1.0`, handed to the component library — `D153`, and what makes GPUI's rem-relative spacing scale follow the UI scale |
| `UI_SCALE_MIN`/`MAX`, `TEXT_RATIO_MIN`/`MAX`, `TRIM_MIN`/`MAX` | What each axis is allowed to be. Clamped in the setters, never at a call site |
| `SIZE_STEP` | The quantisation step, 0.05, both size sliders share — chosen because it divides every range end and 1.0 too, so a stop sits on the default |
| `UI_SCALE_STOPS`, `TEXT_RATIO_STOPS` | The ladder each axis quantises to — 13 and 8 — measured from zero |
| `EDITOR_FONT_MIN`, `EDITOR_FONT_MAX` | Left with no caller: the status bar's control is the size popover, which offers no point size for them to bound |
| `ICON_SM`, `ICON_MD`, `ICON_LG` | The three icon sizes, read through `theme::icon_sm/md/lg()` and fed to the component library's `Size::Size(px)`. Its own `Size` enum is discrete and does not scale, which is why these exist |
| `DISPLAY_FONT_SIZE` | The one size off the scale, private and read through `theme::font_display()`: the device-login user code, a number to be read off a screen and typed into a phone rather than a heading |
| `TITLEBAR_HEIGHT`, `STATUS_BAR_HEIGHT`, `RAIL_WIDTH` | The chrome the user cannot drag |
| `EXPLORER_WIDTH`, `CHAT_WIDTH`, `DOCK_HEIGHT` | The size each of the dock's three edge regions opens at. What the user drags one to is remembered per project, inside the arrangement blob, and is what a restored window opens on |
| `INSPECTOR_WIDTH`, `TASKS_HEIGHT`, `GRAPH_DOT_PITCH` | The orchestration screen: the inspector beside its graph, the tasks drawer under it, and the pitch of the dotted ground at 100% zoom |
| `AGENT_SIDEBAR_WIDTH`, `NEW_COLUMN_STRIP` | The agents screen: the sidebar that lists every agent, and the strip past the last column that a dragged tab is split off into. How narrow a column itself may get is `state::agents::COLUMN_MIN_WIDTH` instead, because that is a fact about a conversation rather than about this window |
| `EMPTY_START_SIZE`, `EMPTY_START_ICON` | The start control on an empty chat panel: about three times a chrome `kit::icon_button`, because it is the page's whole subject rather than one control among a row of them |
| `PERMISSION_DETAIL_MAX_H` | A permission ask's own detail — a `switch_mode` prompt's plan, a pre-approval diff — capped and scrolled in its own region rather than left to grow the card without limit (T-175) |
| `MODAL_WIDTH`, `MODAL_MAX_HEIGHT` | A modal: one width, because a modal is one question, and the fraction of the window's height its body scrolls inside |
| `LOGIN_MODAL_WIDTH`, `LOGIN_MODAL_HEIGHT` | The one modal that is not one question: a running harness login, sized through `kit::modal_sized`'s fill mode so a full-screen TUI (`opencode`, `grok`) gets a real terminal instead of the ~50×16 a one-question modal would give it |
| `SETTINGS_WIDTH`, `SETTINGS_HEIGHT` | Application settings: a fixed-size page overlay with a nav, not a one-question modal and not a resizable dialog |
| `A2UI_IMAGE_ICON`, `A2UI_IMAGE_AVATAR`, `A2UI_IMAGE_SMALL`, `A2UI_IMAGE_MEDIUM`, `A2UI_IMAGE_LARGE`, `A2UI_IMAGE_HEADER_H` | The six sizes an A2UI `Image` variant maps onto. The catalog names the variant and this file decides how big it is, because a payload Ubiq did not write must not be able to state a size |
| `A2UI_SVG_MAX` | The box an agent-authored picture is fitted into, aspect preserved — the ceiling on how much of a surface one drawing may take |
| `MdWidth`, `MdDensity`, `MD_AVG_CHAR_WIDTH_EM`, `MD_CODE_LINE_HEIGHT`, `MD_INLINE_CODE_SIZE_EM` and the `md_*` functions | The Markdown preview's typography — `_docs/inbox/markdown-improvement-proposal.md` §3–§7, read by `ui/viewer/markdown.rs`. Unlike the rest of the table these are ratios over the body font size (`em`/`rem`), not `ui_scale`-scaled pixels, so a font-size change scales the preview without a second accessor |
| `MdMinimapSide` | Which edge the markdown minimap docks to (proposal §12, T-118), `Left` by default — a plain two-value enum, not a size, so it takes no `scaled()` accessor; read by both `ui/plan.rs` and `ui/viewer/mod.rs` off the one `UiSettings::md_minimap_side` |

**Every pixel constant in the table is a `pub const` base with a `scaled()` accessor beside it**,
and a call site reads the accessor: `theme::accent_edge()`, `theme::titlebar_height()`,
`theme::explorer_width()`, `theme::settings_width()`, and so on through the whole list. The const
is the size at `ui_scale = 1.0`; the accessor is that size in the window the user actually has, and
`scaled` rounds to whole pixels (a chrome row a fraction of a pixel tall is a seam in the rule
under it) and never to nothing. Two exceptions: `theme::hairline()` returns 1.0 at every scale,
because a rule that grows blurs rather than reads, and `MODAL_MAX_HEIGHT` is a ratio rather than a
length.

**The dragged-region rule is re-stated, not broken.** Everything from `EXPLORER_WIDTH` down is what
a *fresh* window opens a region at; what the user then drags is remembered per project inside the
arrangement blob. So the **default** scales — the accessor — and the **stored** value does not:
scaling the blob would fight a size the user set by hand, and freezing the default would leave a
300px explorer beside a window drawn 40% larger. `theme.rs`'s test asserts exactly that pair.

`kit::row_height` / `kit::row_indent` follow `theme::ui_scale()` directly, being computed from a
font size rather than a constant.

A change to `ui_scale` or to a trim re-dresses every open emulator — the new `TERMINAL_PADDING` and
point size change the cell grid, and the emulator's own re-measurement fires the resize that tells
the harness — and re-dresses the component library, because its `font_size` is the window's rem
size. Both, and the `SetPreferences` write, are **debounced** behind `AppState::settle_metrics`, on
`schedule_markdown_reflow`'s generation-token device: a slider drag must not send the harness two
hundred resizes. The walk covers every project this window holds, not the showing one, because the
content size belongs to the interface.

The Git screen's own four — `SIDEBAR_WIDTH`, `CHANGES_WIDTH`, `DIFF_HEIGHT` and the graph's
`LANE_PITCH` — are in `state::git` rather than here, on the same reasoning `COLUMN_MIN_WIDTH` is in
`state::agents`: a screen's furniture is the screen's, and only the window's own areas belong in
`theme.rs`.

A region's constant is what a fresh window opens it at; what the drag will not pass is the dock's
own, so a region is one number rather than a triple. The orchestration screen's three are the same
shape for a different reason: its inspector and its drawer are shown and hidden rather than dragged,
and so is the agents screen's sidebar.

Syntax colours are the one thing not tokenised here. They come from the component library's own
highlighter theme, which `theme::set_theme` keeps in step with Ubiq's palette through the `Mode` the
registry row carries, so the editor and the
chat's markdown never sit in a different mode from the chrome. That is the same posture as the
library's buttons and scrollbars: not a literal, and so not an exception to the rule.

Adding a colour means adding a token to its group, giving it a value in **every** palette in the
registry, and using the accessor — unless it is an accent-hued one, which is a derivation in
`with_accent` instead, since a hue written out per palette is the thing the accent axis removed. It
does **not** mean a row in `EDITABLE_TOKENS`: what a theme author may write is grounds and ink, and
a new status or terminal token inherits from the fork like the rest of its group (`D152`).
Adding a group means a role none of the seven covers, which is rare enough to be worth
arguing about in [`decisions.md`](./decisions.md) — `Project` carries `D19`, and `Terminal` is the
selection and link colours a pane's emulator paints.

Every token has a call site, and for one of them the only one is a specimen. The style reference
draws all of them by name — that is what the page is for — but `selected` fills only the row a
list left its cursor on while the keyboard is elsewhere, and
`border_focus` marks the focused text field rather than a pane: the use it was designed for, focus
across split panes, is still designed ahead of the code. That is listed as a gap in
[`../backlog.md`](../backlog.md) rather than quietly resolved by the drawing, because a specimen is
evidence a token has a value, not evidence anything uses it.

The type scale is looked at the same way: `typography()` on the style reference draws every role
across the three families, one column each, so an axis moved in the interface prefs is read off the
page rather than reasoned about.

## Conventions for a screen

- **Focus is shown on the surface's left edge**, through `border_focus`. It is the one signal that
  must be readable at a glance across a window of panes, so nothing else competes for it — but no
  pane carries it yet, because focus across split panes is designed ahead of the code. A text field
  that holds the keyboard keeps that left edge and **adds an underline** on the bottom, so the
  active box is the one that is underlined. That treatment is `kit::field` in
  `crates/ubiq/src/ui/kit/controls.rs`, the container every free-text input sits in — a surface with
  a coloured left edge, joined by a bottom underline in the focus colour while the input holds the
  keyboard. The command field, the project search, the chat composer, the orchestration
  inspector's composer, each agents column's composer, the board's filter and form fields, and the explorer's and the file picker's
  filters all draw themselves with it.
- **Status is shown by colour from the status group**, never by wording alone. A stopped agent and a
  failed one are different colours.
- **Pane chrome stays two rows at most.** Identity and state on the first, context — folder, model,
  remaining context window — on the second. Anything more takes space from the terminal, which is
  the thing the user is actually reading.
- **The terminal body is never styled by Ubiq.** A pane's emulator is given the surface, text and
  cursor tokens plus the terminal group — `pane_bg`, `text`, `accent`, `selection_background`,
  `link_underline`, `link_underline_hover` — so the surface it sits on matches the shell. The
  sixteen ANSI colours are the emulator's own defaults, because those are the colours the harness is
  choosing between, and remapping them changes what the agent said.
  `crates/ubiq/src/ui/terminal.rs` builds that palette in `config()`, and it is the only
  place in the UI that converts a token into anything but a GPUI colour.
- **Spacing comes from the framework's scale**, not from arbitrary pixel values. Sizes that are part
  of the layout — chrome heights, panel widths — are constants in `theme.rs` instead.
- **There are no radii.** See *The shape of a surface* below; a corner radius anywhere is a defect
  in the same way a literal colour is.
- **An empty page is the brand, not furniture.** `crates/ubiq/src/ui/mark.rs` owns it for every
  screen that has one: `alone()` is the mark at 200px and half opacity on the window's ground, which
  is what the centre panel shows with nothing open, and `backdrop()` paints the same mark at 0.16
  under a page's own words. The ring is an `svg()` tinted with `theme::mark`, one file for both
  palettes, and the cubes are a full-colour `img()` per palette — a consequence of *The icon set*'s
  alpha mask, which carries one colour, where each cube's three faces are three shades.

## The shape of a surface

**Nothing is rounded, and the left edge does the identifying.** Ubiq's surfaces are square; a
coloured border on the left is what says what a surface is — accent for the thing the user is
acting in, the status colour for something being reported, the project colour for the window
itself. `ACCENT_EDGE` in `theme.rs` is its width, and `ui::kit::slab` is the shape.

**The widgets Ubiq does not draw are square too.** `theme::dress_component_library` writes
`Theme::radius` and `Theme::radius_lg` as zero, which is the one number every corner the component
library rounds comes from — including the ones it would otherwise keep round whatever the theme
said, since its `radius_full()` answers zero rather than a pill when the base radius is. A slider
thumb, an avatar and a badge dot square off with everything else because of it.

This replaces the more usual "box with a border all the way round". A GPUI element has one
`border_color` for all four sides, so a grey box with one coloured edge is two elements; one edge
and no box is one, and it reads more clearly at the sizes this UI uses.

**The edge collapses onto its container's edge.** A coloured border only reads as identifying the
surface if it sits *on* the boundary; floating it a few pixels inside makes it decoration. So a
container gives a surface with a coloured edge **no left, top or bottom inset** — no margin, no
padding — and the edge runs the full height of what it marks. Right padding is the one judgement
call: keep it where the content needs breathing room from the next panel, drop it where the surface
should span.

In practice that means the containers do the yielding: the chat's transcript pads only on the right
so each turn's edge lands on the panel border, the composer has no margin at all, the terminal card
fills its half of the dock, and the explorer's tree pads only on the right so a selected row's
accent runs to the panel edge. Inline controls — chips, pills, tabs — are not surfaces in this
sense and keep their own spacing.

**Chrome is flush, and its controls are square.** A control that belongs to a chrome row — the
titlebar's switches, a panel's filter field, the mark above the rail — takes the row's full height
and no margin of its own: it touches the row's top and bottom borders, and the first and last touch
its ends. Side by side they are separated by a hairline rather than a gap, and where a group needs
telling from the next one it is a 1px rule, not whitespace. An icon button keeps its 30px width, so
what fills the height reads as a square in a short row — which is why `TITLEBAR_HEIGHT` is short
enough for that to be true. Content pads; chrome does not. The titlebar's back and forward controls
are that rule plus the one thing `icon_button` has no room for — a control with nowhere to go: they
keep the 30px square and the full row height, and at the history's end they draw in `text_faint()`
and answer neither pointer nor click, which is why they have their own helper in `ui/titlebar.rs`
and a 1px rule rather than a gap between them and the `⌘K` field.

Circles survive in exactly one place: state dots, which are dots.

**A modal is that same surface, over the window.** One question at a time, drawn by `kit::modal`:
`MODAL_WIDTH` wide because a modal is one question rather than a panel, at most `MODAL_MAX_HEIGHT` of
the window's height with its body scrolling inside, square, filled with `surface_raised`, and
identified by the coloured left edge — `accent` for a question, `danger` for something that will not
come back. `scrim` is what it lays over the window.

Three rules come with it, and none of them is the caller's to re-decide. **It is painted where it is
asked for**, through `deferred` and `anchored` at the window's origin, so a modal raised inside a
dock panel covers the window instead of being clipped to the panel — which is what lets a screen own
its own modal rather than the shell keeping a layer for one screen's sake. **It is dismissed by an
outside click and by its own close**, through `on_mouse_down_out` on the panel, exactly as the kit's
dropdown is, so the two behave the same way and neither uses the scrim as a click target. And **the
scrim occludes the mouse**, so nothing behind a modal can be clicked while it is up. It sits above
the dropdowns in `deferred` priority, because a modal a menu could cover is not modal.

**A dropdown has no scrim, so it consumes its own click instead.** `menu_panel` and `context_panel`
in `ui/kit/menu.rs` stop a left mouse-down on the list itself, because a list painted at `deferred`
priority over whatever raised it sits at the same screen point as a control underneath — without
the stop, a click on a row would also land on that control.

**A layer painted above a modal is outside it**, so an outside click belongs to the topmost layer
and to nothing under it. `on_mouse_down_out` is a capture-phase handler over the panel's own
bounds, and a layer painted above — a dropdown opened with `Picker::above_modal`, a question raised
over a page, a picker dialog raised over that question — is tested against those bounds all the
same. A click in it reads as a click outside every panel below, and no `stop_propagation` from
above can take that back, because capture runs back to front. Left alone, one click peels the whole
stack.

**So a dismissal consults the order, rather than naming the layers it happens to know about.**
`state/overlay.rs`'s `Layer` is that order — `ui::shell`'s paint order, declared bottom-up, with a
rung for every overlay the window root paints and a top rung for any dropdown a form keeps open on
its own state. `AppState::top_layer` reads the state each rung is raised from, `AppState::covered`
answers "is anything above me", and an overlay something can be painted over hands its dismissal
through `ui::dismiss(&view, rung, …)` instead of `ui::handler`, which yields while it is covered.
One gesture peels one layer, exactly as Escape does. **Adding an overlay is a rung there and an arm
in `top_layer`, and nothing else** — every layer below it is protected without being edited, which
is what the hand-written guard per pair could never give (`D141`). `crates/ubiq/tests/dismiss.rs`
asserts it.

**Escape is the window's, not the modal's.** A `kit::overlay` modal is a function returning an
element: it holds no focus, so a key never arrives at it, and a `key_context` per modal would be one
more answer to a question the window answers. The key is bound once — `Workbench` and `Input`,
by the late-registration device below — and `AppState::cancel_dialog` in `app/shell.rs` reads the
paint order from the top to decide which layer it takes. **It peels one layer**: a dropdown open over
a modal closes and the modal stays, because dropping a half-filled form because a menu was down loses
everything typed into it. With nothing up it calls `cx.propagate()`, so a bare Escape is still the
explorer's, the terminal's and the search panel's; a surface that binds Escape at its own depth — the
file picker, the navigator, the explorer's filter — still wins, because a deeper binding fires first.
A modal raised in `ui::shell` without a rung in that list is a modal Escape walks past, which is what
`crates/ubiq/tests/dismiss.rs` asserts.

**A modal's body is not always static.** The harness login modal embeds a live `TerminalView` in
its running step — a pane with a real byte stream, resized and focused exactly as a dock pane is.
A terminal measures its own bounds to decide the geometry it reports to the harness, and a box
inside a scrolling column never resolves a height to measure — fine for a line-based `claude auth
login`, but a full-screen TUI (`opencode auth login`, bare `grok`) redraws garbage for whatever
size it is told it has. So the running step alone draws through **`kit::modal_sized`**, at
`LOGIN_MODAL_WIDTH`/`LOGIN_MODAL_HEIGHT` with `fill_height: Some(..)`: the body becomes a
fixed-size flex column instead of a scroller, so the terminal's `flex_1`/`min_h(0)` — the same
pattern `ui/terminal.rs::pane` uses — resolves against a real height rather than a hugged one.
Every other step keeps `kit::modal`'s ordinary `MODAL_WIDTH` and scrolling body; a modal that
hosts a pane still follows every rule above, only what fills the body changes.

Two primitives are built on top of `kit::modal` rather than beside it, in `ui/kit/overlay.rs`, so a
screen never hand-rolls a confirm or a "type a name" dialog again. **`confirm_modal`** is a question
with two answers — `danger: true` draws the `danger` edge for something irreversible, `false` draws
`accent` for an ordinary question — with a Cancel and a labelled confirm underneath, in the ghost
and primary button shapes every other footer in the window uses. **`prompt_modal`** is one labelled
field and a confirm: the caller owns the field's `InputState`, so what was typed survives a redraw
and the caller reads it back when the confirm fires, and `confirm_enabled: false` dims the confirm
button the same `.opacity(0.5)` way a disabled action dims everywhere else — there is no second
disabled style. Both close over `kit::modal`'s scrim, edge and dismiss rules rather than repeating
them; a screen that needs a bespoke body still reaches for `kit::modal` directly, as the harness
login does.

**A dialog is that same modal, worked in rather than answered.** The file picker — `ui/file_picker.rs`,
raised over any screen — keeps every rule the modal keeps and differs in the three ways a dialog with
work in it has to: it opens at `DEFAULT_WIDTH` by `DEFAULT_HEIGHT` and is **resized from a corner
grip**, never below `MIN_WIDTH` by `MIN_HEIGHT` and never past what the window can hold; the drag is
tracked on the full-window layer rather than on the panel, so a pointer that outruns the corner does
not strand it; and **whether an outside click dismisses it is the caller's**, because a dialog that
holds the window until it is answered and one that goes away the moment attention leaves it are two
different asks. The four sizes live beside the state, in `state/file_picker.rs`, because they are
what a resize is clamped against rather than what a screen is laid out on.

**A page overlay is that same dialog, with a nav, and it does not resize.** Application settings
and project settings are this shape: `SETTINGS_WIDTH` by `SETTINGS_HEIGHT` (project settings is
the same width), clamped to the viewport, body scrolling inside, switching nav sections must not
change the panel's size. They keep the modal's scrim, coloured left edge, outside-click dismiss and
`deferred` priority, and they are painted from the shell over the window rather than from
`kit::modal`. The furniture — `heading`, `setting_row`, `hint_row`, `label_hint`, `nav_item` — lives in `ui/kit/settings.rs`
so the kitchen sink draws the same rows.

**A list that hangs off a control is not a modal, and does not get the modal's device.** The ⌘K
navigator — `ui/navigator.rs` — is drawn the way the kit's dropdown in `ui/kit/menu.rs` is: an
`anchored()` with **no `.position()`**, made a child of the element it belongs to, so it is placed
against that element rather than against the window's origin. Offset by `TITLEBAR_HEIGHT` to fall
under the titlebar's field, snapped into the window with an 8px margin, at `deferred` priority 2 —
above the dropdowns at 1, because it is raised over the whole chrome. No scrim and no outside-click
dismiss: the list belongs to a field the user is still typing in. Its key context and every handler
go on the **field**, not on the panel, because the keyboard is in the input inside the field and a
deferred panel is nowhere on the focus path — which is why each of its keys is bound twice, for
`Navigator` and for `Navigator > Input`, by the rule below.

**A heading-and-thread navigator is a kit primitive because a second markdown-with-threads surface
would want the same shape, not the same meaning.** `kit::md_navigator` (`ui/kit/md_navigator.rs`)
draws the same anchored-list device as `kit::menu`'s dropdown and the ⌘K navigator above it — no
scrim, dismissed by an outside click this time, because unlike the ⌘K field this trigger is not
something the user is mid-keystroke in. It does not go through `kit::Picker`: a row needs an indent
by heading depth and two independent counts — open threads, settled ones — that `Picker`'s
plain-label rows have no place for, so it owns its row instead of forcing one shape to answer two
questions. `crate::ui::plan` is the first caller, over `state::document::heading_sections`'s pure
data; a second annotated document reuses the component, not a copy of it.

**A minimap is geometry and colour, nothing else — no plan-specific type crosses into it.**
`kit::minimap` (`ui/kit/minimap.rs`) is a fixed-width strip that fills whatever height its parent
gives it, over three plain-data shapes: a `MinimapMark` (a block's own shape — a heading's bar, a
paragraph's or a table's line at its own length, a code block's or an image's rectangle — `top`,
`height` and `length` all `0.0`–`1.0` fractions of the strip's own box, a `dotted` flag for the
table-row shape, and an `Rgba` the caller resolved from a token), a `MinimapTick` (a small coloured
mark on the strip's outer edge — a thread's open/resolved state today), and an optional
`MinimapViewport` (the translucent, draggable rectangle standing for the visible region). The
primitive names no thread, no plan and no block either way — T-110's rework of the first cut (one
full-width tick per thread and nothing else) added the shapes and the viewport without teaching the
kit what a block kind is.

The plan editor's own strip is built from two pure functions in `state::document`:
`minimap_rows` for the shapes — a heading or a code block draws one row, a paragraph or a table
draws one per real source line, `length` measured in characters against a fixed column-width
constant rather than real glyph width, because there is no second layout pass to measure by and the
markdown renderer exposes no per-line fragment geometry to place one against instead — and
`thread_marks` for the ticks, the same pure-data shape as `heading_sections`, so either can be
tested and resolved back to what it stands for without a `Window` in reach. `ui/document.rs` turns
a row's `block_index` into a real span down the strip from `ScrollHandle::bounds_for_item` once the
preview has a painted frame to measure, spreading a block's several rows evenly across that span
(no per-line pixel position exists either), and falls back to spreading blocks evenly for the frame
a document opens in, before there is one. A short document draws at a real, 1:1 scale rather than
being stretched to fill the strip. `on_scrub` is the strip's one interaction callback beyond a
tick's own `on_select`: a click or a drag anywhere hands back the fraction the pointer landed at,
and the plan editor answers by scrolling the preview so that point lands roughly centred — one
formula for a click and a drag alike, recomputed from scratch each time rather than tracked from a
drag anchor.

The standard viewer's own strip (T-118, `ui/viewer/mod.rs`'s `markdown_preview`) is the primitive's
second caller, and stays the simpler shape T-110 did not touch: one `TextView` rather than a block
per heading means there is no per-heading layout to measure at all, so `markdown::heading_marks`
positions each mark by its heading's byte offset over the document's length, drawn as a full-width
tick (no ticks, no viewport, no `on_scrub` wired up — it answers with nothing). Both callers answer
to the one `UiSettings` pair — `md_minimap` (generalised from the plan-only `plan_minimap`) and
`md_minimap_side` — set from the Markdown header's own popover, `ui/viewer/md_options.rs`,
`kit::popover` over `kit::choice_pill` rows and a `kit::check_box`, the same width preset and
density pills T-116 first drew loose.

**Two marks say a file has bookmarks, and neither is a new colour.** A bookmarked line is a
`TextDecoration` over the line's byte range with `accent_soft()` behind it, set through the open
file's `EditorState`: the component library's only public decoration surface is that collection over
byte ranges, so the mark sits on the line rather than in the gutter. A file's tab wears a small mono
count chip in `accent()` on `accent_soft()` while those lines are off screen; the explorer's
bookmarks section is a `kit::disclosure` over plain rows; and a bookmark that has lost its line
draws in `warning()`. **No token was added for any of this** — `accent_soft()` is the fill behind a
selected row and `warning()` is what a thing that went wrong but still works is drawn in.

**A pinned tab's mark is an icon, not a token — the one case that needed a new one.**
`assets/icons/tab-pin.svg` sits before the label on any pinned tab, file, terminal or chat alike,
tinted `theme::accent()` through the same `Window::paint_svg` path every icon takes: the colour is
what accent means on every other surface, so the addition is a shape, not a shade.

**The outline panel parses the buffer a second time.** The component library's `SyntaxHighlighter`
holds a tree-sitter tree, with a public `tree()` accessor, but the instance sits inside a
private `TreeSitterInputHighlighter` behind `Box<dyn InputHighlighter>` with no downcast, and
`EditorState::highlighter()` is `pub(super)` — unreachable from `ui/outline.rs`. So the panel parses
the same bytes again, with its own grammar set, on `cx.background_spawn` behind a 200ms debounce.
`G153` names the gap; it closes the day `InputHighlighter` grows a tree accessor.

**A dialog is worked from the keyboard, and a binding against a field has to be registered late.**
The component library's input binds `up`, `down`, `left`, `right`, `enter` and `escape` for itself, in
the `Input` context — the deepest node in the tree, and depth is what breaks a keymap's ties. A screen
that wants those keys while the focus is in a field binds each of them twice: once for its own
context, and once for `ItsContext > Input`, which matches at the same depth as the library's and wins
by being registered afterwards. `app::install_key_bindings` is called after `gpui_component::init` for
exactly that reason. A handler that turns out to have no answer calls `cx.propagate()`, and the field
gets its key back — which is how the picker's `left` and `right` are caret keys again in a flat list.

**A filter field and the tree under it are two focuses, and that is the default for every tree and
collapsible list in Ubiq.** A key means what the focus it is aimed at says it means: the field keeps
every key a field keeps — Backspace first of all, which must never reach a row — so the list's own
keys are bound at the panel and are live only while the list holds the keyboard. Three keys cross
the boundary, because a field with a list under it is one control to the hand: `down` and `tab` step
from the field onto the list, `escape` clears the query from either, and `enter` opens what the
query landed on without leaving the field. `tab` and `shift-tab` step back off the list onto the
field. Clicking a row puts the keyboard on the list.

**A row is one line, and a value that does not fit is elided.** `kit::elided` truncates with the
system ellipsis and carries the whole string as its tooltip, which is why it takes an element id;
it takes the size as `Pixels`, so a call site hands it a `theme::font` rather than a number.
`kit::elided_with` is the same control with the hover said separately, for a row that has more to
add than the string it is cutting — the agents sidebar's, whose hover is what the conversation is
about and falls back to the name in full when nothing has said. A
name, a path or a title that wrapped instead would push everything under it down, and a column of
rows is scanned by its left edge — so nothing in a row, a footer or a card header is allowed a second
line.

**A file row is sized from its text.** `kit::file_row` derives its height (`kit::row_height`) and
the tree indent (`kit::row_indent`) from the size it draws at, over `theme::ui_scale()` — so the
content zoom changes how tight the tree is. A surface the content family does not reach — the file
picker, the ref list — passes `kit::row_font()`, which is the chrome family's `Body`: a dialog's
rows are furniture, and the content zoom is not theirs to follow.

## How a screen is put together

**`gpui-component` first.** Its `Icon`, `Kbd`, `Badge`, `Editor`, `Textarea`, `Scrollbar`, markdown
view and dock are used directly — the dock being the largest widget in the library and the whole of
the window's arrangement, `D42`. `crates/ubiq/src/ui/kit/` holds only what the library does not give
us — the slab every surface is drawn in and the card that is a slab you can pick, the field every
text entry sits in, the state dot, the pill, the state chip, the removable tag whose `×` drops it
and whose label does something when clicked, the toggle pill for an independent facet and the choice
pill for one value of a set, the tick box a row is chosen with where several may be, the elided run
that says the whole of itself on hover, the filled button a screen's single obvious action is drawn
as, the stepper, the flat meter, the slider, the disclosure bar, the section label, the panel header, the shared
tab strip, the progress ring, the colour picker, the painted layers in `canvas.rs`, the file-list chrome the picker and
the explorer share in `files.rs`, and the one dropdown mechanism every menu in the window uses —
plus the context menu a right-click, or a control that has no room for a trigger, raises at the
pointer: that same panel, opened at a point rather than under a chip. A diagonal ribbon — a word
across one corner of its parent — is `kit::ribbon`, because GPUI rotates pictures and not boxes. Its rows are labels, a
disabled label, or a separator — a hairline that still takes an index, because a menu's rows and the
actions behind them are matched by position. A row may also carry `ContextItem::tooltip(...)`, which
`context_panel` draws on hover: the place for a fact too long for a label and too specific to guess
at, such as the file the conversation menu's dump row is writing. A tooltip never carries the row's
meaning — the label says what the row does, and a row whose label needs the hover to be understood
is a label that wants rewriting. `kit::Picker` may carry a filter field of its own,
through `.search(&state, focused)` — one `Entity<InputState>` drawn at the top of the panel, in the
same `field(...)` shape `project_menu.rs`'s hand-rolled search uses. The picker never filters: the
caller narrows `items` and keeps a parallel values list in lockstep before building the picker, so
`on_pick(index)` stays correct by construction, and an empty result after filtering draws one muted
"No matches" row rather than a panel with nothing in it. `.disabled(indices)` marks rows drawn but
not pickable — a conversation attached to another chat tab, say — in the same faint,
click-less style the context menu's own disabled label uses; a disabled row is never dropped from
`items`, because a row that vanishes reads as gone rather than taken, and a picker's own selected
row stays pickable even if the caller also passed its index to `.disabled(...)`.
`.separators(indices)` draws the same hairline the context menu's own separator does at those rows
instead of text, so a searchable picker can carry group headings — themselves plain `.disabled(...)`
rows — in the one `items` list a caller builds and a pick indexes into, the way the agents screen's
column `+` groups the bench from what is on screen elsewhere. `.tooltip(text)` is what a
picker drawn with no label says on hover — the chat header's chevron is one, and with no words on
the trigger the hover is the only place the question it asks can go. `.dots(colours)` gives a
single-select `Picker` the same per-row status colour `MultiPicker` always drew, `None` at an index
leaving that row bare for a choice like "All tasks" that stands for no one colour; `.dim(indices)`
mutes a row without disabling it — pickable, drawn faint, for a choice that still answers but is not
where the reader's attention belongs, such as a completed mission in the board toolbar's mission
filter.

**A picker has three shapes, and the third is for a form.** `PickerStyle::Plain` is the bare
trigger, `Chip` the small filled one a composer's config controls wear, and `Field` the shape
`kit::field` gives a text input: a filled box on its own coloured left edge, taking the width it is
given, the value truncating rather than pushing the chevron off the end. A column of pickers in a
form reads as a column that way rather than as a ragged edge, and a picker among text inputs reads
as something to click rather than as a line of text.

**A question whose answer is a set is `kit::MultiPicker`, not a second picker.** It is the same
trigger, the same panel, the same search field and the same `MenuId`, built from the parts
`kit::Picker` is built from rather than a copy of them: what differs is that a row **toggles** and
the list stays down until it is dismissed, and that the closed trigger says every value that is
ticked — comma-separated in the list's own order, elided through `kit::elided` so the truncation
and the hover are one string. The selection goes *in* as well as out (`.selected(indices)`), which
is what lets one control be a filter preselected with what is narrowing the screen and a form
preselected with what a record holds. Row order is `kit::multi_order`: with the search field empty
the ticked rows are drawn first, under a query nothing is pinned — a ticked row lifted above better
matches reads as a match it is not — and a list with no search field is never reordered at all.
`tech/components.md` carries the rest, including the states filter that is its first use.

**The colour picker is the kit's, and it is the one place the no-literal-colour rule bends.**
`kit::colour_picker` in `ui/kit/colour.rs` is a 16×10 saturation/value plane over a painted wash, a
24-step hue strip, a preview block and the caller's `#RRGGBB` field. It is told a hue, a saturation
and a value and it reports the three back — what they colour is the caller's business, which is how
project settings and the theme editor share one control. The swatches it generates are *content*,
the same way a scene's stroke and the sixteen ANSI colours are: they are the thing being picked.
Its chrome is not — the cursor box, the preview's border and the hex field are tokens, and the
chrome's sizes go through `theme::scaled()`. The HSV↔RGB maths sits in `theme.rs` beside `rgba_of`,
because `state/` and `ui/kit/` may both name that file and may not name each other.
`gpui-component`'s own `ColorPicker` stays where it is, on the image editor's stroke: a popover of
featured swatches is a different control from a full HSV surface.

**The slider is the library's, skinned.** `kit::Slider` in `ui/kit/slider.rs` wraps
`gpui_component::slider` rather than drawing a track of its own: the drag, the pointer capture, the
keyboard and the accessibility role come with it, and none of them is worth rewriting for a
palette. What the kit adds is the palette — the track's fill through the library's `background` and
the thumb through its `text`, which is how a widget we do not draw gets Ubiq's tokens — a leading
and a trailing icon slot, and a mandatory tooltip, taken by `Slider::new` because a slider carries
no number and no unit and is therefore always an unlabelled control. The icons are the scale:
`size-interface-small` at one end and `size-interface-large` at the other says what the axis means
more directly than a label would, and they draw at `theme::icon_sm()` like every other inline glyph.

`kit::slider_state(min, max, stops, value)` is the other half, and the reason the control exists in
this shape. It quantises the axis to a ladder — nine or eleven stops across the range — so a drag
lands on a value the user can return to rather than on 1.0374. The library rounds to multiples of
the step measured from zero rather than from `min`, so a range whose `min` is not itself a multiple
reaches its ends by the clamp; choose the three numbers so the step divides them. The
`Entity<SliderState>` lives on `AppState` with its subscription on `_subscriptions`, which is the
stated exception to the no-component-library-type rule: a slider's position *is* its model, and
there is no second copy of it to keep in `state/`. `SliderEvent::Change` is what a caller listens
to — `Release` only answers when the pointer is let go, which is a control that lags. The two
sliders that carry the size axes are `AppState::ui_scale_slider` and `text_ratio_slider`, and
`ui/size.rs` is the one place both the status bar's size popover and the Size settings section
build their controls from, so the two surfaces never drift from each other.
`AppState::sync_size_sliders` puts the sliders back on the stored axes whenever a surface that
draws them opens — the popover's trigger, the settings nav, a preset pick, a reset — because the
metrics arrive from the host after the window is built and the message path carries no
`&mut Window` to push them in directly.

**A dense form's notes live on a hint mark, not under the row.** `kit::label_hint(id, label, hint)`
draws the label with an `Info` mark beside it and the words on the mark's hover, and
`kit::hint_row(id, label, hint, control)` is `setting_row`'s shape built from one: label and mark
left, control right, one line high, with no rule between rows — a hairline under every one of eight
rows reads as eight sections. `setting_row` and `label_block` spend a whole line on the note, which
is what turns a form of eight questions into a form that scrolls; the words are worth having and
worth reading once. Both take an id, because a tooltip needs a stateful element to hang off — the
same bargain `kit::elided` makes.

The state dot itself is what a conversation's lifecycle reading is drawn as — no primitive of its
own. `state::status::conversation_status` derives a `Status` from the conversation's own fields —
**a pair**: a `Lifecycle` (Starting, Ready, Idle, Working, Waiting, Unloaded, Ended) and a `Doing`
(Queued, Thinking, Writing, Tools, NeedsYou, Done, Failed, Unknown) — and `lifecycle_dot` puts
`lifecycle_colour`'s answer on a `status_dot`, with `Status::label`'s word in a tooltip, one or two
of them, never a sentence. **One vocabulary for agents and delegates alike**, which is why it sits
in `state::status` rather than in a UI module; `ui::work` is the only place it becomes a colour or
a glyph, and `theme.rs` the only place a colour has a value.

**Where a mark has room for both halves it is a hexagon, and that is `kit::hex_mark(id, border,
fill, side, pulse)`.** The outer hexagon is a *stroke only* — it has no fill of its own, so
whatever it sits on shows through and the mark cannot become a second background for the block it
is on — and it takes the lifecycle's colour. A smaller filled hexagon inside it takes the
activity's or the result's. The two readings are then independent: a lifecycle transition changes
the border without destroying the activity reading, and a result changes the fill without claiming
the execution is still going. `fill` is `None` where nothing reports an activity, which draws the
outline alone rather than a guessed colour. It is flat-topped so it sits beside a line of text
without pushing the row taller, and the hexagon is deliberately the only non-rectilinear silhouette
in the window: this window draws no radii, so a status mark has no rounded badge to be told apart
by and gets a shape instead.

**The outline and the core are two layered elements, not one draw.** `pulse` asks only the core for
the same slow, shallow fade `lifecycle_dot` gives a tab's dot, and animating the whole mark would
fade the outline's lifecycle reading along with the core's — so the core is a second, absolutely
positioned canvas over the first, and only it carries the animation. `id` names that animation, so
two marks on the same screen never share a clock. `ui::teams::status::status_mark(status, side,
id)` is the shared caller: the outer hexagon reads `status.lifecycle`'s colour, the core reads
`status.doing` — grey rather than a colour once `Doing::Done`, so a delegate that finished clean
does not keep reading as still going — and the core pulses only while `status.lifecycle` is
`Working`, the one reading a moving core would not cry wolf over. It draws on the Teams agent and
delegate cards, the agents column's own tab strip and the dock's chat tab (T-99, T-102) — one mark,
the same primitive, everywhere a conversation's state is shown as more than a dot.

**Grey-for-done does not travel past the mark on its own — a card's edge and its chip take it
through a second function, `ui::teams::status::card_colour(status)` (T-106).** It answers
`text_faint` for `Doing::Done` and `status_colour(status)` otherwise, and both a Teams agent card's
own edge and `status_chip` read it rather than `status_colour` directly — one function so the mark,
the edge and the chip cannot disagree about what "done" looks like on the one surface that draws all
three. This is a Teams-only override: `ui::work::doing_colour`, which every non-Teams reader of the
`Doing` dictionary still calls (the tasks board's columns, `[Teams]`'s own cards), keeps mapping
`Done` to `success` green — a delegate that finished is spent, not merely a passing check, but a
completed *task* elsewhere in the window still reads as one.

**A state dot has four readings and only four: `warning` wants you, `info` is working, `success` is
idle, `text_faint` has stopped.** What a dot read at a glance across a window full of columns has
to answer is whether that conversation wants the reader, and four colours is as many as the glance
holds — so every working turn is one `info` rather than a palette per activity, which kind of work
being the `Doing` half's question and answered beside the dot rather than inside it. Every value is
a status token the window gives that meaning elsewhere, so a dot invents no colour.
`ui::work::doing_colour` is the activity's own four — `info` moving, `warning` blocked, `success`
returned, `danger` failed, `text_faint` queued or unreported — drawn from the same token set.

**Two of the four move, and that is the fifth fact about the dot: `Waiting` and `Working` pulse, the
other two are still.** `lifecycle_pulses` is the rule — those two are the readings something is
expected to happen in, and stillness is the wrong thing to draw for them — and the fade is slow and
shallow, opacity 0.45 to 1.0 over two seconds, the same construction the transcript's `writing_mark`
is built from. A dot on a tab strip nobody is looking at is a hint at the edge of vision, not an
alarm; the turn itself is watched at the tail. `lifecycle_pulses` answers `false` for a reader who
asked the system for reduced motion (`App::reduce_motion`), because motion used as a signal is
exactly the motion that setting is about. Carrying the animation is why `kit::status_dot` returns a
`Div` rather than an opaque element.

**The plain dot survives for the readings that are not a lifecycle/activity pair.** A terminal's
running state and a file's dirty mark are one fact each, not two, so the dock's tab strip still
draws `ui::conversation::lifecycle_dot` — colour, pulse, the ring it sits on and an element id for
the animation — for `TabInfo::dot_colour`/`dot_pulse` on every tab kind but a chat one. A chat
tab's own conversation state is `TabInfo::dot_status` instead (T-99, T-102): where it is `Some`,
the skin draws `status_mark` in the dot's place, and the plain-dot arm never runs. A tab with
neither has no mark: there is no state to report.

**One first line, shared by every surface that hosts a conversation.**
`ui::conversation::lifecycle_header(app, conversation, view, switch, cx)` is `ConversationView`'s
`header: bool` row (T-102) — the three-dots menu at the left, an optional `switch` element beside
it, and `ui::teams::status::status_chip(status, zoom)` flush against the strip's own right edge
with no margin, chrome drawing no padding of its own. The agents column calls it with `switch:
None`, since a column's tabs come from its own bench picker rather than a free list a chevron could
attach from; the chat tab passes its own change-agent chevron as `switch`, and draws the same row
alone — chevron only, no menu, no chip — when nothing is attached, since there is no conversation
to read either off. One function, one row shape, on both surfaces, rather than each assembling its
own fragments.

**Neither a state mark nor the persistence mark sit on this row.** The state reading lives entirely
on the hexagon every tab wears: `lifecycle_mark`, the fragment the chat toolbar once drew beside
the menu, and `state_chip`, the agents column's own duplicate beside the name, are both gone from
their call sites. `persistence_mark` lost its callers the same way: `persistent` still lives on the
`WorkAgent` record, but nothing draws it until a card asks for one (`backlog.md`, `G334`).

**A row that gathers several controls this way drops their labels for tooltips, not for a second
icon set.** The chat panel's toolbar is icon-only: the lifecycle menu and the change-agent chevron
keep their icon and lose `ghost_button`'s inline label, the label reappearing as the same hover
tooltip every other icon-only control in the window uses — the titlebar's `new-project` cluster,
the agents column tab's `×` (`Put on the bench`), the chevron's own `change agent`. Two controls
that both add something must still read as different actions at a glance, so a row is never given
the same icon twice with only the tooltip to tell them apart, and the dock's two `+` controls — a
terminal pane's and a chat view's — keep their own icons for exactly that reason.

**No action or icon-only control is unlabelled without also being unexplained.** A button that
draws no text — an icon-only `ghost_button`, a bare rail or dock icon, a chevron with nothing beside
it — carries `.tooltip(...)` naming what it does; a value elided for space carries the full string
the same way, through `kit::elided`. A control's meaning lives in a word somewhere it can be read,
never in the icon alone. `G294` names where the tree still falls short of this — the titlebar's
region toggles and a handful of others draw no tooltip yet.

**Some surfaces are painted, not laid out.** Flexbox and `gpui-component` cover almost everything;
what is left is geometry a box model cannot express — a dotted ground, a cubic connector between two
points, a dashed outline, a trail of grains, a ring at a percentage. Those go through GPUI's
`canvas` element, and the reusable ones are `crates/ubiq/src/ui/kit/canvas.rs`: each is one layer
that fills its parent absolutely, takes no click, and knows nothing about what it is drawing, so a
caller stacks them in the order they should read. The canvas element itself is sized to fill that
layer; a canvas that only laid out to its content would paint into a strip at the top of the pane. `progress_ring` in `controls.rs` is the same
device inline; it is one line over `progress_ring_in`, which takes the fill as an argument, so a
second ring on a surface can carry its own token rather than a second accent. `progress_ring_pair`
draws two concentric bands over the same painter, outer first and each thinner than a single ring's,
for the one glyph — the footer's quota ring — that has to carry two readings of the same kind at
once.

**A graph is a board of blocks, and one module draws every one of them.**
`crates/ubiq/src/ui/kit/blocks.rs` is the layer above `canvas.rs`: the stack those painted layers go
into, in the one order a graph reads in — ground, the fences on it, the links between what they hold,
the blocks, and whatever goes over the lot. A caller adds things as it measures them and gets that
order anyway, so two screens cannot draw the same graph with the z-order a notch apart. `Board` also
accumulates the extent as it goes, which is what makes the canvas as big as what is on it without
anyone measuring the geometry twice; `block` and `handle` answer the positioned, styled element and
the caller hangs its own `on_drag` on it, which keeps the carried type the screen's rather than
something the kit has to name. It takes plain rectangles at 100% zoom and scales all of them by the
one zoom, so no call site multiplies a coordinate itself. The Teams canvas
(`crates/ubiq/src/ui/teams/graph.rs`) and the kitchen sink's teamsim testbed
(`crates/ubiq/src/ui/sink/teamsim.rs`) both draw through it. What it does *not* know is where
anything goes: that is `state/layout.rs`'s.

**The kit knows nothing about the workbench.** Its interactive helpers take a plain
`Fn(&mut Window, &mut App)`, and call sites bridge to the root view with `ui::handler` and
`ui::indexed`, or with `cx.listener` where the signature fits directly. A kit function that names
`AppState` has stopped being a primitive.

**Screen areas are free functions, not views.** One module per area under `ui/`, each a
`fn(&AppState, &mut Context<AppState>) -> impl IntoElement`. One place owns state and one place
requests redraws. A helper that takes `cx` and is called in a loop returns `AnyElement`, because
Rust 2024's capture rules make `impl IntoElement` borrow the context.

That signature is what makes a screen area a **panel** for nothing: the dock's adapter calls it
inside `app.update(...)` from its own render, which is sound because a child view's render runs in
the layout pass, after the parent's has returned. The one thing a panel may not do is read
`AppState` outside a render — the dock asks a panel whether it is visible while the window is
mid-update, and the window pushes that answer to the panel rather than the panel reading it back.

**The dock is the component library's; the skin is Ubiq's.** `crates/ubiq/src/ui/dock/skin.rs`
implements the library's three renderer traits and draws every pixel of a group: the tab strip at
the same height as `kit::tab_strip`, the displayed tab marked on its bottom edge, a dot per panel,
a close only where the panel offers one, the drop indicator, and the regions' resize strips. The
tokens, the square surfaces and the coloured left edge therefore hold inside a group exactly as
outside one, and Ubiq writes no drag, no drop geometry and no layout serialisation.

**Exactly one menu is open at a time**, tracked as a single `Option<MenuId>` on the workbench state.
A trigger *opens* rather than toggles, so the open panel's outside-click dismissal cannot race the
click that was meant to close it.

**A filling pane needs `flex_1` and `min_h(px(0.))`** — or `min_w` — together. One without the other
is the standard way a GPUI flex child refuses to shrink.

**Scrolling needs an `.id(...)` and a tracked handle.** `.overflow_y_scroll()` does nothing without
an id. A scrollbar is a sibling of the scroll area, absolutely positioned over it, so it stays put
while the content moves under it — not a child, which would scroll with the content.

**A control that floats over a scroll area is a sibling too**, in a `relative` wrapper around it,
for the same reason and one more: nothing in the scrolled content moves when the control appears,
so the line under it stays where the reader put it. The transcript's `Go to last message` overlay
is the pattern.

**A list whose length is data, not layout, is virtualized — and there is no floor.** `gpui::uniform_list`
where every row is one height, the `ui/logs.rs` precedent; `gpui_component::v_virtual_list` where
they are not, which takes an `Rc<Vec<Size<Pixels>>>` of per-row heights and an `Entity<V>` and
builds only the range it can see. Both are in the tree, and hand-rolling a third — building every
child and standing the off-screen ones in an empty box of their last painted height — costs O(n) a
frame below whatever floor the bookkeeping needs, which is the length most lists actually are.

**`gpui::list` is the third, for rows that are variable-height and whose heights nothing upstream
has measured yet** — unlike `v_virtual_list`, it takes no `Rc<Vec<Size<Pixels>>>` up front: its own
`ListState` lays each row out once, caches what it measured, and answers from that cache on every
render after. One `ListState` per list, made the first time it draws and kept — a `RefCell` beside
the state it belongs to when that state is drawn from a `&self` rather than a `&mut self`, since
rebuilding it on every frame would throw the cache away — and `reset()` only when the row count
itself changes, never on every render. The tasks board's lane is the pattern (`T-108`,
`ui::board::mod::column`, `state::board::BoardState::lane_list`): a card is variable-height by
design, so `uniform_list` is the wrong shape for it, and nothing upstream of a lane knows every
card's height the way `v_virtual_list`'s caller would have to.

**A uniform row cannot grow, so its content is reached by scrolling rather than by wrapping.** The
width has to be known before the first row is laid out, which is a computed content width and
`ListHorizontalSizingBehavior::Unconstrained` over it — not a measurement of item zero, which in a
diff is a short `@@` header. That is a design choice about the list and not only about the element:
a row of code that runs off the pane is one line the reader scrolls to, rather than a row that is
suddenly six lines tall in the middle of a comparison. `ui/viewer/diff.rs` is the pattern, and
`char_advance()` there is where an assumed glyph width buys it (`G221`).

**A virtual list of variable rows is told every height before it lays one out, so whoever feeds it
remembers what each row measured** — keyed by the row's identity rather than its position, and
against the content and the width it was measured at, because a height is only a height at one
width. A row it has no measurement for is laid out at an estimate, measured by the frame that draws
it, and asks for one more frame; so an estimate is on screen for the frame that discovers it and no
longer. `state::conversation::TranscriptScroll` is that shape.

**A scroll position belongs to the element, and what it is a position *in* belongs beside it.** A
surface that shows several documents through one scroll handle keys the offsets it saves by which
document it was showing, restores on the way in and follows the tail only for a reader who is on
it; anything else is a reader dragged away from what they were reading by an arrival somewhere
else. `state::conversation::TranscriptScroll` is that shape, one per composer slot, and every field
of it is interior-mutable because `render` holds `&AppState` and the values are readings of the
last frame rather than state the application owns.

**A row keyed by a ULID takes its id through `ui::eid`.** `ElementId`'s tuple form carries a `u64`
and a ULID is twice that, so a row that names a task, a step or a project is built by `eid` and
`eid2` in `crates/ubiq/src/ui/mod.rs` — one place, rather than the same `format!` at every call
site. Hashing the id into a `u64` instead would collide silently, and an id that is not the id is a
trap rather than a shortcut. A row keyed by an *enum discriminant* keeps the tuple form: a column
and a filter pill are one of a fixed few, and nothing is gained by naming them in words.

**Every primitive has a specimen, and the kitchen sink's style reference is where it is.** The page
draws each token and each kit function under the name a call site reaches it by, wired to real state
where the primitive has a state, so a control's off state, a token's value in the other palette and a
surface whose edge floats inside its container are all looked at in one place rather than hunted for
across screens. A primitive added to `ui/kit/` gets a specimen there in the same change; that is also
where a convention drawn ahead of its use — `border_focus`, the modal — becomes something a reader
can see instead of something the documentation asserts. What the page holds and how it is built is
[`../features/workbench.md`](../features/workbench.md)'s.

To add an area to the window:

1. Put its state in `crates/ubiq/src/state/`, as data plus small mutators. Nothing that draws, and no
   component-library type unless the widget's own state *is* the thing being modelled —
   `state/editor.rs` holds a buffer per open file for that reason, and it is the exception.
2. Add a field to `AppState`, or to `OpenProject` when it belongs to a project rather than to the
   window, and a mutator that ends in `cx.notify()`. Any component-library state the window itself
   needs — an `InputState` — is an `Entity` field on `AppState`, with its subscription pushed onto
   `_subscriptions`. A screen that reads *across* the projects a window holds rather than through
   the one it is pointed at keeps all three of its own facts on `AppState` — how far it reaches,
   its own view over that reach, and a map back from a record to the project that minted it — and
   resolves that project per record rather than assuming the active one. The Teams span is the
   worked example (`D154`).
3. Write `crates/ubiq/src/ui/<area>.rs` as `fn render(&AppState, &mut Context<AppState>)`. Hang it
   off `shell.rs` if it is chrome, or give it a `PanelKind` and an arm in `ui::dock::body` if it is
   a panel — [`../features/workbench.md`](../features/workbench.md) has that path in full.
4. Reach for a `gpui-component` widget first. If there is none, and a second caller wants the same thing,
   it belongs in `ui/kit/`, must not name `AppState`, and gets a specimen on the style reference.
5. Colours through tokens, sizes through constants, no radii, and a coloured left edge on anything
   that reads as a surface.

## The icon set

`gpui-component` embeds the Lucide set and exposes it as `IconName`, and that is what `icon_button`,
`Icon::new` and the menus take. What it does not cover — a harness, a pane's state, a permission
mode, an orchestration node — lives in `assets/icons/` as a monochrome 24x24 SVG, listed with its
goal in `assets/icons/icons.yaml`. One name per icon: the file name, the registry key and the Rust
variant are the same kebab-case word.

**The file carries no colour.** `Window::paint_svg` rasterises through resvg into an alpha mask,
caches it in the sprite atlas under path and size only, and tints it with an `Hsla` when the sprite
is composited. So a token decides an icon's colour at the call site, the two palettes and any future
accent cost nothing and re-rasterise nothing, and a gradient or a second colour in an SVG is
silently discarded. An icon that genuinely needs two colours is two `Icon` elements with two tokens.
The same is true of a transformation: rotate and scale are a matrix on the cached tile, always about
the element's own centre.

Hence the spec — `0 0 24 24`, `currentColor` at `stroke-width` 2, all ink inside 1.5-22.5, at most
four shapes, and none of `<text> <use> <defs> <style> <mask>`, the gradients or `<animate>`. Three
of those apply only to what we draw: a harness's mark follows its owner's shape, and an icon adopted
from `gpui-component` came out of a coherent family, so the shape count, the ink envelope
and the two-decimal rule are lifted for both — `sun` is nine shapes, `github` fills the box.
`just icons-check` enforces the mechanical half; `just icons-sheet` and `just icons-audit` render
the review sheets an icon is judged on, at 16, 24 and 64px in both palettes, against a frozen canon
strip. The rules and the drawing loop are the `ubiq-icons` skill.

Micro-animation is not part of any of this: it is a state's property, driven by `with_animation` on
the element, and it never produces an icon variant. The writing mark in `ui/conversation/mod.rs` is
five divs and an opacity ramp. The brand mark's spin in `ui/mark.rs` is the longest one the app
draws, and it turns the ring alone, because a rotation is a matrix on the cached tile of a single
`svg()` and the cubes stacked over it are an image.

## Design assets

`_docs/design/` holds the material screens are built against. It is assets, not documents: nothing
in it carries frontmatter, and the documentation checks skip it entirely, because a captured
prototype edited to satisfy a linter stops being evidence of what was designed.

| Path | Holds |
|---|---|
| `_docs/design/wireframe-opus/` | The four earlier screen wireframes — project launcher, session, subagents, settings — plus a combined board. Superseded by `ubiq-layout.png` for the shell; see `D16` |
| `_docs/design/output/` | HTML prototypes and their stylesheet, captured from a design tool |
| `_docs/design/_old/` | Superseded wireframes, kept for reference |
| `_docs/design/ubiq-layout.png` | **The target layout.** The workbench shell is built against this one |

The wireframes are authored in a compact YAML form rather than by hand-editing diagram JSON; that
format and its converter are described in [`diagram-format.md`](./diagram-format.md).

When a screen and its wireframe disagree, the wireframe is a record of intent and the code is the
record of fact — neither silently outranks the other. Reconcile it deliberately, and if the design
changed, re-render the wireframe in the same commit.

## Rationale

**Why GPUI rather than a web view?** A pane is a terminal at full refresh under a stream of escape
sequences, several at once. A GPU-drawn native tree keeps that cheap and keeps the whole application
in one language, with no bundler, no serialisation boundary in the middle of the render path, and no
second runtime to ship.

**Why token accessors rather than passing a theme value around?** Because a colour is read at the
leaves of a deep element tree, and threading a palette through every builder would put theme
plumbing in every component signature. The cost is a thread-local read per colour; the benefit is
that no component has an opinion about theming.

## Related docs

- [`architecture.md`](./architecture.md) — why the UI holds no process and no pseudo-terminal
- [`diagram-format.md`](./diagram-format.md) — how the wireframes are authored and rendered
- [`../features/panes-and-terminals.md`](../features/panes-and-terminals.md) — what a pane shows and how focus behaves
