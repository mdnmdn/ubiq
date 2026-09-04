---
id: wip-refactor-plan
title: Pre-editions refactoring plan
kind: wip
status: current
summary: Phases 0-3 are done and so are phase 4's composition root and preference round-trip; three phase-4 items remain, each blocked or deferred for a recorded reason, and every `just verify` check now passes but docs-lint — whose open question is what that lint should apply to, since 156 of its 161 failures are inbox documents.
read_when: you are picking up refactoring work ahead of the editions split
updated: 2026-09-04
depends_on: [inbox-editions]
---

# Pre-editions refactoring plan

Source: a four-way audit (read-only subagents) of the largest files in `ubiq`, `ubiq-host`,
`ubiq-proto` and `agent-manager`, cross-checked against `inbox/editions-proposal.md`. Deleted when
phase 4 closes — promote anything durable to `tech/` or `backlog.md` before that.

**Phases 0-3 landed on 2026-09-04, and so did phase 4's first item.** What is left of phase 4 is
three items, each with a recorded reason it is left.

## What the split produced

| Was | Is | Largest file now |
|---|---|---|
| the old `app.rs`, 8120 lines, one 7060-line `impl AppState` | `crates/ubiq/src/app/`, 16 files | `app/wire.rs`, 1241 |
| `receive()`, a 775-line match over the whole bus protocol | 31 lines chaining nine `receive_<family>` helpers | `receive_file`, 172 |
| the old `state/explorer.rs`, 1405 lines | `crates/ubiq/src/state/explorer/`, 6 files | `tree.rs`, 454 |
| the old `cli/account.rs`, 1754 lines | `crates/agent-manager/src/cli/account/`, 6 files | `import.rs`, 439 |
| five harness files carrying five copies of three things | `harness/shared.rs` | −419 lines, test count unchanged |

`crates/ubiq/src/app/sink.rs` and `app/board.rs` are the two screen-private files
editions-proposal §4.2 needs as its base-side user. `app/wire.rs`'s chain ends at the same
unhandled-message warning the `Extension` arm slots in before.

`_docs/tech/code-map.md` is the current map of both new directories.

## What the audit got wrong

Recorded because the same mistakes are easy to make again from the same reading.

| The claim | What is actually true |
|---|---|
| Child modules cannot see parent-private **fields**, so the split needs a visibility bump | They can — a private item is visible in its module *and all descendants*. No field changed. What did need bumping was 41 private **methods** now called from a *sibling* module, which is not a descendant relationship |
| `explorer.rs` splits along a grid of doc-comment banners | There were two banners, both inside one impl. The split was made along responsibilities the file left implicit |
| The five tree walkers consolidate into one generic visitor | `node_of`/`node_mut` is a const/mut pair Rust cannot unify without a macro, and `paint_nodes`/`collect_cache` share only the recursion skeleton. Left alone |
| `turn_ended` is a dead method on a `ubiq`-side io bridge | It was a **field** in `crates/agent-manager/src/io/`, and only the struct's copy was dead — the reader thread's clone was live. It is now a plain thread-local `bool` |
| `state/sink.rs` has a dead `rgb` the app duplicates | Two separate findings: `ProjectDemo::rgb` had no callers, and the duplicate was `sink_project_swatch_rgb` against the free `project_swatch_rgb`. Both gone |
| The colour-picker duplication is six helpers, ~90 lines | It was `set_rgb`'s body inlined against a second state in four places, plus a **third** copy of the shape in `ui/sink/project.rs`. One `ColourField` in `state/sink.rs` now owns the maths |
| The env-injection block is identical five times, ~50 lines each | Only the `std::env::var(…).map_err(…)` expression was identical (9 times). The surrounding branch shapes genuinely differ — claude sets key and token independently, grok collapses with `.or()`, opencode pushes two vars per arm. Only the resolver was extracted |
| `PanelKind` "keeps `Copy`" (editions §4.2) | It is not `Copy` today — it holds `File(String)`. Fix the proposal before phase 4 promises it in a derive list |

## What was deliberately not done

