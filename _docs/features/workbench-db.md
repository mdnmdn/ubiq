---
id: feat-workbench-db
title: DB mode — connections, tables and SQL
kind: feature
status: draft
summary: The rail's opt-in DB mode — the project's saved database connections (PostgreSQL, MySQL/MariaDB, SQLite, SQL Server) and the Databases section that edits them, the explorer tree, table tabs that page, filter and edit rows, SQL tabs with a timer, Stop, Explain and a read-only guard enforced in three layers by the host, where `db.toml` and the sealed passwords live, and the `ubiq-db` engine and host sessions behind it.
read_when: you are changing the DB mode — the explorer, a table or SQL tab, the connection form, the Databases settings section, the host's database sessions, how a password is kept, or the `ubiq-db` engine
updated: 2026-10-01
verified: 2026-10-01
code_anchors: [crates/ubiq-db/src/lib.rs, crates/ubiq-db/src/sql/readonly.rs, crates/ubiq-db/src/edit.rs, crates/ubiq-db/src/driver/mod.rs, crates/ubiq-proto/src/db.rs, crates/ubiq-host/src/db/mod.rs, crates/ubiq-host/src/db/session.rs, crates/ubiq-host/src/db/jobs.rs, crates/ubiq-host/src/db/store.rs, crates/ubiq-host/src/db/secrets.rs, crates/ubiq/src/state/db/mod.rs, crates/ubiq/src/state/db/tree.rs, crates/ubiq/src/state/db/table.rs, crates/ubiq/src/state/db/sql.rs, crates/ubiq/src/state/db/pending.rs, crates/ubiq/src/app/db/mod.rs, crates/ubiq/src/app/db/table.rs, crates/ubiq/src/app/db/sql.rs, crates/ubiq/src/ui/db/mod.rs, crates/ubiq/src/ui/db/explorer.rs, crates/ubiq/src/ui/db/table.rs, crates/ubiq/src/ui/db/sql.rs, crates/ubiq/src/ui/db/conn_form.rs, crates/ubiq/src/ui/db/settings.rs]
depends_on: [feat-workbench, tech-transport, tech-ui, tech-architecture]
review_cycle: monthly
---

# DB mode — connections, tables and SQL

## Purpose

DB is the rail mode a project's databases are read and edited in: the saved connections and their
structure down the left, one tab per open table in the centre, and SQL editor tabs along the bottom.
It is off until a project asks for it, and it never opens a connection from the window: every
connection, statement and password belongs to the host, and the interface sends messages
([the database family](../tech/transport-contract.md)). The chrome around it — the rail, the dock,
the tabs and the settings dialog — belongs to [the workbench](./workbench.md); the decisions behind
the engine, the TLS providers and the password key are `D199`, `D200` and `D201` in
[the decision register](../tech/decisions.md).

## Behaviour

**DB is a `PROJECT` mode directly after IDE, drawn only once a project switches it on.** Its
`Availability` is `OptIn`: the tile appears in project settings > General > Modes unlit, and the
rail shows the mode once it is lit (`ViewPrefs::opted_in_modes`). The first visit opens the explorer
on the left, the "No table open" page in the centre, an empty right and an empty bottom region the
first SQL tab lands in. Opening a project in DB asks the host for its connection list once.

**A connection is a name and an engine's fields.** PostgreSQL, MySQL/MariaDB and SQL Server take a
host, port, database, user and password; SQLite takes a file. A connection may be flagged read-only,
and carries a SSL choice. The Databases section of project settings lists them — name, engine,
where it points, a read-only marker, and whether a password is `saved`, `not saved` or held for the
`session` only — each with Test, Edit and Remove (Remove asks in the row itself), and an **Add
connection** button. A warning row says so when this install cannot keep a password. Add and Edit
raise one modal form: the engine, a box that takes a pasted connection string and fills the
fields, the per-engine fields, SSL, read-only, and Test with its answer inline. A SQLite file is
chosen through the host browse picker, or created with **New database…**, which picks a folder and
asks for a file name. The explorer's header carries the same two entry points, **+** and a gear
that opens the settings dialog on the Databases section.

**A password is typed and never read back.** The form's password field is empty on open even when
one is saved; beside it a word says `saved` or `not saved`. Saving with nothing typed keeps the
saved password, typing replaces it, and *Forget password* clears it. *Remember* decides whether a
typed password is kept on disk at all. A connection whose password is missing — a reset keychain,
a copied config root — is never an error: the explorer prompts for it, and answering with
*Remember* seals it again.

**The explorer is a lazy tree.** `connection → database → [schema] → object group → object →
column`, one request per first expansion, and a filter bar over it that narrows what is *loaded* by
name, case-insensitive, keeping each match's ancestors, and never fetches. A connection row says
where it stands — `connecting`, `failed` with a retry, a lock and `read-only` — and a table row
carries its approximate row count, right-aligned (`~12.3k`). Double-click on a table or view opens
its tab. The right-click menu is per row: a connection offers Connect or Disconnect (with New SQL
editor and Refresh when connected), Edit connection… and Remove; a database or schema, New SQL
editor and Refresh counts; a table or view, Open data, Open in SQL editor (its `SELECT`
prefilled), Copy name and Refresh; a column, Copy name. Down and Tab step from the filter onto the
tree, Escape clears the query, Enter opens what the query landed on.

