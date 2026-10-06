# Code organisation audit — `crates/ubiq` (GPUI interface)

Read-only survey, 2026-10-05. Paths are relative to `crates/ubiq/` unless noted. Counts are from
grep/wc and approximate where marked `~`. Inline-test counts include `#[cfg(test)]` modules.

---------------------------------------------------------------------------------------------------

# Section A — Factual snapshot

## A1. Module tree and size

219,485 lines in 296 `src` files (+ 60 integration-test files, 38,077 lines).

| Module | Lines | Files | Role |
|---|---|---|---|
| `src/ui/` | 80,748 | 150 | Drawing: free functions `fn(app: &AppState, window, cx) -> AnyElement`, plus a few real `Render` views (`WorkbenchPanel`, `MdView`, `ResultGrid`, `CellInput`, `JsonEditor`) |
| `src/app/` | 47,658 | 51 | `AppState` (the root view) and ~50 `impl AppState` blocks across 46 files: bus receive, actions, boot |
| `src/state/` | 45,478 | 78 | Plain data/logic per feature; 14 of 78 files import gpui |
| `src/theme.rs` | 3,635 | 1 | Palettes (both modes), font roles, size consts + accessors (171 `pub fn`, 519 hex literals) |
| `src/web_export/` | 2,499 | 6 | Static HTML export + embedded HTTP server (not UI) |
| `src/ext/` | 1,306 | 8 | Extension spine: `Registry<T>`, `SlotId`, specs for rail / settings / menu / runner |
| `tests/` | 38,077 | 60 | Integration tests (630 `#[test]`) |

Sub-module sizes (lines): `ui/mdview` 8,350 · `ui/sink` 7,456 · `ui/db` 6,157 · `ui/settings.rs` 6,052 ·
`ui/conversation` 4,557 · `ui/kit` 4,439 · `ui/board` 3,826 · `app/wire.rs` 3,661 · `ui/viewer` 3,626 ·
`app/db` 3,441 · `ui/mission` 3,316 · `state/db` 3,218 · `app/settings.rs` 3,042 · `state/a2ui` 2,777 ·
`ui/dock` 2,478 · `ui/git` 2,086 · `ui/teams` 2,022.

**25 largest files**

| # | Lines | File |
|---|---|---|
| 1 | 6,052 | `src/ui/settings.rs` |
| 2 | 4,160 | `src/ui/conversation/mod.rs` |
| 3 | 3,661 | `src/app/wire.rs` |
| 4 | 3,635 | `src/theme.rs` |
| 5 | 3,042 | `src/app/settings.rs` |
| 6 | 2,939 | `src/state/conversation.rs` |
| 7 | 2,668 | `tests/conversation.rs` |
| 8 | 2,317 | `src/ui/sink/project.rs` |
| 9 | 2,218 | `src/app/editor.rs` |
| 10 | 2,168 | `src/app/mod.rs` |
| 11 | 2,121 | `tests/settings.rs` |
| 12 | 2,060 | `src/app/boot.rs` |
| 13 | 1,940 | `src/app/remote_connect.rs` |
| 14 | 1,903 | `src/state/layout.rs` |
| 15 | 1,869 | `src/ui/board/form.rs` |
| 16 | 1,802 | `src/app/agents.rs` |
| 17 | 1,765 | `tests/git.rs` |
| 18 | 1,750 | `src/ui/new_agent.rs` |
| 19 | 1,750 | `src/app/shell.rs` |
| 20 | 1,671 | `src/state/sink.rs` |
| 21 | 1,666 | `tests/plan.rs` |
| 22 | 1,635 | `src/state/workbench.rs` |
| 23 | 1,625 | `tests/files.rs` |
| 24 | 1,603 | `src/app/explorer.rs` |
| 25 | 1,577 | `src/app/projects.rs` |

**Files > 2000 lines: 12 in `src`** (all above except tests) + 2 in `tests/` = 14 total counting
`tests/`. `src/ui/settings.rs` alone is 2.7% of the crate.

**Longest functions** (awk scan, > 250 lines):