- **`ExplorerAction`'s NewFile/NewFolder/Rename/Delete.** The dead-end is real — `app/explorer.rs`
  collapses all four to `cx.notify()` — but it is *documented as deliberate* in
  `state/explorer/mod.rs`, and `G70`/`G105` own wiring them. `ExplorerKey::ShiftEnter` is **not**
  dead: it picks pin-vs-preview and stays.
- **A deferred-action queue** replacing the eight pending flags. They are heterogeneous
  (`Vec<PanelEdit>`, `Option<Value>`, `Option<(bool,bool,bool)>`, `bool`, `Option<TaskId>`), each is
  drained by a named settler, and `Render` calls those settlers in a load-bearing, commented order
  that a closure queue would lose. Revisit if a ninth flag appears.
- **`nudge_font`/`nudge_agents`/`nudge_warn`/`nudge_idle`.** 15 lines; a generic version pushes the
  bounds into every `ui/` call site.
- **Deleting `harness::TemplateStore`.** One implementation, yes — but it is documented public API
  of a standalone MIT library in `crates/agent-manager/_docs/am-as-library.md`, listed beside
  `Registry`, `AccountStore` and `SessionStore`, which editions §1 calls proven seams. Narrowing a
  published library contract to delete 12 lines is a net loss.

## Phase 4 — editions-proposal groundwork

In the proposal's own order (`inbox-editions` §14).

### Landed on 2026-09-04

1. **Composition root** (§3), `3c51380` — `ubiq-app` is a library: `Stores` (the four boxed store
   traits, with `Stores::files`), `Boot` (one field, `stores: Box<dyn FnOnce(&Path) -> Stores>`,
   because the config root is resolved inside `run`) and `run(boot)`, the former 246-line `main()`
   verbatim. `main.rs` is three lines. `Cargo.toml` gained `[lib] name = "ubiq_app"`. Zero behaviour
   change, and the boot gained a test that hands in the memory stores.
   The proposal's `Boot::features` and `Boot::contributions` are absent by design — `Feature` and
   `Contribution` do not exist, and the proposal's own rule is that an extension point with no
   base-side user is not built.
- **The `ViewPrefs`/`InterfacePrefs` unknown-key round-trip** (§6), `3db7b4a` — out of §14's phase 4,
  because it is a real forward-compatibility hole on its own: serde dropped every key the structs did
  not name, so a blob written by a newer build lost them the first time an older build wrote it back.
  Both gained `#[serde(flatten, default)] rest: BTreeMap<String, Value>` — a general catch-all rather
  than §6's named `extensions` map, since the mechanism is the same and only the general one has a
  base-side user today. `remember_interface` rebuilds `InterfacePrefs`, so the unknown keys park on
  `WorkbenchState::interface_rest`.

### Not done, and why

2. **`Feature` dispatch arm** — deliberately deferred. It drops straight in before `app/wire.rs`'s
   chain-ending warning and `coordinator.rs`'s equivalent, with no match restructuring needed first,
   but landing it today ships a trait with zero implementations and four unreachable message
   variants. `inbox-editions` §14 puts it after the second repository exists, which is what can
   validate it.
3. **`Screen`/`Contribution` trait** — blocked on `inbox-routing` phase 1, which is `status:
   proposal` with no `Destination` type in the code. When it unblocks: model it as a proper
   `dyn Trait` with named methods, the same shape as `ui/dock/skin.rs::Skin`'s
   `DockAreaRenderer`/`TabGroupRenderer`/`TilesRenderer`. **Not** `skin.rs`'s `NewPane` (a bag of
   loose closures) — that shape is forced by crossing `gpui_component`'s foreign renderer trait, not
   a template to repeat.
4. **`RailMode`/`PanelKind` `Extension` variants** — blocked on the same `inbox-routing` phase 1.
   ~4 touch points for `RailMode`, ~10 for `PanelKind`. Fix the `Copy` claim above before promising
   it.

## Next steps — where this was left on 2026-09-04

### `just verify` is one check away from green

