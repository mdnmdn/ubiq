---
id: wip-refactor-plan
title: Pre-editions refactoring plan
kind: wip
status: current
summary: What to clean up before the editions-proposal's registry work lands — sized, ordered, sourced from a four-agent audit of the largest files in each crate.
read_when: you are picking up refactoring work ahead of the editions split
updated: 2026-09-04
depends_on: [inbox-editions]
---

# Pre-editions refactoring plan

Source: a four-way audit (Sonnet subagents, read-only) of the largest files in `ubiq`, `ubiq-host`,
`ubiq-proto` and `agent-manager`, cross-checked against `inbox/editions-proposal.md`. Deleted when
this refactor closes — promote anything durable to `tech/` or `backlog.md` before that.

## Headline numbers

- `crates/ubiq/src/app.rs`: **7685 lines**, `AppState` ~60 fields + `OpenProject` ~15, a 690-line
  constructor, a 707-line 48-arm message dispatcher. Nine times the size of the next-biggest file in
  the workspace.
- `crates/agent-manager/src/harness/*.rs`: 5 files, 5149 lines, **~20-25% (1000-1200 lines)
  structural duplication** — identical trait-identity boilerplate and an identical account
  env-injection block, five times over.
- `crates/ubiq-host/src/coordinator.rs` + `crates/ubiq-proto/src/messages.rs`: **healthy, no
  restructuring needed.** The dispatch match already delegates by family; the flat message enum is
  adequate at 88 variants.
- `crates/ubiq/src/state/*.rs`, `ui/dock/*.rs`: **mostly healthy.** One tangled file
  (`state/explorer.rs`), a handful of small ponytail findings, and one factual correction to the
  editions-proposal (below).

## Correction to `inbox/editions-proposal.md`

§4.2's table claims `PanelKind` "keeps `Copy`" when it gains an `Extension` variant. **`PanelKind` is
not `Copy` today** — it holds a `File(String)` variant, so it's `Clone, PartialEq, Eq, Hash, Debug`
only. Not a blocker, but the proposal's phase 2 needs to know this going in rather than discover it
mid-change.

## Phase 0 — quick deletions and shrinks (no architectural risk, do first, any order)

Ponytail-tagged findings, each independent and small:

- `delete:` `state/sink.rs`'s dead `rgb`-equivalent (app.rs's `sink_project_rgb` duplicates it inline
  instead of calling it) — pick one, delete the other.
- `delete:` `io/copilot.rs::CopilotBridge.turn_ended` and `io/opencode.rs::OpencodeBridge.turn_ended`
  — both `#[allow(dead_code)]`, both genuinely dead (a clone does the real work).
- `yagni:` `state/explorer.rs::ExplorerAction` (NewFile/NewFolder/Rename/Delete) and
  `ExplorerKey::ShiftEnter` — wired to menus/dispatch for filesystem mutations and a temp-vs-permanent
  distinction that don't exist on the bus. Either wire them for real (there's backlog appetite —
  see `G70`, `G105`) or strip the dead paths.
- `yagni:` `harness/mod.rs::TemplateStore` — trait with one production impl, doc comment admits it's
  speculative ("so an embedder can back templates with a database"). Leave as a plain `FsTemplateStore`
  until a second implementation is real.
- `shrink:` `app.rs`'s sink-vs-project-form colour-picker logic (`apply_sink_project_hex` /
  `sync_sink_project_hex` / `sink_project_rgb` / `sink_project_swatch_rgb` vs `apply_project_form_hex`
  / `sync_project_form_hex`, ~90 lines) is the same HSV/hex math branched twice. One `ColourField`
  helper covers both targets.
- `shrink:` `state/sink.rs`'s four enum→match→string boilerplate blocks (`SinkSection`, `SettingsNav`,
  `ProjectNav`, `SinkModal`, ~150 lines) — collapse using the array-of-structs pattern the same file
  already uses for `HarnessFixture`.
- `shrink:` `state/sink.rs`'s four one-line clamp functions (`nudge_font`/`nudge_agents`/`nudge_warn`/
  `nudge_idle`) differ only by field/range — one generic `nudge(value, delta, range)`.
- `stdlib:` `state/explorer.rs`'s five near-identical recursive tree walkers (`node_of`/`node_mut`/
  `collect_cache`/`collect_expanded`/`paint_nodes`) — one generic "recurse into `NodeKind::Dir`" walker
  parameterized by what each caller does per node.

## Phase 1 — split `app.rs` (the big one)

`AppState` carries three kinds of fields today: window-shell state that's genuinely central (panes,
dock, bus), per-project domain state that already delegates to `state/*.rs` (explorer, editor, git),
and screen-private state with nowhere else to live (the kitchen sink's ~17 fields and ~45 methods,
the board's methods, the agents/orchestration methods). Split along those lines, one `impl AppState`
block per file (no trait needed for this step):

| New file | Carries | Est. lines |
|---|---|---|
| `app/shell.rs` | `AppState` struct, slimmed `for_project()`, project sync/enter/drop, pane accessors, key bindings, window lifecycle | ~600 |
| `app/bus.rs` | `receive()` split into `receive_git`/`receive_explorer`/`receive_agents`/`receive_files`, pane open/close plumbing | ~750 |
| `app/dock.rs` | `settle_panels`/`settle_mode`/`settle_layout`/`settle_visibility`/`enforce_placement`, `PanelEdit` | ~400 |
| `app/sink.rs` | All ~45 `sink_*` methods, unified with the project-form colour picker via `ColourField` | ~450 |
| `app/explorer.rs` | Explorer mutators (`toggle_folder` … `pick_explorer_action`) | ~600 |
| `app/editor.rs` | Editor mutators (`select_file` … `save_active_file`) | ~750 |
| `app/agents.rs` | Agent/conversation mutators through tab-drag settling | ~800 |
| `app/graph.rs` | Orchestration graph methods | ~250 |
| `app/board.rs` | Task board methods | ~500 |
| `app/chat.rs`, `app/diagrams.rs`, `app/viewport.rs` | Remaining small groups, `Render` impl | ~300 combined |
| `app/project_mgmt.rs` | Add/edit/close project, preference persistence | ~450 |

Also worth doing in the same pass: the 8 bespoke "drain a pending queue next frame because this
mutator needs a `Window`" flags (`pending_panels`, `pending_regions`, `pending_layout`,
`refill_fields`, `refill_columns`, `fill_project_form`, `form_filled`) are the same workaround
repeated 8 times — one small deferred-action queue drained once in `render()` replaces all of them.

