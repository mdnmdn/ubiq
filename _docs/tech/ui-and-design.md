---
id: tech-ui
title: UI and design
kind: tech
status: current
summary: The GPUI rendering model, the complete theme token set and the rule that no colour escapes it, how a palette is switched, the shape every surface, modal and dialog is drawn in, the page every primitive is looked at on, and the design assets screens are built against.
read_when: you are building or restyling a screen, adding a colour or a size, switching or extending a palette, raising a modal or the file picker, looking at a primitive on the style reference, or looking for the wireframe a layout came from
updated: 2026-09-10
verified: 2026-09-10
code_anchors: [crates/ubiq/src/theme.rs, assets/icons/icons.yaml, _tools/icons.py, crates/ubiq/src/app/mod.rs, crates/ubiq/src/app/shell.rs, crates/ubiq/src/app/wire.rs, crates/ubiq/src/ui/viewer/diff.rs, crates/ubiq/src/ui/mod.rs, crates/ubiq/src/ui/work.rs, crates/ubiq/src/ui/outline.rs, crates/ubiq/src/ui/kit/mod.rs, crates/ubiq/src/ui/kit/controls.rs, crates/ubiq/src/ui/kit/files.rs, crates/ubiq/src/ui/kit/menu.rs, crates/ubiq/src/ui/kit/canvas.rs, crates/ubiq/src/ui/kit/overlay.rs, crates/ubiq/src/ui/kit/settings.rs, crates/ubiq/src/ui/explorer.rs, crates/ubiq/src/ui/file_picker.rs, crates/ubiq/src/state/file_picker.rs, crates/ubiq/src/state/prefs.rs, crates/ubiq/src/ui/sink/style.rs, crates/ubiq/src/ui/shell.rs, crates/ubiq/src/ui/ribbon.rs, crates/ubiq/src/ui/settings.rs, crates/ubiq/src/ui/terminal.rs, crates/ubiq/src/ui/dock/mod.rs, crates/ubiq/src/ui/dock/skin.rs, crates/ubiq/src/ui/conversation/mod.rs, crates/ubiq/src/ui/titlebar.rs, crates/ubiq/src/ui/navigator.rs, crates/ubiq/src/ui/viewer/scene.rs]
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
| Text | `text`, `text_muted`, `text_faint`, `on_accent` | Primary copy, secondary copy, the faintest tier — ignored rows, timestamps, hints — and copy sitting on a filled surface |
| Accent | `accent`, `accent_muted`, `accent_soft`, `accent_id` | The interactive colour, its subdued form, the fill behind a selected row, and which accent the window is dressed in — all three colours derived from one seed, below |
| Terminal | `selection_background`, `link_underline`, `link_underline_hover` | Selected cells in a pane, and the underline on an OSC 8 or detected URL — brighter when the pointer is over it |
| Border | `border`, `border_focus` | Ordinary separation, and the focused pane's edge |
| Status | `danger`, `success`, `warning`, `info`, each with a `_soft` variant | Agent and process states, and the fills behind them — a diff line, a status chip, a state dot's ring |
| Ribbon | `ribbon_alpha`, `ribbon_beta`, `ribbon_ink` | The build-channel ribbon in the window's bottom-left corner — the same value in both palettes, because it marks the build rather than the mood |
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
second axis. Three families each carry a base of their own, because the three are read
differently and resize for different reasons: `Family::Chrome` is the furniture — titlebar, status
bar, rail, tabs, menus, modals, settings, pickers, notifications, dialogs; `Family::Content` is what
code is read at — editor, viewer, explorer tree, search results, terminal panes; `Family::Conversation`
is prose — the transcript, tool blocks, the composer, the agents columns and their sidebar. Five
roles are ratios over the base — `Role::Title` 1.15, `Body` 1.00, `Label` 0.92, `Meta` 0.85, `Micro`
0.80 — and the product is rounded to the nearest half point, the grid the hand-picked sizes it
replaced sat on. A fourth family would be arguing about where a boundary falls; these three fall on
boundaries the code draws for other reasons. **The function is `font`, not `text`**, because `theme::text()` is
the primary text *colour* — the one name collision in the file worth knowing before reading it.

**The rule is held mechanically, the way the crate boundary is.** `just ui` rejects
`text_size(px(<digit>` anywhere under `crates/ubiq/src`, beside the check that the interface never
names the host. The digit is load-bearing: `text_size(px(font))`, where the size is computed from
the project's zoom, is what `ui/explorer.rs`, `ui/search.rs` and `ui/outline.rs` legitimately do,
and a bare `text_size(px(` would reject those.