| Lines | Where | Note |
|---|---|---|
| **2,048** | `app/boot.rs:9` `AppState::for_project` | the constructor; builds 90 `InputState`s inline and wires ~217 fields |
| 636 | `ui/file_dialog.rs:40` `render` | |
| 590 | `ui/a2ui.rs:160` `draw` | big component match |
| 580 | `ui/shell.rs:29` `render` | root layout/overlay stacking |
| 478 | `app/wire.rs:1575` `receive_work` | message match |
| 470 | `ui/teams/graph.rs:86` `render` | |
| 447 | `ui/new_agent.rs:619` `body` | |
| 444 | `ui/dock/skin.rs:383` `render_tab_bar` | |
| 387 | `ui/titlebar.rs:22` `render` | |
| 327 / 309 / 300 | `app/wire.rs` `receive_file` / `receive_conversation` / `receive_account` | |
| 318 | `ui/rail.rs:70` `modes` | ten `RailModeSpec` literals |

**Layering.** Single crate, three folders that all see each other:
`app` (state holder + handlers) -> `state` (data) and `ui` (drawing). Nominal direction is
`ui -> app::AppState -> state`, but:

- `ui` names `AppState` in 93 files (533 `&AppState`/`&mut AppState` mentions in 101 files). UI is
  not a set of views over their own state; it is functions over the one god-struct.
- `state` imports `crate::ui` in 11 files and `crate::app` in 13 (e.g. `state/editor.rs:573` holds
  `Entity<ui::mdview::view::MdView>`, `state/db/sql.rs:28` names `ui::db::sql::plan::PlanState`).
  Cycle `ui <-> state` exists at module level.
- `app` imports `crate::ui` in 16 files (handlers pull draw-side constants and helpers).

**UI layering** (top -> bottom): `ui/shell.rs` (root layout) -> `ui/titlebar|rail|status_bar|dock` ->
screens/panels (`ui/board`, `ui/conversation`, `ui/git`, `ui/db`, `ui/mission`, `ui/teams`, `ui/kb`,
`ui/viewer`, `ui/mdview`, `ui/settings.rs`...) -> overlays/modals (`ui/new_agent.rs`, `ui/clone.rs`,
`ui/file_dialog.rs`, `ui/ask.rs`, `*_menu.rs`) -> `ui/kit` (4,439 lines: `controls` 741, `menu` 1,039,
`overlay` 362, `panel` 174, `popover` 69, `blocks`, `settings`, `files`, `canvas`, `colour`,
`slider`, `ribbon`, `icons`). `ui/sink/` (7,456 lines) is the "kitchen sink" style reference
screen; it duplicates screen shapes as fixtures (see A4).

**mod.rs vs file-module.** 25 `mod.rs` files vs 271 file-style modules; mod.rs files hold real code
(`ui/conversation/mod.rs` 4,160, `app/mod.rs` 2,168, `ui/dock/mod.rs` 1,573, `ui/board/mod.rs` 1,267).
The newer subtree style (`ui/mdview`, `ui/db`, `state/explorer`) is directory + small files; the
older big screens are monoliths (`ui/settings.rs`, `ui/new_agent.rs`).

**Visibility.** Everything is `pub`: 3,461 `pub fn`, 371 `pub struct`, 202 `pub enum`, 236 `pub mod`;
0 `pub(crate) fn`, 0 `pub(crate) mod`. `pub(super)` 285 uses (almost all in `app/` — 248 restricted
fns vs 1,254 `pub fn` there), `pub(crate)` 77. `AppState` has 217 fields, 160 of them `pub`. Only the
`ubiq` -> `ubiq-app` boundary matters for `pub` in practice, so `pub` gives no signal of intended
API vs internal helper.

## A2. State and event flow

**State holder.** One root `Entity<AppState>` per window (`app/mod.rs:668`, struct spans 720 lines, 217
fields, 7 of which are feature `*State` structs; the rest are flat fields and pending flags). Feature
state lives in `src/state/<feature>.rs` structs owned as fields of `AppState`; `ui` reads
`app.<field>`. Real child entities are rare:

| Entity type | Uses | Notes |
|---|---|---|
| `Entity<InputState>` | 136 | input widgets from gpui-component, 90 built in `boot.rs` before first frame |
| `Entity<AppState>` | 64 | passed into closures |
| `Entity<TextareaState>` / `EditorState` | 27 / 24 | |
| `Entity<MdView>` / `WorkbenchPanel` / `DockArea` | 19 / 17 / 15 | the only structured views |

`impl Render` exists for only **12** types (3 are `Ghost`/`Empty` drag stubs); `#[derive(IntoElement)]`
/ `RenderOnce` 6. So "a panel" is mostly a free function `fn panel(app, window, cx) -> AnyElement`
rebuilt every frame from `AppState`; one `cx.notify()` repaints everything (1,228 `cx.notify()`
calls in ui/app/state; 868 `cx.listener`; 183 `ui::handler(` bridges, `ui/mod.rs:83`).

