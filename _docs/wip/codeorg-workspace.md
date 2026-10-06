# Code organisation review: workspace level

Scope: base `ubiq/` (Cargo workspace, 9 members) and Studio wrapper at the parent (3 crates + 4 spike members).
Read-only analysis, 2026-10-05. Counts are from grep/wc; "prod" counts use a heuristic (lines before the first
`#[cfg(test)]` of each non-`tests/` file), so they are indicative, not exact.

---------------------------------------------------------------------
# Section A: factual snapshot
---------------------------------------------------------------------

## A1. Workspaces and crate graph

| Workspace | Members | Lock | Notes |
|---|---|---|---|
| Base `ubiq/Cargo.toml` | `agent-manager`, `ubiq`, `ubiq-app`, `ubiq-db`, `ubiq-drone`, `ubiq-host`, `ubiq-md`, `ubiq-proto`, `vendor/gpui-terminal` | `ubiq/Cargo.lock` (1325 packages, 130 names at 2+ versions) | resolver 2, only a `[profile.release]` (thin LTO, cgu 1, strip) |
| Studio `Cargo.toml` | `crates/ubiq-studio`, `-app`, `-host`, `spike/markdown-viewer/crates/{mdview,mdview-md}`, `spike/db-explorer/crates/{dbx,dbx-core}`; `exclude = ["ubiq"]` | `Cargo.lock` (1241 packages, 114 dup names), seeded by `cp ubiq/Cargo.lock` | `[patch."https://github.com/mdnmdn/ubiq"]` maps 4 crates (`ubiq`, `ubiq-app`, `ubiq-host`, `ubiq-proto`) to `ubiq/crates/*`; must stay last section (Cargo.toml, `pinned-build.sh` strips at first `[patch`) |

Source size (`.rs` only, with `tests/` included):

| Crate | LOC | files | role |
|---|---|---|---|
| `ubiq` (GPUI interface) | 219,485 | 357 | UI; depends on `ubiq-proto`, never `ubiq-host` |
| `ubiq-host` | 86,849 | 153 | coordinator, PTYs, stores, workers |
| `agent-manager` | 41,886 | 65 | harness library, `am` CLI/TUI |
| `ubiq-proto` | 18,965 | 45 | messages, bus, wire |
| `ubiq-db` | 12,763 | 18 | DB engine leaf |
| `vendor/gpui-terminal` | 7,094 | - | vendored terminal widget (in-tree member, not a patch) |
| `ubiq-drone` | 5,094 | 15 | remote lean host |
| `ubiq-md` | 2,671 | 5 | markdown block table leaf |
| `ubiq-app` | 1,696 | 4 | composition root + `Boot`/`Contributions` |
| Studio: `ubiq-studio-host` / `ubiq-studio` / `ubiq-studio-app` | 6,844 / 1,572 / 56 | | |
| Spikes (4 crates) | 12,776 | | `dbx-core` and `mdview-md` are forks of `ubiq-db` / `ubiq-md` |

Dependency edges (from manifests): `ubiq-app` -> `ubiq` (opt, `ui`), `ubiq-host`, `ubiq-proto`; `ubiq-host` -> `agent-manager`
(feature `harness`), `ubiq-db` (feature `db`), `ubiq-proto`; `ubiq` -> `ubiq-proto`, `ubiq-db` (no `drivers`), `ubiq-md`;
`agent-manager` has no UI dep and no workspace-internal dep. Studio: `ubiq-studio-app` -> `ubiq-app` (git, rev `509b469`),
`ubiq-studio` (opt `ui`), `ubiq-studio-host`; `ubiq-studio` -> `ubiq`, `ubiq-proto`, `gpui`, `gpui-component`.
Boundaries are enforced by greps in `just host | ui | relay | headless | apple` (dependency-tree greps: no gpui in host, no
`ubiq-host` in `ubiq`, no `foundation-models` outside macOS), not by the type system.

Features worth knowing:
- `agent-manager`: `default=frontend(cli+tui)`, `remote`, `pty`, `inproc-mcp`, `keyring-store`. `just core` checks `--no-default-features`.
- `ubiq-host`: `default=full`; `git`, `index`, `harness`, `listener`, `desktop`, `db`. `harness` and `listener` are declared
  independent but **do not build alone** (Cargo.toml comment, T-235): supported configs are nothing, `git`, `index`, `desktop`,
  `harness+listener`, `full`.