**Two of the three bases belong to the interface, the third to the project.**
`InterfacePrefs.chrome_font_size` and `InterfacePrefs.conversation_font_size` are `Option<f32>`
(`serde(default)`, so no schema bump), written by `AppState::remember_interface` and read back by
`apply_preferences` through `theme::set_text_scale`, defaulting to `CHROME_FONT_SIZE` and
`CONVERSATION_FONT_SIZE`. The content base is `ViewPrefs.content_font_size`, per project
(`serde(default, alias = "ui_font_size")`, so a blob written under the older name keeps its zoom),
reached through `AppState::content_font_size`, `content_font_size_or_default`, `set_content_font_size`
and `nudge_content_font_size` — the status bar's eleven-entry ladder and the `EDITOR_FONT_MIN` /
`EDITOR_FONT_MAX` clamp are that family and only that family. Because the theme is one process-wide
thread-local while that base is a *project's*, `ui::shell::render` pushes the showing project's size
into the scale at the top of every window's render, so two windows on two projects each draw at
their own size instead of at the last one set.

**Switching goes through `theme::set_theme`, never through `Theme::set`.** It takes three of the four
axes — palette, accent, density — resolves them into the one `Theme` every accessor reads, and dresses
the component library; `theme::set_mode` and `theme::set_density` are that call with the other axes
left as they stand. The text scale is the fourth and is deliberately not an argument, and not a
field on `Theme` either: it is the one axis with nothing to resolve, because a base size is a
number the user set rather than something derived from the palette in hand. So it lives in a
thread-local cell of its own beside the theme's, read through `theme::text_scale` and set through
`theme::set_text_scale`, which is what makes a palette, accent or density switch structurally
unable to undo a size the user chose — there is no resolution for it to be dropped by. Two theme systems are live at once: Ubiq's tokens, and the component library's
own theme, which is what colours the editor, the textarea, the scrollbars and the markdown view.
`set_theme` moves both — `Theme::change` first, from the palette's `Mode`, then
`theme::dress_component_library` writes Ubiq's tokens into the library's `ThemeColor` through
`DerefMut` and `sync_base` pushes the result down to the layer that paints scrollbars and resize
handles. Only the fields that plainly correspond are written, and the two easy to mistake are the
library's `accent`, which is its hover ground, and `primary`, which is the brand colour Ubiq's
`accent` maps to. The theme is process-wide, so a second window opens in the palette, accent and
density the first is in, and switching in either switches both.

A pane's emulator is the one surface that does not read a token when it draws: it is built with a
copy of the palette, so `AppState`'s `toggle_theme`, `set_accent` and `set_density` each push a
rebuilt configuration into every open emulator through `redress_terminals` as well as switching the
theme. Any component given a palette rather than reading one has to be walked the same way.

`theme.rs` also owns the constants that are not colours, for the same reason it owns the colours:
restyling the shell should be one file to visit.