**Host messages.** `app/hosts.rs` wraps `ubiq_proto` clients (`client.send(message)` at
`app/hosts.rs:336`); inbound messages are drained and passed to `AppState::receive(host, message, cx)`
(`app/wire.rs:522`), a chain of `receive_<family>` fns: pane, project, file, git, kb, work,
conversation, notifications, session, account, search, assist in `wire.rs` (3,661 lines) plus
`receive_ask/repo/feedback/help/host_browse/tasksrc/web_assets` in their own app files. Each
`receive_*` mutates `AppState` fields and calls `cx.notify()`. Background wakeups: `cx.spawn` loops in
`app/boot.rs:1714-1770` (log nudges, 120 ms / 150 ms / 4 s timers). 95 `cx.subscribe/observe` calls.

**Deferred work in `Render`.** `AppState::render` (`app/shell.rs:1671`) starts with ~20 `settle_*`/
`attach_*`/`fill_*`/`build_*` calls whose order is "load-bearing, commented" (`_docs/wip/refactor-plan.md`
says a pending-action queue was deliberately not built). State changes are therefore made inside the
render pass.

**Actions and key bindings.** `gpui::actions!` in 10 places (`app/mod.rs:152`, `ui/explorer.rs:92`,
`ui/navigator.rs:27`, `ui/file_picker.rs:45`, `ui/run_tool_menu.rs:43`, `ui/db/keys.rs:16`,
`ui/db/explorer.rs:46`, `ui/db/json.rs:375`, `ui/mdview/blockedit.rs:38`, `ui/mdview/outline.rs:29`);
108 `KeyBinding::new` calls registered in 11 files, each next to its context. There is no central keymap
table, no user-rebindable layer in this crate (`state/prefs.rs` has view prefs only).

**Global state.** `impl Global`: `BusHub`, `OpenWindows`, `WindowRegistry` (3). Plus ~16
`thread_local!`/`OnceLock` statics (`theme.rs` x5 for the palette/scale; caches in `ui/viewer/*`,
`ui/mdview/search.rs`, `ui/ident.rs`; TLS roots in `app/remote_connect.rs`; `web_export/server.rs`
`SERVER`).

**Registries (extension spine, `src/ext`).** `Registry<T: Slotted>` with four containers:
`RailModeSpec` (`ext/rail.rs`, 15 fields: icon, ui_id, availability, centre, default_layout,
destination, furniture, side_furniture, on_enter...), `SettingsSectionSpec`, `MenuBlockSpec`,
`RunnerKindSpec`. Base specs: `ui/rail.rs:70` `modes()` (ten literals, 318 lines) and
`ui/sink/ext_demo.rs` (the demo mode). Ids in `ext/ids.rs` (155 lines, `const SlotId`), checked by
`just slots-check`.

**What it takes to add things (measured by reference spread).**

| To add | Places that change |
|---|---|
| A rail mode | `ext/ids.rs` (id), `ui/rail.rs` (spec literal ~30 lines), a `PanelKind` variant if it has a side panel, `UiId` entries (`state/ui_id.rs`), `state/dock.rs` regions/defaults, the dock renderer match (`ui/dock/mod.rs`), view prefs decoding. **Registry-based: genuinely pluggable for the centre/destination, not for panels.** |
| A panel kind | `PanelKind` (`state/dock.rs`, ~26 variants; 129 refs in that file, 75 in `ui/dock/mod.rs`, 26 `app/panels.rs`, 18 `app/editor.rs`, 12 `app/shell.rs`, 10 `ui/rail.rs`...) — 5-13 files per variant (Chat 13, Terminal 10, Kb 8). No registry: closed enum + match arms in ~6 modules (title, icon, render, serialise, region rules, close/focus). |
| A document kind (centre tab) | `PanelKind` variant + `state/<x>.rs` + `app/<x>.rs` attach/settle fn in `render` + `ui/dock` arm + persistence in `state/layout.rs` (1,903). `DocumentBody` (`state/document.rs:49`) covers only the annotation surface. |
| A settings section | `ext/settings.rs` spec + `ext/ids.rs` + `ui/settings.rs` body fn (6,052-line file) + `app/settings.rs` handler (3,042) + `state/settings.rs`; tests in `tests/settings.rs` |
| A bar menu block | `ext/menu.rs` registration + `ui/menus.rs` builder (data-driven, see A3) |
| A bus message family | `ubiq-proto` + new `receive_<family>` + chain entry in `wire.rs:522` |
| A modal/overlay | new `ui/<x>.rs` fn + field + open/close flags on `AppState` + hook into `ui/shell.rs:29` stack (580-line fn); 10+ distinct `overlay()` fns today |

