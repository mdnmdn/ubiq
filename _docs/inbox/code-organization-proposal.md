---
id: inbox-code-organization-proposal
title: Proposal — how Ubiq organises code, features, components and tests, and the gradual way there
kind: proposal
status: proposal
summary: A verdict (keep / improve / fix) on every practice area of the workspace — crate layout, the message set, the UI state model, seams, styling, size tokens, placeholders and mock data, workers, persistence, errors, security, tests, tooling, the drone, spikes — then the target conventions as short recipes (feature, panel, rail mode, message family, harness, kit component, menu, settings section, store) with token, visibility, size, error, test and security rules, and a seven-phase migration in which every step ships alone and passes `just verify`, tracked by a metrics table with today's value and the target.
read_when: you are adding a feature, panel, rail mode, message, harness, menu, setting or store and want the house recipe; or you are picking a refactoring or hardening step and want to know its order, its exit criterion and what it depends on
updated: 2026-10-05
depends_on: [tech-architecture, tech-transport, tech-ui, tech-operations, inbox-component-reuse, inbox-plugin-system, inbox-editions, wip-refactor-plan, feat-drone]
---

# Proposal — how Ubiq organises code, and the gradual way there

The numbers behind every verdict here are in the snapshot,
[`code-organization-snapshot.md`](./code-organization-snapshot.md), and in the three audits it was
built from ([UI](../wip/codeorg-ui.md), [headless side](../wip/codeorg-host.md),
[workspace](../wip/codeorg-workspace.md)). This document cites only the evidence a decision needs.

**The short version.** The architecture is sound and mostly enforced: one contract crate, a UI that
cannot name the host, argv-only spawning, contained paths, atomic writes, colour tokens with zero
literal leaks, ~3,500 tests and almost no `TODO`s. What does not scale is *inside* the crates: one
217-field `AppState` with a 2,048-line constructor, a 2,096-line `dispatch`, closed enums where the
`ext` registry already shows the open shape, six menu mechanisms (two resolving picks by index),
1,567 raw `px(` in `ui/`, strings as errors, all-`pub` visibility, no CI gate. Four security items
are real today: bare `--serve` binds every interface, `Secret` serialises into the bus tape, the MCP
listener reads unbounded bodies with no token, and feedback credentials compile into the binary.

The plan below fixes the guardrails first, then safety, then opens seams, then decomposes — never a
big-bang rewrite. Each step is one PR that passes `just verify`.

## 1. Assessment

Verdicts: **keep** — good, protect it; **improve** — sound shape, extend or tidy; **fix** — wrong or
risky, change it. Criteria scored G (good) / F (fair) / P (poor): **Id** idiomatic Rust, **Mt**
maintainability, **Sc** scalability, **SoC** separation of concerns, **Re** reusability, **Se**
security.