- `ubiq-db`: `default=[]`; `drivers` pulls rusqlite(bundled)/mysql/tiberius/tokio/sqlx-core+sqlx-postgres.
- `ubiq-app`: `ui`, `assist-apple`, `quickjs`. Studio mirrors by forwarding with `default-features=false` upstream.

## A2. Lints, format, toolchain, hygiene

| Item | State |
|---|---|
| `[workspace.lints]` | **absent** in both workspaces |
| `clippy.toml` / `rustfmt.toml` / `rust-toolchain.toml` / `deny.toml` | **absent** (only under `refs/rgitui`, a reference checkout) |
| MSRV (`rust-version`) | **none**; CI uses `dtolnay/rust-toolchain@stable` |
| Edition | 2024 everywhere (all 12 manifests) |
| Crate-level lint attributes | `agent-manager/src/lib.rs:51-52` `#![forbid(unsafe_code)]`, `#![warn(missing_docs)]`; nothing else in any crate |
| `#[allow(...)]` | 66x `clippy::too_many_arguments`, 9x `dead_code`, 4x `type_complexity` across `ubiq`, `ubiq-host`, `agent-manager` |
| `unsafe` | `ubiq` 9, `ubiq-app` 9, `ubiq-host` 7, `agent-manager` 10 (all cfg/Windows/ffi style; the 10 in agent-manager are comments/doc mentions of the forbid), `ubiq-proto` 1, `ubiq-drone` 2 |
| Git deps | 8 manifest-level: `zed` (gpui, gpui_platform; **no `rev` in manifests**, pinned only by `Cargo.lock` at `6840b8d2`), `gpui-component` + assets + `gpui-wry` (rev `df1d07b`), `zed-scap` (rev `4afea48`), `isol8` (rev `14a87bd`); lock also pulls transitive git: zed forks of `xim-rs`, `font-kit`, `reqwest`, `wasm_thread`, and `proptest-rs/proptest` |
| Vendored code | `vendor/gpui-terminal` only (7k LOC, 90 tests, 78 unwraps, 3 allows) |
| Exact pins | `sqlparser =0.63.0`, `sqlx-core/postgres =0.8.6` (comment: 0.9 pulls crates the base lock lacks) |
| Duplicate versions | 130 crate names with 2+ versions in base lock (objc2*, core-foundation, hashbrown, base64, nix, getrandom, darling, itertools, ...) mostly GPUI-driven; `ratatui 0.30` vs `crossterm 0.28` pinned for a `unicode-width` conflict (agent-manager/Cargo.toml comment) |
| Studio pins | 5 manifest lines at base rev `509b469`; base `ubiq/` HEAD is `0bb6e16`, so `just build-pinned` reproduces a stale base |

## A3. Justfile and `just verify`

Base `ubiq/Justfile` (327 lines, ~45 recipes). `verify: check clippy test host relay ui apple docs-lint help-check slots-check`
- `check`: `cargo check --workspace --all-targets` + `cargo check -p ubiq-app --no-default-features --all-targets`
- `clippy`: `cargo clippy --workspace --all-targets -- -D warnings` (no extra lint groups)
- `test`: `cargo test --workspace < /dev/null`
- `host`/`relay`/`headless`/`ui`/`apple`: dependency-tree greps, plus `ui` greps for literal type sizes / icon sizes outside `theme.rs`
- `slots-check`: grep that `SlotId::new("` literals live only in `crates/ubiq/src/ext/` (D178)
- `docs-lint`, `help-check`
- Not in verify: `assist` (needs Swift/macOS 26 SDK), `core` (agent-manager `--no-default-features`; **not in verify**), `headless`, `icons-check`, `drone-manifest-verify`, `docs-check`, `docs-drift`, `web-assets-verify*`.

