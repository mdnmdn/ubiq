---
id: inbox-code-organization-snapshot
title: Snapshot — how the Ubiq code is organised today
kind: proposal
status: proposal
summary: A counted, descriptive snapshot of the code as it stands on 2026-10-05 — the two workspaces and their crate graph, the lint/toolchain/CI state, each crate's size and module layout, and the cross-cutting practices (message set and routing, UI state model, extension seams, threads, stores, errors, styling, menus, strings, placeholders, security surface, tests, the drone, spikes, tooling). No critique and no plan; those live in the sibling proposal.
read_when: you need the numbers and mechanisms behind a refactor discussion, you are asking how many places a new message, panel, rail mode or harness touches, or you are checking where a hardcoded value or a placeholder lives
updated: 2026-10-05
depends_on: [tech-architecture, tech-transport, tech-ui, feat-drone]
---

# Snapshot — how the Ubiq code is organised today

What the tree does, with counts. The critique and the migration plan are in
[code-organization-proposal](code-organization-proposal.md). Counts come from `wc`/`grep` and are
approximate where marked `~`; "prod" counts stop at the first `#[cfg(test)]` of a file. Paths are
under `crates/` unless a root is named. Sources: the three audit notes `wip/codeorg-ui.md`,
`wip/codeorg-host.md`, `wip/codeorg-workspace.md`.

---

# 1. Workspace and crate graph

## 1.1 Two workspaces, two repositories

| Workspace | Members | Lock | Notes |
|---|---|---|---|
| Base `ubiq/Cargo.toml` | `agent-manager`, `ubiq`, `ubiq-app`, `ubiq-db`, `ubiq-drone`, `ubiq-host`, `ubiq-md`, `ubiq-proto`, `vendor/gpui-terminal` (9) | `ubiq/Cargo.lock`: 1,325 packages, 130 names at 2+ versions | resolver 2; only `[profile.release]` (thin LTO, cgu 1, strip) |
| Studio `Cargo.toml` (parent) | `crates/ubiq-studio`, `-app`, `-host`; spikes `markdown-viewer/crates/{mdview,mdview-md}`, `db-explorer/crates/{dbx,dbx-core}`; `exclude = ["ubiq"]` | `Cargo.lock`: 1,241 packages, 114 dup names, seeded by `cp ubiq/Cargo.lock` | `[patch."https://github.com/mdnmdn/ubiq"]` maps `ubiq`, `ubiq-app`, `ubiq-host`, `ubiq-proto` to `ubiq/crates/*`; stays the last manifest section (`pinned-build.sh` cuts at the first `[patch`) |

Studio compiles whatever `ubiq/` has checked out; the base is compiled once, from the Studio root.
`ubiq-studio-app` names the base by git rev `509b469` in 5 manifest lines; the base checkout is at
`0bb6e16`.

## 1.2 Size per crate (`.rs`, tests included)

| Crate | LOC | Files | Role |
|---|---|---|---|
| `ubiq` | 219,485 (+38,077 in `tests/`) | 296 src + 60 tests | GPUI interface; depends on `ubiq-proto`, never `ubiq-host` |
| `ubiq-host` | 86,849 (69,580 src) | 153 | coordinator, PTYs, stores, workers, MCP |
| `agent-manager` | 41,886 | 65 | harness library, `am` CLI/TUI; no UI dependency |
| `ubiq-proto` | 18,965 | 45 | messages, bus, wire framing, log sink |
| `ubiq-db` | 12,763 | 18 | database engine, leaf |
| `vendor/gpui-terminal` | 7,094 | - | vendored terminal widget (in-tree member, not a patch) |
| `ubiq-drone` | 5,094 | 15 | remote lean host |
| `ubiq-md` | 2,671 | 5 | markdown block table, leaf |
| `ubiq-app` | 1,696 | 4 | composition root, `Boot`, `Contributions` |
| Studio `-host` / `ubiq-studio` / `-app` | 6,844 / 1,572 / 56 | | Azure DevOps and ClickUp providers; rail mode; boot |
| Spikes (4 crates) | 12,776 | | `dbx-core` and `mdview-md` are forks of `ubiq-db` and `ubiq-md` |

## 1.3 Dependency edges and boundary enforcement

- `ubiq-app` -> `ubiq` (optional, `ui`), `ubiq-host`, `ubiq-proto`. It is the only crate naming both halves.
- `ubiq-host` -> `agent-manager` (feature `harness`), `ubiq-db` (feature `db`), `ubiq-proto`.
- `ubiq` -> `ubiq-proto`, `ubiq-db` (without `drivers`), `ubiq-md`.
- `ubiq-drone` -> `ubiq-proto` and `ubiq-host` with `default-features = false`.
- Studio: `ubiq-studio-app` -> `ubiq-app`, `ubiq-studio` (optional `ui`), `ubiq-studio-host`; `ubiq-studio` -> `ubiq`, `ubiq-proto`, `gpui`, `gpui-component`.
- Boundaries are dependency-tree greps in `just host | ui | relay | headless | apple` (no gpui in host, no `ubiq-host` in `ubiq`, no `foundation-models` outside macOS), not type-level.

## 1.4 Features

`agent-manager`: `default = frontend (cli+tui)`, `remote`, `pty`, `inproc-mcp`, `keyring-store`; `just core` checks `--no-default-features`. `ubiq-host`: `default = full` = `git`, `index`, `harness`, `listener`, `desktop`, `db`; `harness` and `listener` are declared independent but do not build alone (T-235), the supported sets being nothing, `git`, `index`, `desktop`, `harness+listener`, `full`; `listener` bundles the inbound TCP/TLS server (`remote`) with outbound HTTP clients (`connectors`, `web_assets`). `ubiq-db`: `default = []`, `drivers` pulls rusqlite (bundled), mysql, tiberius, tokio, sqlx. `ubiq-app`: `ui`, `assist-apple`, `quickjs`, forwarded by Studio with `default-features = false`.

## 1.5 Lints, format, toolchain, dependencies

| Item | State |
|---|---|
| `[workspace.lints]`, `clippy.toml`, `rustfmt.toml`, `rust-toolchain.toml`, `deny.toml`, `rust-version` | absent in both workspaces; CI toolchain is `dtolnay/rust-toolchain@stable` |
| Edition | 2024 in all 12 manifests |
| Crate-level lint attributes | only `agent-manager/src/lib.rs:51-52`: `#![forbid(unsafe_code)]`, `#![warn(missing_docs)]` |
| `#[allow(...)]` | 66 `clippy::too_many_arguments` (24 in host), 9 `dead_code`, 4 `type_complexity` |
| `unsafe` | `ubiq` 9, `ubiq-app` 9 (Windows console FFI), `ubiq-host` 7-8, `ubiq-proto` 1, `ubiq-drone` 2 |
| Git deps | 8 at manifest level: `zed` (gpui, gpui_platform) with no `rev`, pinned by `Cargo.lock` at `6840b8d2`; `gpui-component` + assets + `gpui-wry` at `df1d07b`; `zed-scap` `4afea48`; `isol8` `14a87bd` |
| Exact pins | `sqlparser =0.63.0`; `sqlx-core/postgres =0.8.6`; `ratatui 0.30` against `crossterm 0.28` for a `unicode-width` conflict |
| Vendored | `vendor/gpui-terminal` only (90 tests, 78 unwraps, 3 allows) |
| Audit tooling | no `cargo-deny`, `cargo-audit` or coverage configuration |

## 1.6 Justfile, `verify`, CI