| Constant | Is |
|---|---|
| `MONO_FONT` | The family for code, paths, counts and every mono label — the mono that ships with the OS (`Menlo`, `Cascadia Mono`, `DejaVu Sans Mono`), so the text system resolves it instead of falling back to a proportional face |
| `ACCENT_EDGE` | The width of the coloured left border that identifies a surface |
| `TERMINAL_FONT_SIZE`, `TERMINAL_PADDING`, `TERMINAL_SCROLLBACK` | The terminal body: its type size, the inset its output is drawn inside, and how many lines an emulator keeps |
| `CHROME_FONT_SIZE`, `CONVERSATION_FONT_SIZE` | What the chrome and conversation families draw `Role::Body` at when the interface prefs name no base of their own. Both are interface-scoped, which is why they are a pair and the content family's base is not with them |
| `EDITOR_FONT_SIZE`, `EDITOR_FONT_MIN`, `EDITOR_FONT_MAX` | The content family's base point size and the range a project's zoom is allowed to live in — the one size the editor, the viewer, the terminal panes, the search results and the explorer tree are all read at |
| `DISPLAY_FONT_SIZE` | The one size off the scale, private and read through `theme::font_display()`: the device-login user code, a number to be read off a screen and typed into a phone rather than a heading |
| `TITLEBAR_HEIGHT`, `STATUS_BAR_HEIGHT`, `RAIL_WIDTH` | The chrome the user cannot drag: read at the current density, and sized by nothing else |
| `EXPLORER_WIDTH`, `CHAT_WIDTH`, `DOCK_HEIGHT` | The size each of the dock's three edge regions opens at. What the user drags one to is remembered per project, inside the arrangement blob, and is what a restored window opens on |
| `INSPECTOR_WIDTH`, `TASKS_HEIGHT`, `GRAPH_DOT_PITCH` | The orchestration screen: the inspector beside its graph, the tasks drawer under it, and the pitch of the dotted ground at 100% zoom |
| `AGENT_SIDEBAR_WIDTH`, `NEW_COLUMN_STRIP` | The agents screen: the sidebar that lists every agent, and the strip past the last column that a dragged tab is split off into. How narrow a column itself may get is `state::agents::COLUMN_MIN_WIDTH` instead, because that is a fact about a conversation rather than about this window |
| `EMPTY_START_SIZE`, `EMPTY_START_ICON` | The start control on an empty chat panel: about three times a chrome `kit::icon_button`, because it is the page's whole subject rather than one control among a row of them |
| `MODAL_WIDTH`, `MODAL_MAX_HEIGHT` | A modal: one width, because a modal is one question, and the fraction of the window's height its body scrolls inside |
| `LOGIN_MODAL_WIDTH`, `LOGIN_MODAL_HEIGHT` | The one modal that is not one question: a running harness login, sized through `kit::modal_sized`'s fill mode so a full-screen TUI (`opencode`, `grok`) gets a real terminal instead of the ~50×16 a one-question modal would give it |
| `SETTINGS_WIDTH`, `SETTINGS_HEIGHT` | Application settings: a fixed-size page overlay with a nav, not a one-question modal and not a resizable dialog |

The table splits in two. **The grid half follows the density factor** — `ACCENT_EDGE`,
`TERMINAL_PADDING`, `TITLEBAR_HEIGHT`, `STATUS_BAR_HEIGHT`, `RAIL_WIDTH` and `kit::row_height` /
`kit::row_indent`. Those five are private consts read through `theme::accent_edge()`,
`theme::terminal_padding()`, `theme::titlebar_height()`, `theme::status_bar_height()` and
`theme::rail_width()`, because a factor cannot apply to a const; `Density { Compact 0.9, Regular
1.0, Comfortable 1.15 }` is resolved into the `Theme` alongside the palette and the accent, so a
call site reads a scaled size exactly the way it reads a colour. `AppState::set_density` flips it,
persists it in `InterfacePrefs.density` (`serde(default)`, so no schema bump) and re-dresses every
open emulator — the new `TERMINAL_PADDING` changes the cell grid, and the emulator's own
re-measurement fires the resize that tells the harness.

**Everything from `EXPLORER_WIDTH` down does not scale.** Those are what a *fresh* window opens at;
the drag is remembered per project inside the arrangement blob, so scaling them would fight a value
the user already set.

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
`with_accent` instead, since a hue written out per palette is the thing the accent axis removed.
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

The type scale is looked at the same way: `typography()` on the style reference draws the five roles
across the three families, one column each, so a base moved in the interface prefs is read off the
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
- **The no-file page is the brand, not furniture.** In IDE mode with nothing open the centre shows
  Ubiq's mark at 200px and half opacity on the window's ground — `welcome(app)` in
  `crates/ubiq/src/ui/editor.rs` — theme picked exactly as the rail's mark is: the blue logo on a
  light palette, the white on a dark one, so it reads on the empty page.

## The shape of a surface

**Nothing is rounded, and the left edge does the identifying.** Ubiq's surfaces are square; a
coloured border on the left is what says what a surface is — accent for the thing the user is
acting in, the status colour for something being reported, the project colour for the window
itself. `ACCENT_EDGE` in `theme.rs` is its width, and `ui::kit::slab` is the shape.

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

