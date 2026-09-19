---
id: inbox-personalisation
kind: proposal
status: proposal
title: Proposal — personalisation, two size axes and author-made themes
summary: Sizing is four unrelated mechanisms today — a density factor over five constants, three independent type bases, a per-project content size reachable only from a status-bar px dropdown, and twenty-six frozen layout constants — so growing the text does not grow the boxes around it. The proposal collapses them into two axes a slider each can drive, a UI scale that moves every dimension and a text ratio that moves type within it, names the pair as a saveable preset, makes every appearance value one setting for all of Ubiq by moving the content size out of per-project view state, and opens the palette registry to author-made themes as a fork plus a sparse override map.
read_when: you are changing a size constant, the density factor, the type scale, the status bar's size control, the Appearance settings section, or asking how a user would author a theme
updated: 2026-09-19
depends_on: [tech-ui, feat-workbench, tech-decisions, inbox-config, inbox-component-reuse]
---

# Proposal — personalisation, two size axes and author-made themes

Ubiq has four sizing mechanisms and no relationship between them. `Density` is a factor of 0.9,
1.0 or 1.15 applied to exactly five constants. `TextScale` carries three independent bases —
chrome, content, conversation — of which content is per-project and the other two are
per-interface. Twenty-six layout constants under "Dragged regions" in
`crates/ubiq/src/theme.rs` are frozen by test. And the fifty-odd paddings, gaps and field heights
written as `px_3` or `h(px(28.))` follow nothing at all.

The user-visible consequence is that the three controls do not compose. Raising chrome text in
Settings grows the glyphs inside a 34px titlebar that did not move; the status bar's font-size
dropdown says `13 px` and changes the editor, the terminal, the explorer tree and the search
results but nothing they sit in; density moves five chrome heights and the text inside them stays
put. A user who wants "everything bigger" has to find three controls, and still cannot get it.

**One fact makes the fix far cheaper than it looks: GPUI's whole Tailwind-shaped spacing scale is
already rem-relative.** Every `p_3`, `gap_2`, `w_4` and `h_8` expands to `rems(...)` in
`gpui_macros`, and `gpui_component::Root` sets the window's rem size from its own
`Theme::font_size` on every paint — a value Ubiq never writes, so it has sat at 16px forever.
Writing that one number is how several hundred hand-placed spacings, and every
`gpui-component` internal that measures in rems, start scaling with no call-site change at all.

## 1. Two axes, and what each one moves

| Axis | Stored as | Range | Moves |
|---|---|---|---|
| **UI scale** `S` | `InterfacePrefs.ui_scale: f32` | 0.80 – 1.40, default 1.0 | every dimension: the window's rem size, all `theme.rs` px constants, icon sizes, terminal padding, and the type bases through their own base |
| **Text ratio** `T` | `InterfacePrefs.text_ratio: f32` | 0.85 – 1.20, default 1.0 | type only, on top of `S` |

The derivation is one line, and it is the whole model:

```rust
// geometry
pub fn scaled(base: f32) -> f32 { (base * ui_scale()).round().max(1.0) }
// type — TEXT_BASE 13.0, Family::ratio() Chrome 0.96 / Content 1.00 / Conversation 0.96
pub fn font(family: Family, role: Role) -> Pixels {
    let base = TEXT_BASE * ui_scale() * text_ratio() * family.ratio() * role.ratio();
    px((base * content_trim() * 2.0).round() / 2.0)
}
```

At `T = 1` every proportion is held exactly: the text and the box grow by the same factor, so the
optical result is the current window at a different distance. `T` alone is the second thing a user
actually wants — denser or airier text inside the same furniture — and it is the only thing that
changes a proportion, which is precisely what the second slider is for.

**Density is retired into `S`, not kept beside it.** Two grid controls is the defect this proposal
exists to remove. `Density::Compact | Regular | Comfortable` migrate to `ui_scale` 0.9 / 1.0 / 1.15
and the enum goes. The three built-in presets keep those names, so nothing disappears from the
user's vocabulary.