**A table tab pages, filters and edits.** The toolbar holds a WHERE and an ORDER BY field, each
validated as it is typed, Refresh, the page-size choice (50 to 1000, 200 by default), paging and
the count: "~N rows (estimate)" until the exact count arrives with the first page. The grid
edits in place (double-click, Enter or F2 on a cell) or, in the **Form** arrangement of the
Inline | Form switch, through a side form with one input per column. NULL and the empty string are
different cells: backspace on an empty nullable field makes it NULL, a space on a NULL text field
makes it `''`, a boolean is a dropdown, a date a calendar picker, and a JSON column (or any text
cell holding an object or array, or one forced with the `{ }` button) opens a JSON dialog with
Format, Minify and Set NULL. Edits are pending, never sent as typed: an edited cell is bold and its
row tinted by its mark (edited, inserted, deleted), the **SQL preview** shows the statements Apply
will run, and Apply sends them as one batch. A table with no primary key says so with a badge ("edits match on all columns"). Paging, refresh and filtering refuse while edits are pending.

**Read-only is visible, and enforced elsewhere.** A table tab is read-only when its connection is,
when it shows a view or synonym, or when its lock is on; the lock toggles on a read-write
connection's table. A read-only tab hides `+ Row`, Delete and Apply, and its side form inspects a
row without editing it.

**A SQL tab runs what the cursor is on.** Several tabs can be open at once along the bottom, each
with its own connection, database picker and text. **Run** takes the selection, else the statement
under the cursor, which carries a highlight ground and is named in the toolbar (`▸ 2/5 UPDATE`);
**Run all** takes every statement, one result tab each (`1 · SELECT`, `2 · UPDATE`). A timer counts
while it runs and **Stop** cancels it. **Explain** shows the plan of the statement as an indented
tree, the costliest node tinted and named, with the engine's raw text beside it; **Explain
analyze** executes the statement and measures it, so it asks first. A read-only toggle sits in the
toolbar, locked on for a read-only connection; its tab takes a distinct ground and a READ-ONLY
chip. A write statement asks before it runs. Syntax errors, and each statement the read-only check
refuses, are editor diagnostics. Run is cmd/ctrl-Enter, Run all cmd/ctrl-shift-Enter, Stop
cmd/ctrl-`.`, and F2 edits the selected table cell. A SQL tab survives a restart as its connection,
database and text, with no result.

**Three layers keep a read-only session from writing, all in the host.** First, every statement
passes an AST check before anything is sent (`check_read_only`, fail closed); a refusal names the
rule. Second, the engine is told to refuse a write itself: a read-only transaction on PostgreSQL
and MySQL, `query_only` on SQLite, a rolled-back transaction on SQL Server. Third, a timeout (30
seconds for the tree and pages, 5 minutes for the SQL editor), a row cap (1 000 by default, 10 000
at most, and 16 MiB of cells) that sets a *truncated* flag, and a log line per executed statement.
The interface runs the same AST check for its diagnostics and badge, but only the host's answer
decides.

## Contract

The messages, ids and failure kinds are the database family of
[the transport contract](../tech/transport-contract.md); this section holds what is not there.

| Fact | Value |
|---|---|
| Rail slug, help context | `db`, `rail.db` |
| Panel kinds | `DbExplorer` (`ubiq.db.explorer`, Edge / Left), `DbTable` (`ubiq.db.table`, Centre), `DbSql` (`ubiq.db.sql`, Free / Bottom) |
| Table and SQL tab payloads | `db:<conn>:<database>:<schema>:<name>`, `dbsql:<session id>` |
| Settings slot | `ext_ids::PROJECT_DB`, section *Databases*, drawn like the knowledge base's |
| Connection list | `<data dir>/db.toml`, tracked, no secret — follows the project's storage mode (`D173`) |
| Sealed passwords | `<config root>/projects/<ulid>/local/db-secrets.toml`, per machine, never tracked |
| Password key | one per install, 32 bytes, in the OS keychain as `db` / `secrets-key` |
| Sealing | AES-256-GCM through `ring`, a fresh 96-bit nonce per write, associated data `ubiq-db:v1:<project id>:<connection id>` |
| Log | target `ubiq_db::sql` and `ubiq_host::db`, subsystem Database; SQL cut at 2 000 characters; no connection string or password |
| Draft limit | a SQL tab's text is kept to 64 KiB (`DB_SQL_DRAFT_MAX`) |

A `db.toml` drops every parameter that names a secret (`password`, `pwd`, `sslpassword`, `sslkey`)
and stores a SQLite path inside the project relative to its root. A file written by a newer version
is never overwritten.

## Implementation