| Surface | State |
|---|---|
| Base `Justfile` | 327 lines, ~45 recipes. `verify: check clippy test host relay ui apple docs-lint help-check slots-check` |
| `check` / `clippy` / `test` | `cargo check --workspace --all-targets` plus `-p ubiq-app --no-default-features`; `clippy ... -D warnings`; `cargo test --workspace < /dev/null` (stdin closed for PTY passthrough tests) |
| `ui` recipe | greps for literal type sizes and icon sizes outside `theme.rs` |
| `slots-check` | `SlotId::new("` literals only under `crates/ubiq/src/ext/` (D178) |
| Outside `verify` | `core`, `headless`, `icons-check`, `docs-check`, `docs-drift`, `drone-manifest-verify`, `web-assets-verify*`, `assist` (Swift / macOS 26 SDK) |
| Studio `Justfile` | 265 lines. `verify: lock-agrees host ui check help-check base-check studio-test no-composed-project-paths`; also `verify-base` (second build), `verify-all`, `base-test` (cargo test cannot resolve dev-deps of a patched crate), `build-pinned` |
| CI | `ubiq/.github/workflows/` holds `create-release.yml`, `release-macos.yml`, `release-windows.yml` (104 lines, tag/dispatch, run `just bundle`). No workflow runs `verify`, clippy or tests. Studio has no `.github` |

---

# 2. Per crate

## 2.1 `crates/ubiq` — the GPUI interface

| Module | Lines | Files | Role |
|---|---|---|---|
| `src/ui/` | 80,748 | 150 | drawing: free functions `fn(app: &AppState, window, cx) -> AnyElement`, plus a few `Render` views (`WorkbenchPanel`, `MdView`, `ResultGrid`, `CellInput`, `JsonEditor`) |
| `src/app/` | 47,658 | 51 | `AppState` (the root view) and ~50 `impl AppState` blocks across 46 files: bus receive, actions, boot |
| `src/state/` | 45,478 | 78 | plain data and logic per feature; 14 of 78 files import gpui |
| `src/theme.rs` | 3,635 | 1 | palettes (both modes), font roles, size consts + accessors (171 `pub fn`, 519 hex literals) |
| `src/web_export/` | 2,499 | 6 | static HTML export + embedded HTTP server |
| `src/ext/` | 1,306 | 8 | extension spine: `Registry<T>`, `SlotId`, rail / settings / menu / runner specs |
| `tests/` | 38,077 | 60 | integration tests (630 `#[test]`) |

Largest sub-trees: `ui/mdview` 8,350 · `ui/sink` 7,456 · `ui/db` 6,157 · `ui/settings.rs` 6,052 ·
`ui/conversation` 4,557 · `ui/kit` 4,439 · `ui/board` 3,826 · `app/wire.rs` 3,661 · `ui/viewer` 3,626 ·
`app/db` 3,441 · `ui/mission` 3,316 · `state/db` 3,218 · `app/settings.rs` 3,042 · `state/a2ui` 2,777.

Largest files: `ui/settings.rs` 6,052 · `ui/conversation/mod.rs` 4,160 · `app/wire.rs` 3,661 ·
`theme.rs` 3,635 · `app/settings.rs` 3,042 · `state/conversation.rs` 2,939 · `tests/conversation.rs`
2,668 · `ui/sink/project.rs` 2,317 · `app/editor.rs` 2,218 · `app/mod.rs` 2,168 · `app/boot.rs` 2,060.
Twelve `src` files exceed 2,000 lines; `ui/settings.rs` alone is 2.7% of the crate.

Longest functions: `AppState::for_project` 2,048 (`app/boot.rs:9`; builds 90 `InputState`s inline,
wires ~217 fields) · file-dialog `render` 636 (`ui/file_dialog.rs:40`) · a2ui `draw` 590
(`ui/a2ui.rs:160`) · root `render` 580 (`ui/shell.rs:29`) · `receive_work` 478 (`app/wire.rs:1575`) ·
`modes`, ten `RailModeSpec` literals, 318 (`ui/rail.rs:70`).

**Layering.** One crate, three folders that see each other. Nominal direction `ui -> app::AppState -> state`:

| Fact | Count |
|---|---|
| `ui` files naming `AppState` | 93 (533 `&AppState`/`&mut AppState` mentions in 101 files) |
| `state` files importing `crate::ui` / `crate::app` | 11 / 13 (`state/editor.rs:573` holds `Entity<ui::mdview::view::MdView>`; `state/db/sql.rs:28` names `ui::db::sql::plan::PlanState`) |
| `app` files importing `crate::ui` | 16 |

**UI stacking** (top to bottom): `ui/shell.rs` -> `titlebar | rail | status_bar | dock` -> screens
(`board`, `conversation`, `git`, `db`, `mission`, `teams`, `kb`, `viewer`, `mdview`, `settings.rs`) ->
overlays (`new_agent.rs`, `clone.rs`, `file_dialog.rs`, `ask.rs`, `*_menu.rs`) -> `ui/kit` (4,439 lines:
`controls` 741, `menu` 1,039, `overlay` 362, `panel` 174, `popover` 69, plus `blocks`, `settings`,
`files`, `canvas`, `colour`, `slider`, `ribbon`, `icons`). `ui/sink/` (7,456 lines) is the style
reference screen with fixtures.

**Module style.** 25 `mod.rs` against 271 file modules; `mod.rs` files hold real code
(`ui/conversation/mod.rs` 4,160, `app/mod.rs` 2,168, `ui/dock/mod.rs` 1,573, `ui/board/mod.rs` 1,267).
The newer sub-trees (`ui/mdview`, `ui/db`, `state/explorer`) are a directory of small files; the older
big screens are monoliths (`ui/settings.rs`, `ui/new_agent.rs`).

**Visibility.** 3,461 `pub fn`, 371 `pub struct`, 202 `pub enum`, 236 `pub mod`; 0 `pub(crate) fn`, 0
`pub(crate) mod`; 77 `pub(crate)` and 285 `pub(super)` uses (almost all in `app/`). `AppState` has 217
fields, 160 `pub`.

## 2.2 `crates/ubiq-host` — the headless host

69,580 src lines, 17,250 test lines, 153 files, 1,070 `#[test]`. `lib.rs` has 47 `pub mod` and no
private module. 1,113 `pub` items against 31 `pub(crate)`; 33 `allow(` in total.

Modules by lines: `mcp/` 10,874 (11 files) · `coordinator.rs` 8,587 (7,029 prod) · `db/` 4,809 ·
`store/` 3,775 · `agent.rs` 3,655 · `tasksrc/` 3,626 · `plan/` 3,490 · `git/` 2,667 · `assist/` 2,316 ·
`mission/` 2,209 · `connectors/` 2,178 · `conversation.rs` 1,988 · `files/` 1,957 · `web_assets/` 1,896 ·
`work/` 1,699 (346 of `mock.rs`) · `search/` 1,110 · `projects.rs` 1,084 · ~25 others under 1,000
(index, kb, repos, shells, armed, catalog, help, runners, feedback, remote, ...).

Largest files: `coordinator.rs` 8,587 · `agent.rs` 3,655 · `tests/coordinator.rs` 2,471 · `plan/service.rs`
2,296 · `tests/mission.rs` 2,251 · `mcp/server.rs` 2,023 · `conversation.rs` 1,988 · `mcp/mission.rs` 1,941.