## A3. Styling vs logic

**Mechanism.** Inline GPUI builder chains in every `ui/*` fn; colours via `theme::<token>()` functions
(2,745 `theme::xxx()` calls and 471 `theme::UPPER` const uses in `ui`); text size via
`theme::font(Family, Role)` (713 uses, `theme.rs:1005`); icon sizes `theme::icon_{sm,md,lg}()`
(`theme.rs:94-106`); layout consts in `theme.rs` (TITLEBAR_HEIGHT 34, STATUS_BAR_HEIGHT 30, RAIL_WIDTH
56, EXPLORER_WIDTH 300, CHAT_WIDTH 420, ... ~35 consts, each with a `scaled()` accessor; 45 `scaled(`
calls). Rule "no literal colour outside theme.rs" holds: **0** `0x......` hex literals in ui/app/state;
`rgba?(`/`hsla?(` outside theme: 6 in ui, 19 in app, 8 in state — all `.rgb()` method names or
`stroke_hsla` (`ui/viewer/image_edit.rs:48`), i.e. false positives. Colours are clean.

**Spacing and sizes are not tokenised.**

| Pattern in `ui/` | Count |
|---|---|
| `px(` literal total | 1,566 (app 27, state 23, theme 20) |
| of which `px(0.)` | 508 (476 are `min_w(px(0.))`/`min_h(px(0.))` flex idiom) |
| `.h/.w/.min_w/.max_w/.min_h/.max_h(px…)` | 990 |
| Tailwind-style `.p_2 .gap_1 .mx_2 …` | 1,416 (GPUI rem-based) |
| `.text_size(px(` raw | 11 (vs 713 via `theme::font`) |
| `rems(` | 2 |
| `.bg(` calls | 428 |

Top literal heights: `px(30.)` x47, `px(26.)` x38, `px(28.)` x32, `px(16.)` x19, `px(24.)`/`px(22.)` x18,
`px(8.)` x17, `px(20.)` x15, `px(34.)` x12. These are *row heights* — same concept (button row, menu
row, list row) hand-typed at 5 values. Per module `px(`: `ui/sink` 152, `ui/kit` 141, `ui/db` 110,
`ui/mdview` 94, `ui/viewer` 78, `ui/conversation` 76, `ui/board` 72, `ui/git` 70, `ui/mission` 59,
`ui/orchestration` 55, `ui/settings.rs` 52, `ui/teams` 48, `ui/dock` 43. Because raw `px()` is
unscaled and rem spacing is scaled, **the UI-scale setting (`theme::scaled`, `ui_scale` 27 uses)
treats literal-px and rem layout differently**; only the ~35 tokens + `theme::font` scale cleanly.
Style constants as module-local `const X: f32/Pixels` exist (e.g. `ui/mark.rs` 16, `ui/conversation/mod.rs`
8, `ui/mdview/*` 6-7 each), not shared. No typed spacing scale, no row-height token, no `Density`.

**Widget helpers.** `ui/kit` is the shared skin (`icon_button` 60 sites, `ghost_button` 97, `panel` 49,
`slab`, `field`, `pill`, `card`, `modal*`, `confirm_modal`, `prompt_modal`, `popover`, `tab_strip`...).
Per `_docs/inbox/component-reuse-proposal.md` only 7 gpui-component parts are used (editor, input, dock,
text view, tooltip, scrollbar, keycap). Local clone helpers: `fn row` defined 14x and `fn header` 13x
in different modules, `fn label` 4x, `fn tab` 4x, `fn chip` 3x, `fn button` 3x (names only; bodies
differ in height/padding).

### Menus and context menus (specific study)

Six separate mechanisms, only one is a data model separated from rendering:

| # | Mechanism | Where | Model | Pick resolution |
|---|---|---|---|---|
| 1 | **Bar menus** `MenuEntry` | `ui/menus.rs` (520) + `ext/menu.rs` (126) | **Data**: label, icon, tooltip, detail, disabled, **own action closure** (`Rc<dyn Fn(&mut AppState,…)>`); `Registry<MenuBlockSpec>`, one walker `overlay()` draws all | by closure (robust). `overflow_menu.rs`, `new_pane_menu.rs`, `new_project_menu.rs`, `hidden_agents_menu.rs` are 33-35-line stubs on it |
| 2 | **Context menus** `kit::ContextItem` + `context_menu/context_panel` | `ui/kit/menu.rs:853-1039` | Data (label/detail/icon/tooltip/enabled/separator) but **no action**; caller gets `on_pick(index)` | **by index into a list rebuilt by the handler** (`AppState::pick_*_menu`, `mission/menu.rs` says "matched by position"). 28 `ContextItem::new` in 13 call sites (agents, conversation, db/explorer, explorer, git x3, kb, mission x3, sink/style, tab_menu, teams) |
| 3 | **Dropdowns** `kit::Picker` / `MultiPicker` | `ui/kit/menu.rs:44-760` | `Vec<String>` + index sets (selected/disabled/separators/dim/dots/details) — parallel arrays | by index. ~45 call sites (settings 5, sink/settings 4, teams 3, sink/style 3, ...); no keyboard navigation (proposal §1.1) |
| 4 | **Project picker popover** | `ui/project_menu.rs` (794) | bespoke: own rows, filter, hover, outside-click, `deferred+anchored` | own |
| 5 | **Run-tool menu** | `ui/run_tool_menu.rs` (339) + own `actions!` | bespoke | own |
| 6 | **Tab menu / navigator / outline jump / help target** | `ui/tab_menu.rs` (140), `ui/navigator.rs`, `ui/mdview/outline.rs`, `ui/help_target.rs` | each its own `deferred+anchored` | own |

Facts: `deferred(` in 15 files, `anchored()` in 16 files; manual layer priorities
`MENU_LAYER = 1`, `MODAL_MENU_LAYER = 3` (`ui/kit/menu.rs:22,29`) plus a hard-coded `.priority(1)` in
`context_menu`; `MouseButton::Right` handled in 8 files, each opening its own state slot on `workbench`
(`workbench.mission_menu`, etc.). `context_panel` row height is hard-coded `px(28.)` and
`context_menu` duplicates `menu_panel` rendering (proposal calls them duplicates). Menu content in
git/explorer/tabs is computed in two places (draw and `pick_*`) tied by index. Docs claim
`gpui-component`'s `Root` popup layer is live but unused.

## A4. Hardcoded / placeholder values from the first version

`TODO` 4 (2 are doc comments in `state/new_agent.rs:264,878`; 2 are test strings), `FIXME` 0, `XXX` 0,
`unimplemented!` 0, `todo!` 2 (both inside a markdown test string at `ui/mdview/blockedit.rs:678`).
The tree is unusually clean of markers. The substance is elsewhere:

| What | Where | Comment |
|---|---|---|
| ~223 `placeholder` mentions; 111 in boot | `app/boot.rs:27-…` (`.placeholder("Go to file…")`, `"Commit message — subject, blank line, body"`) | user-facing strings baked into the 2,048-line constructor |
| `placeholder("claude")`, `"~/.cache/shared…"`, `"~/.ssh/id_ed25519"` | `app/boot.rs:186,364,499` | example paths/commands as literals |
| Hardcoded harness-name -> icon map | `ui/kit/mod.rs:28-34` (`"claude-code"`, `"codex-acp"`, `"opencode"`, `"GitHub Copilot"`, `"Grok CLI"`…) | adding a harness needs an edit in `ui`; contradicts "agent-manager owns all harness knowledge" (AGENTS.md); display-name strings used as keys |
| `"claude-code"` default agent type | `state/new_mission.rs:187`, `state/new_agent.rs:740`, `state/teamsim.rs:1009` | default harness literal in 3 places |
| `.contains("claude")` special-case | `state/conversation.rs:1354` | harness-sniffing by substring |
| `short_model_label` rules | `state/conversation.rs` (tests use `claude-haiku-4-5-20251001`, `gpt-5-codex`) | model families assumed |
| Harness/model fixtures `MENU_ITEMS = ["Claude Code","Codex","Gemini CLI","opencode"]`, `gpt-5`, `gemini-3-pro` | `state/sink.rs:250,334,342` | kitchen-sink fixtures; acceptable but lives in `state/` not `ui/sink` |
| Demo/sample data | `ui/sink/*` (7,456 lines), `ui/sink/ext_demo.rs` (17 "demo" refs, registered in `ext/rail.rs:187` `base_registry()` **in the production registry**), `state/sink.rs`, `state/teamsim.rs` (1,493), `state/teams.rs` fixtures | demo rail mode ships in the real registry; sink fixtures sit in `state/` |
| Timers / thresholds as bare literals | `app/boot.rs:1726,1758,1770` (120 ms, 4 s, 150 ms), `ui/conversation/mod.rs:673,1649,1680` (2000/1400/1100 ms animations), `ui/kit/controls.rs:446` (2400 ms); 26 `Duration::from_*(literal)` total, 7 named consts | spin duration also at `ui/mark.rs:66 SPIN` -> duplicated 2400 |
| Limits | `ui/outline.rs:31,35` (512 KiB, 2000 defs), `MAX_CELL_CHARS = 300` **duplicated** in `ui/db/results.rs:21` and `ui/db/grid.rs:43`, `app/remote_connect.rs:51,57,61` (6 s, 8 KiB, 200 ms) | |
| Panel sizes | `theme.rs:121-199` consts (EXPLORER_WIDTH 300, CHAT_WIDTH 420, DOCK_HEIGHT 300...) | tokens, but not user-adjustable except via ui_scale |
| Paths | non-test: only placeholder strings above; `/Users/mdn/...` only in tests (`ui/project_menu.rs:768-788`, `app/host_browse.rs`) | OK |
| `Menlo` / `Cascadia Mono` / `DejaVu Sans Mono` / `.SystemUIFont` | `theme.rs:19-27` | platform font defaults, fine |