**The engine, `crates/ubiq-db`.** A leaf crate that names no Ubiq crate. By default the model
(`model.rs`, `plan.rs`) and the pure engine: `conn.rs` (configuration and connection-string
parsing), `value.rs` (`DataType`, `Value`, `parse_input`), `sql.rs` with `sql/readonly.rs` (analysis,
`statement_at`, `check_read_only`), and `edit.rs` (`select_table`, `count_table`,
`validate_fragment`, `RowEdit`, `render_batch`). The `drivers` feature adds `driver/` — the
`Connection` trait, `connect` and the four engines (`sqlite.rs`, `postgres.rs`, `mysql.rs`,
`mssql.rs`), plus the `EXPLAIN` parsers in `driver/plan.rs`. The interface links the default half
and `just ui` fails if a driver crate reaches its tree; only the host enables `drivers`.
`crates/ubiq-proto/src/db.rs` re-exports the model and adds the records the contract owns.

**The host, `crates/ubiq-host/src/db/`.** `mod.rs` is `Db`, a concrete service the coordinator owns
behind the `db` feature (part of `full`), and installs `ring` as the process's default TLS provider
(`D200`). `store.rs` reads and writes `db.toml`; `secrets.rs` seals and opens passwords, with the
key taken from the connector family's secret store (`connectors/store.rs`). `session.rs` is one
worker thread per table or SQL tab, plus one *meta* session per `(window, connection)` for the tree,
each holding one driver connection and a job queue, connecting on its first job; the list itself
runs on one ordered admin thread, and Test and file creation on one-off threads. `jobs.rs` turns
each message into a job and applies the read-only layers. A cancel publishes the running query's
handle and `DbCancel` fires it on a one-off thread. `Db::client_gone`, closing a session, a
disconnect and a saved edit or removal of a connection drop its sessions. `ubiq-drone` is built
without the `db` feature and answers the whole family `Unavailable`.

**The interface.** `state/db/` holds the state on the open project: `mod.rs` (`DbState`, the list,
keystore, connection states, the open tabs and the password prompt), `tree.rs` (nodes, `visible`,
the filter and `db_menu_entries`, all pure), `table.rs`, `sql.rs` (including `pick`, the pure Run
rule), `pending.rs` (the edit buffer) and `form.rs`. `app/db/` holds the handlers: `mod.rs` is
the only place a `Db*` message is built and `receive_db` the only place its replies arrive, called
from `app/wire.rs`; `explorer.rs`, `table.rs`, `sql.rs` and `settings.rs` own their gestures.
`ui/db/` draws: `explorer.rs`, `table.rs` with `grid.rs`, `cell_input.rs` and `json.rs`, `sql.rs`
with `results.rs` and `plan.rs`, `conn_form.rs`, `settings.rs`, and `keys.rs` for the actions and
bindings in the `DbTable` and `DbSql` key contexts. Widgets that need a `Window` are queued and built
in `render`. A table or SQL tab mints a `DbSessionId` when it opens and every request carries it, so
a reply for a closed tab is discarded by id; a run mints a `DbQueryId`, which is also what
Stop cancels by. The mode is registered in `ui/rail.rs`'s `modes()` straight after IDE, with
`RailMode::DB` (`state/workbench.rs`), `View::Db` (`state/nav.rs`) and the three `PanelKind` arms in
`state/dock.rs`. Its colours are seven `db_*` theme tokens, listed in
[UI and design](../tech/ui-and-design.md).

## Failure

| Situation | What happens |
|---|---|
| The mode is not switched on for a project | The rail does not draw it |
| The host is a drone, or built without the `db` feature | Every request is answered `Unavailable`; the keystore reads `Unavailable` and nothing is saved |
| The keychain is unusable | Nothing is written to `db-secrets.toml`; a typed password is held in the host's memory for the run and lists as `session` |
| The key is missing or a sealed password does not open | The connection lists `Missing`; its next use answers `NeedsPassword` and the explorer prompts |
| A write reaches a read-only session | The AST check refuses it with `ReadOnly` naming the rule; failing that, the engine refuses it |
| A statement runs past its timeout | The reply is `Timeout` |
| Stop is pressed | The running statement ends `Cancelled` |
| A batch of edits fails at one statement | The whole batch rolls back and the failing statement's index and text are reported |
| A result is past the row or size cap | The rows are cut and the result is marked truncated |
| A connection is removed or edited | Its sessions close and its open tabs are closed |
| A window closes | Its sessions are dropped |

## Related docs

- [The workbench](./workbench.md) — the rail, opt-in modes, the dock and the settings dialog
- [Transport contract](../tech/transport-contract.md) — the database family
- [Decision register](../tech/decisions.md) — `D199`, `D200` and `D201`
- [UI and design](../tech/ui-and-design.md) — the `db_*` tokens and the kit the explorer is built from
- [Project structure](../tech/project-structure.md) — where `db.toml` and the sealed passwords sit

## Next steps

1. A `ubiq-db` MCP server beside `ubiq-kb`, so an agent can list the project's connections and run
   a read-only query through the three layers without being handed a password.
2. Per-person connections that stay out of a project-managed project's shared `db.toml`.
3. SSH tunnels, reusing the profiles the remote family files secrets for.
4. Server-side cursors for results past the row cap, in place of a cut and a truncated flag.
5. Closing idle sessions.
