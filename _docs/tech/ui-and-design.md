---
id: tech-ui
title: UI and design
kind: tech
status: current
summary: The GPUI rendering model, the complete theme token set and the rule that no colour escapes it, how a palette is switched, the shape every surface is drawn in, and the design assets screens are built against.
read_when: you are building or restyling a screen, adding a colour or a size, switching or extending a palette, or looking for the wireframe a layout came from
updated: 2026-08-31
verified: 2026-08-31
code_anchors: [crates/ubiq/src/theme.rs, crates/ubiq/src/app.rs, crates/ubiq/src/ui/mod.rs, crates/ubiq/src/ui/kit/mod.rs, crates/ubiq/src/ui/shell.rs]
depends_on: [tech-architecture]
review_cycle: quarterly
---

# UI and design

## The rendering model

The UI is GPUI — Zed's retained-mode, GPU-accelerated framework — with the `gpui-component` widget
set on top for the components an application expects to be given rather than to write.

Three properties shape how UI code reads:

**Views are structs that render.** A type implementing `Render` owns its state and produces an
element tree from it. `AppState` in `crates/ubiq/src/app.rs` is the root and the only one: it owns
the panes, the focused pane and the layout mode, plus the workbench, explorer, editor and chat
state, and its render delegates to `crates/ubiq/src/ui/shell.rs`.

**Mutation ends in a redraw request.** Nothing repaints because a field changed; it repaints because
the code that changed it said so through its context. Every state-mutating method on `AppState` ends
that way, and one that forgets is a pane that stops updating.

**Layout is flexbox.** Elements are composed with the same direction, grow, gap and alignment
vocabulary as CSS flexbox, in Rust builder form.

`crates/ubiq/src/main.rs` installs the component library and its assets, sets the palette and binds
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
| Surface | `app_bg`, `pane_bg`, `surface`, `surface_raised`, `hover`, `selected` | The stack of backgrounds, from the window down to a selected row |
| Text | `text`, `text_muted`, `text_faint`, `on_accent` | Primary copy, secondary copy, the faintest tier — ignored rows, timestamps, hints — and copy sitting on a filled surface |
| Accent | `accent`, `accent_muted`, `accent_soft` | The interactive colour, its subdued form, and the fill behind a selected row |
| Border | `border`, `border_focus` | Ordinary separation, and the focused pane's edge |
| Status | `danger`, `success`, `warning`, `info`, each with a `_soft` variant | Agent and process states, and the fills behind them — a diff line, a status chip, a state dot's ring |
| Project | `project_colour(n)`, `project_colour_count()` | The identity of one project, wherever it appears |

The `_soft` variants are declared with their own alpha in `theme.rs` rather than computed at a call
site with `.alpha(...)`. A shade that only exists at one call site is a shade a palette swap cannot
reach.

The project group is the one group whose members carry no role. A swatch means *this project* and
nothing else, and a project keeps the same one everywhere it is drawn: its dot in the picker, the
fill behind its name in the titlebar, the mark above the rail, and the window's whole left edge.
`project_colour` wraps, so the number of projects is not bounded by the number of swatches.

Two palettes are built in, dark and light, both defined in the same file and both complete — a token
that exists in one exists in the other. The active theme is thread-local and read through the
accessor, so a token call site never learns which palette answered it.

**Switching a palette goes through `theme::set_mode`, never through `Theme::set`.** Two theme
systems are live at once: Ubiq's tokens, and the component library's own theme, which is what
colours the editor, the textarea, the scrollbars and the markdown view. `set_mode` moves both, so
they cannot drift into different modes. `ThemeId::toggled` gives the other palette, which is all the
titlebar's toggle needs. The palette is process-wide, so a second window opens in the mode the first
one is in, and switching in either switches both.

`theme.rs` also owns the constants that are not colours, for the same reason it owns the colours:
restyling the shell should be one file to visit.

| Constant | Is |
|---|---|
| `MONO_FONT` | The family for code, paths, counts and every mono label |
| `ACCENT_EDGE` | The width of the coloured left border that identifies a surface |
| `TITLEBAR_HEIGHT`, `STATUS_BAR_HEIGHT`, `RAIL_WIDTH` | The fixed chrome, which does not resize |
| `EXPLORER_WIDTH`/`_MIN`/`_MAX`, `CHAT_WIDTH`/`_MIN`/`_MAX`, `DOCK_HEIGHT`/`_MIN`/`_MAX` | The default and permitted size of each resizable panel |

A panel's three constants travel together: the default is what a fresh window opens at, and the two
bounds are what the drag handle will not pass. Adding a panel means adding all three.