## A5. Error handling and robustness

| Metric | ui | app | state | ext | web_export | theme |
|---|---|---|---|---|---|---|
| `.unwrap()` (incl. tests) | 8 | 8 | 35 | 0 | 86 | 0 |
| `.expect(` | 28 | 20 | 54 | 1 | 9 | 4 |
| `let _ =` | 15 | 46 | 8 | 0 | 28 | 0 |
| `.ok();` (discard) | 22 | 11 | 1 | 0 | 0 | 0 |

Production-path (before the first `#[cfg(test)]` in each file) unwrap/expect is small: `web_export/server.rs`
18, `app/remote_connect.rs` 7, `web_export/routes.rs` 6, `app/ssh_connect.rs` 2, `app/editor.rs` 2, and
1 each in 8 other files. `panic!` 35 (mostly in tests/assertions: `state/conversation.rs` 9,
`state/a2ui/path.rs` 4, `ext/registry.rs` 3 are **deliberate boot assertions** for duplicate slots,
`app/ssh_connect.rs` 3). 4 `unreachable!`. No `println!/eprintln!` in non-test code.

How errors reach the user: **no common mechanism**. Roughly 6 field families like
`project_error: Option<String>`, `*_error`, `notice` on `workbench`/`settings`/`kb`/`catalog`/
`tasksrc`/`web_panel`/`teamsim` (~150 error-ish assignments in `app/`), each rendered by its own panel
as a red line or banner. The bell (`state/notifications.rs`, host-owned list) is for host-originated
notifications, not for UI-side operation failures. `ubiq_proto::log::logs()` is the sink for logs.
Discarded results (`let _ =`, `.ok();`, 100+) are mostly channel sends and clipboard writes; no policy
documented.

## A6. Tests

| Kind | Count | Where |
|---|---|---|
| Inline `#[test]` | 570 in 67 files | state 266, ui 179, app 86, theme 15, web_export 13, ext 11 |
| Integration tests | 630 `#[test]` in 60 files / 38,077 lines | `tests/*.rs` (conversation 39 gpui-test uses, teams 27, new_mission 16, ...) |
| GPUI harness | `gpui` with `features = ["test-support"]` (`Cargo.toml:243`); `TestAppContext`/`cx.add_window` in `tests/{ask,new_mission,remote,conversation,teams,run_tool,...}.rs` (~900 harness-term mentions) | tests mount a **small purpose-built view/harness**, e.g. `ConversationHarness` (`tests/conversation.rs:1612`), not the real `AppState` render |

What is tested: pure state transitions, projections (conversation fold, layout/restore, nav, vim,
prefs, rail/settings container, ui_id hit-test), markdown/DB pure logic, message-in -> state-out for
chosen families. Biggest ui test islands: `ui/mdview/prose.rs` 24, `inline.rs` 21, `blockedit.rs` 16,
`blocks.rs` 13, `ui/db/*`. **Gaps:** 125 of 150 `ui/` files have no inline test; none of
`ui/settings.rs` (6,052), `ui/sink/project.rs`, `ui/board/form.rs`, `ui/dock/mod.rs` (1,573),
`ui/kit/menu.rs` (1,039), `app/wire.rs` (3,661), `app/settings.rs`, `app/editor.rs`, `app/boot.rs`
(the 2,048-line constructor) has a `#[test]`. `app/wire.rs` is covered only indirectly by tests/ that
push messages. No visual/layout test; no test that every rail mode/panel kind renders; no menu test
(draw list vs pick list consistency).

