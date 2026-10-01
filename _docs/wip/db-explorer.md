---
id: wip-db-explorer
title: The database explorer — a DB rail mode, from the proof to the base
kind: wip
status: draft
summary: The design for bringing the database-explorer proof into the base as an opt-in rail mode after IDE — a fifth crate `ubiq-db` (the engine, drivers behind a feature), a database message family scoped by project, host-side sessions on worker threads with three read-only layers, connections in a tracked `db.toml` with passwords AES-256-GCM encrypted under a per-install key kept in the OS keychain, three new panel kinds (explorer left, table tabs centre, SQL tabs bottom), a Databases section in project settings, and the ordered work packages that build it.
read_when: you are building any part of the DB rail mode — the `ubiq-db` crate, the database message family, the host's sessions or secrets, the explorer, table or SQL panels, or the Databases settings section
updated: 2026-10-01
verified: 2026-10-01
code_anchors: [crates/ubiq/src/ui/rail.rs, crates/ubiq/src/state/dock.rs, crates/ubiq/src/ui/kb/mod.rs, crates/ubiq/src/ui/sink/project.rs, crates/ubiq-proto/src/messages.rs, crates/ubiq-host/src/store/project_dir.rs, crates/ubiq-host/src/connectors/store.rs]
depends_on: [tech-architecture, tech-transport, feat-workbench, tech-decisions, wip-kb]
---

# The database explorer — a DB rail mode, from the proof to the base

## What this is

A database explorer for the project: saved connections to PostgreSQL, MySQL/MariaDB, SQLite and SQL
Server, a structure tree, table-data tabs that page, filter and edit rows, and SQL editor tabs with a
timer, Stop, Explain and a read-only guard. It exists today as a stand-alone proof — two crates,
the proof engine (no GPUI) and the proof app (a GPUI app). This document is
the plan for moving it into the base as **DB**, a rail mode in the PROJECT group directly after IDE,
`Availability::OptIn` so a project draws it only once it is switched on in project settings >
General > Modes.

It goes in the base, not in the `Contributions` extension seam, for two reasons that seam cannot meet: the
mode must sit after IDE inside the base's own group, and it needs three new `PanelKind` arms in a
closed enum (`crates/ubiq/src/state/dock.rs`).

## 1. Crate layout

**A fifth crate, `crates/ubiq-db`** — the proof engine moved, renamed, and split along one cargo feature:

