---
id: tech-ui
title: UI and design
kind: tech
status: current
summary: The GPUI rendering model, the complete theme token set and the rule that no colour escapes it, and the design assets that screens are built against.
read_when: you are building or restyling a screen, adding a colour, or looking for the wireframe a layout came from
updated: 2026-08-31
verified: 2026-08-31
code_anchors: [crates/ubiq/src/theme.rs, crates/ubiq/src/app.rs, crates/ubiq/src/ui/mod.rs]
depends_on: [tech-architecture]
review_cycle: quarterly
---

# UI and design

## The rendering model

The UI is GPUI — Zed's retained-mode, GPU-accelerated framework — with the `gpui-component` widget
set on top for the components an application expects to be given rather than to write.

Three properties shape how UI code reads:

**Views are structs that render.** A type implementing `Render` owns its state and produces an
element tree from it. `AppState` in `crates/ubiq/src/app.rs` is the root: it owns the pane map, the
focused pane and the layout mode, and its render builds the titlebar and the pane area from them.

**Mutation ends in a redraw request.** Nothing repaints because a field changed; it repaints because
the code that changed it said so through its context. Every state-mutating method on `AppState` ends
that way, and one that forgets is a pane that stops updating.

**Layout is flexbox.** Elements are composed with the same direction, grow, gap and alignment
vocabulary as CSS flexbox, in Rust builder form.

The window itself is opened in `crates/ubiq/src/main.rs`: it installs the component library and its
assets, sets the theme, binds the quit action, and constructs the root view. Everything else belongs
under `crates/ubiq/src/ui/`.

## Theme tokens

**This document owns the token set.** Every colour in the UI comes from a token accessor in
`crates/ubiq/src/theme.rs`. A literal colour anywhere else is a defect, with no exceptions worth
carving out — a one-off shade is the mechanism by which a themed application stops being themeable.

Tokens are grouped by role, and the role is the point: a token names what a colour is *for*, so that
a palette swap changes every surface consistently.

| Group | Tokens | For |
|---|---|---|
| Surface | `app_bg`, `pane_bg`, `surface`, `surface_raised`, `hover`, `selected` | The stack of backgrounds, from the window down to a selected row |
| Text | `text`, `text_muted`, `on_accent` | Primary copy, secondary copy, and copy sitting on an accent fill |
| Accent | `accent`, `accent_muted` | The interactive colour and its subdued form |
| Border | `border`, `border_focus` | Ordinary separation, and the focused pane's edge |
| Status | `danger`, `success`, `warning`, `info` | Agent and process states, and messages about them |

Two palettes are built in, dark and light, both defined in the same file and both complete — a token
that exists in one exists in the other. The active theme is thread-local and read through the
accessor, so a token call site never learns which palette answered it.

Adding a colour means adding a token to its group, giving it a value in **both** palettes, and using
the accessor. Adding a group means a new role that none of the five covers, which is rare enough to
be worth arguing about in [`decisions.md`](./decisions.md).

## Conventions for a screen

- **Focus is shown on the border**, through `border_focus`. It is the one signal that must be
  readable at a glance across a window of panes, so nothing else competes for it.
- **Status is shown by colour from the status group**, never by wording alone. A stopped agent and a
  failed one are different colours.
- **Pane chrome stays two rows at most.** Identity and state on the first, context — folder, model,
  remaining context window — on the second. Anything more takes space from the terminal, which is
  the thing the user is actually reading.
- **The terminal body is never styled by Ubiq.** Its colours come from the harness's own output.
  Ubiq draws the frame and stays out of the content.
- **Spacing and radii come from the framework's scale**, not from arbitrary pixel values.

## Design assets

`_docs/design/` holds the material screens are built against. It is assets, not documents: nothing
in it carries frontmatter, and the documentation checks skip it entirely, because a captured
prototype edited to satisfy a linter stops being evidence of what was designed.

| Path | Holds |
|---|---|
| `_docs/design/wireframe-opus/` | The four screen wireframes — project launcher, session, subagents, settings — plus a combined board. Its `README.md` explains each and how to re-render them |
| `_docs/design/output/` | HTML prototypes and their stylesheet, captured from a design tool |
| `_docs/design/_old/` | Superseded wireframes, kept for reference |
| `_docs/design/ubiq-layout.png` | The layout sketch the pane arrangement follows |

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