**The content size stops being per-project.** Appearance is one setting for all of Ubiq, and
`content_font_size` is the last thing that is not: it lives in `ViewPrefs`, in a project's
`view.toml`, so the same window renders the same editor at two sizes depending on which project is
adopted, and `ui/shell.rs` has to push the active project's value into the process-wide scale at
the top of every paint. That push goes away. The field becomes
`InterfacePrefs.content_trim: f32` (default 1.0), a multiplier applied to `Family::Content` only,
and `cmd-=` / `cmd--` nudge it by ±0.05 rather than ±1 px — so zoom keeps working, and now works
with no project open, which it cannot today.

## 2. What has to become scalable for a proportion to hold

The two sliders are a day's work; making them *accurate* is the proposal.

- **The rem bridge.** `theme::dress_component_library` writes `Theme::font_size = REM_BASE * S`
  (`REM_BASE` 16.0, the value in force today, so `S = 1` is a no-op) and
  `Theme::mono_font_size` from the content base. This is the single highest-leverage change in the
  document, and it must land first so the rest can be measured against it.
- **Every px constant in `theme.rs` becomes a `scaled()` accessor.** Today five are. The
  exceptions are a true 1px hairline — a rule must not blur — and `MODAL_MAX_HEIGHT`, a ratio.
- **The dragged-region rule is re-stated, not broken.** Those twenty-six constants are the size a
  *fresh* window opens a region at; the user's dragged size lives in the dock arrangement blob.
  Scaling the default is correct and scaling the stored value is not, so the accessor scales and
  the blob is left alone. The test at `theme.rs` changes from "a dragged region is not scaled" to
  "a dragged region's *stored* size is not scaled".
- **Icon sizes get accessors.** They are literal `px()` at call sites or the component library's
  discrete `Size` enum, neither of which moves. `theme::icon_sm/icon_md/icon_lg` return scaled
  pixels fed to `Size::Size(px)`, and `just ui` grows a lint for a literal icon size, matching the
  one it already runs for `text_size(px(...))`.
- **The fourteen existing `text_size(px(N))` violations go through `font()`.** They are all
  content-family zoom plumbing in `crates/ubiq/src/ui/search.rs`, `ui/outline.rs`,
  `ui/kb/mod.rs`, `ui/viewer/mod.rs` and `ui/viewer/markdown.rs`, and the ad-hoc `- 0.5` deltas
  beside them are a `Role` written by hand. A `Role::Dense` at 0.96 replaces all of them.
- **Terminal re-dress is debounced.** A slider drag changes `terminal_padding()` and the content
  base on every frame; each change rebuilds the emulator config, which re-measures the cell grid
  and emits `TerminalResize` to the harness. Dragging a slider must not send the harness two
  hundred resizes. The existing debounce pattern beside `schedule_markdown_reflow()` is the model,
  and the same debounce covers the `SetPreferences` write.

## 3. The control — a size popover, replacing the px dropdown

The status bar's `font_size_dropdown` and its `10 … 32 px` ladder go. In its place, an icon-only
trigger opens an anchored panel (`kit::context_panel`, not a modal — no scrim, dismissed by
outside click) holding four things, in this order:

1. **The presets**, a row of `kit::choice_pill`. The lit one is the active preset, or none is lit
   and the trigger's tooltip reads *Custom*.
2. **The interface slider**, a small-window icon at the low end and a large-window icon at the
   high end. No number, no unit, no px.
3. **The text slider**, the same glyph small at the low end and large at the high end.
4. **Save preset…**, a `kit::ghost_button` raising `kit::prompt_modal` for the name, and
   **Reset**, which returns both axes to 1.0.

**A preset is a name and the two numbers, and nothing else.** `SizePreset { name, ui_scale,
text_ratio }` in `InterfacePrefs.size_presets: Vec<SizePreset>`. Resisting the urge to let a preset
also carry a palette is what keeps the popover one idea; a user who wants both saved is asking for
a workspace, which is a different document.