**Why this is the prerequisite, not a nice-to-have:** editions-proposal §4.2 says a closed screen
needs to own its state because it cannot add a field to `AppState`. `app/sink.rs` and `app/board.rs`
are the two pieces of today's own UI that are *already* screen-private in everything but syntax —
they're the base-side user the `Screen` trait needs to justify existing (§4.2, and the opening rule's
own test). Do this split before the trait, not as part of it: it's independently valuable (nothing
here is over 1000 lines any more) and it turns the trait migration into "move two already-separated
files behind an interface" instead of "extract state from a monolith and design an interface at the
same time."

## Phase 2 — split `state/explorer.rs`

One 715-line `impl ExplorerState` block tangles five unrelated responsibilities that already have
their own doc-comment banners in the file: tree merge, git-status paint, filter cache, keyboard
handling, context-menu state. Split along those banners into submodules under `state/explorer/`.
Apply the phase-0 tree-walker consolidation here too.

## Phase 3 — `agent-manager` harness dedup

Extract `harness/shared.rs` (or free functions re-exported from `harness/mod.rs` — four of the trait's
methods already have default bodies, so this is a continuation of an existing pattern, not a new one):

- An identity macro/table for the five copy-pasted `id`/`display_name`/`command`/`aliases`/
  `io_support` blocks (~25 lines × 5).
- `inject_account_env(spec, primary_var, fallback_var)` for the copy-pasted account env-injection
  block in every `provision()` (~50 lines × 5).
- A `harness_conformance_tests!` macro for the near-verbatim `write_skill()` helper and the "missing
  skill path" / "no secret leaked" tests repeated 4-5×.

Estimated savings: ~500-650 lines removed across the five harness files; the genuinely
harness-specific 75-80% (native config serialization, model discovery, argv shape) is untouched.

Separately, `cli/account.rs` (1754 lines) is nine CLI subcommands with parsing, business logic and
formatting interleaved in one file with no behavior problem — split into
`cli/account/{mod,dump,import,login,check_renew}.rs` for readability. No logic change.

## Phase 4 — editions-proposal groundwork

Only after phases 1-3 land, per the proposal's own phase order (`inbox-editions` §14):

1. **Composition root** (§3) — `ubiq-app` becomes a library (`Boot`, `Stores`, `run()`) with a
   three-line `main.rs`. Mechanical, zero screen impact, can actually happen any time — it doesn't
   depend on the app.rs split. Cheapest phase-4 item to do first.
2. **`Feature` dispatch arm** — confirmed by the host/proto audit: drop straight in before
   `coordinator.rs`'s existing unhandled-message warning, no match restructuring needed first.
3. **`Screen`/`Contribution` trait** — model it as a proper `dyn Trait` with named methods, the same
   shape as the existing `DockAreaRenderer`/`TabGroupRenderer`/`TilesRenderer` pattern `ui/dock/
   skin.rs::Skin` already implements. **Not** `skin.rs`'s `NewPane` (a bag of four loose closures) —
   that shape is a one-off forced by crossing `gpui_component`'s foreign renderer trait, not a
   template to repeat.
4. **`RailMode`/`PanelKind` `Extension` variants** — ~4 touch points for `RailMode`, ~10 for
   `PanelKind` across `dock.rs` and `ui/dock/mod.rs`. Fix the `Copy` claim (above) before promising it
   in the variant's derive list.

## What's confirmed healthy — leave alone

- `coordinator.rs`'s dispatch match: already delegates by family (`work_job`/`file_job`/`git_job`
  helpers collapse 13 work-family arms to one line each). Two optional, unrelated shrinks if ever
  touching that code: a `pane_job` helper for `TerminalInput`/`TerminalResize`'s duplicated
  owns-check-then-error shape, and extracting `SetAgentConfig`'s inline 30-line body to its own
  method for consistency with every other multi-line arm.
- `messages.rs`: flat 88-variant enum, family-grouped by comment, is adequate. Splitting into
  per-family sub-enums doesn't buy incremental-compile granularity (same crate either way) and isn't
  worth the churn.
- `ui/dock/mod.rs`, `ui/dock/skin.rs`, `state/file_picker.rs`, `state/editor.rs`, `theme.rs`'s
  `Palette`: clean, no dead code, no speculative generality. `file_picker.rs::PickerOwner` even
  documents honestly that it has one variant rather than gold-plating for a second.

## Related docs

- [`../inbox/editions-proposal.md`](../inbox/editions-proposal.md) — the target architecture phase 4 builds toward
- [`../tech/architecture.md`](../tech/architecture.md) — the rules this refactor must not cross
- [`../backlog.md`](../backlog.md) — where the `ExplorerAction`/`ShiftEnter` yagni finding overlaps `G70`/`G105`