**A layer painted above a modal is outside it**, and the modal that raised the layer is what has to
know. `on_mouse_down_out` is a capture-phase handler over the panel's own bounds, and a dropdown
opened with `Picker::above_modal` is painted at a higher priority but tested against those bounds
all the same — so a click in the list, its filter field included, reads as a click outside the
modal, and no `stop_propagation` from the layer above can take it back, because capture runs back to
front. A modal with its own dropdowns therefore ignores the outside click while one is down: the
list dismisses itself against its own bounds and the form under it stays, which is one gesture
peeling one layer, exactly as Escape does. `ui/new_agent.rs` is the one that reads.

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
the tree indent (`kit::row_indent`) from the size it draws at — so the explorer's zoom changes the
tree's density. A surface no project zoom reaches — the file picker, the ref list — passes
`kit::row_font()`, which is the chrome family's `Body`: a dialog's rows are furniture, and the
project's zoom is not theirs to follow.

## How a screen is put together

**`gpui-component` first.** Its `Icon`, `Kbd`, `Badge`, `Editor`, `Textarea`, `Scrollbar`, markdown
view and dock are used directly — the dock being the largest widget in the library and the whole of
the window's arrangement, `D42`. `crates/ubiq/src/ui/kit/` holds only what the library does not give
us — the slab every surface is drawn in and the card that is a slab you can pick, the field every
text entry sits in, the state dot, the pill, the state chip, the removable tag whose `×` drops it
and whose label does something when clicked, the toggle pill for an independent facet and the choice
pill for one value of a set, the tick box a row is chosen with where several may be, the elided run
that says the whole of itself on hover, the filled button a screen's single obvious action is drawn
as, the stepper, the flat meter, the disclosure bar, the section label, the panel header, the shared
tab strip, the progress ring, the painted layers in `canvas.rs`, the file-list chrome the picker and
the explorer share in `files.rs`, and the one dropdown mechanism every menu in the window uses —
plus the context menu a right-click, or a control that has no room for a trigger, raises at the
pointer: that same panel, opened at a point rather than under a chip. Its rows are labels, a
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
not pickable — a conversation already attached to another chat tab, say — in the same faint,
click-less style the context menu's own disabled label uses; a disabled row is never dropped from
`items`, because a row that vanishes reads as gone rather than taken, and a picker's own selected
row stays pickable even if the caller also passed its index to `.disabled(...)`.
`.separators(indices)` draws the same hairline the context menu's own separator does at those rows
instead of text, so a searchable picker can carry group headings — themselves plain `.disabled(...)`
rows — in the one `items` list a caller builds and a pick indexes into, the way the agents screen's
column `+` groups the bench from what is already on screen elsewhere. `.tooltip(text)` is what a
picker drawn with no label says on hover — the chat header's chevron is one, and with no words on
the trigger the hover is the only place the question it asks can go.

**A picker has three shapes, and the third is for a form.** `PickerStyle::Plain` is the bare
trigger, `Chip` the small filled one a composer's config controls wear, and `Field` the shape
`kit::field` gives a text input: a filled box on its own coloured left edge, taking the width it is
given, the value truncating rather than pushing the chevron off the end. A column of pickers in a
form reads as a column that way rather than as a ragged edge, and a picker among text inputs reads
as something to click rather than as a line of text.

**A dense form's notes live on a hint mark, not under the row.** `kit::label_hint(id, label, hint)`
draws the label with an `Info` mark beside it and the words on the mark's hover, and
`kit::hint_row(id, label, hint, control)` is `setting_row`'s shape built from one: label and mark
left, control right, one line high, with no rule between rows — a hairline under every one of eight
rows reads as eight sections. `setting_row` and `label_block` spend a whole line on the note, which
is what turns a form of eight questions into a form that scrolls; the words are worth having and
worth reading once. Both take an id, because a tooltip needs a stateful element to hang off — the
same bargain `kit::elided` makes.

The state dot itself is what a conversation's lifecycle reading is drawn as — no primitive of its
own. `ui::conversation::lifecycle` derives one `Lifecycle` from the conversation's own fields
(Starting, Ready, Waiting, Working carrying an `Activity`, Idle, Unloaded, Ended), and
`lifecycle_dot` puts `lifecycle_colour`'s answer on a `status_dot`, with the word in a tooltip, one
or two of them, never a sentence.

**A state dot has four readings and only four: `warning` wants you, `info` is working, `success` is
idle, `text_faint` has stopped.** What a dot read at a glance across a window full of columns has
to answer is whether that conversation wants the reader, and four colours is as many as the glance
holds — so every working turn is one `info` rather than `Activity`'s own palette, which kind of work
being a question the transcript beside it answers. Every value is a status token the window
gives that meaning elsewhere, so a dot invents no colour.