Studio `Justfile` (265 lines). `verify: lock-agrees host ui check help-check base-check studio-test no-composed-project-paths`;
`verify-base` (base's own gate, second build), `verify-all`, `base-test` (cargo test impossible inside the patched resolution),
`build-pinned`. CI: `ubiq/.github/workflows/` has **only** `create-release.yml`, `release-macos.yml`, `release-windows.yml`
(104 lines total; tag/dispatch triggered, runs `just bundle`). **No workflow runs `just verify`, clippy or tests.** Studio has no
`.github` at all.

## A4. `crates/agent-manager` (41,886 LOC, 65 files)

Modules: `harness/{mod 1029, claude 2407, codex 1494, grok 1407, copilot 1052, opencode 1009, shared}`, `io/{acp_client 4944, jsonl 2438,
acp 1350, codex 1313, model 1246, acp_caps, structured, agui, passthrough}`, `isolate.rs` 2035, `resolve.rs` 1756, `profile.rs` 1148,
`provision.rs` 818, `registry/*` (remote, manage, import, fs, mcp_parse, mcp_registry), `credentials/{mod, os 645, keyring_store, file, keychain, memory}`,
`account.rs`, `session.rs`, `settings.rs`, `quota.rs`, `home.rs`, `mcp/server.rs`, `cli/*`, `tui.rs` (a stub).

Harness definition: **code per harness, behind one trait**. `pub trait Harness` (`harness/mod.rs:481`, 24 trait methods) with
5 impls (`Claude`, `Codex`, `Copilot`, `Grok`, `Opencode`); registered by hand in `harness::all()` (`mod.rs:711`) as 7 boxes (Claude
and Codex each twice: native + `*-acp` sibling). `resolve()` and `known_ids()` iterate `all()`. Per-harness editable preference
templates are a data seam (`TemplateStore`, `FsTemplateStore`, default JSON seeded on first read) but the defaults live in Rust.
`HarnessId = String` (`spec.rs:14`), so a typo in an id is not caught at compile time.

Places that change to add a harness (observed):
1. new `harness/<x>.rs` (800-2400 LOC each so far) + `pub use`/`mod` + `all()` entry (agent-manager)
2. `io/*` bridge if it has a structured wire (ACP reuses `acp_client`; Claude and Codex have bespoke bridges)
3. `ubiq-host/src/agent.rs` (49 literal id sites, `store/harness.rs` 30, `coordinator.rs` 15, `conversation.rs` 6, `quota.rs` 4)
4. `ubiq/src/state/workbench.rs` (19), `state/settings.rs` (9), `ui/kit/mod.rs` and `kit/icons.rs` (icon/name per id), `state/sink.rs` (3)
5. quota/usage module if the harness reports quota; docs (`_docs/tech/agent-manager.md`, harness notes)

Most literal-id sites are test fixtures; production dispatch is spread over ~9 `match`/`==` sites on harness ids in `ubiq-host`/`ubiq`.
A harness listed in the docs (Gemini CLI, `ubiq/AGENTS.md`) has **no** harness impl; "gemini" appears only in `copilot.rs`,
`quota.rs`, `mcp_parse.rs`, assist providers and an icon.

Hardcoded config paths: 54 non-comment sites naming `.claude`, `.codex`, `.copilot`, `.grok`, `.config/opencode`, mostly inside
`harness/*.rs` (the intended owner) and `home.rs` (10). Model names are not hardcoded in non-test code for Claude/Codex
(`ModelInfo` is discovered at runtime: `codex debug models` fixture, claude list); hardcoded model id strings appear in
`io/jsonl.rs` (26, mostly tests), `copilot.rs` (6), `codex.rs` (8, tests).

Errors: `pub type Result = anyhow::Result` (library crate); 124 `bail!`/`ensure!` sites; `thiserror` declared in Cargo.toml but
**0 `thiserror`/`derive(Error)` uses in the crate**. No typed error enum a caller can match. 183 `anyhow` references.

Tests: 614 `#[test]`, 47 files with `#[cfg(test)]`, `tests/` with 4 integration files (`jsonl_bridge`, `codex_bridge`,
`confined_bridge`, `passthrough`), shell fakes (`fake-claude-streamjson.sh`, `fake-codex-appserver.sh`, `fake-harness.sh`,
`fake-claude-interrupt.sh`) and `tests/fixtures/`. Prod `unwrap()`/`expect()`: 2/6 (heuristic); total incl. tests 935/276.
Examples: `conpty_confine_probe`, `confined_shell_probe` (382 lines, diagnostic).

TODO/placeholder: 3 `TODO(P2+)` (opencode.rs:313,323 account env/baseURL; codex.rs:133 base_url -> model_providers) = harness
account `base_url` not honoured for Codex/opencode. `harness/mod.rs` doc admits MCP-as-skill is a pointer stub, not context saving.

## A5. Leaf crates

### `ubiq-db` (12,763 LOC, 18 files)
- Modules: `conn` (1401: connection string parsers for URL, libpq, ADO.NET/ODBC, JDBC), `value`, `sql` (+`readonly` 584 +
  `readonly_corpus` 445 +`functions`), `edit` (1298, DML rendering), `dbml` (780), `model`, `plan`; `driver/{mod,mssql,mysql,postgres,sqlite,plan,structure}` behind `drivers`.
- API: `lib.rs` is 27 lines, `pub mod` of all 7 modules + re-exports of `model`/`plan` types. No trait surface at crate root;
  `driver::mod.rs` (378) holds the engine abstraction. Error type `DbError` (thiserror, 4 derive uses); `ParseError`, `EditError`.
- Tests: 172 `#[test]` in 14 files, no `tests/`. 4 `#[ignore]`d (live-server). The read-only guard has a 445-line corpus file.
  Driver tests: mssql 5, mysql 2, postgres 8, sqlite 15 (thin relative to 4,600 driver LOC).
- Quality: unwrap total 225 (prod ~7). Comments/doc density high. Forked verbatim into `spike/db-explorer/crates/dbx-core` (`conn.rs`
  differs by 4 diff lines, `edit.rs` by 1 line).

### `ubiq-md` (2,671 LOC, 5 files)
- `parse() -> Document` of source-ranged `Block`s over `pulldown-cmark` offset iter; `find`, `autolink`, `build`. 43 tests, all in `tests.rs` (1027 lines in the spike copy), no integration dir. Deps: `pulldown-cmark`, `serde`.
- Forked in `spike/markdown-viewer/crates/mdview-md` (15 diff lines vs base). A *third* split of markdown block handling exists
  in `ubiq_proto::blocks` (shared id walk, referenced by 49 sites in `ubiq`/`ubiq-host`), plus the UI's own `ui/mdview/*` (e.g. `blockedit.rs`). The crate doc says the tree "parses markdown four times today".

## A6. Extension seams

All seams are fields of `ubiq_app::Contributions` (ubiq-app/src/lib.rs:175), seeded by `Default` with the base's own entries, handed in via `Boot { stores, contributions }` to `ubiq_app::run`:

| # | Field | Type | Kind | Base seeded |
|---|---|---|---|---|
| 1 | `task_providers` | `ubiq_host::tasksrc::Registry` (`TaskProvider` trait, tasksrc/mod.rs:99) | host service | Trello |
| 2 | `runner_sources` | `ubiq_host::runners::Registry` | host service | make/just/mise |
| 3 | `settings_sections` | `ext::Registry<SettingsSectionSpec>` | UI container (app overlay + project dialog) | 15+8 |
| 4 | `rail_modes` | `ext::Registry<RailModeSpec>` (13 fields incl. `centre`, availability, `icon: fn()`) | UI | 10 |
| 5 | `bar_menus` | `ext::Registry<MenuBlockSpec>` | UI | 4 |
| 6 | `runner_kinds` | `ext::Registry<RunnerKindSpec>` | UI | 3 |
| - | `stores` | `Box<dyn FnOnce(&Path) -> Stores>` | store decorators (encryption seam) | files |

Facts: UI containers share one generic `ext::Registry<T: Slotted>` (274 lines; duplicate id panics; relabel/reorder/remove only; D177), each
`install()`ed once into a `OnceLock` (process-wide global; panics on second install). The two host registries are separate types
(`tasksrc::Registry`, `runners::Registry`) with the same verbs but not the generic one. No cargo feature, inventory or dynamic loading;
`ext/ids.rs` (155 lines) is the single id registry, enforced by `just slots-check`. `ext_demo.rs` is a base-side demo user.
No seam yet for: document kinds, panels in edge regions beyond rail centre, dialogs, agent-facing MCP tools, message families
(`inbox-editions` lists these; the plugin proposal is runtime-registry future).

Studio consumes 3 lines in `ubiq-studio-app/src/lib.rs:boot()`: `register_task_providers`, `ado::settings_section`, `rail::rail_modes`
(two `cfg(feature="ui")`). Studio host: `ubiq-studio-host` (6,844 LOC: Azure DevOps and ClickUp providers).

Spikes: the two present (`markdown-viewer`, `db-explorer`) **do not use these seams at all**; each is a standalone GPUI binary
that "depends on nothing under `crates/` or `ubiq/`" and forks the engine crate. Both graduated into the base (DB rail mode, `ubiq-md`)
but remain as active workspace members (compile cost + drift). `spike/archify` is untracked docs only. The `ubiq-spike` skill
describes a different model (spike runs the full workbench and adds a rail mode via the seams); no existing spike demonstrates it.
Touchpoints to add a spike mode under the described model: a `spike/<n>/crates/*` member in the Studio `Cargo.toml`, a `Contributions` mutation in a new `main`, optionally a settings section; for a *new document kind* the base must change (no seam).

## A7. Tests

| Crate | `#[test]` | files w/ `cfg(test)` | `tests/*.rs` | prod unwrap / expect | total unwrap / expect |
|---|---|---|---|---|---|
| agent-manager | 614 | 47 | 4 | 2 / 6 | 935 / 276 |
| ubiq | 1,200 | 67 | 60 | 21 / 27 | 399 / 700 |
| ubiq-host | 1,070 | 65 | 24 (+ fixtures, `data/`) | 21 / 31 | 1,804 / 497 |
| ubiq-proto | 152 | 18 | 4 | 0 / 1 | 124 / 35 |
| ubiq-db | 172 | 14 | 0 | 7 / 4 | 225 / 12 |
| ubiq-md | 43 | 1 | 0 | 1 / 3 | 8 / 8 |
| ubiq-drone | 33 | 5 | 6 | 2 / 22 | 2 / 134 |
| ubiq-app | 21 | 1 | 0 | ~0 (16 counted are in `handoff.rs` test mod) | 16 / 11 |
| gpui-terminal | 90 | 9 | 0 | 13 / 5 | 78 / 5 |
| ubiq-studio / -host / -app | 14 / 83 / 0 | 2 / 3 / 0 | 2 / 5 / 0 | 0 / 1 | |
| **Total** | **~3,490** | | | | |

- Helpers: shell fakes for harnesses (agent-manager), Trello fixtures (`ubiq-host/tests/fixtures/trello`), `readonly_corpus.rs`,
  `tempfile`. `gpui` `test-support` feature used by `ubiq` dev-deps (`ubiq/Cargo.toml:243`).
- **No snapshot, property, fuzz or benchmark framework** in any manifest (`proptest` appears only as a transitive git dep in the lock).
- Largest tests: `ubiq/tests/conversation.rs` 2,668; `ubiq-host/tests/coordinator.rs` 2,471.
- 2 coordinator tests fail only in the agent sandbox (memory note). `just test` closes stdin for PTY passthrough tests.
- Gaps: `ubiq-app` (1.7k LOC composition root, 21 tests, no `tests/`), `ubiq-db` drivers against real servers (4 `#[ignore]`),
  `ubiq-md` no test of HTML/edge input robustness beyond unit; `ubiq` UI is tested by logic/state tests (60 integration files) rather than render; 
  harness `claude`/`codex` bridges tested only against shell fakes; `agent-manager` Windows paths (`os.rs` DPAPI/PowerShell) tested unit-only. No coverage tooling configured.

## A8. Hardcoded values and placeholders (production code)

Marker grep (`TODO|FIXME|XXX|todo!|unimplemented!`), excluding tests and false positives: **5** real:
`agent-manager/harness/opencode.rs:313,323`, `codex.rs:133` (base_url mapping); `ubiq/state/new_agent.rs:264` (plan-mode follow-up, T-64);
`ubiq/state/new_agent.rs:878`. Very low debt-marker density for 400k LOC; the tracker/backlog carries the rest.
`placeholder|dummy|hardcod|stub`: `ubiq-host/shell_integration.rs` 9, `agent-manager/harness/mod.rs` 9, `provision.rs` 7, `ubiq-host/assist/stub.rs` 4 (the non-macOS on-device model stub).

Hardcoded URLs in production code:
- `ubiq-host/connectors/providers.rs` GitHub/GitLab/Azure DevOps/Atlassian/Google/Trello cloud endpoints (const table, acceptable).
- `ubiq-host/assist/api.rs:221-223` default base URLs for OpenAI/Anthropic/Gemini.
- `ubiq-host/feedback/issues.rs:47` GitHub issues API.
- `ubiq-host/web_assets/manifest_drawio.rs` generated, pinned to `drawio@31.4.5` on jsDelivr with SHA-256 (good).
- `agent-manager/account.rs:322` `https://api.anthropic.com`; `harness/opencode.rs:440` opencode schema URL.
Magic numbers: not exhaustively scanned; largest concentrations are in `ubiq/src/theme.rs` (3,635 LOC, tokens by design) and timeouts in host workers.

Files over 2,000 LOC: 23. Top: `ubiq-host/coordinator.rs` 8,587; `ubiq/ui/settings.rs` 6,052; `agent-manager/io/acp_client.rs` 4,944;
`ubiq/ui/conversation/mod.rs` 4,160; `ubiq-proto/messages.rs` 4,075 (one file by design); `ubiq/app/wire.rs` 3,661; `ubiq-host/agent.rs` 3,655;
`ubiq/theme.rs` 3,635; `ubiq/app/settings.rs` 3,042.

## A9. Security snapshot

| Area | Finding |
|---|---|
| Harness accounts | `account.rs:33`: `api_key_env` / `auth_token_env` hold env var **names**, `helper` is a command run by the harness; value read at launch, never written to disk (`harness/claude.rs:195-203`). Reference-only holds for accounts. |
| Credential store | `agent-manager/credentials/` is the exception to "references only": it stores **secret blobs** (login JSON). Engines: file (0600 on unix, `file.rs:44`), `os` (macOS `security`, Linux `secret-tool`, Windows DPAPI via PowerShell), `keyring`, in-memory. |
| Secret on argv | `credentials/os.rs:295-306,383-395`: `security add-generic-password -w <secret>` passes the vault password and the credential value on argv (visible to `ps`); the code carries a `ponytail:` comment acknowledging it. |
| PowerShell | `os.rs:520-545` builds a script by `format!` with `path.display()` inside single quotes; secret goes through `$env:AM_SECRET` (good). A path containing `'` would break the script (low risk, local path). |
| DB passwords | `ubiq-db::ConnectionConfig` has `password: Option<String>` with `#[derive(Debug, Serialize)]` (`conn.rs:126-150`): Debug prints and plain serialisation reveal it unless a host strips it first. Host `ubiq-host/db/secrets.rs` seals passwords (AEAD with keystore key, AAD per project+conn) and keeps a session fallback; has a test that it never reaches disk in the clear. `to_url(mask_password)` exists. |
| SQL building | `quote_ident` per dialect (`edit.rs:45`), `br()`/`lit()` for MSSQL; `format!` used for `USE`, `KILL QUERY {id}`, `pg_cancel_backend({pid})` with numeric ids, `{db}` always via `quote_ident` (sqlite `approx_rows`). User SQL is executed as given by design, gated by `sql::readonly` guard (parser based, 584 LOC + corpus). No injection hole found in the sampled sites. |
| Process launch | `agent-manager::Launch` carries `program`, `args`, `env`, `env_remove`, `env_clear`; confined runs set `env_clear` (isol8 sandbox, `isolate.rs` 2,035 LOC); `ubiq-host/pty` launches shells as login shells with inherited env. `isolate::kill_descendants_on_exit` job object / process group. 23 non-test mentions of dangerous/bypass permission flags, in the harness provisioners. |
| Dependency risk | Floating `gpui` git deps without `rev` in manifests (lock-pinned only; a `cargo update` moves the UI toolkit); `isol8` is the owner's own git repo, rev-pinned; `rusqlite` `bundled`; TLS via rustls/ring in listener and drivers. No `cargo-deny`/`cargo-audit` configuration. |

## A10. Tooling and docs coupling

`_tools/` (5,482 lines of Python/sh): `docs.py` 1150, `tape-subagents.py` 1268, `webassets.py` 927, `helpbundle.py` 622, `icons.py` 609, `dump.py` 409,
`drone.py` 235, `toolchain-smoke.sh` 194, `icns.py` 68, `teamsim/*` (own tests `test_algos.py`), `Info.plist`. PEP 723 `uv run` scripts.
`_tools/README.md` table lists `excalidraw.py` (missing from the tree and from git) and omits `dump.py`, `drone.py`, `helpbundle.py`,
`toolchain-smoke.sh`. Base also has `_devops/scripts/bundle-version.sh`; Studio has `bundle*.sh`, `pinned-build.sh`.

Docs: 128 markdown files under `ubiq/_docs/` with YAML front matter (`id`, `status`, `read_when`, `updated`, `verified`, `code_anchors`,
`depends_on`, `review_cycle`). `just docs-lint` (in verify) checks mechanics; `docs-drift` finds docs whose anchored files moved after
`verified`; `docs-touched` maps a diff to owed documents; `docs-index` generates `INDEX.md` and the code map (`docs-check` verifies,
not in verify). Studio docs graph is directed (Studio -> base only). The rule is "same commit" and relies on discipline plus lint, not CI.
Eight skills under `ubiq/.claude/skills/` are the working references.

## A11. Error handling conventions

| Crate | Style |
|---|---|
| agent-manager | `anyhow::Result` alias everywhere, `bail!` 124, no typed errors; `thiserror` dep unused |
| ubiq-proto | many small error enums (`HandshakeError`, `ConnectError`, `GitError`, `FileError`, `SearchError`, `WireError`, `CloneError`, `DroneError`, `HostPathError`) hand-written, 0 `thiserror` uses |
| ubiq-host | 222 `Result<_, String>` sites (workers answering with strings that go on the wire), `StoreError`, `SecretsError`; `thiserror` 3 uses, `anyhow` 60 |
| ubiq-db | `thiserror` `DbError`/`ParseError`/`EditError`; `Result<T, String>` appears in `driver/plan.rs` |
| ubiq | `anyhow` 9; mostly state-machine/UI results with `String` messages |

`panic!(` counts: agent-manager 89, ubiq 61, ubiq-host 177, ubiq-proto 14 (mostly tests). `expect(` in production: `ubiq` 27, `ubiq-host` 31,
`ubiq-drone` 22. `ext::Registry` deliberately panics on duplicate or late install (boot-time invariants).

---------------------------------------------------------------------
# Section B: pain points, ranked
---------------------------------------------------------------------

| # | Severity | Pain point | Evidence | Suggested remedy |
|---|---|---|---|---|
| 1 | High | **No CI gate**: `just verify` (check/clippy/test/boundary/docs) is never run by a workflow; only release workflows exist, and Studio has no CI. Boundary rules rest on contributors running it. | `ubiq/.github/workflows/` = 3 release files; none contain `verify`/`test`/`clippy`; no `.github` in Studio | Add a PR workflow running `just verify` (macOS runner, `DOCS_RS=1` for the Swift crate) in both repos |
| 2 | High | **Spike forks of engine crates remain active workspace members**; they have drifted by 4 to 15 lines already and double compile cost (GPUI shared, but sqlx/tantivy-less crates recompiled) and review surface. | `spike/db-explorer/crates/dbx-core/src/conn.rs` vs `ubiq-db/src/conn.rs` (4 diff lines), `mdview-md` vs `ubiq-md` (15), Studio `Cargo.toml` members list; memory says both spikes are frozen/graduated | Remove graduated spikes from `members` (keep as docs/archive), or make them depend on `ubiq-db`/`ubiq-md` |
| 3 | High | **Adding a harness touches 5+ places, with harness ids as bare strings** in host and UI, and a closed code-per-harness set; Gemini CLI is promised in AGENTS.md but absent. | `harness::all()` (`mod.rs:711`), 49/30/19/15 literal id sites in `agent.rs`, `store/harness.rs`, `workbench.rs`, `coordinator.rs`; `HarnessId = String` (`spec.rs:14`) | Add per-harness metadata to the trait (display name, icon key, capabilities) so UI/host stop matching on ids; a `HarnessId` newtype; fix the doc claim |
| 4 | High | **Credential material on argv** and unredacted password in a `Debug`+`Serialize` DB config. | `credentials/os.rs:295-306,383-395` (`-w <secret>`; comment concedes it); `ubiq-db/src/conn.rs:126-150` derives Debug, `password: Option<String>` | Feed `security` via stdin/`-w` prompt alternative or use the `keyring` engine on macOS; wrap password in a `Secret<String>` type with redacting Debug/skip-serialize |
| 5 | Medium | **No workspace lint policy, MSRV or toolchain pin**; clippy is whatever `stable` is today, 66 `too_many_arguments` allows suppress a design smell instead of fixing it. | no `[workspace.lints]`, `clippy.toml`, `rust-toolchain.toml`, `rust-version`; `grep allow(` = 66 `too_many_arguments` | Add `[workspace.lints]` (inherit in every crate), `rust-toolchain.toml`, `rust-version`; group argument lists into structs where >7 |
| 6 | Medium | **Giant files** concentrate change and merge risk: 23 files >2,000 LOC, with `coordinator.rs` 8,587, `settings.rs` 6,052, `acp_client.rs` 4,944. | `wc -l` across `crates/` | Split along existing seams (coordinator per message family, settings per section; the `ext` container already gives section seams) |
| 7 | Medium | **Unstable build inputs**: `gpui` git deps without `rev` in manifests; Studio pins base rev `509b469` while the checkout is `0bb6e16`; lock seeded by copying; `build-pinned` can silently build a stale base. | `ubiq/crates/ubiq/Cargo.toml:51-52`, `ubiq-app/Cargo.toml:95-96`, `crates/ubiq-studio/Cargo.toml` revs; `Cargo.lock` seeding rule in `AGENTS.md` | Add `rev` to zed deps; add a CI check that Studio's rev is an ancestor of base `main`; keep `lock-agrees` in CI |
| 8 | Medium | **Seams are non-uniform**: 6 containers but two registry types (`ext::Registry<T>` vs `tasksrc::Registry` and `runners::Registry`), process-wide `OnceLock` installs that panic on second install, and none for document kinds/panels/dialogs/MCP/message families; spikes cannot plug in without touching the base. | `ubiq-app/src/lib.rs:175-221`; `ext/rail.rs:195-215`; `inbox-editions` | Make host registries share the generic container (or document why not); thread registries through `Boot` instead of globals; add a seam only on its first base user as the doctrine says |
| 9 | Medium | **Feature matrix lies**: `harness` and `listener` are declared independent but do not compile alone; only 6 configs build, none checked beyond `check` defaults + `ubiq-app --no-default-features`. | `ubiq-host/Cargo.toml` comment (T-235); Justfile `check`, `relay` | Merge into one feature or fix the cross-module imports; add `cargo hack`-style matrix over the 6 supported configs to CI |
| 10 | Medium | **agent-manager error type is opaque**: library returns `anyhow::Result` everywhere (124 `bail!`), the unused `thiserror` dependency, so the host cannot branch on failure kind and ends up with 222 `Result<_, String>` sites across host. | `agent-manager/src/lib.rs` alias; Cargo.toml `thiserror = "2"` with 0 uses | Introduce a small `Error` enum for the boundary calls the host branches on (not found, unavailable, confinement, auth); keep anyhow inside |
| 11 | Medium | **Three-way markdown block logic** (`ubiq_proto::blocks`, `ubiq-md`, `ui/mdview/*`), plus a fork in the spike. | `ubiq-md/src/lib.rs` doc ("parses markdown four times today"), 49 `ubiq_proto::blocks` references | Finish the consolidation onto `ubiq-md`; delete `mdview-md` |
| 12 | Medium | **Test blind spots**: no coverage tooling, no property/snapshot tests (parsers: `conn.rs` 1.4k LOC, `readonly` SQL guard are ideal targets), driver tests thin (30 tests on 4.6k LOC, 4 ignored), `ubiq-app` and harness bridges only via shell fakes. | A7 table; manifests contain no proptest/insta | Add `proptest` for connection-string round-trips and the read-only guard; a docker-compose gated driver suite; `cargo llvm-cov` job |
| 13 | Low-Med | **`just core` and several other checks are outside `verify`** (`core`, `headless`, `icons-check`, `docs-check`, `drone-manifest-verify`, `web-assets-verify`), so the rules they encode (agent-manager builds with no features, generated blocks fresh) can rot. | Justfile line 158 vs recipe list | Add `core headless docs-check icons-check` to `verify` or to the CI workflow |
| 14 | Low-Med | **Docs/tooling drift**: `_tools/README.md` lists `excalidraw.py` (not in tree, so `just diagram` fails) and omits four scripts. | `ls _tools`, Justfile `diagram` recipe line 193 | Fix the README table; remove or restore `diagram`; make `docs-lint` check `_tools/README` vs the dir |
| 15 | Low-Med | **Unsupported account fields silently ignored**: `base_url` for Codex/opencode is a TODO, so an account configured with a gateway runs against the default endpoint. | `harness/codex.rs:133`, `opencode.rs:313,323` | Fail loudly (warn event) when an account sets a field the harness cannot honour, or implement mapping |
| 16 | Low | **Panic-on-misuse containers**: duplicate `SlotId`, late `install`, relabel of unknown id all `panic!`; acceptable at boot but they crash the window rather than show an error when an edition misregisters. | `ext/registry.rs` asserts, `ext/rail.rs:210` | Return `Result` from `register` and let `Boot` report once, cleanly |
| 17 | Low | **Vendored `gpui-terminal`** (7k LOC) carries 13 prod unwraps and 3 `allow`s outside the project's rules, unreviewed against upstream. | `vendor/gpui-terminal` counts | Record the upstream commit in a `VENDOR.md`/Cargo metadata and an update procedure |
| 18 | Low | **Hardcoded provider/endpoint tables** for assist (OpenAI/Anthropic/Gemini base URLs) and issue feedback URL in code rather than config. | `ubiq-host/assist/api.rs:221-223`, `feedback/issues.rs:47` | Fine as defaults; ensure each is overridable by settings (assist) and documented |

## Quality summary

Strengths: strict architecture rules enforced mechanically, unusually low TODO density (5) and low production unwrap counts, strong
test volume (~3,500 tests), reference-only accounts, sealed DB passwords, quoted identifiers, an exact-pinned and hash-verified web-asset mirror,
`forbid(unsafe_code)` in the library, docs coupled by lint and drift tools.
Weaknesses: no CI gate, no lint policy/MSRV, opaque error type at the agent-manager boundary, stringly harness ids, a few 4-8k LOC files,
unmaintained spike duplicates, and a seam set that is uniform only for UI containers.
