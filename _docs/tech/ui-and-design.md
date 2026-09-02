---
id: tech-ui
title: UI and design
kind: tech
status: current
summary: The GPUI rendering model, the complete theme token set and the rule that no colour escapes it, how a palette is switched, the shape every surface, modal and dialog is drawn in, the page every primitive is looked at on, and the design assets screens are built against.
read_when: you are building or restyling a screen, adding a colour or a size, switching or extending a palette, raising a modal or the file picker, looking at a primitive on the style reference, or looking for the wireframe a layout came from
updated: 2026-09-02
verified: 2026-09-02
code_anchors: [crates/ubiq/src/theme.rs, crates/ubiq/src/app.rs, crates/ubiq/src/ui/mod.rs, crates/ubiq/src/ui/kit/mod.rs, crates/ubiq/src/ui/kit/controls.rs, crates/ubiq/src/ui/kit/files.rs, crates/ubiq/src/ui/kit/menu.rs, crates/ubiq/src/ui/kit/canvas.rs, crates/ubiq/src/ui/kit/overlay.rs, crates/ubiq/src/ui/file_picker.rs, crates/ubiq/src/state/file_picker.rs, crates/ubiq/src/ui/sink/style.rs, crates/ubiq/src/ui/shell.rs, crates/ubiq/src/ui/terminal.rs, crates/ubiq/src/ui/dock/mod.rs, crates/ubiq/src/ui/dock/skin.rs]
depends_on: [tech-architecture]
review_cycle: quarterly
---

# UI and design

## The rendering model

The UI is GPUI — Zed's retained-mode, GPU-accelerated framework — with the `gpui-component` widget
set on top for the components an application expects to be given rather than to write.

Three properties shape how UI code reads:

**Views are structs that render.** A type implementing `Render` owns its state and produces an
element tree from it. `AppState` in `crates/ubiq/src/app.rs` is the root: it owns the window's own
state — the dock, the chat, the console, the emulators — and one `OpenProject` per project the
window holds, carrying that project's tree, files and panes. Its render delegates to
`crates/ubiq/src/ui/shell.rs`.

**`AppState` is the only owner of state, and not the only view.** The window's arrangement is a
dock of movable panels, and the component library requires each panel to be an entity that renders,
focuses and emits — `D42`, which half reverses `D17`. A panel is an adapter: `WorkbenchPanel` in
`crates/ubiq/src/ui/dock/mod.rs` holds a weak `AppState` handle and a panel kind, and its render is
a `match` that delegates to the same free functions every screen area is.

**Mutation ends in a redraw request.** Nothing repaints because a field changed; it repaints because
the code that changed it said so through its context. Every state-mutating method on `AppState` ends
that way, and one that forgets is a pane that stops updating.

**Layout is flexbox.** Elements are composed with the same direction, grow, gap and alignment
vocabulary as CSS flexbox, in Rust builder form.

`crates/ubiq-app/src/main.rs` installs the component library and its assets, sets the palette and binds
the quit action, then asks for the first window. Windows themselves are opened by
`app::open_project_window`, which is the only place one is created, so the first window and "open in
a new window" go through the same code. Each window owns its own `AppState` and they share nothing
but the palette, which is process-wide. Everything drawn belongs under `crates/ubiq/src/ui/`.

## Theme tokens

**This document owns the token set.** Every colour in the UI comes from a token accessor in
`crates/ubiq/src/theme.rs`. A literal colour anywhere else is a defect, with no exceptions worth
carving out — a one-off shade is the mechanism by which a themed application stops being themeable.

Tokens are grouped by role, and the role is the point: a token names what a colour is *for*, so that
a palette swap changes every surface consistently.

| Group | Accessors | For |
|---|---|---|
| Surface | `app_bg`, `pane_bg`, `surface`, `surface_raised`, `hover`, `selected`, `scrim` | The stack of backgrounds, from the window down to a selected row — and what a modal lays over the window it took the keyboard from |
| Text | `text`, `text_muted`, `text_faint`, `on_accent` | Primary copy, secondary copy, the faintest tier — ignored rows, timestamps, hints — and copy sitting on a filled surface |
| Accent | `accent`, `accent_muted`, `accent_soft` | The interactive colour, its subdued form, and the fill behind a selected row |
| Terminal | `selection_background`, `link_underline`, `link_underline_hover` | Selected cells in a pane, and the underline on an OSC 8 or detected URL — brighter when the pointer is over it |
| Border | `border`, `border_focus` | Ordinary separation, and the focused pane's edge |
| Status | `danger`, `success`, `warning`, `info`, each with a `_soft` variant | Agent and process states, and the fills behind them — a diff line, a status chip, a state dot's ring |
| Project | `project_colour(n)`, `project_colour_count()` | The identity of one project, wherever it appears |