## A7. Reuse and duplication (evidence)

- **Match-heavy dispatch.** `receive_*` families (wire.rs 478/327/309/300-line fns); `PanelKind`
  matches in 6 modules; `ui/a2ui.rs::draw` 590-line match; `RailMode::` referenced 60x in 22 files.
- **Popover plumbing** (`deferred(anchored().position().snap_to_window…)` + outside-click dismiss) is
  re-implemented in ~15 files; `ui/kit/popover.rs` is 69 lines and `kit/overlay.rs` `modal` exists but
  `ui/mission/full.rs:54` defines its own `modal`, `ui/settings.rs:89` its own `dialog`,
  `ui/acp_capabilities.rs:34` another.
- **Filterable list**: `kit::Picker` search + `ui/navigator.rs` + `app/nav.rs` + file picker + project
  menu each carry cursor/filter logic (proposal item 3).
- **Hand-rolled grid** `ui/stats.rs:219-328`; DB grid (`ui/db/grid.rs`, `results.rs`) duplicates
  `MAX_CELL_CHARS` and sample logic.
- **Forms**: `ui/board/form.rs` (1,869), `ui/tools.rs:200 editor_form` (268), `ui/db/conn_form.rs:115`
  (261), `ui/kb/source_form.rs:91` (253), `ui/new_agent.rs` (1,750), `ui/sink/project.rs` (2,317) each lay
  out label + `InputState` + validation by hand; `kit/settings.rs` (273) helps only settings.
- **Local helper names**: `fn row` x14, `fn header` x13 across `ui/*`.
- **Colour picker** consolidated into `ColourField` (done, per refactor-plan).
- **`Entity<InputState>` boot pool**: 102 `InputState::new` (90 in boot) — each new text field means a new
  field on `AppState` and a line in `for_project`.

## A8. Strings / i18n

No i18n layer (no fluent/gettext/`t!`). User-facing strings are inline literals in `ui/*`,
`app/*` (placeholders, notices), `state/*` (labels via `fn label()`/`fn note()` on enums, e.g.
`MissionMenuRow::label()`, `RailModeSpec.label`/`.note`), ~70 `.child("literal")` and ~1,200
capitalised multi-word literals in ui/app. Partial centralisation: `RailModeSpec` fields, `MenuEntry`
data, `state/help.rs` + `ui/help` (`unavailable.md`) for in-place help keyed by `UiId`, `ui/kit/settings.rs`
row labels. Everything is English; `locale` is not read.

---------------------------------------------------------------------------------------------------

# Section B — Pain points, ranked