The `Coordinator` struct has 57 fields (`coordinator.rs:165-395`). Longest functions:
`Coordinator::dispatch` 2,096 lines (`coordinator.rs:1503-3598`, 183 `Message::` arms), `Coordinator::new`
307, `compose_run` 297 (`agent.rs`), `start_conversation` 264, `conversation::pump` 238, `launch` 206,
`spawn_workspace` 194.

Shape:

```
ubiq-app boot -> coordinator::start (one thread "ubiq-coordinator", JoinHandle dropped, coordinator.rs:144)
  Coordinator: panes HashMap<PaneId,Pty>, owners routing table, conversations, watchers,
    4 workers (Files/Git/Search/Index), tasksrc sync thread,
    stores Projects / Work / Settings behind 4 traits (store/mod.rs; file + memory impls),
    mcp::Serving (own loopback listener thread), assist, quota, connectors, repos, kb, plan, mission, db
  services (projects.rs, work/, plan/, mission/, kb/, tasksrc/) return Vec<Reply>; Coordinator::answer addresses
```

Services are testable without a bus. The logic of ~15 families still sits in `dispatch` arms or
`Coordinator` methods (`launch`, `spawn_workspace`, `start_conversation`, `suggest_job`).

## 2.3 `crates/ubiq-proto` — the contract

17,182 src lines, 45 files, 152 tests, 616 `pub` items, 0 `pub(crate)`. `messages.rs` (4,075) is the
enum; the rest are per-family payload modules: `tasksrc` 1,061 · `mission` 909 · `bus` 856 · `work`
800 · `connectors` 619 · `db` 618 · `settings` 580 · `conversation` 567 · `projects` 557 · `git` 511 ·
`assist` 490 · `log` 443 · `notifications` 431 · `carrier` 418 · `plan` 390 · `ids` 386 · `files` 385 ·
`wire` 339 · `kb` 326 · `drone/` 336. It holds the contract, the in-process bus (`bus.rs`: Hub,
HostEnd, Client, Mailbox, Voice, Tape), the log sink (`log.rs`) and the wire framing.

## 2.4 `crates/ubiq-app` — composition root

1,655 src lines, 4 files, 21 tests (all in `handoff.rs`). `lib.rs` 1,182: hand-parsed argv (`--serve`,
`--bind`, `--port`, `--tls-*`), Windows console FFI (9 `unsafe`), `announce`, `Contributions`, `Boot`.
`handoff.rs` 468: single-instance door (unix socket, or Windows loopback port + token file).
`main.rs` 5 lines, `build.rs` 41.

## 2.5 `crates/agent-manager` — harness library

41,886 LOC, 65 files. `harness/{mod 1029, claude 2407, codex 1494, grok 1407, copilot 1052, opencode
1009, shared}`, `io/{acp_client 4944, jsonl 2438, acp 1350, codex 1313, model 1246, acp_caps, structured,
agui, passthrough}`, `isolate.rs` 2,035, `resolve.rs` 1,756, `profile.rs` 1,148, `provision.rs` 818,
`registry/*`, `credentials/{mod, os 645, keyring_store, file, keychain, memory}`, `account`, `session`,
`settings`, `quota`, `home`, `mcp/server.rs`, `cli/*`, `tui.rs` (a stub).

| Aspect | State |
|---|---|
| Harness definition | code per harness behind `pub trait Harness` (`harness/mod.rs:481`, 24 methods); 5 impls (`Claude`, `Codex`, `Copilot`, `Grok`, `Opencode`); registered by hand in `harness::all()` (`mod.rs:711`) as 7 boxes (Claude and Codex twice: native + `*-acp`) |
| Ids | `HarnessId = String` (`spec.rs:14`); per-harness preference templates are a data seam (`TemplateStore`) with defaults in Rust |
| Config paths | 54 non-comment sites naming `.claude`, `.codex`, `.copilot`, `.grok`, `.config/opencode`, mostly in `harness/*.rs` and `home.rs` (10) |
| Models | discovered at runtime (`ModelInfo`); model id strings only in `io/jsonl.rs` (26, mostly tests), `copilot.rs` (6), `codex.rs` (8, tests) |
| Gemini CLI | named in AGENTS.md; has no harness impl; "gemini" appears in `copilot.rs`, `quota.rs`, `mcp_parse.rs`, assist providers and an icon |
| Errors | `pub type Result = anyhow::Result`; 124 `bail!`/`ensure!`; `thiserror` declared, 0 uses |
| Tests | 614 `#[test]`, 47 files with `cfg(test)`, 4 integration files; shell fakes (`fake-claude-streamjson.sh`, `fake-codex-appserver.sh`, `fake-harness.sh`, `fake-claude-interrupt.sh`) |
| Markers | 3 `TODO(P2+)`: `opencode.rs:313,323`, `codex.rs:133` (account `base_url` not honoured) |

## 2.6 Leaf crates

| Crate | Layout | Tests | Notes |
|---|---|---|---|
| `ubiq-db` (12,763 LOC, 18 files) | `conn` 1,401 (URL, libpq, ADO.NET/ODBC, JDBC parsers), `value`, `sql` (+`readonly` 584, `readonly_corpus` 445, `functions`), `edit` 1,298, `dbml` 780, `model`, `plan`; `driver/{mod 378, mssql, mysql, postgres, sqlite, plan, structure}` behind `drivers` | 172 in 14 files, no `tests/`; 4 `#[ignore]` (live server); drivers: mssql 5, mysql 2, postgres 8, sqlite 15 | `lib.rs` 27 lines of `pub mod`; `DbError`/`ParseError`/`EditError` via thiserror; forked into `dbx-core` (4 and 1 diff lines) |
| `ubiq-md` (2,671 LOC, 5 files) | `parse() -> Document` of source-ranged `Block`s over `pulldown-cmark`; `find`, `autolink`, `build` | 43, all in `tests.rs` | forked into `mdview-md` (15 diff lines); markdown block logic also in `ubiq_proto::blocks` (49 reference sites) and `ui/mdview/*` |

---

# 3. Cross-cutting practices

## 3.1 The message set and routing

| Aspect | Practice |
|---|---|
| The enum | `ubiq-proto/src/messages.rs:72` `pub enum Message`, `#[serde(tag="type", content="payload")]`, ~350 variants in ~4,000 lines; grouped by ~70 `// -- X family: UI -> host --` banners (pane, session, account, quota, feedback, agent definition, catalog, connector, repository, task-source, project, shell, host browse, file, git, kb, database, work, plan, mission, conversation, search, assist, notification, web asset, help, carrier) |
| Direction | documentation only; no type separates UI->host from host->UI; the last `dispatch` arm names the response-direction variants |
| Encoding | 4-byte BE length + MessagePack (`rmp_serde::to_vec_named`, `wire.rs`); 77 `serde(default` + 32 `skip_serializing_if`, 0 `deny_unknown_fields`; `MAX_FRAME` 64 MiB (`wire.rs:26`) |
| Versioning | `MESSAGE_SCHEMA = 1` (`wire.rs:48`) checked only in the drone handshake (`DroneHello/DroneReady`); the `--serve` TCP attach exchanges no version |
| Skew | `carrier::pump` reads `while let Ok(message) = wire::read_frame(..)` (`carrier.rs:140`); a frame that fails to decode ends the session like EOF; any `write_frame` error ends it |
| Routing | `Hub::connect` -> `Client`; host reads one `HostEnd` of `FromClient::{Connected, Said, Gone}`; replies `To::Client(id)` or `To::Everyone`; `owners: HashMap<PaneId, ClientId>`, `owns()` gates pane messages; `Mailbox`/`MovingAddress`/`Voice` for the host talking to itself |
| Channels | all `flume::unbounded` (`bus.rs:106,163,349,350`) |
| UI side | `app/hosts.rs` wraps clients (`client.send` at `:336`); `AppState::receive(host, message, cx)` (`app/wire.rs:522`) chains `receive_<family>` fns; 140 `Message::` arms in `wire.rs` |
| Bus tape | `bus.rs:564-790`: a global 500-entry ring of every message as JSON (8 KiB cap per entry); dumped to `$TMPDIR/ubiq-tape-<secs>.jsonl` or streamed to `$UBIQ_TAPE_DIR` |
| Tests | `wire.rs` has 8 tests; no per-variant serde round trip |

