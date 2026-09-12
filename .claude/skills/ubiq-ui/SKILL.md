---
name: ubiq-ui
description: Reference for working on Ubiq's GPUI interface (crates/ubiq) — the render model, theme tokens, the kit primitives, where every state/app/ui module lives, and the rules a screen must follow. Use when building, restyling or debugging any screen, panel, modal, control, colour or layout in the UI crate.
---

# Ubiq UI development

The interface is `crates/ubiq` — GPUI (Zed's retained-mode GPU framework) plus the
`gpui-component` widget set. This skill is the working reference; the owning document is
`_docs/tech/ui-and-design.md`, and the screen inventory is `_docs/features/workbench.md`.

## Before you touch anything

1. **Read `_docs/tech/ui-and-design.md`.** It owns the token set and the surface shape. This
   skill summarises it; it does not replace it.
2. Add `_docs/features/workbench.md` if you are adding a screen area, panel or rail mode;
   `_docs/features/panes-and-terminals.md` for pane focus/resize/chrome;
   `_docs/features/chat.md` for chat tabs and message renderers.
3. Your change updates the documents it touched, in the same commit. `just docs-touched` names
   them.

## The three layers

| Layer | Path | Rule |
|---|---|---|
| `state/` | `crates/ubiq/src/state/` | Data plus small mutators. **Renders nothing.** Names no process, path or file descriptor. |
| `app/` | `crates/ubiq/src/app/` | `AppState` — the root view, the only owner of state. Every mutator ends in `cx.notify()`. |
| `ui/` | `crates/ubiq/src/ui/` | The element tree. Names no coordinator, process, path or descriptor. |

`crates/ubiq` does **not** depend on `crates/ubiq-host`. Everything crossing that line is a
message in `crates/ubiq-proto/src/messages.rs`. `just ui` enforces it.

## Hard rules

- **No literal colour outside `theme.rs`.** Every colour is a token accessor with a value in
  *both* palettes. `theme::fade` is the one allowed transform; a fixed-alpha fill is a `_soft`
  token, not a `fade` at the call site. Content colour (a scene's stroke, an image's pixels,
  the 16 ANSI colours) is data, not a design decision — it passes through.
- **No radii.** Surfaces are square. A coloured **left edge** (`ACCENT_EDGE` wide, `kit::slab`)
  is what identifies a surface — accent for what the user acts in, a status colour for something
  reported, the project colour for the window.
- **The edge collapses onto its container's edge** — no left/top/bottom inset on a surface that
  carries one. Containers yield; right padding is the only judgement call.
- **Chrome is flush**: a chrome-row control takes the row's full height, no margin, separated by
  a 1px rule rather than a gap. Content pads; chrome does not.
- **Screen areas are free functions**, not views: `fn(&AppState, &mut Context<AppState>) -> impl
  IntoElement`. A helper called in a loop returns `AnyElement` (Rust 2024 capture rules make
  `impl IntoElement` borrow the context).
- **The kit knows nothing about the workbench.** A `ui/kit/` function that names `AppState` has
  stopped being a primitive. Bridge with `ui::handler` / `ui::indexed`.
- **`gpui-component` first.** Reach for its `Icon`, `Kbd`, `Badge`, `Editor`, `Textarea`,
  `Scrollbar`, markdown view and dock before writing anything. `ui/kit/` holds only the gap.
- **Every new kit primitive gets a specimen** on the style reference, `ui/sink/style.rs`, in the
  same change.
- **Spacing from the framework's scale**; layout sizes are constants in `theme.rs`.
- **Status is shown by colour from the status group**, never by wording alone.
- **A row is one line.** Use `kit::elided` — it truncates and carries the whole string as a
  tooltip, which is why it takes an element id.

## GPUI gotchas that bite

- **A filling pane needs `flex_1` *and* `min_h(px(0.))`** (or `min_w`). One without the other is
  the standard refuses-to-shrink bug.
- **Scrolling needs `.id(...)`** — `.overflow_y_scroll()` does nothing without one. A scrollbar
  is an absolutely-positioned *sibling* of the scroll area, never a child.
- **A ULID-keyed row takes `ui::eid` / `ui::eid2`.** `ElementId`'s tuple form carries a `u64`;
  hashing a ULID into one collides silently. Enum-discriminant rows keep the tuple form.
- **A canvas layer fills its parent absolutely** and takes no click; one sized to its content
  paints into a strip at the top.
- **A key binding against a field must be registered late.** The library's input binds
  `up/down/left/right/enter/escape` in the `Input` context — the deepest node, and depth breaks
  ties. Bind each key twice: for your context, and for `YourContext > Input`, which matches at
  the same depth and wins by being registered after. `app::install_key_bindings` runs after
  `gpui_component::init` for exactly this. A handler with no answer calls `cx.propagate()`.
- **A panel may not read `AppState` outside a render** — the dock asks for visibility mid-update;
  the window pushes the answer instead.
- **Exactly one menu is open at a time** — a single `Option<MenuId>` on the workbench state. A
  trigger *opens*, never toggles.

## Overlays

| Shape | Drawn by | Is |
|---|---|---|
| Modal | `kit::modal` | One question. `MODAL_WIDTH`, ≤`MODAL_MAX_HEIGHT`, body scrolls, `surface_raised`, coloured left edge (`accent`, or `danger` for the irreversible), over `scrim`. Painted at the window origin via `deferred`+`anchored`, dismissed by outside click and its own close, scrim occludes the mouse. |
| Confirm / prompt | `kit::confirm_modal`, `kit::prompt_modal` | Built on `modal`. Never hand-roll these. |
| Sized modal | `kit::modal_sized` | Fixed-size flex body instead of a scroller — for a modal hosting a live `TerminalView` (harness login), so `flex_1`/`min_h(0)` resolves against a real height. |
| Dialog | `ui/file_picker.rs` | A modal with work in it: corner-grip resize clamped to `state/file_picker.rs`'s four sizes, drag tracked on the full-window layer, outside-click dismissal is the caller's choice. |
| Page overlay | `ui/settings.rs`, project settings | `SETTINGS_WIDTH`×`SETTINGS_HEIGHT`, a nav, does not resize; switching sections must not change the panel size. Furniture from `ui/kit/settings.rs`. |
| Anchored list | `ui/navigator.rs`, `kit::menu` | **Not** a modal: `anchored()` with no `.position()`, child of its trigger, no scrim, no outside-click dismiss. Key context and handlers go on the **field**, not the panel. |

**Escape is the window's, not the modal's.** `AppState::cancel_dialog` in `app/shell.rs` reads
the paint order top-down and peels **one** layer. A modal raised in `ui::shell` without a rung in
that list is a modal Escape walks past — `crates/ubiq/tests/dismiss.rs` asserts this.

## Adding a screen area

1. State in `state/` — data plus mutators, and no component-library type **unless the widget's
   state is the model**. That is the whole rule, and several modules qualify: `state/editor.rs`,
   `state/search.rs` and `state/a2ui/live.rs`, which holds one `Entity<InputState>` per bound field
   plus its `Subscription`. A type held for convenience rather than because it *is* the model does
   not qualify.
2. A field on `AppState` (or `OpenProject` if it belongs to a project), and a mutator ending in
   `cx.notify()`. Any `InputState` the window needs is an `Entity` field with its subscription
   pushed onto `_subscriptions`.
3. `ui/<area>.rs` as `fn render(&AppState, &mut Context<AppState>)`. Hang off `shell.rs` for
   chrome, or give it a `PanelKind` and an arm in `ui::dock::body` for a panel.
4. `gpui-component` first; a second caller wants the same thing → `ui/kit/` + a specimen.
5. Tokens, constants, no radii, coloured left edge.

## Reference files

- [`reference/file-map.md`](reference/file-map.md) — every module under `state/`, `app/`, `ui/`
  and `tests/`, with what it holds.
- [`reference/kit-and-theme.md`](reference/kit-and-theme.md) — every token accessor, every
  layout constant, and every `kit` primitive with where it lives.

## Verifying

`just verify` (= `check clippy test host ui docs-lint`) is what a change has to pass. `just ui`
alone checks the crate boundary; `just fmt` before committing. UI tests live in
`crates/ubiq/tests/` and are state-level — they drive `AppState` and assert on state, not pixels.