| # | Sev | Pain point | Evidence | Remedy |
|---|---|---|---|---|
| 1 | High | **`AppState` is a 217-field god object** and the only state object UI functions see; every feature adds fields, flags and an `impl AppState` block | `app/mod.rs:668-1387`; 50 `impl AppState` in 46 files; 93 ui files import it; 533 `&AppState` params | Per-feature `Entity<FeatureView>` with own state and `Render` (git, board, kb first); `AppState` keeps ids + routing |
| 2 | High | **`AppState::for_project` is a 2,048-line constructor** building 90 inputs and every substate | `app/boot.rs:9-2057`, 111 placeholder strings in it | Each feature owns `Feature::new(window, cx)`; boot only composes |
| 3 | High | **Menus: six unrelated mechanisms**, two of them (context, picker) resolve picks by *index into a list the handler rebuilds* — draw/pick drift risk, no keyboard nav, hand-set layer priorities | A3 table; `kit/menu.rs:22,29,929-1039`; `mission/menu.rs`; `project_menu.rs:121-660`; `deferred(` 15 files | Make `MenuEntry` (action-carrying, `ui/menus.rs`) the one model; build `context_menu`/`Picker` bodies on it (or on gpui-component `PopupMenu` as the reuse proposal says) and delete index matching |
| 4 | High | **God files in the UI**: 6,052-line `settings.rs`, 4,160-line `conversation/mod.rs`, 3,661 `wire.rs`, 3,042 `app/settings.rs`, 2,317 `sink/project.rs` — each with zero or few tests | A1 tables | Split by section into `ui/settings/<section>.rs` (one per `SettingsSectionSpec`); same for wire by family |
| 5 | High | **Layering inversions and a single crate hiding them**: `state` -> `ui` (11 files) and `state` -> `app` (13); `ui` and `app` mutually dependent | A1 "Layering" | Move view-bound types (`MdView`, `PlanState`, `GridDelegate`) behind traits/ids in `state`; consider `ubiq-state` crate to enforce |
| 6 | Medium-High | **Adding a panel/document kind touches 6-13 files** (closed `PanelKind` enum, ~26 variants; 129 refs in `state/dock.rs`); rail modes are registry-driven but their panels are not | A2 table | Extend the `ext` registry to `PanelSpec { title, icon, render, persist, region_rule }` and replace matches by lookup |
| 7 | Medium-High | **Sizes untokenised and unscaled**: 1,566 `px()` (≈1,050 non-zero), row heights hand-typed at 5 values (30/26/28/24/22), module-local consts duplicated; raw `px` ignores `ui_scale` while rem spacing scales | A3 stats | Add `theme::{row_h(Row::Compact/Normal/Tall), gap, pad}` + `min_w_0()` helper (476 `px(0.)`); sweep kit first |
| 8 | Medium | **Render-time mutation**: ~20 order-dependent `settle_*/attach_*/fill_*` calls at top of `render` | `app/shell.rs:1671-1700`; refactor-plan "deliberately not done" | Move to message/event handlers or a typed `PendingWork` drain with explicit phases |
| 9 | Medium | **Harness knowledge in the UI**: name-keyed icon map, default `"claude-code"` x3, `contains("claude")` | `ui/kit/mod.rs:28-34`; `state/new_mission.rs:187`; `state/conversation.rs:1354` | Take icon key/default from `AgentTypeInfo` supplied by agent-manager |
| 10 | Medium | **Demo/sink in production paths**: `ext_demo` rail mode registered in `base_registry()`; sink fixtures live in `state/` (`sink.rs` 1,671, `teamsim.rs` 1,493); 7.4k-line `ui/sink` always compiled | `ext/rail.rs:187`; A4 | Gate behind a feature/`cfg(debug_assertions)` and move fixtures next to `ui/sink` |
| 11 | Medium | **No unified error surface**: ~6 ad-hoc `*_error: Option<String>` fields, each with its own banner; 100+ silent `let _ =/.ok();` | A5 | One `UiError`/toast channel on the root (gpui-component `Root` notification layer already live) |
| 12 | Medium | **Test blind spots where risk is**: 125/150 ui files, wire, boot, settings, dock, menus untested; tests mount ad-hoc harnesses not real screens | A6 | Table-driven smoke test that renders each rail mode/panel kind in `TestAppContext`; menu draw-vs-pick consistency test |
| 13 | Medium | **Everything `pub`**: no `pub(crate)` for crate-internal APIs, 3,461 `pub fn` | A1 visibility | Default to `pub(crate)`; `pub` only for `ubiq-app`/Studio seams (`ext`, `ui::menus`, `theme`) |
| 14 | Low-Med | **Duplicate helpers/consts**: `MAX_CELL_CHARS` x2, spin 2,400 ms x3, `fn row` x14 / `fn header` x13, 3 modal/dialog frames, 10+ `overlay()` fns | A4, A7 | Move shared consts to `theme`/kit; fold copies into `kit::{row, section_header, modal}` |
| 15 | Low-Med | **Boot-time constants/user strings inline**: placeholders, timers (120 ms/150 ms/4 s), animation durations | `app/boot.rs`, `ui/conversation/mod.rs` | Named consts in the owning module; strings via a `ui/strings.rs` if i18n is ever wanted |
| 16 | Low | **No i18n path** | A8 | Not worth acting on until a second language is planned; keep strings next to their `label()`/spec |
| 17 | Low | **Key bindings scattered** (108 `KeyBinding::new` in 11 files; no rebinding) | A2 | One `keymap.rs` table that `ui` modules contribute to via a registry |

Notes on what is already good (keep): colour discipline (0 literal colours outside `theme.rs`); the
`ext` spine and `MenuEntry` model; production `unwrap` count is low; markers (`TODO/FIXME`) are near
zero; refactor of `receive()` into families; the `ui/kit` skin concept; `ColourField` consolidation.