## 3.2 What it takes to add a thing (places that change)

| To add | Places that change |
|---|---|
| A message | variant in `messages.rs` (+ payload type in a family module); row in `tech/transport-contract.md` (3,312 lines); arm in `Coordinator::dispatch` (+ method/job); handler arm in `app/wire.rs`, often an `AppState` field; `pane_id_of` (`app/hosts.rs:207`, catch-all returns "no pane") if it carries a pane id; a decisions row if structural. The drone relay needs nothing: its catch-all refuses typed, seven messages have no error variant and are dropped with a warn. 5-6 places, two silent-failure sites |
| A message family (UI half) | `ubiq-proto` + new `receive_<family>` + chain entry at `wire.rs:522` |
| A rail mode | id in `ext/ids.rs`; spec literal (~30 lines) in `ui/rail.rs`; `PanelKind` variant if it has a side panel; `UiId` entries (`state/ui_id.rs`); `state/dock.rs` regions/defaults; renderer match in `ui/dock/mod.rs`; view-prefs decoding |
| A panel kind | closed `PanelKind` enum (`state/dock.rs`, ~26 variants; 129 refs there, 75 in `ui/dock/mod.rs`, 26 `app/panels.rs`, 18 `app/editor.rs`, 12 `app/shell.rs`, 10 `ui/rail.rs`); 5-13 files per variant (Chat 13, Terminal 10, Kb 8); title, icon, render, serialise, region rule, close/focus arms |
| A document kind (centre tab) | `PanelKind` variant + `state/<x>.rs` + `app/<x>.rs` attach/settle fn run from `render` + `ui/dock` arm + persistence in `state/layout.rs` (1,903). `DocumentBody` (`state/document.rs:49`) covers only the annotation surface |
| A settings section | `ext/settings.rs` spec + `ext/ids.rs` + body fn in `ui/settings.rs` + handler in `app/settings.rs` + `state/settings.rs`; test in `tests/settings.rs` |
| A bar menu block | `ext/menu.rs` registration + `ui/menus.rs` builder |
| A modal/overlay | new `ui/<x>.rs` fn + field and open/close flags on `AppState` + hook into the `ui/shell.rs:29` stack; 10+ `overlay()` fns exist |
| A harness | `agent-manager/src/harness/<x>.rs` (800-2,400 LOC each) + `mod`/`pub use` + `all()` entry; `io/*` bridge if it has a structured wire; `ubiq-host/agent.rs` (49 literal id sites), `store/harness.rs` (30), `coordinator.rs` (15), `conversation.rs` (6), `quota.rs` (4); `ubiq/state/workbench.rs` (19), `state/settings.rs` (9), `ui/kit/mod.rs` and `kit/icons.rs`, `state/sink.rs` (3); quota module; docs. ~9 `match`/`==` sites on harness ids in production |

## 3.3 UI state model

- One root `Entity<AppState>` per window (`app/mod.rs:668`; struct spans 720 lines, 217 fields, 7 of them feature `*State` structs, the rest flat fields and pending flags). Feature state lives in `state/<feature>.rs` structs held as `AppState` fields; `ui` reads `app.<field>`.
- `impl Render` exists for 12 types (3 are `Ghost`/`Empty` drag stubs); `RenderOnce`/`IntoElement` derives: 6. A panel is a free function rebuilt each frame; one `cx.notify()` repaints everything.

| Entity / call | Count |
|---|---|
| `Entity<InputState>` | 136 (102 `InputState::new`, 90 in `boot.rs`) |
| `Entity<AppState>` | 64 |
| `Entity<TextareaState>` / `EditorState` | 27 / 24 |
| `Entity<MdView>` / `WorkbenchPanel` / `DockArea` | 19 / 17 / 15 |
| `cx.notify()` / `cx.listener` / `ui::handler(` bridges (`ui/mod.rs:83`) | 1,228 / 868 / 183 |
| `cx.subscribe/observe` | 95 |

- `AppState::render` (`app/shell.rs:1671`) opens with ~20 `settle_*`/`attach_*`/`fill_*`/`build_*` calls whose order is commented as load-bearing; state changes happen inside the render pass. A pending-action queue was deliberately not built (`wip/refactor-plan.md`).
- Background wakeups: `cx.spawn` loops in `app/boot.rs:1714-1770` (120 ms, 150 ms, 4 s timers).
- Actions: `gpui::actions!` in 10 places; 108 `KeyBinding::new` in 11 files, each beside its context. No central keymap, no user rebinding (`state/prefs.rs` holds view prefs only).
- Globals: `impl Global` for `BusHub`, `OpenWindows`, `WindowRegistry`; ~16 `thread_local!`/`OnceLock` statics (`theme.rs` x5, caches in `ui/viewer/*`, `ui/mdview/search.rs`, `ui/ident.rs`, TLS roots in `app/remote_connect.rs`, `web_export/server.rs` `SERVER`).

## 3.4 Extension seams

All seams are fields of `ubiq_app::Contributions` (`ubiq-app/src/lib.rs:175`), seeded by `Default` with
the base's entries and passed as `Boot { stores, contributions }` to `ubiq_app::run`.

| # | Field | Type | Kind | Base seeds |
|---|---|---|---|---|
| 1 | `task_providers` | `ubiq_host::tasksrc::Registry` (`TaskProvider`, `tasksrc/mod.rs:99`) | host service | Trello |
| 2 | `runner_sources` | `ubiq_host::runners::Registry` | host service | make / just / mise |
| 3 | `settings_sections` | `ext::Registry<SettingsSectionSpec>` | UI container | 15 + 8 |
| 4 | `rail_modes` | `ext::Registry<RailModeSpec>` (13-15 fields: icon `fn()`, availability, `centre`, `default_layout`, `destination`, furniture, `on_enter`) | UI | 10 |
| 5 | `bar_menus` | `ext::Registry<MenuBlockSpec>` | UI | 4 |
| 6 | `runner_kinds` | `ext::Registry<RunnerKindSpec>` | UI | 3 |
| - | `stores` | `Box<dyn FnOnce(&Path) -> Stores>` | store decorators (encryption seam) | files |

- `ext::Registry<T: Slotted>` is 274 lines; duplicate id panics; supports relabel, reorder, remove (D177). Each container is `install()`ed once into a process-wide `OnceLock`; a second install panics.
- The two host registries are separate types with the same verbs. Ids live in `ext/ids.rs` (155 lines, `const SlotId`), checked by `just slots-check`.
- Base users: `ui/rail.rs:70` `modes()` and `ui/sink/ext_demo.rs` (the demo mode, registered in `base_registry()` at `ext/rail.rs:187`).
- Studio consumes three of them in `ubiq-studio-app/src/lib.rs:boot()`: `register_task_providers`, `ado::settings_section`, `rail::rail_modes` (`ubiq-studio-host`, 6,844 LOC: Azure DevOps and ClickUp providers).
- No cargo feature, inventory or dynamic loading. No seam for document kinds, panels beyond the rail centre, dialogs, agent-facing MCP tools or message families.