**Two of the four move, and that is the fifth fact about the dot: `Waiting` and `Working` pulse, the
other two are still.** `lifecycle_pulses` is the rule — those two are the readings something is
expected to happen in, and stillness is the wrong thing to draw for them — and the fade is slow and
shallow, opacity 0.45 to 1.0 over two seconds, the same construction the transcript's `writing_mark`
is built from. A dot on a tab strip nobody is looking at is a hint at the edge of vision, not an
alarm; the turn itself is watched at the tail. `lifecycle_pulses` answers `false` for a reader who
asked the system for reduced motion (`App::reduce_motion`), because motion used as a signal is
exactly the motion that setting is about. Carrying the animation is why `kit::status_dot` returns a
`Div` rather than an opaque element.

**The dot is one element, not a colour every surface redraws.** `ui::conversation::lifecycle_dot` —
colour, pulse, the ring it sits on and an element id for the animation — is what the agents column's
header title and each of its tabs draw, and what the dock's tab strip draws for every tab kind
through `TabInfo::dot_colour` and `dot_pulse`, with `ui/dock/mod.rs`'s `PanelKind::Chat` arm the
only one filling the pulse in. So the reading, the mapping *and the element* live in that one module
regardless of caller, and a surface adopting the dot draws no dot of its own. A tab with no
conversation behind it has no dot: there is no state to report.

**Whether the shared conversation view draws the three-dots menu itself is per surface, not fixed.**
`ConversationView` carries `header: bool` beside its existing `footer` and `composer` — the agents
column keeps it `true` and gets a bordered strip holding the menu; the chat panel sets it `false`
and draws the identical fragments, `ui::conversation::lifecycle_mark` and `lifecycle_menu`, at
opposite ends of its own toolbar row instead, with the chevron that changes what the tab is looking
at between them. The state's
reading, the element it is drawn as and the menu's enable rule — `lifecycle`, `lifecycle_colour`,
`lifecycle_dot` and `lifecycle_menu_rows` —
are read in exactly one place regardless of which surface calls them, so a second surface adopting
the shared view is a `ConversationView` field, never a forked copy of any of the three.

**The persistence mark is a second glyph, never a fifth dot colour.** A conversation the user marked
to keep draws `ui::conversation::persistence_mark` — the `pane-persistent` anchor — beside the
lifecycle dot, on the agents column's title and at the head of a chat tab, and nothing at all when
it is not kept. The two answer different questions: the dot says what the conversation is doing, the
anchor says whether it will still be here after a restart, and folding the second into the first
would cost the dot the one reading it is scanned for. `persistent` lives on the `WorkAgent` record
rather than on `Conversation`, so both surfaces read it off the work projection they already hold.
`accept_all` and `debug_dump` live there for the same reason — `ui::conversation::accepts_all` and
`dump_path` read them — but neither earns a glyph beside the dot: they are states the three-dots
menu names in words, and a second and third mark on a tab strip would spend the glance the dot is
there for.

**A row that gathers several controls this way drops their labels for tooltips, not for a second
icon set.** The chat panel's toolbar is icon-only: the lifecycle menu and the change-agent chevron
keep their icon and lose `ghost_button`'s inline label, the label reappearing as the same hover
tooltip every other icon-only control in the window already uses — the titlebar's panel toggles, the
agents column tab's `×` (`Put on the bench`), the chevron's own `change agent`. Two controls that
both add something must still read as different actions at a glance, so a row is never given the
same icon twice with only the tooltip to tell them apart, and the dock's two `+` controls — a
terminal pane's and a chat view's — keep their own icons for exactly that reason.

**Some surfaces are painted, not laid out.** Flexbox and `gpui-component` cover almost everything;
what is left is geometry a box model cannot express — a dotted ground, a cubic connector between two
points, a dashed outline, a trail of grains, a ring at a percentage. Those go through GPUI's
`canvas` element, and the reusable ones are `crates/ubiq/src/ui/kit/canvas.rs`: each is one layer
that fills its parent absolutely, takes no click, and knows nothing about what it is drawing, so a
caller stacks them in the order they should read. The canvas element itself is sized to fill that
layer; a canvas that only laid out to its content would paint into a strip at the top of the pane. `progress_ring` in `controls.rs` is the same
device inline; it is one line over `progress_ring_in`, which takes the fill as an argument, so a
second ring on a surface can carry its own token rather than a second accent.

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
   `_subscriptions`.
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
the most complex one the app draws, and it is five divs and an opacity ramp.

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