| # | Area | Verdict | Id | Mt | Sc | SoC | Re | Se | Good today | Wrong today | Phase |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | Crate layout and boundaries | **keep** + improve | G | F | F | G | G | G | Six crates with a one-way graph; `just host`/`ui`/`relay` grep the dependency tree; `agent-manager` has no UI dependency and builds bare | Boundaries checked by greps, not types; inside `crates/ubiq`, `state` imports `ui` in 11 files and `app` in 13 — a module cycle the single crate hides | 3 |
| 2 | Message set and dispatch | **improve** | G | P | P | F | F | F | One serde enum, MessagePack with named fields so `#[serde(default)]` additions are compatible; `MAX_FRAME` checked before allocation; services return `Vec<Reply>` and are testable without a bus | ~350 variants; direction is a comment; `dispatch` 2,096 lines and UI `wire.rs` 3,661; adding a message touches 5-6 places, two of which fail silently (`pane_id_of` catch-all, drone drops); one undecodable frame ends a remote session | 2, 3 |
| 3 | UI state model | **fix** | F | P | P | P | P | — | `state/` holds plain, testable data; newer subtrees (`ui/mdview`, `ui/db`, `state/explorer`) are small files with real `Render` views | `AppState` 217 fields (160 `pub`), 50 `impl` blocks, 93 `ui` files take `&AppState`; `for_project` builds 90 inputs inline; ~20 order-dependent `settle_*` calls mutate inside `render` | 3 |
| 4 | Extension seams and registration | **keep** + improve | G | G | F | G | G | G | `ext::Registry<T: Slotted>` with `SlotId` consts checked by `just slots-check`; `Contributions` through `Boot`; Studio plugs in with three lines | Panels, document kinds, dialogs, MCP tools and message families have no seam (closed `PanelKind`, 5-13 files per variant); host registries are separate types; `OnceLock` installs panic on misuse; the demo mode ships in `base_registry` | 2 |
| 5 | Styling vs UI logic, incl. menus | **improve** (menus **fix**) | F | F | P | F | P | — | `ui/kit` holds the house skin; bar menus (`MenuEntry`) are data with an action closure each, one walker draws them | Six menu mechanisms; `ContextItem` and `Picker` return an index into a list the handler rebuilds; hand-numbered `MENU_LAYER`/`MODAL_MENU_LAYER`; popover plumbing copied into ~15 files; three private modal frames | 4 |
| 6 | Colour tokens | **keep** | G | G | G | G | G | — | 0 hex literals outside `theme.rs`, both palettes, a lint in `just ui` | gpui-component draws in its own palette until the bridge in the [reuse proposal](./component-reuse-proposal.md) lands | 4 |
| 7 | Size, spacing, typography tokens | **fix** | F | P | P | F | P | — | `theme::font(Family, Role)` (713 uses), icon-size accessors, ~35 layout consts with `scaled` | 1,567 `px(` in `ui/`; row heights hand-typed at 30/28/26/24/22; raw `px` ignores `ui_scale` while rem spacing scales, so one setting scales half the layout | 4 |
| 8 | Hardcoded values, placeholders, mock data | **fix** (mock) / improve | F | F | F | F | — | F | 5 real `TODO`s in 400k lines; named consts dominate in the host (169) | `work/mock.rs` mints 11 fake agents and 5 fake sessions for **every real project**; `ext_demo` in the production registry; sink fixtures in `state/`; harness-name icon map in `ui/kit/mod.rs` keyed by display names; `"claude-code"` default in 3 files; `contains("claude")` sniffing; 111 placeholder strings inside the constructor | 1, 2 |
| 9 | Concurrency and workers | **improve** (panics **fix**) | F | F | F | G | P | — | Threads and channels, no async runtime to reason about; the PTY reader never waits for the UI; the coordinator never blocks on fs or spawn except the stores (G46) and the spawn probe (G38) | The coordinator `JoinHandle` is dropped and there is no panic hook: a dead worker turns every later request into a silent drop while the window stays up; the same 12-line worker copied 4 times; 42 ad-hoc `thread::Builder` sites; unbounded per-client channels | 1, 3 |
| 10 | Persistence | **keep** + improve | G | G | F | G | G | F | `write_atomic` (temp, fsync, rename, dir sync) at 45 sites; corrupt files preserved aside; every envelope versioned; credentials as references through the OS keychain | No lock: every store is last-writer-wins across `ubiq` + `ubiq-studio` + `--serve` (G29 is wider than `projects.toml`); temp files at default umask; no migration hook | 1, 3 |
| 11 | Error handling | **fix** | P | F | P | F | F | — | Typed proto errors for files, git, search, connect, clone; `DbError` via `thiserror`; low production `unwrap` (≈50 in all crates) | 186 `Result<_, String>` in the host; `agent-manager` is `anyhow` end to end with `thiserror` declared and unused; ~6 ad-hoc `*_error: Option<String>` banners in the UI; 100+ silent `let _ =` with no policy | 3 |
| 12 | Security | **keep** + **fix** four items | G | — | — | — | — | F | argv-only spawn, no `sh -c`; `files/path.rs` canonicalise + containment at 29 sites; ssh target refusal and `-l`; PKCE OAuth; hash-pinned web assets; sealed DB passwords | Bare `--serve` binds `0.0.0.0`, plaintext, all-or-nothing token in the URL, no connection cap; `Secret` is `#[serde(transparent)]` and the tape stores every message as JSON; MCP body unbounded, no Host/Origin check, no token; `option_env!` feedback token; `ag` query without `--`; secrets on `security -w` argv; `ConnectionConfig` password in `Debug` | 1 |
| 13 | Tests | **improve** | G | F | F | G | F | — | ~3,500 tests; services driven without a bus; shell fakes for harnesses; `TestAppContext` in 60 UI integration files | No PTY fake (tests need a real pty, unix only); no per-variant round trip; 125 of 150 `ui/` files and `wire.rs`, `boot.rs`, `settings.rs`, menus have no test; no property tests for the parsers | 5 |
| 14 | Tooling and CI | **fix** | — | P | P | — | — | F | `just` is one command surface; `verify` bundles check, clippy, test, boundaries, docs and slots | No workflow runs `verify`; no `[workspace.lints]`, no `rust-toolchain.toml`, no MSRV, no `cargo-deny`; `core`, `headless`, `docs-check`, `icons-check` sit outside `verify`; `gpui` git deps without `rev` | 0 |
| 15 | Drone (WIP, never run against a real sshd) | **improve** | G | G | F | G | F | F | Clean layering (proto + lean host only), `--` on every search argv, socket `0600` in a `0700` dir, handshake carries the schema | Empty `BINARIES` manifest; seven refused messages with no error variant; silent linger expiry; `relay.rs` 1,208 lines with no inline test; no Windows build (G291); no read-only root (G267) | 5 |
| 16 | Spikes and editions | **fix** (spikes) / keep (editions) | — | F | F | G | P | — | The editions split through `[patch]` and `Contributions` works; the Studio doc graph is directed | Graduated spikes stay workspace members as forks of `ubiq-db`/`ubiq-md` and drift (4 and 15 lines); no spike uses the seams the `ubiq-spike` skill describes; Studio pins base `509b469` while the checkout is newer | 6 |

**Where this differs from [`refactor-plan.md`](../wip/refactor-plan.md).** That plan judged the
dispatch match and the flat enum healthy at 88 variants and declined a deferred-action queue "until
a ninth flag appears". The enum is ~350 variants and `render` calls ~20 settlers, so both triggers
have fired. The one-enum decision stands (sub-enums buy no compile granularity in one crate); what
changes is *where the arms live* and *how direction is stated* (§2.4, phase 2-3).

## 2. Target conventions — how we build things

Simple rules, each one checkable. Where a rule names a type or a helper that does not exist yet, the
phase that creates it is in brackets.

### 2.1 Layout

| Rule | Detail |
|---|---|
| Crates stay as they are | Six crates plus `agent-manager`. A new crate only when a boundary must be *enforced* (a leaf engine like `ubiq-db`, or a contract). Never split a crate for size. |
| Inside `crates/ubiq`: `state` < `app` < `ui` | `state/` names no `gpui` view type and no `crate::ui`/`crate::app`; `app/` names no `ui::` drawing helper; `ui/` reads state through a view, never mutates it except by `cx.listener`. A `_tools` grep enforces it [P3]. |
| Feature folders, not layer files | A feature with more than ~800 lines is a directory: `state/<feature>/`, `app/<feature>.rs` or `app/<feature>/`, `ui/<feature>/`. `mod.rs` re-exports and holds no logic beyond ~150 lines. |
| One concern per file | Size budget: **soft 800, hard 1,500 lines** of production code per file; **function 150 lines** (render functions 250). Over budget is allowed only with a row in [`backlog.md`](../backlog.md) that names the split. Measured by a `_tools` check [P0]. |
| Visibility | `pub(crate)` by default; `pub` only on items `ubiq-app`, Studio, another crate or `tests/` uses. Leaf modules private, re-exported from `lib.rs`. New code follows it now; old code moves when touched. |