| Part | Modules | Feature | Who uses it |
|---|---|---|---|
| Model | `model` (from `driver/mod.rs`'s types: `DbError`, `ObjectKind`, `TableRef`, `DbObject`, `ColumnMeta`, `ResultSet`, `ExecOutcome`, `ExecOptions`) and `plan` types (`Plan`, `PlanFormat`, `PlanNode`) | default | proto, UI, host |
| Pure engine | `conn` (config + connection-string parsing), `value` (`DataType`, `Value`, `parse_input`), `sql` (analysis, `statement_at`, `check_read_only`), `edit` (`select_table`, `count_table`, `validate_fragment`, `RowEdit`, `render_batch`) | default | UI (diagnostics, SQL preview, cell validation), host |
| Drivers | `driver` (the `Connection` trait, `connect`, `create_sqlite_database`, the four engines, the `EXPLAIN` parsers) | `drivers` | host only |

- **Default features carry no driver and no async runtime**: `serde`, `thiserror`, `sqlparser`
  `=0.63.0` (`std`, `visitor`, no `recursive-protection`), `tracing`. `drivers` adds `rusqlite`,
  `mysql`, `tiberius`, `sqlx-core`/`sqlx-postgres` `=0.8.6`, `tokio`, `tokio-util`, `futures-util`,
  `chrono` — exactly the proof's set and features.
- **`log` becomes `tracing`.** The statement log is `tracing::info!` under target `ubiq_db::sql`; the
  base's log sink collects it (§3).
- **The model types gain `Serialize`/`Deserialize`.** `ConnectionConfig` already has them; `Value`,
  `DataType`, `ColumnMeta`, `ResultSet`, `TableRef`, `DbObject`, `ObjectKind`, `RowEdit`, `Plan`,
  `PlanNode` get them. The wire is MessagePack (`ubiq_proto::wire`), so `f64` and bytes are fine.
- **`ubiq-db` is a leaf**: it names no Ubiq crate, the way `crates/agent-manager` names none.
- **Wire types live in `ubiq-db`, re-exported by `ubiq-proto`.** `ubiq-proto` depends on `ubiq-db`
  with default features and a new `ubiq-proto/src/db.rs` `pub use`s the model, then adds only what
  is the contract's own: the ids, `DbConnection`, `SecretEdit`, `DbNode`, `DbListing`, `DbFailure`,
  `DbRun`, `DbOutcome`. Defining the model twice (once for the engine, once for the wire) is the drift
  this avoids; the cost is that a contract change to `ColumnMeta` is a change in `ubiq-db`.
- **`ubiq-host` gains a `db` feature** — `["dep:ubiq-db", "ubiq-db/drivers", "dep:rustls", "dep:ring",
  "dep:base64"]` — and `full` includes it, so the coordinator gates on `full` as it does today.
  `ubiq-drone` builds the host without defaults and so without `db`; it answers the family with
  `DbFailure::Unavailable`.
- **The checks.** `just ui` builds `-p ubiq` alone, so `drivers` is off there and any interface code
  naming `ubiq_db::driver` fails to compile — the dependency graph enforces "the interface opens no
  connection" the way the proof's crate split did. `just ui` gains one grep: `cargo tree -p ubiq -e
  no-dev` must contain none of `rusqlite`, `mysql`, `tiberius`, `sqlx-core`. `just host` and `just
  core` are unchanged and keep passing: `ubiq-db` has no GPUI and is not in `agent-manager`'s graph.

**TLS, the cost nobody chose.** `tiberius`'s `rustls` feature takes `tokio-rustls` with default
features, which turns on rustls's `aws_lc_rs` — so `aws-lc-rs`, `aws-lc-sys` (a C build), `cmake` and
`fs_extra` enter the base's graph, and two crypto providers are compiled in. Two consequences, both
decided:

1. `mysql` 28 calls `ClientConfig::builder()`, which panics when two providers are compiled and none
   is installed. The host's new `ubiq-host/src/db/mod.rs` installs `ring` as the process default once, in
   `Db::new` (`CryptoProvider::install_default`, its `Err` — already installed — ignored). Every
   existing rustls site in the base names its provider explicitly (`connectors::tls::provider`,
   `remote::tls_provider`), so none of them changes behaviour. `tiberius` honours an installed
   default, so the whole database stack runs on `ring`.
2. `aws-lc-sys` must build on every shipped target; `just bundle-win` is the one not yet proven
   (`windows-build.md`). A backlog row tracks it.

## 2. The message set

**Scoping.** Every variant carries `project_id`, and `Message::project_id()` lists every one of them —
the knowledge-base family's discipline (`wip/kb.md`). Architecture rule 4, a pane ID on every
message, governs the pane family; this family has no pane and names its own objects with ids, as the
Kb, work and connector families do.

**New ids** (`crates/ubiq-proto/src/ids.rs`): `DbConnId` — minted by the **host** on first save, on
`AiProviderId`'s discipline, stable across renames; `DbSessionId` — minted by the **interface**, one
per table or SQL tab, so a reply for a closed tab is discarded by id; `DbQueryId` — minted by the
interface per run, `SearchId`'s discipline, and the handle Stop cancels by; `DbProbeId` — one Test.

**Records** (`ubiq-proto/src/db.rs`): `DbConnection { id, config, password: PasswordState }`
where `config.password` is always `None` on the wire towards the interface and `PasswordState` is
`None | Saved | Missing | Session`; `SecretEdit { Keep, Set(String), Clear }`; `DbNode { Databases,
Schemas { database }, Objects { database, schema }, Columns { table } }`; `DbListing { Names(Vec),
Objects(Vec<DbObject>), Columns(Vec<ColumnMeta>) }`; `DbRun { Query, Explain { analyze } }`;
`DbRunOptions { read_only, timeout_ms, row_limit }`; `DbOutcome { Rows(ResultSet), Affected(u64),
Plan(Plan) }`; `DbFailure { kind, message, position }` with kind `Config | Connect | Query | ReadOnly
| Cancelled | Timeout | NeedsPassword | NotFound | Disconnected | Unsupported | Unavailable`;
`DbKeystore { Ready | Unavailable(String) }`.

UI → host:

| Variant | Carries | Answered by |
|---|---|---|
| `DbConnections` | project | `DbConnectionsListed` |
| `SaveDbConnection` | `id: Option<DbConnId>`, `config` (password `None`), `password: SecretEdit`, `remember: bool` | `DbConnectionsListed` |
| `DeleteDbConnection` | `id` | `DbConnectionsListed`; its sessions close, its ciphertext goes |
| `TestDbConnection` | `probe`, `id: Option<DbConnId>`, `config`, `password: SecretEdit` (`Keep` = the saved one) | `DbTested` |
| `DbPassword` | `conn`, `password`, `remember` | `DbConnectionState` — the answer to `NeedsPassword` |
| `DbTree` | `conn`, `node: DbNode` | `DbTreeListing` |
| `DbTablePage` | `conn`, `session`, `query`, `table`, `filter`, `order_by`, `limit`, `offset`, `count: bool`, `read_only` | `DbTablePageResult` |
| `DbQuery` | `conn`, `session`, `query`, `database: Option`, `statements: Vec<String>`, `run: DbRun`, `opts` | one `DbQueryResult` per statement |
| `DbApplyEdits` | `conn`, `session`, `query`, `table`, `edits: Vec<RowEdit>` | `DbEditsApplied` |
| `DbCancel` | `session`, `query` | nothing — the running reply ends `Cancelled` |
| `DbCloseSession` | `session` | nothing |
| `DbDisconnect` | `conn` | `DbConnectionState` |
| `CreateDbFile` | `path` (a host path from the host-browse picker) | `DbFileCreated` or `DbFileError` |

Host → UI: `DbConnectionsListed { connections, keystore }` (to every window of the project, as
`KbSourcesListed` is), `DbTested { probe, result: Result<String /*server version*/, DbFailure> }`,
`DbConnectionState { conn, state: Idle | Connecting | Connected { server } | NeedsPassword |
Failed(DbFailure) }`, `DbTreeListing { conn, node, result }`, `DbTablePageResult { session, query,
columns: Vec<ColumnMeta>, rows: ResultSet, exact_count: Option<u64>, elapsed_ms }`, `DbQueryResult {
session, query, index, last: bool, result: Result<DbOutcome, DbFailure>, elapsed_ms }`,
`DbEditsApplied { session, query, result: Result<u64, (usize, String, DbFailure)> }` (the failing
statement's index and text), `DbFileCreated { path }`, `DbFileError { path, message }`. Everything
but the listing and the connection state goes to the asker only.

**Structured, not SQL, where the host must be authoritative.** A table page and a batch of row edits
travel as `TableRef` + fragments and as `RowEdit`s, never as rendered SQL: the host renders them with
the same `ubiq_db::edit` functions the interface used for the preview, validates the WHERE/ORDER BY
with `validate_fragment`, and so the statement that runs is the statement the preview showed by
construction. Only the SQL editor sends text, because the text *is* the user's input.

**Large results.** `DbQuery`'s `row_limit` defaults to 1 000 and is capped by the host at 10 000; a
result past the cap or past 16 MiB of cells is cut and `ResultSet::truncated` set, so no reply comes
near `MAX_FRAME`. Table tabs page (200 rows, `LIMIT/OFFSET` or `OFFSET … FETCH` by engine); the exact
count is a separate `count_table` the first page asks for with `count: true`. There are no
server-side cursors.

## 3. The host — `crates/ubiq-host/src/db/`

| File | Holds |
|---|---|
| `mod.rs` | `Db`, the concrete service (`Kb`'s shape): the store, the secrets, the session table, the `ring` default; `Arc<Db>` on the coordinator |
| `store.rs` | `db.toml` read/write (§4) |
| `secrets.rs` | the ciphertext file and the AEAD (§4) |
| `session.rs` | one worker thread per session, owning a `Box<dyn Connection>` |
| `jobs.rs` | each message turned into a job: tree, page, query, edits, test, create file |

**Threads.** Every driver call blocks, so none runs on the coordinator's thread. A **session** is one
thread holding one driver connection and a `flume` job queue; it connects lazily on its first job,
answers through a `Mailbox` addressed to the asking client (`files::Job`'s `reply_to` shape), and ends
when its sender is dropped. There is one session per open table or SQL tab (`DbSessionId`) plus one
**meta** session per `(client, connection)` for the tree — so a long query in one tab never stalls
the tree or another tab, each tab has its own transaction and current database, and the proof's
`use_database` re-select before every run is gone. Sessions are keyed `(ClientId, DbSessionId)`;
`client_gone`, `DbCloseSession`, `DbDisconnect`, a saved edit to the connection and its deletion all
drop them. `TestDbConnection` and `CreateDbFile` run on one-off threads.

**Cancel.** A session publishes `Connection::cancel_handle()` for the query it is running into a
`Mutex<HashMap<DbQueryId, Arc<dyn CancelHandle>>>` shared with the coordinator. `DbCancel` takes the
handle out and calls it **on a one-off thread** — a PostgreSQL cancel opens a connection of its own —
and the running statement's reply arrives as `DbFailure::Cancelled`.

**Read-only, three layers, all enforced by the host.** A session is read-only when its connection is
(`ConnectionConfig::read_only`, forced) or when the message says so (the tab's lock, a view or
synonym's tab).

1. *AST* — before anything is sent to a server, every statement of a read-only run passes
   `ubiq_db::sql::check_read_only` (fail closed); a refusal is `DbFailure::ReadOnly` naming the rule.
   The interface runs the same check for its live diagnostic and badge, but only the host's answer
   decides.
2. *Engine* — `query_with`/`execute_with` with `ExecOptions { read_only: true }`: Postgres/MySQL
   read-only transactions, SQLite `query_only` + `sqlite3_stmt_readonly`, SQL Server's `BEGIN TRAN …
   ROLLBACK` backstop. `DbApplyEdits` on a read-only session is refused before rendering.
3. *Limits and log* — `timeout_ms` (meta and pages 30 s; the SQL editor 5 min unless the tab sets
   another), `row_limit` (above), and every executed statement logged at `info` under `ubiq_db::sql`
   with project, connection, session, elapsed time and row count, the SQL cut at 2 000 characters.
   `crates/ubiq-proto/src/log.rs` gains `Subsystem::Db`, classified from `ubiq_db` and
   `ubiq_host::db`, and `DEFAULT_FILTER` gains `ubiq_db=debug`. No connection string and no password
   is ever logged.

**Apply edits** renders with `edit::render_batch`, runs the statements in one session between
`BEGIN`/`BEGIN TRANSACTION` and `COMMIT`, sends `ROLLBACK` on the first error and reports its index.

## 4. Storage and secrets

**Connections — `<data dir>/db.toml`, tracked.** `ProjectData::db_connections()` returns
`self.dir.join("db.toml")`, beside `kb_sources()`; every path goes through `ProjectDirs::data(id)`,
never composed. The file follows the project's storage mode (`D173`): under the config root for a
Ubiq-managed project, in `.ubiq/` — committed — for a project-managed one, so a team shares its
connection list. `FOLLOWS` in `project_dir.rs` gains `"db.toml"`, and `GITIGNORE`'s committed list
names it. Written with `atomic::write_atomic`, `version = 1` at the top, missing read as empty,
`FileTaskStore`'s convention.

```toml
version = 1
[[connection]]
id = "01K…"
name = "orders (staging)"
kind = "postgres"
host = "db.staging.internal"
database = "orders"
user = "reader"
read_only = true
[[connection]]
id = "01K…"
name = "local cache"
kind = "sqlite"
path = "data/cache.sqlite"   # relative to the project root when inside it
```

**It never holds a secret.** The store writes `ConnectionConfig` with `password` cleared, and drops
from `params` every key that names one (`password`, `pwd`, `sslpassword`, `sslkey`) before writing; a
connection string pasted with a password in it puts the password into the form's password field, not
the file. A SQLite path inside the project is stored relative to its root, so a teammate's clone
resolves it.

**Passwords — `<config root>/projects/<ulid>/local/db-secrets.toml`, per machine, never tracked.**
`ProjectData::under_config(root, id).local()`, whatever the storage mode — the key that opens it is
this machine's, so the file belongs to this machine's config root and never to a project folder a
`git add -f` could reach. Not in `FOLLOWS`; Forget removes it with the project's directory. One entry
per connection: `nonce` and `ciphertext`, base64. The AEAD is `ring::aead::AES_256_GCM` through
`LessSafeKey`, a fresh 96-bit nonce from `SystemRandom` on every write, and associated data
`ubiq-db:v1:<project id>:<connection id>` — a ciphertext copied onto another entry fails to open.

**The key — one per install, in the OS keychain.** 32 bytes from `SystemRandom`, generated on the
first password ever saved, filed through the base's one secret store,
`crates/ubiq-host/src/connectors/store.rs`'s `Store`, as a fifth namespace: `db_key` =
`CredentialId { harness: "db", name: "secrets-key" }` with `set_db_key`/`db_key`, beside
`ai_key`/`ssh_key`. Not the `keyring` crate `crates/ubiq` declares: the host's `OsSecretStore` is the
platform's real store, and `Store::usable` already answers whether it works. **Per install, not per
project**, because a per-project key buys no isolation — the same user, keychain and process open
every one of them — while costing a keychain item, and on macOS an access prompt, per project; the
associated data already keeps one project's ciphertext out of another's entries.

**When the key is not there.** The keychain unusable (`Store::usable` errs): `DbKeystore::Unavailable`
is listed, nothing is written to `db-secrets.toml`, and a password typed is held in the host's memory
for the session only (`PasswordState::Session`). The key missing or a ciphertext that fails to open
(a reset keychain, a copied config root): those connections list `PasswordState::Missing`; the next
use answers `DbConnectionState::NeedsPassword`, the interface prompts, and `DbPassword { remember:
true }` re-encrypts under the current key — a fresh one if it was gone, dropping every entry the old
key sealed. A missing password is a prompt, never an error.

**The interface never receives a decrypted password.** The editor's password field is write-only: it
shows `saved` / `not saved` from `PasswordState`, and sends `SecretEdit::Keep` unless the user types
(`Set`) or presses *Forget password* (`Clear`). Test with `Keep` uses the saved password host-side.
Plaintext crosses the bus only towards the host, inside `SaveDbConnection`, `TestDbConnection` and
`DbPassword`.

## 5. The interface

**The mode.** `ids::RAIL_DB = "ubiq.rail.db"` (`crates/ubiq/src/ext/ids.rs`), `RailMode::DB`
(`state/workbench.rs`), `uid::RAIL_MODE_DB = "rail.mode.db"` (`state/ui_id.rs`), `View::Db`
(`state/nav.rs`), and a `RailModeSpec` registered in `ui/rail.rs`'s `modes()` **immediately after
IDE's** — registration order is rail order:

| Field | Value |
|---|---|
| `group`, `label`, `slug` | `RAIL_PROJECT`, `"DB"`, `"db"` |
| `note` | "Browse and query the project's databases." |
| `icon` | `UbiqIcon::ModeDb`, `icons/mode-db.svg`, drawn by the `ubiq-icons` loop |
| `availability` | `Availability::OptIn` — off until General > Modes ticks it (`ViewPrefs::opted_in_modes`) |
| `has_pane_region`, `needs_project` | `true`, `true` |
| `opens_left`, `opens_right` | `true`, `false` |
| `centre` | `ui::db::centre` — the "no table open" page over `ui::mark::backdrop`, `ui::kb::centre`'s twin |
| `default_layout` | `ui::dock::default_db_layout`: explorer left, centre page, an empty right, an empty bottom the first SQL tab lands in |
| `furniture`, `side_furniture` | `queue_db_furniture`; `Left → DbExplorer` |
| `on_enter` | `ask_db_connections_on_arrival` — sends `DbConnections` once |

**Three panel kinds** (`state/dock.rs`, one adapter arm each in `ui/dock/mod.rs`):

| Kind | `name()` | Class / home | Drawn when | Payload |
|---|---|---|---|---|
| `DbExplorer` | `ubiq.db.explorer` | `Edge` / Left | project, DB mode | — |
| `DbTable(String)` | `ubiq.db.table` | `Centre` / Centre | project, DB mode, tab open | `db:<conn>:<database>:<schema>:<name>` |
| `DbSql(String)` | `ubiq.db.sql` | `Free` / Bottom | project, DB mode, tab open | `dbsql:<session id>` |

All three are mode-owned. `from_name` answers `DbExplorer` only; the two tabs are rebuilt from their
payload, as `Kb(String)` is. A restored `DbTable` re-queries its first page; a restored `DbSql` gets
its connection, database and text back from `ViewPrefs::db_sql_drafts` (the view blob, per machine,
64 KiB a draft), with no result. Closing either sends `DbCloseSession`; a table with pending edits
and a SQL tab mid-run ask first (`kit::confirm_modal`).

**State.** `crates/ubiq/src/state/db/` on the open project (`open.db`, beside `open.kb`): `mod.rs`
(`DbState` — connections, keystore, tree, menu, the open tabs, the password prompt), `tree.rs` (the
proof's `structure.rs` with its loaders removed: nodes, `Load`, `visible`, `humanise_rows`, the
filter), `pending.rs` (the proof's, unchanged — it is pure), `table.rs`, `sql.rs`. Widget entities a
tab needs a `Window` for (`InputState`, the code editor, `TableState`) are queued and built in
`render`, `pending_kb_docs`' pattern. Handlers live in `crates/ubiq/src/app/db/` (`mod.rs` with
`receive_db`, called from `app/wire.rs`; `explorer.rs`, `table.rs`, `sql.rs`, `settings.rs`).

**The explorer — KB-like.** `kit::panel` with `kit::panel_header("Databases")` carrying two
`kit::icon_button`s: `+` (raise the connection form) and the gear (`open_db_settings`, the project
settings dialog open on the Databases section — `open_kb_settings`'s twin). Under it `kit::filter_bar`,
the IDE explorer's, over a window-owned `db_filter` input: it filters the **loaded** nodes by name,
case-insensitive, keeping each match's ancestors, and never fetches. The tree is `connection →
database → [schema] → object group → object → columns`, one `DbTree` per first expansion, rows drawn
with `kit::file_row`/`kit::twisty` and per-kind icons; a connection row carries a state word
(`connecting`, `failed` with retry, a lock and `read-only`), a table row its approximate count
right-aligned (`~12.3k`). Double-click on a table or view opens its `DbTable`. Every row has a
right-click menu through `kit::context_menu`, `KbMenu`'s discipline (a pure `db_menu_entries`, a pick
that is an index): connection — Connect/Disconnect, New SQL editor, Refresh, Edit connection…, Remove;
database and schema — New SQL editor, Refresh counts; table/view — Open data, Open in SQL editor (its
`SELECT` prefilled), Copy name, Refresh; column — Copy name.

**`DbTable` — an editor-like centre tab.** The proof's `table_view.rs`: a toolbar with WHERE and ORDER
BY fields (validated as typed by `edit::validate_fragment`), Refresh, paging and the count ("~N rows
(estimate)" until the exact count arrives); the grid on `gpui_component::table`; inline edit (double
click, Enter, F2) and the side form, the Inline | Form switch; pending edits tinted by row mark, the
SQL preview from `render_batch`, Apply and Discard; the "no primary key" badge; a read-only badge, and
on a read-write connection's table a lock toggle. Paging, refresh and filter refuse while edits are
pending. `cell_input.rs` keeps the NULL/empty-string rule and the bool select, date pickers and JSON
routing.

**`DbSql` — a bottom-dock tab, several at once.** The proof's `sql_editor.rs`: gpui-component's code
editor with the `sql` grammar (already a feature of `crates/ubiq`'s `gpui-component`), a connection
and database picker, live diagnostics from `sql::analyze` and `check_read_only`, Run (statement under
the cursor or the selection) and Run all (each statement, one result tab each), the timer and Stop
(`DbCancel`), Explain and Explain analyze into `plan.rs`, a read-only toggle (forced on a read-only
connection), and a confirm before a write statement runs.

**The Databases section.** `crates/ubiq/src/ui/sink/project.rs` gains `ext_ids::PROJECT_DB` and a
`databases` section, `SectionGate::WithRecord`, drawn like `kb`: one row per connection — name, kind,
host/database or file, read-only marker, `saved`/`not saved`/`session` — with Test, Edit and Remove,
a keystore warning row when `DbKeystore::Unavailable`, and **Add connection**. Add and Edit raise
`ui/db/conn_form.rs`, a `kit::modal` painted at the window root like `ui/kb/source_form.rs` and under
its three-layer dismissal guard: kind, a connection-string field and Parse (`conn::parse_connection_string`,
in the interface — the pure half), the per-kind fields, the write-only password with *Remember* and
*Forget password*, read-only, and Test with its answer inline. A SQLite path is chosen through the
host-browse picker (`PickerOwner::DbFile`, `begin_host_browse` from the project root — the KB folder
picker's two steps); *New database…* picks a folder, prompts a file name and sends `CreateDbFile`.

**Mapping the proof's interface.**

| Proof app file | Lands in | What changes |
|---|---|---|
| `app.rs` | `app/db/*.rs` | every handler sends a message; no `SharedConn`, no `run_blocking` |
| `workbench.rs`, `ui/{mod,topbar,dock,centre,status}.rs`, `runtime.rs`, `main.rs` | — | deleted: the dock, rail and status are Ubiq's |
| `store.rs` | `ubiq-host/src/db/store.rs` | host-side TOML, no passwords |
| `structure.rs` | `state/db/tree.rs` | loaders become `DbTree` messages |
| `pending.rs` | `state/db/pending.rs` | none |
| `theme.rs` | `crates/ubiq/src/theme.rs` | see below |
| `ui/tree.rs` | `ui/db/mod.rs` | kit rows, filter bar, `kit::context_menu` instead of `ContextMenuExt` |
| `ui/connection_dialog.rs` | `ui/db/conn_form.rs` | `kit::modal`; host picker instead of `prompt_for_paths`; write-only password |
| `ui/table_view.rs`, `ui/result_grid.rs` | `ui/db/table.rs`, `ui/db/grid.rs` | reads `DbState`, sends `DbTablePage`/`DbApplyEdits` |
| `ui/cell_input.rs`, `ui/json_editor.rs` | `ui/db/cell_input.rs`, `ui/db/json.rs` | tokens and sizes only |
| `ui/sql_editor.rs`, `ui/plan_view.rs` | `ui/db/sql.rs`, `ui/db/plan.rs` | sends `DbQuery`/`DbCancel`; results from `DbQueryResult` |

**Look and feel.** No literal colour outside `theme.rs`: the proof's palette maps onto Ubiq's existing
tokens (surfaces, text, borders, accent, danger, warning, success), and seven tokens are new, with a
value in both palettes — `db_row_edited`, `db_row_inserted`, `db_row_deleted`,
`db_statement_active`, `db_read_only`, `db_read_only_soft`, `db_plan_hot`. Every type size is
`theme::font(Family, Role)` and every icon size `theme::icon_sm/md/lg` — `just ui` greps for both.
The proof's `text_sm`/`text_base`/`text_code` and `hairline` go; grid and editor text use the
content family at the project's zoom, as the IDE editor does. Key bindings (Run, Run all, Stop, F2)
are declared once in the interface's keymap under a `"DbSql"`/`"DbTable"` context.

## 6. Documents and help the change owes

- A new feature document, `features/workbench-db`, copied from `workbench-ide.md`'s shape:
  *Behaviour* is §4–§5's user-visible half, *Implementation* the files. Then `just docs-index` for
  the INDEX row and the code map.
- `features/workbench.md` — DB in the mode list and the PROJECT group order; the three panel kinds.
- `tech/transport-contract.md` — a database family section, every variant above.
- `tech/decisions.md` — three rows: the `ubiq-db` crate with wire types re-exported by the proto;
  `aws-lc-rs` accepted with `ring` installed as process default; per-install AES-256-GCM key in the
  keychain over ciphertext in the config root's `local/`.
- `tech/project-structure.md`, `AGENTS.md` — five crates plus the library, not four.
- `tech/operations.md` — the `just ui` driver grep, `cargo test --manifest-path ubiq/Cargo.toml -p ubiq-db --features drivers`.
- `tech/ui-and-design.md` — the seven tokens; `features/logs.md` — `Subsystem::Db`.
- `crates/ubiq-host/src/store/project_dir.rs` — `GITIGNORE`'s list and `FOLLOWS` (code, but read as a
  document by every store author).
- A help page, `help/db/overview`, bound to `rail.db`; a `"db/"` entry in `help/manifest.toml`; the
  mode in `help/workbench/` and the glossary.
- Skills: the file map of `ubiq-ui`, the module map of `ubiq-host`, the message list of
  `ubiq-transport` (each skill's `reference/` folder).
- `backlog.md` — `aws-lc-sys` on Windows unproven; the drone answers `Unavailable`; no idle-session
  close; no server-side cursors; no SSH tunnel.

## 7. Work packages

Each is one Sonnet subagent; at most three run at once and no two own a file at the same time. Cargo
runs from the workspace root: `cargo build -p ubiq-proto|ubiq-host|ubiq` resolves the patched base. Base
tests cannot run in the root's resolution, so they run as `just base-test` does — `cargo test
--manifest-path ubiq/Cargo.toml -p <crate> <filter>` — paying the second build. Every prompt says:
Edit/Write only, never a script for a single edit; do not stash or revert others' work. The whole
series lands as one base change, so the documents written in WP9 are in the same commit as the code.

**The lock procedure** (WP1, measured against a scratch copy of the base). Adding the drivers to a
base crate and resolving (`cargo metadata --manifest-path ubiq/Cargo.toml --format-version 1`) only
*adds* entries to `ubiq/Cargo.lock` — no existing one moves — but resolves fresh: `whoami` 1.6.1
(with `redox_syscall` 0.8.1 and `plain`), `io-enum` 1.3.0, `derive_utils` 0.16.0, where the root lock
has 1.5.2, 1.2.1 and 0.15.1. Then `cargo update --manifest-path ubiq/Cargo.toml -p whoami --precise
1.5.2`, `-p io-enum --precise 1.2.1`, `-p derive_utils --precise 0.15.1` — the versions the
engine was built and tested with — and the workspace root's lock is re-seeded from the base's;
`just lock-agrees` then finds no disagreement. `lru` 0.18.3 needs no pin: the base already resolves it. From then on **the base lock
owns the database graph** and the workspace root's lock follows it; `aws-lc-rs`, `aws-lc-sys`, `cmake`
and `fs_extra` are new to the base lock, and are the TLS cost of §1.

| # | Package | Owns | After | Acceptance |
|---|---|---|---|---|
| WP1 | **`ubiq-db`**: move `dbx-core`, split `model`, `drivers` feature, serde derives, `log`→`tracing`; base workspace member; the lock procedure; the `just ui` driver grep; the crate and TLS decision rows | `ubiq/crates/ubiq-db/**`, `ubiq/Cargo.toml`, `ubiq/Cargo.lock`, `Cargo.lock`, `ubiq/Justfile` (`ui`), `tech/decisions.md` | — | `cargo test --manifest-path ubiq/Cargo.toml -p ubiq-db --features drivers`; `cargo tree --manifest-path ubiq/Cargo.toml -p ubiq-db -e no-dev` shows no driver; `just lock-agrees` |
| WP2 | **Contract**: ids, `ubiq-proto/src/db.rs`, the variants, `project_id()` arms, `Subsystem::Db`; the new variants named in the coordinator's and `app/wire.rs`'s catch-all arms only; transport-contract section | `crates/ubiq-proto/**`, those two match arms, `tech/transport-contract.md` | WP1 | `cargo build -p ubiq-proto -p ubiq-host -p ubiq`; `cargo test --manifest-path ubiq/Cargo.toml -p ubiq-proto` (a round trip per variant) |
| WP3 | **Storage and secrets**: `db/store.rs`, `db/secrets.rs`, `ProjectData::db_connections`, `FOLLOWS`/`GITIGNORE`, `Store::db_key` namespace, the `db` feature; the secrets decision row | `crates/ubiq-host/src/db/{store,secrets}.rs`, `store/project_dir.rs`, `connectors/store.rs`, `crates/ubiq-host/Cargo.toml` | WP2 | `cargo build -p ubiq-host`; `cargo test --manifest-path ubiq/Cargo.toml -p ubiq-host db::` — round trip, a wrong AAD refused, a lost key reads `Missing`, no password in `db.toml` |
| WP4 | **Sessions and dispatch**: `db/{mod,session,jobs}.rs`, the `ring` default, the coordinator's family block, `client_gone` cleanup, cancel | `crates/ubiq-host/src/db/{mod,session,jobs}.rs`, `coordinator.rs` | WP3 | `cargo build -p ubiq-host`; host tests against a temp SQLite file: tree, page, query, read-only refusal (layer 1 and 2), edits rolled back on error, cancel |
| WP5 | **Interface shell**: mode spec and ids, `RailMode::DB`, `View::Db`, icon and SVG, three `PanelKind` arms and adapter, `default_db_layout`, `state/db/mod.rs` with every field (sub-states stubbed), `app/db/mod.rs` + `receive_db`, all seven theme tokens, the keymap actions, `PROJECT_DB` slot | `ext/ids.rs`, `state/{workbench,ui_id,nav,dock}.rs`, `ui/rail.rs`, `ui/kit/icons.rs`, the SVG, `ui/dock/mod.rs`, `app/panels.rs`, `state/db/mod.rs`, `app/db/mod.rs`, `app/wire.rs`, `theme.rs`, the keymap, `tests/rail_container.rs` | WP2 | `cargo build -p ubiq --all-targets` and the base `just ui` greps; `cargo test --manifest-path ubiq/Cargo.toml -p ubiq --test rail_container` (DB after IDE, hidden until opted in) |
| WP6 | **Explorer and settings**: the explorer panel, filter, menus, `state/db/tree.rs`, the Databases section, `conn_form.rs`, the password prompt | `ui/db/mod.rs`, `ui/db/conn_form.rs`, `state/db/tree.rs`, `app/db/{explorer,settings}.rs`, `ui/sink/project.rs` | WP5 (WP4 to run it) | `cargo build -p ubiq --all-targets`; tests for `db_menu_entries` and the filter over a loaded tree |
| WP7 | **Table tab**: `state/db/{table,pending}.rs`, `ui/db/{table,grid,cell_input,json}.rs`, `app/db/table.rs` | those files | WP5 | `cargo build -p ubiq --all-targets`; the proof's `pending` tests moved and passing |
| WP8 | **SQL tab**: `state/db/sql.rs`, `ui/db/{sql,plan}.rs`, `app/db/sql.rs`, `db_sql_drafts` in `state/prefs.rs` | those files | WP5 | `cargo build -p ubiq --all-targets`; a test that Run picks the statement under the cursor |
| WP9 | **Documents, help, verify**: §6's list, `help/db/`, the skills' maps; then the whole gate | `_docs/**`, `help/**`, `.claude/skills/**`, `AGENTS.md` | WP1–WP8 | `just verify` at the workspace root, `just base-test`, `just docs-lint`, `just docs-check` |

Parallel lanes: WP3 → WP4 (host) runs beside WP5 (interface) once WP2 lands; WP6, WP7 and WP8 run
together once WP5 lands — they share no file, because WP5 wrote every shared one (`state/db/mod.rs`,
`theme.rs`, the keymap, `app/wire.rs`) up front. Running the mode for real needs WP4 and WP6–WP8.

## Next steps

1. A `ubiq-db` MCP server beside `ubiq-kb`, so an agent can list the project's connections and run a
   read-only query through layer 1–3 without being handed a password.
2. Per-person connections — a connection a user keeps out of the shared `db.toml` of a
   project-managed project.
3. SSH tunnels, reusing the SSH profiles the remote family already files secrets for.
4. Server-side cursors for results past the row cap, instead of a cut and a `truncated` flag.

## Related docs

- [`../tech/architecture.md`](../tech/architecture.md) — the bus every database message crosses, and
  the rule nothing writes inside a project folder except `.ubiq/`
- [`../tech/transport-contract.md`](../tech/transport-contract.md) — where the database family's
  section goes
- [`../tech/decisions.md`](../tech/decisions.md) — `D173`, which `db.toml` follows, and the three rows
  this work appends
- [`../features/workbench.md`](../features/workbench.md) — the rail groups, opt-in modes and dock
  conventions the DB mode reuses
- [`kb.md`](kb.md) — the knowledge base, whose explorer, settings section and message scoping this
  design copies