The `_soft` variants are declared with their own alpha in `theme.rs` rather than computed at a call
site with `.alpha(...)`. A shade that only exists at one call site is a shade a palette swap cannot
reach.

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
`project_colour_count` is what project settings offers when a project is recoloured. The mark reads
its swatch's luminance through `project_mark_dark` to pick the pale or the blue logo; the other
places go through `AppState::project_tint`. A window holding no project — which happens when the
catalogue is empty — has one neutral appearance decided in a single place rather than four call
sites each falling back to swatch zero.

Two palettes are built in, dark and light, both defined in the same file and both complete — a token
that exists in one exists in the other. The active theme is thread-local and read through the
accessor, so a token call site never learns which palette answered it.

**Switching a palette goes through `theme::set_mode`, never through `Theme::set`.** Two theme
systems are live at once: Ubiq's tokens, and the component library's own theme, which is what
colours the editor, the textarea, the scrollbars and the markdown view. `set_mode` moves both, so
they cannot drift into different modes. `ThemeId::toggled` gives the other palette, which is all the
titlebar's toggle needs. The palette is process-wide, so a second window opens in the mode the first
one is in, and switching in either switches both.

A pane's emulator is the one surface that does not read a token when it draws: it is built with a
copy of the palette, so `toggle_theme()` pushes a rebuilt configuration into every emulator as well
as calling `set_mode`. Any component given a palette rather than reading one has to be walked the
same way.

`theme.rs` also owns the constants that are not colours, for the same reason it owns the colours:
restyling the shell should be one file to visit.

| Constant | Is |
|---|---|
| `MONO_FONT` | The family for code, paths, counts and every mono label |
| `ACCENT_EDGE` | The width of the coloured left border that identifies a surface |
| `TERMINAL_FONT_SIZE`, `TERMINAL_PADDING`, `TERMINAL_SCROLLBACK` | The terminal body: its type size, the inset its output is drawn inside, and how many lines an emulator keeps |
| `EDITOR_FONT_SIZE`, `EDITOR_FONT_MIN`, `EDITOR_FONT_MAX` | The editor's base point size and the range a project's zoom is allowed to live in — the same project font size the editor, the terminal panes and the explorer tree follow |
| `TITLEBAR_HEIGHT`, `STATUS_BAR_HEIGHT`, `RAIL_WIDTH` | The fixed chrome, which does not resize |
| `EXPLORER_WIDTH`, `CHAT_WIDTH`, `DOCK_HEIGHT` | The size each of the dock's three edge regions opens at. What the user drags one to is remembered per project, inside the arrangement blob, and is what a restored window opens on |
| `INSPECTOR_WIDTH`, `TASKS_HEIGHT`, `GRAPH_DOT_PITCH` | The agents screen: the inspector beside its graph, the tasks drawer under it, and the pitch of the dotted ground at 100% zoom |
| `MODAL_WIDTH`, `MODAL_MAX_HEIGHT` | A modal: one width, because a modal is one question, and the fraction of the window's height its body scrolls inside |

A region's constant is what a fresh window opens it at; what the drag will not pass is the dock's
own, so a region is one number rather than a triple. The agents screen's three are the same shape
for a different reason: its inspector and its drawer are shown and hidden rather than dragged.

Syntax colours are the one thing not tokenised here. They come from the component library's own
highlighter theme, which `theme::set_mode` keeps in step with Ubiq's palette, so the editor and the
chat's markdown never sit in a different mode from the chrome. That is the same posture as the
library's buttons and scrollbars: not a literal, and so not an exception to the rule.

Adding a colour means adding a token to its group, giving it a value in **both** palettes, and using
the accessor. Adding a group means a role none of the seven covers, which is rare enough to be worth
arguing about in [`decisions.md`](./decisions.md) — `Project` carries `D19`, and `Terminal` is the
selection and link colours a pane's emulator paints.

Every token has a call site, and for one of them the only one is a specimen. The style reference
draws all of them by name — that is what the page is for — but `selected` fills no row, and
`border_focus` marks the focused text field rather than a pane: the use it was designed for, focus
across split panes, is still designed ahead of the code. That is listed as a gap in
[`../backlog.md`](../backlog.md) rather than quietly resolved by the drawing, because a specimen is
evidence a token has a value, not evidence anything uses it.