## 3.5 Concurrency and worker pattern

| Aspect | Practice |
|---|---|
| Runtime | threads only; zero `tokio`/`async fn` in the four headless crates. 16 `flume::unbounded` + 8 bounded in host, 9 `std::sync::mpsc`; 126 `Mutex` mentions, 49 `AtomicBool` |
| Coordinator | one thread, `run` loop (`coordinator.rs:1205`): `recv_timeout` capped by `CONVERSATION_POLL` 500 ms and `TASK_SYNC_EVERY` 2 s, then `dispatch` and 7 housekeeping calls (`collect_scans`, `sync_tasks_due`, `remember_sessions`, `name_conversations`, `adopt_harness_titles`, `reap_conversations`, `register_clones`) plus `projects.flush_due`; flushes projects on disconnect |
| PTY reader | per-pane reader thread -> unbounded mailbox (`Pty::forward_output`); never blocked by the UI |
| Workers | `Files`, `Git`, `Search`, `Index`: each `start() -> (Sender<Job>, thread)` with `flume::unbounded` and a `submit()` that logs on send failure (`files/mod.rs:851`, `git/mod.rs:84`, `search/mod.rs:37`, `index/mod.rs:67`), the same ~12 lines four times. `watch::start` returns a handle whose drop stops it. tasksrc has its own `Sync` handle thread |
| One-off threads | 42 `thread::Builder` sites in host (9 in `coordinator.rs`) with ad hoc names: clone, kb sync, web_assets, connector flows, repos, quota, feedback, suggest, naming |
| MCP | one `ubiq-mcp` accept thread handles requests inline; thread-per-call for ask and SQL (SQL capped by `MAX_CALLS`) |
| Panics | `catch_unwind` only in `host_meta.rs` (2); no panic hook; `coordinator::start` drops the `JoinHandle`; a panicking worker leaves later requests logged and dropped |
| Blocking on the coordinator | `dispatch` has no direct `std::fs`/`Command`/`sleep` (one `remove_dir_all` at 3472); exceptions are the three file-backed stores (G46) and the spawn folder probe (G38) |
| Shutdown | no explicit host shutdown; `Pty` has no `Drop`; `client_gone` runs five ordered teardown steps; workers end when senders drop; remote carrier: `STOP_POLL` 200 ms, 20 s heartbeat, 3 misses = gone |
| Remote | `remote.rs` thread-per-connection (`accept_loop`, `:240`), 8 KiB header cap, 10 s handshake deadline, no connection cap; `carrier::pump` = 2 threads per session over an unbounded per-client channel |

## 3.6 Persistence and stores

Config root resolves flag > `UBIQ_CONFIG_DIR` > nearest `ubiq.toml` > `~/.config/ubiq` (`config.rs`).

| Aspect | Practice |
|---|---|
| Files | `projects.toml`, `preferences.toml`, `host-settings.toml`, `ui-settings.toml`; per project `project.toml`, `tasks.toml`, `view.toml`, `kb.toml`, `db.toml`, `tasksrc.toml`, `mission.toml`, plan sidecars, a notes file; `accounts/`, `profiles/`, `environment.toml`, `harness-models.toml`, `ai-models.json`, `catalog.json`, `usage.db` (SQLite), per-conversation `conversation.json` + `store.jsonl`, `keychain/`, search index dirs, `ui/` workarea |
| Writes | `atomic::write_atomic` (temp sibling `.name.pid.counter.tmp`, `sync_all`, rename, dir sync), 45 production call sites; temp file takes the default umask; `write_atomic_with` carries an existing mode. Raw `fs::write` remains in `help/mod.rs:236`, `web_assets` (`.part`), append logs (`conversation.rs:208`, `store/mission.rs:125`, `log.rs:354`), `bus.rs:751` |
| Debounce | projects flush 400 ms (`projects.rs:74`); whole file held in memory and rewritten wholesale |
| Corruption | parse failure -> `atomic::preserve_aside` to `*.corrupt-<stamp>`; `gc::collect` only after a good load |
| Versions | every envelope `version: u32 = 1` (`CATALOGUE_VERSION`, `TASKS_VERSION`, `PREFERENCE_VERSION`, `SETTINGS_ENVELOPE_VERSION`, `MISSION_VERSION`, `HARNESS_CACHE_VERSION`, `ANNOTATIONS_VERSION`); `StoreError::UnknownVersion` refuses a newer file; no migration code beyond `project_dir::migrate_project` and an `agent.rs` profile rename |
| Locking | none (`flock`/`fs2`: 0 hits); last-writer-wins on every store (G29); `handoff.rs` hashes `current_exe()`, so it blocks only the same executable path |
| Credentials | `agent_manager::credentials::OsSecretStore` (OS keychain); records hold references; DB passwords sealed with a keychain key (`ubiq-host/db/secrets.rs`); `Secret` newtype has redacting `Debug` but `#[serde(transparent)]` (`messages.rs:3944`). `agent-manager/credentials/` stores secret blobs through file (0600), `os` (macOS `security`, Linux `secret-tool`, Windows DPAPI), `keyring`, memory engines |
| Drone | `state.rs` JSON state file beside the socket (5 s refresh); scrollback ring in memory only |

## 3.7 Error handling

| Crate | Style | Counts |
|---|---|---|
| `ubiq` | strings on `workbench`/`settings`/`kb`/`catalog`/`tasksrc`/`web_panel`/`teamsim` | ~6 field families (`project_error: Option<String>`, `*_error`, `notice`), ~150 error assignments in `app/`; `anyhow` 9 |
| `ubiq-host` | `Result<_, String>` 186-222 sites; `anyhow` 50-60; 67 `map_err(format!)`; `StoreError`, `SecretsError` (thiserror, 3 uses) | `let _ =` 113, `.ok();` 24, `tracing::warn/error` 152 |
| `ubiq-proto` | hand-written enums with hand-written `Display`: `WireError`, `FileError`, `GitError`, `CloneError`, `SearchError`, `ConnectError`, `HandshakeError`, `DroneError`, `HostPathError` | 0 thiserror |
| `agent-manager` | `anyhow` alias | 124 `bail!`, 183 `anyhow` refs |
| `ubiq-db` | thiserror `DbError`/`ParseError`/`EditError` | `Result<T, String>` in `driver/plan.rs` |

- Errors reach the UI as per-family message variants (`PaneError{error:String}`, `ProjectError`, `SuggestError`; typed for files, git, search, connect, clone).
- Host-originated notices go through the bell (`state/notifications.rs`); UI-side failures render as a red line or banner per panel. Logs go to `ubiq_proto::log::logs()`.
- Production `unwrap`/`expect`: host 21/31, `ubiq` 21/27, drone 2/22, proto 0/1, agent-manager 2/6, db 7/4. Drone `expect`s are mostly `thread::Builder::spawn(..).expect` and mutex poison. `ext::Registry` boot assertions panic by design.
- `panic!(`: agent-manager 89, `ubiq` 61, host 177, proto 14 (mostly tests). No `println!`/`eprintln!` in non-test UI code.

## 3.8 Styling, tokens and sizes