`check`, `clippy`, `test`, `host`, `ui` and `core` all pass. `docs-lint` is the only red one, and
`just verify` has been red on `main` for long enough that the count is a backlog rather than a
regression.

**`cargo test --workspace` is green as of `1c322c2`, and the `codex_bridge` "flake" was never a
flake.** Cargo unifies features across workspace members; Zed's `gpui`/`http_client` crates, which
`crates/ubiq` needs, enable `serde_json/preserve_order`, which serialises map keys in insertion
order instead of alphabetically. So the same JSON-RPC request is `{"id":1,"jsonrpc":…}` under
`cargo test -p agent-manager` and `{"jsonrpc":"2.0","id":1,…}` under `--workspace`, and
`tests/fake-codex-appserver.sh` extracted `id` with a `sed` anchored to `^{"id":`. Deterministic per
build configuration, not timing. **Nothing in this workspace may assume serde_json key order.**

### The open decision: what docs-lint should apply to

161 failures across 47 documents, and **156 of them are in `_docs/inbox/`**:

| Location | Failures | |
|---|---|---|
| `inbox/completed/` | 51 | shipped proposals citing symbols and files as they were when written |
| `inbox/` | 40 | live proposals citing code that partly does not exist yet, by design |
| `inbox/backlog/` | 29 | shelved designs, the same |
| `tech/`, `wip/`, `backlog.md`, `features/` | 16 | genuinely stale, worth fixing |

By check: L2 "referenced file/symbol/document not found" 123, L7 "orphan, not linked from INDEX" 23,
L4 length ceiling 13, L1 frontmatter 5, L9 repeated link 1.

The question to settle before anyone spends a day here: **should L2 and L7 apply to
`inbox/completed/` at all?** A shipped proposal saying `app.rs:4360` was true when it was written;
rewriting it to `app/explorer.rs` arguably falsifies the record and goes stale at the next refactor.
Three ways out, and this is a policy call, not a cleanup:

1. Scope the lint in `_tools/docs.py` — L2 and L7 skip `inbox/completed/` and `inbox/backlog/` —
   then fix the ~16 real failures in the live documents. Probably reaches green.
2. Rewrite all 156 citations, add the missing frontmatter, link the orphans, split the over-length
   documents. Large, and it rewrites records of moments into claims about today.
3. Drop `docs-lint` from `just verify` and run it separately. Green immediately; removes the
   pressure that keeps the live documents honest.

Sub-questions worth answering with it: whether `inbox/` (unbuilt proposals) should be treated
differently from `inbox/completed/`; whether the 23 orphans are a signal worth keeping, since a
proposal nobody linked may genuinely be lost; and whether `just verify` should gate on documents at
all, or whether that belongs in a pre-merge check.

### After that

Phase 4's items 2–4 above, in the order their blockers clear: `inbox-routing` phase 1 first (it
unblocks 3 and 4), then the second repository (`inbox-editions` §14 phase 3), which is what makes
item 2 worth landing.

## What is confirmed healthy — leave alone

- `coordinator.rs`'s dispatch match: already delegates by family. Two optional shrinks if ever
  touching it: a `pane_job` helper for `TerminalInput`/`TerminalResize`'s duplicated
  owns-check-then-error shape, and extracting `SetAgentConfig`'s inline body to its own method.
- `messages.rs`: the flat 88-variant enum, family-grouped by comment, is adequate. Per-family
  sub-enums buy no incremental-compile granularity in one crate.
- `ui/dock/mod.rs`, `ui/dock/skin.rs`, `state/file_picker.rs`, `state/editor.rs`, `theme.rs`'s
  `Palette`.

## Related docs

- [`../inbox/editions-proposal.md`](../inbox/editions-proposal.md) — the target architecture phase 4 builds toward
- [`../tech/code-map.md`](../tech/code-map.md) — the map of `app/` and `state/explorer/`
- [`../tech/architecture.md`](../tech/architecture.md) — the rules this refactor did not cross
- [`../backlog.md`](../backlog.md) — where the `ExplorerAction` finding overlaps `G70`/`G105`