## Conventions for a screen

- **Focus is shown on the surface's left edge**, through `border_focus`. It is the one signal that
  must be readable at a glance across a window of panes, so nothing else competes for it — but no
  pane carries it yet, because focus across split panes is designed ahead of the code. A text field
  that holds the keyboard keeps that left edge and **adds an underline** on the bottom, so the
  active box is the one that is underlined. That treatment is `kit::field` in
  `crates/ubiq/src/ui/kit/controls.rs`, the container every free-text input sits in — a surface with
  a coloured left edge, joined by a bottom underline in the focus colour while the input holds the
  keyboard. The command field, the project search, the chat composer, the agents
  inspector's composer, the board's filter and form fields, and the explorer's and the file picker's
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

**A dialog is that same modal, worked in rather than answered.** The file picker — `ui/file_picker.rs`,
raised over any screen — keeps every rule the modal keeps and differs in the three ways a dialog with
work in it has to: it opens at `DEFAULT_WIDTH` by `DEFAULT_HEIGHT` and is **resized from a corner
grip**, never below `MIN_WIDTH` by `MIN_HEIGHT` and never past what the window can hold; the drag is
tracked on the full-window layer rather than on the panel, so a pointer that outruns the corner does
not strand it; and **whether an outside click dismisses it is the caller's**, because a dialog that
holds the window until it is answered and one that goes away the moment attention leaves it are two
different asks. The four sizes live beside the state, in `state/file_picker.rs`, because they are
what a resize is clamped against rather than what a screen is laid out on.

**A dialog is worked from the keyboard, and a binding against a field has to be registered late.**
The component library's input binds `up`, `down`, `left`, `right`, `enter` and `escape` for itself, in
the `Input` context — the deepest node in the tree, and depth is what breaks a keymap's ties. A screen
that wants those keys while the focus is in a field binds each of them twice: once for its own
context, and once for `ItsContext > Input`, which matches at the same depth as the library's and wins
by being registered afterwards. `app::install_key_bindings` is called after `gpui_component::init` for
exactly that reason. A handler that turns out to have no answer calls `cx.propagate()`, and the field
gets its key back — which is how the picker's and the explorer's `left` and `right` are caret keys
again in a flat list.

**A row is one line, and a value that does not fit is elided.** `kit::elided` truncates with the
system ellipsis and carries the whole string as its tooltip, which is why it takes an element id. A
name, a path or a title that wrapped instead would push everything under it down, and a column of
rows is scanned by its left edge — so nothing in a row, a footer or a card header is allowed a second
line.

## How a screen is put together

**`gpui-component` first.** Its `Icon`, `Kbd`, `Badge`, `Editor`, `Textarea`, `Scrollbar`, markdown
view and dock are used directly — the dock being the largest widget in the library and the whole of
the window's arrangement, `D42`. `crates/ubiq/src/ui/kit/` holds only what the library
does not give us — the slab every surface is drawn in and the card that is a slab you can pick, the
field every text entry sits in, the state dot, the pill, the state chip, the toggle pill for an
independent facet and the choice pill
for one value of a set, the tick box a row is chosen with where several may be, the elided run that
says the whole of itself on hover, the filled button a screen's single obvious action is drawn as,
the stepper, the flat meter, the disclosure bar, the section label, the panel header, the shared tab
strip, the progress ring, the painted layers in `canvas.rs`, the file-list chrome the picker and the
explorer share in `files.rs`, and the one dropdown mechanism every menu in the window uses — plus
the context menu a right-click raises, which is that same panel opened at the pointer rather than
from a trigger.

**Some surfaces are painted, not laid out.** Flexbox and `gpui-component` cover almost everything;
what is left is geometry a box model cannot express — a dotted ground, a cubic connector between two
points, a dashed outline, a trail of grains, a ring at a percentage. Those go through GPUI's
`canvas` element, and the reusable ones are `crates/ubiq/src/ui/kit/canvas.rs`: each is one layer
that fills its parent absolutely, takes no click, and knows nothing about what it is drawing, so a
caller stacks them in the order they should read. The canvas element itself is sized to fill that
layer; a canvas that only laid out to its content would paint into a strip at the top of the pane. `progress_ring` in `controls.rs` is the same
device inline.

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