| Aspect | Practice |
|---|---|
| Colour and scale | `theme::<token>()` functions (2,745 calls) and `theme::UPPER` consts (471) in `ui`; 0 hex literals outside `theme.rs`; `ui_scale` scales tokens and `theme::font`, not raw `px()` |
| Text size | `theme::font(Family, Role)` (713 uses, `theme.rs:1005`); 11 raw `.text_size(px(`; 2 `rems(` |
| Icon size | `theme::icon_{sm,md,lg}()` (`theme.rs:94-106`) |
| Layout tokens | ~35 consts in `theme.rs:121-199` (TITLEBAR_HEIGHT 34, STATUS_BAR_HEIGHT 30, RAIL_WIDTH 56, EXPLORER_WIDTH 300, CHAT_WIDTH 420, DOCK_HEIGHT 300), each with a `scaled()` accessor (45 `scaled(` calls) |
| Spacing | not tokenised: 1,566 `px(` in `ui` (app 27, state 23, theme 20); `px(0.)` 508 (476 are `min_w/min_h(px(0.))`); 990 `.h/.w/.min_*/.max_*(px…)`; 1,416 Tailwind-style rem calls (`.p_2`, `.gap_1`); 428 `.bg(` |
| Row heights | hand-typed: `px(30.)` x47, `px(26.)` x38, `px(28.)` x32, `px(16.)` x19, `px(24.)`/`px(22.)` x18, `px(8.)` x17, `px(20.)` x15, `px(34.)` x12 |
| Widget helpers | `ui/kit`: `icon_button` (60 sites), `ghost_button` (97), `panel` (49), `slab`, `field`, `pill`, `card`, `modal*`, `confirm_modal`, `prompt_modal`, `popover`, `tab_strip`; local clones: `fn row` x14, `fn header` x13, `fn label` x4, `fn tab` x4, `fn chip` x3, `fn button` x3 |
| gpui-component | 7 parts used (editor, input, dock, text view, tooltip, scrollbar, keycap); window wrapped in `Root` (`app/mod.rs:1097`) whose dialog/popup/notification layers are live but unused (`inbox-component-reuse`) |
| Popover plumbing | `deferred(anchored()...)` + outside-click dismiss in ~15 files; `deferred(` in 15 files, `anchored()` in 16; own `modal`/`dialog` in `ui/mission/full.rs:54`, `ui/settings.rs:89`, `ui/acp_capabilities.rs:34` |

## 3.9 Menus and context menus — six mechanisms

| # | Mechanism | Where | Model | Pick resolution |
|---|---|---|---|---|
| 1 | Bar menus `MenuEntry` | `ui/menus.rs` (520) + `ext/menu.rs` (126) | data: label, icon, tooltip, detail, disabled, own action closure `Rc<dyn Fn(&mut AppState,…)>`; `Registry<MenuBlockSpec>`; one walker `overlay()` | by closure; `overflow_menu.rs`, `new_pane_menu.rs`, `new_project_menu.rs`, `hidden_agents_menu.rs` are 33-35-line stubs on it |
| 2 | Context menus `kit::ContextItem` + `context_menu`/`context_panel` | `ui/kit/menu.rs:853-1039` | data (label/detail/icon/tooltip/enabled/separator), no action | `on_pick(index)` into a list the handler rebuilds (`AppState::pick_*_menu`); 28 `ContextItem::new` in 13 call sites |
| 3 | Dropdowns `kit::Picker` / `MultiPicker` | `ui/kit/menu.rs:44-760` | `Vec<String>` + index sets (selected, disabled, separators, dim, dots, details) | by index; ~45 call sites; no keyboard navigation |
| 4 | Project picker popover | `ui/project_menu.rs` (794) | bespoke rows, filter, hover, outside-click | own |
| 5 | Run-tool menu | `ui/run_tool_menu.rs` (339) + own `actions!` | bespoke | own |
| 6 | Tab menu, navigator, outline jump, help target | `ui/tab_menu.rs` (140), `ui/navigator.rs`, `ui/mdview/outline.rs`, `ui/help_target.rs` | each its own `deferred+anchored` | own |

Layer priorities: `MENU_LAYER = 1`, `MODAL_MENU_LAYER = 3` (`ui/kit/menu.rs:22,29`) and a hard-coded
`.priority(1)` in `context_menu`. `MouseButton::Right` is handled in 8 files, each opening its own state
slot on `workbench` (`workbench.mission_menu`, ...). `context_panel` row height is `px(28.)`;
`context_menu` and `menu_panel` render separately. `mission/menu.rs` documents "matched by position".

## 3.10 Strings and i18n

No i18n layer (no fluent, gettext or `t!`). Strings are inline literals in `ui/*`, `app/*`
(placeholders, notices) and `state/*` (`fn label()`/`fn note()` on enums, e.g. `MissionMenuRow::label()`);
~70 `.child("literal")` and ~1,200 capitalised multi-word literals in ui/app. Partial centralisation:
`RailModeSpec.label`/`.note`, `MenuEntry` data, `state/help.rs` + `ui/help` keyed
by `UiId`, `ui/kit/settings.rs` row labels. English only; `locale` is not read.

## 3.11 Hardcoded values and placeholders

Markers: `TODO` 4 in `ubiq` (2 doc comments in `state/new_agent.rs:264,878`, 2 test strings),
5 real in the tree (3 in agent-manager); `FIXME` 0, `XXX` 0, `unimplemented!` 0; host 0. Gaps are
tracked in `_docs/backlog.md`.

| What | Where |
|---|---|
| ~223 `placeholder` mentions, 111 in boot (`"Go to file…"`, `"Commit message — subject, blank line, body"`) | `ubiq/src/app/boot.rs:27-…` |
| Example paths/commands as literals (`"claude"`, `"~/.cache/shared…"`, `"~/.ssh/id_ed25519"`) | `app/boot.rs:186,364,499` |
| Harness-name -> icon map (`"claude-code"`, `"codex-acp"`, `"opencode"`, `"GitHub Copilot"`, `"Grok CLI"`) keyed by display name | `ui/kit/mod.rs:28-34` |
| Default agent type `"claude-code"` | `state/new_mission.rs:187`, `state/new_agent.rs:740`, `state/teamsim.rs:1009` |
| `.contains("claude")` special case | `state/conversation.rs:1354` |
| `short_model_label` family rules | `state/conversation.rs` |
| Kitchen-sink fixtures (`MENU_ITEMS = ["Claude Code","Codex","Gemini CLI","opencode"]`, `gpt-5`, `gemini-3-pro`) | `state/sink.rs:250,334,342` |
| Demo/sample data: `ui/sink/*` (7,456), `ui/sink/ext_demo.rs` (registered in the production registry, `ext/rail.rs:187`), `state/sink.rs` (1,671), `state/teamsim.rs` (1,493), `state/teams.rs` | `ubiq/src/{ui,state}` |
| Mock work: `work/mock.rs` (346 lines), five fixed sessions and eleven agents with literal ULIDs minted for every project in `Work::mock`, answered through `prepare()` on every public method (G48, G52, G94) | `ubiq-host/src/work/mod.rs:230-241,1070,1203` |
| Bare timers (120 ms, 4 s, 150 ms) | `app/boot.rs:1726,1758,1770` |
| Animation durations 2000/1400/1100 ms | `ui/conversation/mod.rs:673,1649,1680` |
| Spin 2400 ms (duplicated) | `ui/kit/controls.rs:446`, `ui/mark.rs:66` |
| `Duration::from_*(literal)`: 26 in `ubiq` (7 named consts); 52 inline in host production code; 169 named `const` Duration/usize/u64 in the three library crates | `ubiq`, `ubiq-host` |
| Limits: 512 KiB / 2000 defs; `MAX_CELL_CHARS = 300` x2; 6 s / 8 KiB / 200 ms | `ui/outline.rs:31,35`; `ui/db/results.rs:21`, `ui/db/grid.rs:43`; `app/remote_connect.rs:51,57,61` |
| Host constants: `CONVERSATION_POLL` 500 ms, `TASK_SYNC_EVERY` 2 s, `RUNNER_FRESH` 3 s, `SUGGEST_DEADLINE` 60 s, `HANDSHAKE_TIMEOUT` 10 s, `MAX_HEADER` 8 KiB, `MAX_FRAME` 64 MiB, ping 20 s, pane start 80x24, watch `QUIET` 150 ms / `LATEST` 1.2 s, `IDLE_GRACE` 5 min; drone linger 600 s, `STARTUP_GRACE` 30 s, `TICK` 250 ms, `IDLE` 3600 s; db `IDLE` 10 min, `EDITOR_TIMEOUT` 5 min | `ubiq-host`, `ubiq-drone` |
| Fixed endpoints: OAuth redirect `127.0.0.1:47821` (`ubiq-proto/connectors.rs:33`, `flow.rs:330`); MCP `127.0.0.1:0`; remote default port 7420 on all interfaces; MCP protocol `"2024-11-05"` (`mcp/server.rs:51`); `/bin/sh` fallback (`shells.rs:50`); shell list `zsh,bash,fish,sh` (`shells.rs:24`) | `ubiq-host`, `ubiq-proto` |
| Provider URLs: `connectors/providers.rs` (GitHub, GitLab, Azure DevOps, Atlassian, Google, Trello, `api.trello.com`), `assist/api.rs:221-223` (OpenAI, Anthropic, Gemini), `feedback/issues.rs:47`, `agent-manager/account.rs:322`, `harness/opencode.rs:440`; `web_assets/manifest_drawio.rs` pinned to `drawio@31.4.5` with SHA-256 | `ubiq-host`, `agent-manager` |
| Compiled-in secrets: `option_env!("UBIQ_FEEDBACK_API_KEY")`, `UBIQ_FEEDBACK_GITHUB_TOKEN`, OAuth client ids | `ubiq-host/src/feedback/mod.rs:35-40`, `feedback/issues.rs:21-24` |