### 2.2 Recipes

Each recipe names the places a change touches **after** the phase in brackets lands; before it,
follow the existing pattern and file the gap.

| To add | Where it goes | Steps | Must not |
|---|---|---|---|
| **A feature** (vertical) | `ubiq-proto/src/<family>.rs` payloads; `ubiq-host/src/<family>/` service; `ubiq/src/{state,app,ui}/<family>/` | 1 payloads + variants; 2 a service returning `Vec<Reply>`, unit-tested without a bus; 3 a coordinator handler file [P3]; 4 a feature view entity with its own state [P3]; 5 a `PanelSpec`/`RailModeSpec` registration; 6 docs anchors | Add a field to `AppState` beyond the entity handle; put logic in a dispatch arm |
| **A panel** (dock or edge) | `ext/panel.rs` `PanelSpec` [P2] + `ui/<feature>/panel.rs` | Register id, title, icon, region rule, persist key and a render fn; the dock looks it up | Add a `PanelKind` variant and match arms in six modules |
| **A rail mode** | `ext/ids.rs` + a `RailModeSpec` beside the feature, not in `ui/rail.rs` | One `SlotId` const, one spec in the feature's own module, its panels via `PanelSpec` | Edit the central `modes` list for a non-base mode |
| **A message family** | `ubiq-proto/src/<family>.rs`, a banner in `messages.rs`, a row in `transport-contract.md` | Variants carry `pane_id` when pane-scoped; classify each in the exhaustive `Message::direction` / `capability` match [P2]; one error variant per request; a round-trip test is generated by the contract test [P5] | Rely on a catch-all arm (`_ =>`) in `pane_id_of`, the drone relay or the classification |
| **A harness** | `crates/agent-manager` only | Implement `Harness`; return metadata — `HarnessId`, display name, icon key, capabilities, default flag [P2]; register in `harness::all` | Name a harness id, display name or config path in `ubiq-host` or `ubiq` |
| **A kit component** | `ui/kit/<group>.rs` | Behaviour from gpui-component, skin from `theme` (reuse proposal §2); a `RenderOnce` struct with a builder, not a free fn with eight params; one sink entry | Live in a screen module; take `&AppState` |
| **A screen component** | `ui/<feature>/` | Reads its own feature state; uses kit pieces; if it is the second copy of a shape, promote the shape to the kit | Define a local `row`, `header`, `modal` or `dialog` |
| **A menu or context menu** | data in `state/` or `app/`, drawing in `ui/kit/menu.rs` | Build a `Vec<MenuItem>` whose items carry an **action value** (enum or `Box<dyn Action>`), never an index; one `menu` renderer over Root's popup layer [P4] | Rebuild the list in the pick handler; set a layer priority by hand |
| **A settings section** | `ui/settings/<section>.rs` + `app/settings/<section>.rs` [P3] | A `SettingsSectionSpec` with its body fn and handler in the section's own files; rows from `ui/kit/settings.rs` | Grow a central settings file |
| **A store** | `ubiq-host/src/store/<name>.rs` | A trait in `store/mod.rs` with file + memory impls; envelope `version` + `migrate` hook [P3]; `write_atomic`; corrupt file preserved aside; writes off the coordinator thread [P3] | Raw `fs::write`; a store without a memory impl for tests |
| **A worker thread** | `ubiq-host/src/worker.rs` `Worker<Job>` [P1] | Named thread, `catch_unwind` per job, a typed error reply when the thread is gone, bounded or coalescing queue | A bare `thread::Builder` with `.expect` |

### 2.3 Menus: one data model, one renderer

The bar menu's `MenuEntry` is already the right shape — label, icon, detail, disabled, an action of
its own. Generalise it, and every other mechanism becomes a caller:

```rust
pub struct MenuItem<A> { label: SharedString, icon: Option<UbiqIcon>, detail: Option<SharedString>,
                         enabled: bool, kind: ItemKind<A> }        // ItemKind::{Action(A), Separator, Submenu(Vec<MenuItem<A>>)}
pub fn menu<A: Clone + 'static>(items: Vec<MenuItem<A>>, on_pick: impl Fn(A, &mut Window, &mut App) + 'static) -> impl IntoElement
```