Syntax colours are the one thing not tokenised here. They come from the component library's own
highlighter theme, which `theme::set_mode` keeps in step with Ubiq's palette, so the editor and the
chat's markdown never sit in a different mode from the chrome. That is the same posture as the
library's buttons and scrollbars: not a literal, and so not an exception to the rule.

Adding a colour means adding a token to its group, giving it a value in **both** palettes, and using
the accessor. Adding a group means a role none of the six covers, which is rare enough to be worth
arguing about in [`decisions.md`](./decisions.md) — `Project` is the only group added since the
original five, and it carries `D19`.

Not every token has a call site. `border_focus`, `accent_muted`, `selected` and `info_soft` are
defined in both palettes and unused, because the conventions they belong to — focus across split
panes, in particular — are designed ahead of the code. They are listed as a gap in
[`../backlog.md`](../backlog.md) rather than quietly deleted, because deleting a token is how a
convention silently stops being available.

## Conventions for a screen

- **Focus is shown on the surface's left edge**, through `border_focus`. It is the one signal that
  must be readable at a glance across a window of panes, so nothing else competes for it. With a
  single pane on screen there is nothing to contrast against, so the token has no call site yet.
- **Status is shown by colour from the status group**, never by wording alone. A stopped agent and a
  failed one are different colours.
- **Pane chrome stays two rows at most.** Identity and state on the first, context — folder, model,
  remaining context window — on the second. Anything more takes space from the terminal, which is
  the thing the user is actually reading.
- **The terminal body is never styled by Ubiq.** Its colours come from the harness's own output.
  Ubiq draws the frame and stays out of the content.
- **Spacing comes from the framework's scale**, not from arbitrary pixel values. Sizes that are part
  of the layout — chrome heights, panel widths — are constants in `theme.rs` instead.
- **There are no radii.** See *The shape of a surface* below; a corner radius anywhere is a defect
  in the same way a literal colour is.

## The shape of a surface

**Nothing is rounded, and the left edge does the identifying.** Ubiq's surfaces are square; a
coloured border on the left is what says what a surface is — accent for the thing the user is
acting in, the status colour for something being reported, the project colour for the window
itself. `ACCENT_EDGE` in `theme.rs` is its width, and `ui::kit::slab` is the shape.

This replaces the more usual "box with a border all the way round". A GPUI element has one
`border_color` for all four sides, so a grey box with one coloured edge is two elements; one edge
and no box is one, and it reads more clearly at the sizes this UI uses.

Circles survive in exactly one place: state dots, which are dots.

## How a screen is put together

**`gpui-component` first.** Its `Icon`, `Kbd`, `Badge`, `Editor`, `Textarea`, `Scrollbar`, markdown
view and resizable group are used directly. `crates/ubiq/src/ui/kit/` holds only what the library
does not give us — the state dot, the pill, the section label, the panel header, the shared tab
strip, the progress ring, and the one dropdown mechanism every menu in the window uses.

**The kit knows nothing about the workbench.** Its interactive helpers take a plain
`Fn(&mut Window, &mut App)`, and call sites bridge to the root view with `ui::handler` and
`ui::indexed`, or with `cx.listener` where the signature fits directly. A kit function that names
`AppState` has stopped being a primitive.

**Screen areas are free functions, not views.** One module per area under `ui/`, each a
`fn(&AppState, &mut Context<AppState>) -> impl IntoElement`. Keeping one view means one place owns
state and one place requests redraws. A helper that takes `cx` and is called in a loop returns
`AnyElement`, because Rust 2024's capture rules make `impl IntoElement` borrow the context.

**Exactly one menu is open at a time**, tracked as a single `Option<MenuId>` on the workbench state.
A trigger *opens* rather than toggles, so the open panel's outside-click dismissal cannot race the
click that was meant to close it.

**A filling pane needs `flex_1` and `min_h(px(0.))`** — or `min_w` — together. One without the other
is the standard way a GPUI flex child refuses to shrink.

**Scrolling needs an `.id(...)` and a tracked handle.** `.overflow_y_scroll()` does nothing without
an id. A scrollbar is a sibling of the scroll area, absolutely positioned over it, so it stays put
while the content moves under it — not a child, which would scroll with the content.

To add an area to the window:

1. Put its state in `crates/ubiq/src/state/`, as data plus small mutators. No component-library
   types and nothing that draws.
2. Add a field to `AppState`, and a mutator that ends in `cx.notify()`. Any component-library state
   it needs — an `InputState`, an `EditorState` — is an `Entity` field there too, with its
   subscription pushed onto `_subscriptions`.
3. Write `crates/ubiq/src/ui/<area>.rs` as `fn render(&AppState, &mut Context<AppState>)`, and hang
   it off `shell.rs`.
4. Reach for a `gpui-component` widget first. If there is none, and a second caller wants the same thing,
   it belongs in `ui/kit/` and must not name `AppState`.
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