## 3.12 Security-relevant practices

| Surface | Practice |
|---|---|
| Process spawn | `CommandBuilder::new(program)` + `.arg()` per arg (`pty/mod.rs:139-147`, `agent.rs`); no `sh -c`; env via `env`/`env_remove`/`env_clear`; login shells via `new_default_prog`; the program name is whatever the client sent. Other `Command` sites: `kb/ops.rs:160-182`, `runners/mod.rs:175`, `shells.rs:413`, `search/fallback.rs:62`. `search/fallback.rs:82` passes the `ag` query without `--`; the drone's `search.rs:461-492` uses `--` |
| File paths | `files/path.rs`: textual component refusal, `canonicalize`, containment, leaf symlink refused on write, 64-component cap; 29 call sites |
| Host browse | `BrowseHostDir` and `JobKind::HostWrite` take absolute host paths with no project |
| `--serve` (`remote.rs`, `ubiq-app/lib.rs:355-430`) | TCP; bare `--serve` binds every interface; plaintext HTTP unless `--tls-cert/--tls-key`; one process-lifetime 256-bit token in `GET /attach?token=`, constant-time compare, 401 otherwise; a holder reaches the whole message set (`SpawnWorkspace` with arbitrary program, host browse/write, secrets, git write); `announce()` states so; no rate limit, cap, scope or rotation |
| MCP listener (`mcp/server.rs`) | tiny_http on `127.0.0.1:0`, one port for all agents; path `/mcps/<agent-ulid>/<name>`; no auth beyond loopback and the unguessable id; no Origin/Host check; request body `read_to_string` unbounded (`:222`). 12 servers: test, project-info, manage-ubiq-tasks, use-task, ubiq-plan, ubiq-mission, use-mission, ubiq-kb, ubiq-help, ubiq-ask, ubiq-sql-read, ubiq-sql-write; per-profile opt-in |
| Bus tape | serialises every message (`bus.rs:624-647`) and `Secret` serialises transparently; dumps go to `$TMPDIR` (default perms, predictable name) or `$UBIQ_TAPE_DIR` |
| Drone transport | stdio over `ssh`, or unix socket `0600` in dir `0700` (`socket.rs:300-320`, `set_permissions` result ignored at `:307`); no auth inside the protocol; SHA-256 hash-pinned binaries (not signed); manifest `BINARIES` is empty |
| ssh argv (`app/ssh_connect.rs`) | target refused if empty, leading `-` or control chars (`:305,717`); user via `-l`; `BatchMode=yes` without a secret; `SSH_ASKPASS_REQUIRE=force` helper |
| Accounts | `account.rs:33` `api_key_env`/`auth_token_env` hold env var names; value read at launch (`harness/claude.rs:195-203`) |
| Secret on argv | `credentials/os.rs:295-306,383-395` passes the vault password and credential value as `security add-generic-password -w <secret>`; carries a `ponytail:` comment. PowerShell script built by `format!` with a quoted path (`os.rs:520-545`); secret via `$env:AM_SECRET` |
| DB config | `ubiq-db::ConnectionConfig` has `password: Option<String>` with `#[derive(Debug, Serialize)]` (`conn.rs:126-150`); `to_url(mask_password)` exists; host seals passwords with AEAD (AAD per project + connection) and tests that they never reach disk in clear |
| SQL | `quote_ident` per dialect (`edit.rs:45`); user SQL runs as given behind the parser-based `sql::readonly` guard (584 LOC + 445-line corpus) |
| Confined runs | `agent-manager::Launch` (`program`, `args`, `env`, `env_remove`, `env_clear`); `isolate.rs` (2,035 LOC, isol8 sandbox, `kill_descendants_on_exit`); 23 non-test mentions of dangerous/bypass permission flags in provisioners |
| Dependencies | `gpui` git deps without `rev` in manifests (`ubiq/crates/ubiq/Cargo.toml:51-52`, `ubiq-app/Cargo.toml:95-96`); `rusqlite` bundled; TLS via rustls/ring |

## 3.13 Tests

| Crate | `#[test]` | Files with `cfg(test)` | `tests/*.rs` |
|---|---|---|---|
| `agent-manager` | 614 | 47 | 4 |
| `ubiq` | 1,200 (570 inline in 67 files: state 266, ui 179, app 86, theme 15, web_export 13, ext 11; 630 in `tests/`) | 67 | 60 |
| `ubiq-host` | 1,070 (~480 in `tests/`, ~590 inline) | 65 | 24 |
| `ubiq-proto` | 152 | 18 | 4 |
| `ubiq-db` | 172 | 14 | 0 |
| `ubiq-drone` | 33 (14 unit, 19 integration) | 5 | 6 |
| Studio `ubiq-studio` / `-host` / `-app` | 14 / 83 / 0 | 2 / 3 / 0 | 2 / 5 / 0 |
| **Total** | **~3,490** | | |