- The pick returns the action value; the handler matches on it. Draw list and pick list cannot drift.
- `A` is a per-menu enum (`GitRowAction::{Stage, Discard, ...}`) — a `match` the compiler checks.
- Context menus, `Picker`, the project menu, the run-tool menu, the tab menu all render through it;
  the body uses gpui-component `PopupMenu` inside `Root` (reuse proposal #1, #2, #6), so keyboard
  navigation and layering come with it and `MENU_LAYER` / `MODAL_MENU_LAYER` go away.
- One test pattern covers every menu: build the items from a state fixture, pick each action, assert
  the effect.

### 2.4 Messages and dispatch

| Rule | Why |
|---|---|
| One `Message` enum, family banners, payloads in family modules | Agrees with refactor-plan: sub-enums buy nothing in one crate |
| An exhaustive `fn direction(&self) -> Direction` and `fn pane_id(&self) -> Option<PaneId>` in `ubiq-proto`, **no `_` arm** | Moves "direction is documentation" and the `pane_id_of` trap into the compiler. Same match the plugin proposal's `Message::capability` needs (its phase 0) and that `--serve` scopes need (P1.1) — build it once |
| `Coordinator::dispatch` is a router: one line per family into `coordinator/<family>.rs` | Arms become `self.git.handle(client, msg)`; family fields group into sub-structs (57 fields today) |
| UI `wire.rs` mirrors it: `app/wire/<family>.rs`, each forwarding to the feature entity | The 478-line `receive_work` becomes the work view's `on_message` |
| Every request family has an error variant with a typed `kind` | The drone's seven silent refusals and the UI waiting forever both disappear |
| Unknown or undecodable frame: log, skip, continue | One newer peer must not kill a session (`carrier.rs` today ends it) |

### 2.5 UI state

- **One entity per feature.** `Entity<GitView>`, `Entity<BoardView>`, … each owning its `state/`
  struct, inputs and `Render`. `AppState` keeps window-level things: hosts, routing, the dock, the
  rail, the overlay stack, the entity handles. Target ≤ 60 fields.
- **Each feature builds itself.** `GitView::new(window, cx)` creates its own `InputState`s and
  placeholders. `for_project` composes features in a few lines.
- **No mutation in `render`.** Work that waits for a frame goes through one typed queue —
  `PendingWork` with explicit phases (`Attach`, `Settle`, `Fill`) drained before draw — or moves to
  the message handler that caused it. The order comment becomes the phase order.
- **Overlays are a stack, not flags.** `overlays: Vec<Overlay>` where `Overlay` is an enum or a
  boxed view; `ui/shell.rs` draws the stack. Escape pops the top.
- **Errors reach the user one way.** A `notify(UiNotice { level, text, source })` onto Root's
  notification layer; inline field errors stay inline. Delete the per-panel `*_error` banners as each
  feature migrates.

### 2.6 Style and tokens

| Rule | Token or helper | Check |
|---|---|---|
| No literal colour outside `theme.rs` (unchanged) | `theme::<token>()` | `just ui` |
| No raw `px(` for a layout metric in `ui/` | `theme::space(Space::Xs..Xl)`, `theme::row_h(Row::Compact / Normal / Tall / Header)`, `theme::radius`, `theme::border_w`, panel widths already in `theme.rs` | `_tools` ratchet [P0] |
| All tokens scale through `ui_scale` | Every accessor returns `scaled(..)`; rem spacing and token px agree | snapshot test of the token table at 1.0 / 1.25 |
| Text size only through `theme::font(Family, Role)` | 11 raw `.text_size(px(..))` go | `just ui` grep |
| Flex idiom has a name | `min_w_0` / `min_h_0` helpers replace the 476 `min_w(px(0.))` | ratchet |
| Durations and limits are named consts beside their owner | `SPIN`, `MAX_CELL_CHARS` defined once and imported | review |
| Exempt: hand-painted canvas geometry (`ui/kit/canvas.rs`, graphs), 1-px hairlines via `theme::hairline()` | — | allowlist in the ratchet |

User-facing strings live beside the thing they label (a spec field, an enum `label`, the feature's
constructor) — not in the composition root. No i18n layer until a second language is planned.

### 2.7 Errors

| Layer | Convention |
|---|---|
| Library crates (`agent-manager`, `ubiq-db`, `ubiq-md`) | A `thiserror` enum per public boundary the caller branches on (`NotFound`, `Unavailable`, `Auth`, `Confinement`, …); `anyhow` stays inside. `agent-manager` already declares `thiserror` |
| `ubiq-proto` | Hand-written error enums are fine; new ones use `thiserror`. Across the bus an error is `{ kind: <FamilyErrorKind>, detail: String }` — **no bare `String` error crossing the bus** |
| `ubiq-host` | `Result<_, String>` only as the last hop to `detail`; services return typed errors |
| `ubiq` UI | Errors from the host arrive typed; the UI branches on `kind` and shows `detail` through `UiNotice` |
| Discarding | `let _ =` only for a send to a gone peer or a best-effort clipboard write, with a comment when anything else; `.expect` only for boot invariants and poisoned locks (prefer `parking_lot::Mutex`, already a proto dependency) |

### 2.8 Concurrency

One `Worker<Job>` helper in `ubiq-host` replaces the four copies and is the default for new threads:
a named thread, `catch_unwind` around each job, a typed `Gone` answer when the thread has died, a
stop by sender drop. A process panic hook (in `ubiq-app`) logs and puts a host-fatal notice on the
bus. Per-client channels for **remote** clients coalesce `TerminalOutput` past a high-water mark.
Store writes leave the coordinator thread through a coalescing writer (G46). No async runtime.

### 2.9 Security rules

| Rule | Applies to |
|---|---|
| Spawn by argv only; a user- or model-supplied operand follows `--` or `-e` | every `Command`, search fallbacks, drone |
| Paths from a peer go through `files/path.rs` containment; absolute host paths only in families marked host-scope | files, kb, plan, search, host browse |
| Secrets travel as `Secret` and **never serialise in clear outside the wire codec**: tape, logs and `Debug` redact | `messages.rs`, `bus.rs`, `ubiq-db` `ConnectionConfig` |
| Network listeners default to loopback; anything wider needs an explicit flag, TLS and a scoped token | `--serve`, MCP, OAuth redirect |
| Every listener caps body size, header size, connections and handshake time | `remote.rs`, `mcp/server.rs` |
| No credential material compiled in; build-time values are endpoints, not tokens | `feedback/` |
| No secret on argv or in an environment visible beyond the child | `credentials/os.rs` |
| Files that hold references to credentials are created `0600` | `atomic.rs` callers in sensitive stores |
| Mock and demo data compile only under `cfg(any(test, feature = "demo"))` | `work/mock.rs`, `ext_demo`, sink fixtures |

### 2.10 Tests

| Level | What | Tools | Rule |
|---|---|---|---|
| 1 Pure logic (most tests) | state transitions, projections, parsers, services | `#[test]`, `tempfile`; `proptest` for parsers (connection strings, the read-only SQL guard, markdown blocks) | Every `state/` and service module has one |
| 2 Fakes for the outside | PTY, harness, clock, store | a `FakePty` behind the `pty` boundary; the existing shell fakes; memory stores | No test needs `/bin/sh` unless it tests the real PTY; those are `#[cfg(unix)]` and named `real_pty_*` |
| 3 Contract | every `Message` variant | one generated round-trip test (MessagePack and JSON), a snapshot of the variant list that fails on an unannounced change (G256), an exhaustiveness test for `direction`/`pane_id` | Adding a variant without its classification fails CI |
| 4 Views | each panel and rail mode renders; each menu's items map to actions | `gpui` `TestAppContext`, a table over the `PanelSpec`/`RailModeSpec` registries | A new spec is smoke-tested by being registered |
| 5 Integration | coordinator + services through the bus; drone over a local `sshd` | `tests/`; a CI container job for the drone | Slow tests behind a `just` recipe, not `#[ignore]` |

### 2.11 Spikes and editions

A spike is a `spike/<name>/` binary that runs the base workbench and contributes through
`Contributions` — the model the `ubiq-spike` skill describes. It never forks an engine crate. When it
graduates, it leaves the workspace `members` the same day, and its folder is archived or deleted.
A seam is added when its first base-side user exists (`inbox-editions` doctrine); panels and message
families have that user today — the base's own 26 panel kinds and ~70 families.

## 3. Migration plan

Principles: one step = one PR, green `just verify`, no behaviour change unless the step is a fix;
ratchets instead of sweeps (a check fails only when a count *rises*, and the baseline file is lowered
as code moves); refactor a file when it is open for a feature, except where a step says otherwise.
Sizes: **S** ≤ 1 day, **M** 2-4 days, **L** 1-2 weeks.

### Phase 0 — guardrails (do first, all S)

| Step | Goal | Actions | Exit criterion | Size | Deps |
|---|---|---|---|---|---|
| 0.1 | CI runs the gate | `.github/workflows/verify.yml`: macOS runner, `DOCS_RS=1`, `just verify` on PR and `main`; same in Studio over `just verify` | A PR with a clippy warning is red | S | — |
| 0.2 | Toolchain pinned | `rust-toolchain.toml` (stable channel, a fixed version) in both repos; `rust-version` in `[workspace.package]` | CI and local report the same `rustc -V` | S | 0.1 |
| 0.3 | Workspace lints | `[workspace.lints]` (`rust.unsafe_op_in_unsafe_fn`, `clippy.dbg_macro`, `clippy.todo`, `clippy.print_stdout` deny; `clippy.too_many_arguments` warn); `lints.workspace = true` in every crate | 9 crates inherit; no new `allow` without a reason comment | S | 0.1 |
| 0.4 | Missing checks join `verify` | add `core`, `headless`, `docs-check`, `icons-check` | `just verify` runs them | S | 0.1 |
| 0.5 | Ratchet tool | a `ratchet` script in `_tools` with a committed baseline file: files over 1,500 lines, functions over 150, `px(` in `ui/`, `pub fn` vs `pub(crate)`, `Result<_, String>` in host, `thread::Builder` outside `worker.rs`, `ContextItem::new`, `deferred(` files; `just ratchet` in `verify` | Any count rising fails; the baseline equals the metrics table in §4 | S | 0.4 |
| 0.6 | Pinned build inputs | `rev` on the `zed` git deps; a Studio check that its pinned base rev is an ancestor of base `main`; `lock-agrees` in Studio CI; `cargo-deny` with advisories + licences | `cargo deny check` green in CI | S | 0.1 |

### Phase 1 — security and safety fixes (each independent)

| Step | Goal | Actions | Exit criterion | Size | Deps |
|---|---|---|---|---|---|
| 1.1 | `--serve` safe by default | default bind `127.0.0.1` (`serve_bind`, `parse_bind` in `crates/ubiq-app/src/lib.rs`); non-loopback requires `--tls-*` or an explicit `--insecure-plaintext`; connection cap and auth-failure backoff in `crates/ubiq-host/src/remote.rs`; token in an `Authorization` header, URL query accepted one release for compatibility | `ubiq --serve` listens on loopback only; tests: wide bind without TLS refused; a connection over the cap refused | M | — |
| 1.2 | Scoped remote token | `read-only` vs `full` scope checked against `Message::capability` | A read-only client's `SpawnWorkspace` is refused with an error variant | M | 2.2 |
| 1.3 | Secrets out of the tape | tape serialises through a scrubbing serializer that writes `Secret` as `"<redacted>"`; dumps use `create_new` + `0600` (`crates/ubiq-proto/src/bus.rs`) | test: a message holding a `Secret` appears redacted in the ring and the dump | S | — |
| 1.4 | MCP listener limits | body cap 1 MiB, read timeout, `Host`/`Origin` check, per-run bearer token injected with the MCP URL, a small accept pool (`crates/ubiq-host/src/mcp/server.rs`) | tests: 2 MiB body → 413; wrong `Host` → 403; missing token → 401 | M | — |
| 1.5 | Worker panic isolation | `ubiq-host/src/worker.rs` `Worker<Job>`; migrate Files/Git/Search/Index; panic hook in `ubiq-app`; keep the coordinator `JoinHandle` and report its death | test: a job that panics yields a `Gone` error reply and the next job still runs | M | — |
| 1.6 | Mock data gated | `work/mock.rs`, `ui/sink/ext_demo.rs` registration and sink fixtures behind `cfg(any(test, feature = "demo"))`; `just dev` enables `demo` | Release build: 0 mock agents in a fresh project; `base_registry` has no demo slot | S | — |
| 1.7 | No compiled-in token | drop `UBIQ_FEEDBACK_GITHUB_TOKEN`/`_API_KEY` from `crates/ubiq-host/src/feedback/mod.rs`; feedback posts to the project's endpoint only | `strings` on a release binary shows no token; `option_env!` names only URLs | S | — |
| 1.8 | Small hardening | `--` before the `ag` query (`crates/ubiq-host/src/search/fallback.rs`), share the drone's argv builder; `0600` temp files in sensitive stores (`crates/ubiq-host/src/atomic.rs`); `symlink_metadata` before stale-socket removal and checked `set_permissions` (`crates/ubiq-drone/src/socket.rs`); redacting `Debug` and `skip_serializing` for `ConnectionConfig.password` (`crates/ubiq-db/src/conn.rs`) | test per item | S | — |
| 1.9 | Secret off argv | macOS `security` fed through stdin or the `keyring` engine (`crates/agent-manager/src/credentials/os.rs`) | `ps` during a store shows no secret (manual check recorded in the PR) | M | — |
| 1.10 | One writer per config root | advisory lock on `<root>/.lock` at boot; hand-off keyed by root, not by `current_exe` (`crates/ubiq-app/src/handoff.rs`) | A second `ubiq-studio` on the same root attaches or refuses; closes G29 | M | — |

### Phase 2 — seams and registries

| Step | Goal | Actions | Exit criterion | Size | Deps |
|---|---|---|---|---|---|
| 2.1 | Typed harness identity | `HarnessId` newtype in `crates/agent-manager/src/spec.rs`; `Harness` gains `meta`: display name, icon key, capabilities, `is_default`; UI icon and default come from `AgentTypeInfo` | 0 harness literals in `ui/kit/mod.rs`, `state/new_*.rs`, `state/conversation.rs` production code | M | — |
| 2.2 | Message classification | exhaustive `direction`, `pane_id`, `capability` in `ubiq-proto` (no `_` arm); `pane_id_of` in `crates/ubiq/src/app/hosts.rs` delegates to it; drone relay refuses with an error variant for every request it does not serve | removing a classification arm fails to compile; 0 silent drone drops | M | — |
| 2.3 | Panel registry | `ext/panel.rs` `PanelSpec { id, title, icon, region, persist_key, render, on_close }` in a `Registry`; dock title/icon/render/serialise look it up; `PanelKind` keeps only an id (`PanelKind::Spec(SlotId)` beside the old variants, migrated one by one) | ≤ 2 files change to add a panel; `PanelKind::` references outside `state/dock.rs` ≤ 40 (from 224) | L | 0.5 |
| 2.4 | Document kinds | `DocumentSpec` on the same pattern for centre tabs (the gap `inbox-editions` lists) | the markdown and DB documents register through it | M | 2.3 |
| 2.5 | One registry shape | `tasksrc::Registry` and `runners::Registry` use the generic container or document why not; `register` returns `Result`, `Boot` reports a misregistration once instead of panicking | 0 `panic!` in registry install paths | S | — |
| 2.6 | Feature-local specs | each rail mode spec moves beside its feature; `ui/rail.rs` `modes` becomes a list of calls | `ui/rail.rs` < 120 lines | S | 2.3 |

### Phase 3 — decomposition

Order inside the phase: 3.1 → 3.2 → 3.3 per feature, starting with git, board and kb (self-contained,
well tested), then conversation; 3.4 and 3.5 any time.

| Step | Goal | Actions | Exit criterion | Size | Deps |
|---|---|---|---|---|---|
| 3.1 | Feature entities | per feature: `Entity<XView>` owning its `state/` struct, inputs and `Render`; `AppState` keeps the handle; `receive_<family>` forwards | per feature: its fields leave `AppState`; overall `AppState` ≤ 60 fields | L (per 3-4 features) | 2.3 |
| 3.2 | Constructor split | each feature's `new` builds its inputs and placeholders; `for_project` composes | `for_project` ≤ 150 lines; 0 `InputState::new` in `app/boot.rs` | M | 3.1 |
| 3.3 | Dispatch per family | `coordinator/<family>.rs` handlers, sub-structs for field groups; `app/wire/<family>.rs` | `dispatch` ≤ 200 lines; no function in `coordinator/` over 150; `wire.rs` gone | L | 2.2 |
| 3.4 | Settings split | `ui/settings/<section>.rs` and `app/settings/<section>.rs`, one per `SettingsSectionSpec` | no settings file over 800 lines | M | — |
| 3.5 | Break `state → ui/app` | move view handles (`Entity<MdView>` in `state/editor.rs`, `PlanState` in `state/db/sql.rs`) into the feature entity; grep check in `just ui` | 0 `crate::ui`/`crate::app` imports in `state/` | M | 3.1 |
| 3.6 | No mutation in `render` | `PendingWork` with phases, or handler-side work; `AppState::render` starts with one `drain` | ≤ 1 settle call in `render` | M | 3.1 |
| 3.7 | Typed errors | `agent-manager` boundary `Error`; host family `ErrorKind`s; `UiNotice` replaces per-panel `*_error` | `Result<_, String>` in host ≤ 40 (from 186); 0 `*_error: Option<String>` fields on `AppState` | L | 3.3 |
| 3.8 | Stores off the coordinator | coalescing writer thread; `migrate(from, to)` hook + fixture test per envelope | `dispatch` does no fsync; a v0→v1 fixture test exists per store | M | 1.5 |
| 3.9 | Visibility | per crate: leaf modules private, `pub(crate)` default, `pub` re-exports in `lib.rs` | `pub(crate)` ≥ `pub fn` in `ubiq-host` and `ubiq` `app/` | M (per crate) | 0.5 |

### Phase 4 — UI components and style

| Step | Goal | Actions | Exit criterion | Size | Deps |
|---|---|---|---|---|---|
| 4.1 | Palette bridge | reuse proposal's first step: `theme.rs` builds gpui-component `ThemeConfig` from Ubiq tokens | library widgets draw in Ubiq colours in both modes | S | — |
| 4.2 | Size tokens | `theme::{space, row_h, radius, hairline, min_w_0, min_h_0}`, all scaled; sweep `ui/kit` first, then `ui/settings`, then one module per PR | `ui/kit` and `ui/settings*`: 0 `px(` literals outside the exempt list; `ui/` total ≤ 300 (from 1,567) | M + S per module | 0.5 |
| 4.3 | One menu model | `MenuItem<A>` + `menu` renderer on `PopupMenu` (§2.3); migrate context menus, `Picker`, project menu, run-tool menu, tab menu | 0 `ContextItem::new`; 0 index-based `on_pick`; `MENU_LAYER` deleted; `deferred(` in ≤ 3 files | L | 4.1 |
| 4.4 | One modal frame | `kit::overlay::modal` (or the library `Dialog` per reuse proposal §2) replaces the private frames in `ui/mission/full.rs`, `ui/settings.rs`, `ui/acp_capabilities.rs`; overlays as a stack (§2.5) | 1 modal frame; `ui/shell.rs` `render` ≤ 250 lines | M | 4.3 |
| 4.5 | Row and header dedupe | `kit::{list_row, section_header}` with a `Row` density; delete local copies | `fn row` / `fn header` defined only in `ui/kit` | M | 4.2 |
| 4.6 | Shared consts | `SPIN`, `MAX_CELL_CHARS`, animation durations defined once | no duplicated literal for the same concept (review list in the PR) | S | — |
| 4.7 | Strings beside owners | placeholders move with 3.2; menu labels live in the item builders | 0 user-facing literals in `app/boot.rs` | S | 3.2 |
| 4.8 | Keymap table | `KeyBinding` contributions collected through one registry, so a later rebinding layer has one place to read | 108 `KeyBinding::new` all registered through it | M | 2.5 |

### Phase 5 — tests and drone hardening

| Step | Goal | Actions | Exit criterion | Size | Deps |
|---|---|---|---|---|---|
| 5.1 | Contract tests | generated per-variant round trip; variant-list snapshot enforcing `MESSAGE_SCHEMA` bumps (G256); skew test: an unknown variant is skipped, the session survives | every variant round-trips; G256 closed | M | 2.2 |
| 5.2 | `FakePty` | a fake behind `pty` for coordinator tests; real-PTY tests renamed and `#[cfg(unix)]` | coordinator tests pass without spawning `/bin/sh` except `real_pty_*` | M | — |
| 5.3 | View smoke tests | `TestAppContext` table over all registered panels, rail modes and menus | every spec renders in both palettes; every menu item maps to an action | M | 2.3, 4.3 |
| 5.4 | Property tests | `proptest` for `ubiq-db` `conn` round trips, the read-only guard, `ubiq-md` blocks | three suites in CI | S | — |
| 5.5 | Coverage report | `cargo llvm-cov` job, report only (no gate) | coverage published per PR | S | 0.1 |
| 5.6 | Drone in practice | build one triple and fill `BINARIES`; CI container job runs `ubiq-drone --stdio` through a local `sshd` and deploys; error variants for the seven refused messages; a linger-expiry notice (G263); `relay.rs` split per family with inline tests | the drone job is green; 0 silent refusals | L | 2.2, 0.1 |
| 5.7 | Drone gaps | read-only roots (G267); Windows build or an explicit `cfg` exclusion so `just check` passes on Windows (G291) | G267 and G291 closed | M | 5.6 |

### Phase 6 — spikes and editions hygiene

| Step | Goal | Actions | Exit criterion | Size | Deps |
|---|---|---|---|---|---|
| 6.1 | Drop graduated forks | remove `spike/db-explorer/crates/*` and `spike/markdown-viewer/crates/*` from Studio `members` (archive the folders); delete `mdview-md`/`dbx-core` | 0 forked engine crates in any workspace | S | — |
| 6.2 | One markdown block model | finish consolidation of `ubiq_proto::blocks`, `ubiq-md` and `ui/mdview` block logic onto `ubiq-md` | markdown parsed once per document | L | — |
| 6.3 | Seam-based spike template | the `ubiq-spike` skill gains a working minimal example that contributes a rail mode and a panel through `Contributions` | a new spike compiles against the base without forking | S | 2.3 |
| 6.4 | Studio pin hygiene | `build-pinned` rev check (0.6) and a recipe that bumps all five pins at once | Studio pin equals a base `main` commit | S | 0.6 |
| 6.5 | Feature matrix honest | fix or merge `harness`/`listener`; split `listener` into `serve` and `net-client`; CI checks the supported configurations | every declared feature combination builds | M | 0.1 |
| 6.6 | Vendor record | upstream commit and update procedure for `vendor/gpui-terminal` in its Cargo metadata | the upstream rev is recorded | S | — |

### Ordering at a glance

| Week band | Steps | Why this order |
|---|---|---|
| 1 | 0.1–0.6 | Nothing else is safe to merge without the gate and the ratchet |
| 2–3 | 1.3, 1.6, 1.7, 1.8, 1.1, 1.4, 1.5 | Cheapest real risk reduction; independent of each other |
| 3–5 | 2.1, 2.2, 1.2, 1.10, 2.5, 4.1 | Classification unlocks scopes, contract tests, dispatch split and the plugin proposal's phase 0 |
| 5–10 | 2.3, 2.6, 3.4, 4.2 (kit, settings), 3.3 | Registries before entities, so features move once |
| 10+ | 3.1/3.2/3.6 feature by feature, 4.3–4.5, 5.x, 3.7–3.9, 6.x | Long tail, interleaved with feature work |

## 4. Metrics — today and target

Today's values are the audits' counts, re-checked with `grep`/`wc` on 2026-10-05. The ratchet (0.5)
records them as its baseline.

| Metric | How measured | Today | Target | Step |
|---|---|---|---|---|
| CI workflow runs `just verify` | `.github/workflows` | 0 | 1 per repo | 0.1 |
| `[workspace.lints]` / toolchain pin | manifests | absent / absent | present / present | 0.2, 0.3 |
| `.rs` source files over 2,000 lines (excl. vendor, tests) | `wc -l` | 19 | 0 | 3.x |
| Longest function | awk scan | 2,096 (`dispatch`), 2,048 (`for_project`) | ≤ 250 | 3.2, 3.3 |
| `AppState` fields | struct count | 217 | ≤ 60 | 3.1 |
| `px(` in `crates/ubiq/src/ui` | `grep -o` | 1,567 | ≤ 300 | 4.2 |
| `px(` in `ui/settings.rs` | `grep -o` | 53 | 0 | 4.2 |
| Menu mechanisms / `ContextItem::new` | audit / grep | 6 / 28 | 1 / 0 | 4.3 |
| Files using `deferred(` | `grep -l` | 15 | ≤ 3 | 4.3 |
| Places to add a panel kind | reference spread | 5–13 files | ≤ 2 | 2.3 |
| Places to add a message | transport contract checklist | 5–6, 2 silent | 3, 0 silent | 2.2 |
| `Result<_, String>` in `ubiq-host/src` | grep | 186 | ≤ 40 | 3.7 |
| `pub(crate)` in `crates/ubiq/src` | grep | 77 | ≥ number of non-seam `pub fn` | 3.9 |
| `thread::Builder` in `ubiq-host/src` | grep | 42 | ≤ 5 (inside `worker.rs` and documented one-offs) | 1.5 |
| `catch_unwind` sites covering workers | grep | 1 file (`host_meta.rs`) | every worker via `Worker` | 1.5 |
| `allow(clippy::too_many_arguments)` | grep | 68 | ≤ 20 | 0.3, 3.x |
| Harness id literals in UI production code | grep | `harness_icon` map + 3 defaults + 1 sniff | 0 | 2.1 |
| Mock agents in a fresh release project | run | 11 | 0 | 1.6 |
| `--serve` default bind | `serve_bind` | `0.0.0.0` | `127.0.0.1` | 1.1 |
| `Secret` in tape | test | clear | redacted | 1.3 |
| MCP body cap / token | `mcp/server.rs` | none / none | 1 MiB / per-run | 1.4 |
| Compiled-in tokens | `option_env!` | 2 | 0 | 1.7 |
| Per-variant round-trip tests | test count | 0 | every variant | 5.1 |
| Coordinator tests needing a real PTY | grep `/bin/sh`, `/bin/cat` | all | `real_pty_*` only | 5.2 |
| Drone run against real sshd in CI | workflow | never | every PR touching the drone | 5.6 |
| Forked engine crates in workspaces | manifests | 2 | 0 | 6.1 |

## 5. What this proposal does not do

- **No new crates for the UI.** A `ubiq-state` crate would enforce §2.1 by type, but the grep in
  3.5 gets the same guarantee for a fraction of the churn. Revisit only if the grep is gamed.
- **No async runtime, no ECS, no global event bus inside the UI.** Threads plus the existing bus are
  understood and fast enough.
- **No sweep of the existing `pub`, `px(` or strings in one PR.** The ratchet moves them as files open.
- **No i18n layer.** Strings move next to their owners, which is what an i18n layer would need first.
- **No replacement of house-style kit pieces** the reuse proposal says to keep (`slab`, `pill`,
  canvas work).

## Open questions

- Should the size ratchet count `px(0.)` at all once `min_w_0` exists, or treat it as a separate
  metric that must reach zero?
- Is a scoped `--serve` token (1.2) worth building before the plugin grant model, or should both
  wait for one capability table designed once?
- Does `PanelSpec` (2.3) absorb the refactor-plan's blocked `RailMode`/`PanelKind` `Extension`
  variants? The routing proposal it waited on is in `inbox/completed/`, so the blocker may be gone.

## Related docs

- [Snapshot](./code-organization-snapshot.md) — the counts this proposal cites
- [Refactor plan](../wip/refactor-plan.md) — the earlier split and its "leave alone" list, revisited in §1
- [Component reuse](./component-reuse-proposal.md) — the palette bridge and library widgets behind phase 4
- [Plugin system](./plugin-system-proposal.md) — `Message::capability`, shared with steps 1.2 and 2.2
- [Transport contract audit](./transport-contract-audit.md) — why new modules need anchors (phase 3 splits)
- [Editions](./editions-proposal.md) — the seam doctrine phase 2 follows
- [Backlog](../backlog.md) — G29, G38, G46, G48, G256, G263, G267, G291 referenced above