Two new things the kit owes this:

- **`kit::slider`** — Ubiq's kit has no continuous control at all; `kit::meter` displays and
  `kit::stepper` is discrete. `gpui-component` ships `Slider` and `SliderState`, unused, and
  adopting it is exactly the case `component-reuse-proposal.md` argues for: real drag, keyboard
  and accessibility behaviour Ubiq would otherwise write. The kit wrapper adds the palette, the
  leading and trailing icon slots, and `.step()` so the value snaps to a ladder of nine stops
  rather than landing on 1.0374. The two `Entity<SliderState>` handles live on `AppState` with
  their subscriptions — the widget's state *is* the model, which is the stated exception to the
  no-component-types-in-`state/` rule.
- **Four icons** — small interface, large interface, small glyph, large glyph — drawn through the
  `ubiq-icons` loop and added to the registry. The sliders are otherwise unlabelled controls,
  and every one of those carries a tooltip.

`MenuId::FontSize` becomes `MenuId::Size`. `nearest_font_index()` goes with the ladder.

## 4. Settings — Appearance splits, and gains a theme editor

Appearance is already ten rows in a fixed 820×560 panel. It splits:

- **Size** (new nav section) — the *same* `kit` sliders and preset pills the popover renders, plus
  what does not belong in a popover: the preset list with rename and delete, the three per-family
  trims for a user who wants conversation text larger than chrome, the content trim, and Reset. The chrome/conversation `BASE_SIZES` pill ladders and the density
  pills are removed; the read-only "Content text size" row becomes the trim.
- **Appearance** — palette, ground, accent, and a new **Themes** row: the author-made themes as
  pills beside the built-ins, an edit affordance on each, and **New theme…**.

### The theme editor

**A custom theme is a fork of a built-in palette plus a sparse override map, never a full
palette.** `PaletteDef` is deliberately shaped to refuse a partial palette — a family cannot ship
one missing a token — and that invariant is worth keeping for the built-ins. Resolution becomes
`palette_for(base)` → apply overrides → `with_accent(seed)`, so an author supplies one colour or
sixteen and the other thirty-odd stay coherent with whatever they forked.

```rust
struct CustomTheme {
    id: String,              // "custom-<ulid>"
    name: String,
    base: ThemeId,           // the built-in it forks
    overrides: BTreeMap<String, u32>,   // token name -> 0x00RRGGBB
}
```

Stored in `InterfacePrefs.custom_themes`, so a theme travels in `preferences.toml` with everything
else the interface remembers and needs no new store, no new message and no host change.

The editable set is the **grounds and the ink** — the eight surfaces, the four text colours, the
two borders, and the accent seed. Status, ribbon, terminal and project-swatch tokens inherit from
the fork: they encode meaning rather than taste, and `D19` already keeps project swatches outside
the accent axis for the same reason. The editor is a `kit::modal_sized` with the token list
grouped as `theme.rs` groups them on the left, a colour picker on the right, and a live specimen
strip below — and that strip should be `ui/sink/style.rs`'s existing `tokens()` renderer, not a
second one.

**The colour picker is promoted, not written.** The HSV grid, hue strip and hex field in
`ui/sink/project.rs` are already the right control and are already the only hand-built one; they
move to `kit::colour_picker` with the project-settings call site as the first of two callers.
`readable_on()` and `contrast()` already exist and already implement the WCAG loop the accent axis
uses, so the editor warns — never blocks — when a text token falls under 4.5:1 against the surface
it is paired with.

## 5. What this deliberately does not do

- **Nothing here is per-project, and one thing stops being.** Appearance is a property of the
  person, not of the folder they opened. The theme is already a process-wide thread-local shared by
  every window — `ui-and-design.md` states a second window opens in the first's palette — and both
  new axes join it in `InterfacePrefs`, as does the content trim that used to sit in `ViewPrefs`.
  A project keeps its *identity* colour, the swatch in the rail, which `D19` already holds outside
  the theme axis for exactly this reason.