- **Harnesses.** `ubiq`: `gpui` with `features = ["test-support"]` (`Cargo.toml:243`), `TestAppContext`/`cx.add_window`; tests mount small purpose-built views such as `ConversationHarness` (`tests/conversation.rs:1612`), not the real `AppState` render. Host: real types against temp dirs; coordinator tests spawn real `/bin/cat` and `/bin/sh -c` (`tests/coordinator.rs:210,356,1563`), unix-only; `conversation.rs` has an in-file fake harness (`:1383,:1448`). agent-manager: shell fakes. Drivers: 4 `#[ignore]`d live-server tests. Helpers: Trello fixtures, `readonly_corpus.rs`, `tempfile`. Biggest: `ubiq/tests/conversation.rs` 2,668, `ubiq-host/tests/coordinator.rs` 2,471.
- **Host `tests/` spread.** coordinator 53, git 73, files 60, work 59, mission 41, projects 29, search 24, settings 19, diff 17, conversation 16, trello 15, remote 6, watch 5.
- **Not present.** No snapshot, property, fuzz, benchmark or coverage framework in any manifest; no visual/layout test; no test that every rail mode or panel kind renders; no menu draw-versus-pick test; no per-variant serde round trip; no TLS listener test; no panic-in-worker test.
- **Untested files (UI).** 125 of 150 `ui/` files have no inline test; none in `ui/settings.rs`, `ui/sink/project.rs`, `ui/board/form.rs`, `ui/dock/mod.rs`, `ui/kit/menu.rs`, `app/wire.rs`, `app/settings.rs`, `app/editor.rs`, `app/boot.rs`. `app/wire.rs` is covered only by `tests/` that push messages.
- **Environment.** `just test` closes stdin; two coordinator tests (watch events, home listing) fail only in the agent sandbox; `DOCS_RS=1` skips the foundation-models Swift build. Cargo tests for Studio run through `just base-test`.

## 3.14 The drone (`ubiq-drone`) — status

A lean host (no harness, git, index, listener) a Ubiq on another machine drives over `ssh` stdio, or
that lingers behind a unix socket. Binary flags: `--stdio|--listen|--attach|--foreground|--linger|--root|
--root-id|--list|--status|--stop|--probe|--version`, hand-parsed (`main.rs`, 297). 3,518 src lines, 15
files, 33 tests, prod `unwrap` 2, no TODOs; depends on `ubiq-proto` and the lean host only.

Files: `relay.rs` 1,208 (run loop standing in for the coordinator; catch-all refuses typed; 0 inline
tests) · `search.rs` 630 (`rg`/`ag`/`grep` with `--` and `-F/-e`) · `socket.rs` 616 (unix socket,
preamble, `hold()` via tmux/screen/setsid) · `state.rs` 195 · `scrollback.rs` 193 · `linger.rs` 174 ·
`carrier.rs` 124 (stdio into `ubiq_host::carrier::pump`) · `lib.rs` 81.

Also `ubiq-proto/src/drone/` (manifest + verify, 336 lines) and the deployer, ssh profiles and askpass in
`crates/ubiq/src/app/ssh_connect.rs` (1,481 lines).

| Aspect | Status |
|---|---|
| Built | phases 1-9: handshake + heartbeat, ssh profiles, attach over ssh, detach + linger, managed drones, per-project origin `runs_on`, provenance, search |
| Designed, not written | phase 10: drone as an MCP tool for agents (`drone_*`, `RunCommand/CommandFinished`, per-root read-only mode); G266-G268; no `mcp/` code mentions drones |
| Never run | nothing has run against a real `ssh`/`scp`/`sshd` (G258, G261, G262); `BINARIES` manifest is empty so the deployer refuses every triple; `just drone-build` never run |
| Behaviour | linger expiry is silent (G263); seven messages refused with no error variant; `DiffProjectFile` refused; `git` capability never advertised (G264); no Windows support (G291, `state.rs` uses `UnixStream`); schema bump not mechanically verified (G256); heartbeat breaks pre-heartbeat peers (G257) |
| Security model | ssh is the auth; an attached peer gets an arbitrary-program pane plus read/write on roots (no read-only mode, G267); tmux session name `ubiq-drone-<stem>` can collide across users and roots; stale socket removal deletes any non-socket file at the path (`socket.rs:~316`) |

## 3.15 Spikes

`spike/markdown-viewer` (`mdview`, `mdview-md`) and `spike/db-explorer` (`dbx`, `dbx-core`) are
standalone GPUI binaries that "depend on nothing under `crates/` or `ubiq/`". `mdview-md` forks
`ubiq-md` (15 diff lines); `dbx-core` forks `ubiq-db` (`conn.rs` 4 diff lines, `edit.rs` 1). Both
graduated into the base (`ubiq-md`; the DB rail mode + `ubiq-db`) and remain active Studio workspace
members. `spike/archify` is untracked docs only.

Neither existing spike uses the `Contributions` seams. The `ubiq-spike` skill describes a different
model: a `spike/<name>/crates/*` member in the Studio `Cargo.toml`, a `Contributions` mutation in its
own `main`, optionally a settings section; a new document kind needs a base change.

## 3.16 Tooling and docs coupling

- `_tools/` holds 5,482 lines of Python and shell: `docs.py` 1,150, `tape-subagents.py` 1,268, `webassets.py` 927, `helpbundle.py` 622, `icons.py` 609, `dump.py` 409, `drone.py` 235, `toolchain-smoke.sh` 194, `icns.py` 68, `teamsim/*` (own `test_algos.py`), `Info.plist`; PEP 723 `uv run` scripts. `_tools/README.md` lists `excalidraw.py` (not in the tree; the `just diagram` recipe calls it) and omits `dump.py`, `drone.py`, `helpbundle.py`, `toolchain-smoke.sh`.
- Base also has `_devops/scripts/bundle-version.sh`; Studio has `bundle*.sh` and `pinned-build.sh`.
- Docs: 128 markdown files under `ubiq/_docs/` with YAML frontmatter (`id`, `status`, `read_when`, `updated`, `verified`, `code_anchors`, `depends_on`, `review_cycle`). `just docs-lint` (in `verify`) checks mechanics; `docs-drift` finds documents whose anchored files moved after `verified`; `docs-touched` maps a diff to owed documents; `docs-index` generates `INDEX.md` and the code map (`docs-check` verifies, outside `verify`).
- The same-commit duty relies on discipline and lint, not CI. The Studio docs graph is directed (Studio -> base only). Eight skills under `ubiq/.claude/skills/` are the working references.

---

# 4. Key numbers

| Metric | Value |
|---|---|
| Workspaces / base members / Studio members | 2 / 9 / 7 |
| Base LOC by crate | `ubiq` 219,485 · `ubiq-host` 86,849 · `agent-manager` 41,886 · `ubiq-proto` 18,965 · `ubiq-db` 12,763 · `gpui-terminal` 7,094 · `ubiq-drone` 5,094 · `ubiq-md` 2,671 · `ubiq-app` 1,696 |
| Files over 2,000 lines | 23 (12 in `ubiq` src) |
| Largest file / function | `coordinator.rs` 8,587 / `Coordinator::dispatch` 2,096 |
| `AppState` fields / `for_project` lines / `impl AppState` blocks | 217 (160 `pub`) / 2,048 / ~50 in 46 files |
| `Coordinator` fields / `dispatch` arms | 57 / 183 |
| `Message` variants / `wire.rs` arms | ~350 / 140 |
| `PanelKind` variants / files per variant | ~26 / 5-13 |
| `pub` vs `pub(crate)` (`ubiq` / host) | 3,461 `pub fn`, 0 `pub(crate) fn` / 1,113 vs 31 |
| `px(` literals in `ui` / hex literals outside `theme.rs` | 1,566 / 0 |
| `[workspace.lints]` / MSRV / toolchain pin / CI test workflow | none / none / none / none |
| `thread::Builder` sites in host / worker `start()` copies | 42 / 4 |
| `#[test]` total | ~3,490 |