- **No system-follow.** Nothing in the tree observes `WindowAppearance` and no gap claims it.
  It is a real omission, but it is an orthogonal one and mixing it in would hide it.
- **No theme import/export file.** A theme is a TOML table an author can already copy out of
  `preferences.toml`. A share format is worth doing once two people ask, and costs a schema.
- **No sliders for the per-family trims.** Three more continuous controls for a preference most
  users will never touch is a worse settings page. They stay pills.
- **The library's own text follows `S` but not `T`.** Rem is a length, so it cannot carry a
  type-only ratio. Ubiq draws nearly all of its own text through `font()`; what is left is input
  fields, the dock's tab labels and the table — narrow enough to record as a gap rather than
  design around.

## 6. Migration

`prefs::SCHEMA` moves 4 → 5, because existing fields change meaning rather than appear. `density`
maps to `ui_scale`; `chrome_font_size` and `conversation_font_size` become
`text_ratio = chrome_font_size / (TEXT_BASE * Family::Chrome.ratio())` with the conversation value
folded into its family trim.

**A blob is upgraded, never discarded.** `decode()` today throws a mismatched blob away whole, and
doing that here would silently reset every user's appearance — a migration arm per schema step is
the price of moving these fields at all.

**Content size moves scope, which is the one awkward step.** It is written once per project, in
each `projects/<ulid>/view.toml`, and it is becoming a single interface value, so the upgrade has
to pick. It takes the value from the most recently opened project, writes
`content_trim = size / TEXT_BASE` into `InterfacePrefs`, and leaves the per-project field parsed
and ignored so a downgrade still reads. `ViewPrefs` bumps its own schema in the same change and
drops the field on its next write. A user who had genuinely different sizes per project loses that
distinction — which is the point of the correction, and is worth saying out loud rather than
discovering.

## 7. Phases

- **P1 — the scale layer.** `ui_scale` and `text_ratio` in `theme.rs`, the rem bridge, `scaled()`
  over every constant, icon accessors, the `Role::Dense` sweep, the `just ui` lint, the migration.
  No new control: the existing Settings pills are repointed at the new axes, so the whole change
  is verifiable before anything is drawn.
- **P2 — `kit::slider`** plus its specimen on the style reference, and the four icons.
- **P3 — the size popover**, presets, the naming prompt, and the Size settings section.
- **P4 — `kit::colour_picker`**, `CustomTheme`, and the theme editor.

P4 is independent of P1–P3 and can be dropped or deferred without stranding them.

## 8. Decisions and gaps this would add

- **D151** — sizing is two axes, a UI scale over every dimension and a text ratio over type within
  it; density is retired into the first. *Cost:* one stored number now moves two hundred call
  sites at once, so a bad rounding rule is visible everywhere rather than in one panel.
- **D152** — a custom theme is a fork of a built-in palette plus a sparse override map. *Cost:* a
  forked theme drifts when its base palette is retuned, and an author cannot build one from
  nothing.
- **D153** — the window's rem size is the UI scale. *Cost:* Ubiq inherits `gpui-component`'s
  spacing judgement wholesale, and a library upgrade that re-tunes a rem value moves Ubiq's layout.
- **G303** — library-drawn text tracks the UI scale but not the text ratio.
- **G304** — icon sizes are literal pixels at call sites, with no lint, until P1 lands.

## Related docs

- [`../tech/ui-and-design.md`](../tech/ui-and-design.md) — owns the token set, the type scale and
  the constants table this rewrites.
- [`../tech/decisions.md`](../tech/decisions.md) — `D96` on the palette registry is the precedent
  D152 extends, and carries the cost paragraph to answer.
- [`./component-reuse-proposal.md`](./component-reuse-proposal.md) — why the slider is adopted
  rather than written.
- [`./config-persistence-proposal.md`](./config-persistence-proposal.md) — where
  `preferences.toml` sits and what an interface blob may carry.
