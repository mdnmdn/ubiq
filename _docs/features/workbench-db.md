---
id: feat-workbench-db
title: DB mode — connections, tables and SQL
kind: feature
status: draft
summary: The rail's opt-in DB mode — the project's saved database connections (PostgreSQL, MySQL/MariaDB, SQLite, SQL Server) and the Databases section that edits them, the explorer tree, table tabs that page, filter and edit rows, SQL tabs with a timer, Stop, Explain and a read-only guard enforced in three layers by the host, the DBML export, where `db.toml` and the sealed passwords live, and the `ubiq-db` engine and host sessions behind it.
read_when: you are changing the DB mode — the explorer, a table or SQL tab, the connection form, the Databases settings section, the host's database sessions, how a password is kept, or the `ubiq-db` engine
updated: 2026-10-04
verified: 2026-10-04
code_anchors: [crates/ubiq-db/src/lib.rs, crates/ubiq-db/src/sql/readonly.rs, crates/ubiq-db/src/edit.rs, crates/ubiq-db/src/driver/mod.rs, crates/ubiq-db/src/dbml.rs, crates/ubiq-db/src/driver/structure.rs, crates/ubiq-proto/src/db.rs, crates/ubiq-host/src/db/mod.rs, crates/ubiq-host/src/db/session.rs, crates/ubiq-host/src/db/jobs.rs, crates/ubiq-host/src/db/store.rs, crates/ubiq-host/src/db/secrets.rs, crates/ubiq-host/src/db/agent.rs, crates/ubiq-host/src/db/editors.rs, crates/ubiq/src/state/db/mod.rs, crates/ubiq/src/state/db/tree.rs, crates/ubiq/src/state/db/table.rs, crates/ubiq/src/state/db/sql.rs, crates/ubiq/src/state/db/pending.rs, crates/ubiq/src/app/db/mod.rs, crates/ubiq/src/app/db/table.rs, crates/ubiq/src/app/db/sql.rs, crates/ubiq/src/ui/db/mod.rs, crates/ubiq/src/ui/db/explorer.rs, crates/ubiq/src/ui/db/table.rs, crates/ubiq/src/ui/db/sql.rs, crates/ubiq/src/ui/db/conn_form.rs, crates/ubiq/src/ui/db/settings.rs]
depends_on: [feat-workbench, tech-transport, tech-ui, tech-architecture]
review_cycle: monthly
---

# DB mode — connections, tables and SQL

## Purpose

DB is the rail mode a project's databases are read and edited in: the saved connections and their
structure down the left, one tab per open table in the centre, and SQL editor tabs beside them.
It is off until a project asks for it, and it never opens a connection from the window: every
connection, statement and password belongs to the host, and the interface sends messages
([the database family](../tech/transport-contract.md)). The chrome around it — the rail, the dock,
the tabs and the settings dialog — belongs to [the workbench](./workbench.md); the decisions behind
the engine, the TLS providers and the password key are `D199`, `D200`, `D201` and `D202` in
[the decision register](../tech/decisions.md).

## Behaviour

**DB is a `PROJECT` mode directly after IDE, drawn only once a project switches it on.** Its
`Availability` is `OptIn`: the tile appears in project settings > General > Modes unlit, and the
rail shows the mode once it is lit (`ViewPrefs::opted_in_modes`). The first visit opens the explorer
on the left, the "No table open" page in the centre (where table and SQL tabs land), and an empty right and
bottom. Opening a project in DB asks the host for its connection list once.

**A connection is a name and an engine's fields.** PostgreSQL, MySQL/MariaDB and SQL Server take a
host, port, database, user and password; SQLite takes a file. A connection may be flagged read-only,
and carries a SSL choice. The Databases section of project settings lists them — name, engine,
where it points, a read-only marker, and whether a password is `saved`, `not saved` or held for the
`session` only — each with Test, Edit and Remove (Remove asks in the row itself), and an **Add
connection** button. A warning row says so when this install cannot keep a password. Add and Edit
raise one modal form: the engine, a box that takes a pasted connection string and fills the
fields, the per-engine fields, SSL, read-only, an **Agents** section (access `None` / `Read-only` /
`Read-write` — read-write is unavailable while the connection is read-only, and flagging it
read-only drops an `rw` choice to `ro` — a **Default for agents** checkbox, disabled at `None`, a
one-line description, and on SQL Server a warning that read-only is best-effort so the login should
hold read-only rights; Save always sends these), and Test with its answer inline. A SQLite file is
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

**`⌘W` closes the displayed SQL or table tab.** A SQL tab closes at once (its text is kept as a
draft); a table tab with pending edits first asks on the file tabs' "Unsaved changes" confirm, and
Discard closes it.

**Read-only is visible, and enforced elsewhere.** A table tab is read-only when its connection is,
when it shows a view or synonym, or when its lock is on; the lock toggles on a read-write
connection's table. A read-only tab hides `+ Row`, Delete and Apply, and its side form inspects a
row without editing it.

**A SQL tab runs what the cursor is on.** Several tabs can be open at once in the centre, each
with its own connection, database picker and text. **Run** takes the selection, else the statement
under the cursor, which carries a highlight ground and is named in the toolbar (`▸ 2/5 UPDATE`);
**Run all** takes every statement, one result tab each (`1 · SELECT`, `2 · UPDATE`). A timer counts
while it runs and **Stop** cancels it. **Explain** shows the plan of the statement as an indented
tree, the costliest node tinted and named, with the engine's raw text beside it; **Explain
analyze** executes the statement and measures it, so it asks first. The toolbar is flush chrome
(buttons full height, icon beside each label, no margin). The result grid is dense: the explorer's
row height, the `Dense` text role, 6px cell padding. A read-only toggle sits in the toolbar as an
icon-only button, yellow (`warning`) while on, locked on for a read-only connection; its tab takes
a distinct ground and a READ-ONLY chip. A write statement asks before it runs. Syntax errors, and each statement the read-only check
refuses, are editor diagnostics. Run is cmd/ctrl-Enter, Run all cmd/ctrl-shift-Enter, Stop
cmd/ctrl-`.`, and F2 edits the selected table cell. A SQL tab survives a restart as its connection,
database and text, with no result. The panel is a centre kind (`T-285`): a layout saved while it
sat in the bottom dock has it moved to the centre on load.

**An agent can drive a SQL tab.** `DbEditorChanged` opens the tab for the editor's session if the
window has none (`DbEditorsListed`, answering the `DbEditors` asked once the connections load, opens
the ones that already exist) and sets its text; it raises the tab only when `reveal` is set and
never takes focus or changes the rail mode. Such a tab is marked: a left edge, a ◆ and the ink on
the tab label, a "Controlled by <title>" tooltip and an "Agent · <title>" chip in the toolbar, in
the `agent_controlled` tokens. The user may type in it: after 250 ms of quiet the text goes as a
`DbEditorEdit` with the tab's `rev`, and only when it differs from what the host last held, so the
host's broadcast of that edit does not come back as one; a refused (stale) edit's answer replaces
the text. `DbAgentRun` shows the run in progress with the timer, the `DbQueryResult`s that follow
fill the tab's results, and Stop sends `DbCancel`. An agent tab is never saved as a draft.

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
| Agent settings | `description` in `db.toml` (omitted when empty); `access` (`none`, `ro`, `rw`) and `default` in `<config root>/projects/<ulid>/local/db-agents.toml`, `[[agent]] id, access, default`, per machine, never tracked — a connection it does not name has no agent access |
| Agent invariants | on every save and load: a read-only connection's `rw` reads `ro`; `none` is never the default; saving a default clears every other; several defaults on disk — the first in `db.toml`'s order wins |
| Shared editors | host memory only, keyed `(agent, name)` per project; `rev` bumped per change of text, identical text no change |
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
`mssql.rs`), plus the `EXPLAIN` parsers in `driver/plan.rs` and the structure queries in
`driver/structure.rs`. `dbml.rs` is the structure model and `to_dbml`, in the default half. The interface links the default half
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

**The agents' half of the host.** `store.rs`'s `split` and `join` are the one place that says
which agent setting lives in which file. `agent.rs` is `AgentDb`, a cloneable handle on the same
service (`Db::agent_handle(everyone)`) whose methods block, for the SQL MCP threads: `connections`,
`run`, `tree`, `dbml`, and the editor calls `open_editor`, `read_editor`, `edit_editor` and `run_editor`.
Its sessions are keyed by agent and connection, answer to `Mailbox::nowhere()` so no connection
state or password prompt reaches a window, and are dropped after 10 minutes idle, swept on the
next agent call. A run checks the connection's access (none is not found; a write needs `rw` on a
connection that is not read-only), selects its database every time, applies the parser gate and
`ExecOptions.read_only` when read-only, caps rows at `row_limit` (10 000 at most) and waits its
timeout plus 10 seconds before cancelling. A run shown in a panel is announced as `DbAgentRun` and
its results follow as `DbQueryResult`, both to every window; it is registered with no owning
window, so `DbCancel` from any window stops it. `editors.rs` holds the shared editors: an agent's
change broadcasts `DbEditorChanged` (revealed when new); the editor has *changed since an agent's
last write* when its `rev` is past the one that write left, or, for an agent that never wrote,
when it is past 0. `force` writes; `overwrite_return_previous` writes and returns the replaced
text when it had changed; `keep_if_user_changed` writes nothing when it had. `Db::handle` answers
`DbEditors` and `DbEditorEdit` inline: an edit whose `base_rev` is not the editor's is stale, and
the asker alone is told the editor as it stands; an accepted one is broadcast.

**The interface.** `state/db/` holds the state on the open project: `mod.rs` (`DbState`, the list,
keystore, connection states, the open tabs and the password prompt), `tree.rs` (nodes, `visible`,
the filter and `db_menu_entries`, all pure), `table.rs`, `sql.rs` (including `pick`, the pure Run
rule), `pending.rs` (the edit buffer) and `form.rs`. `app/db/` holds the handlers: `mod.rs` is
the only place a `Db*` message is built and `receive_db` the only place its replies arrive, called
from `app/wire.rs`; `close_active_db_tab()` answers `⌘W` (`close_active_editor()` asks it first) and
raises `FileDialog::DiscardChanges` for a tab with pending edits; `explorer.rs`, `table.rs`, `sql.rs` and `settings.rs` own their gestures.
`ui/db/` draws: `explorer.rs`, `table.rs` with `grid.rs`, `cell_input.rs` and `json.rs`, `sql.rs`
with `results.rs` and `plan.rs`, `conn_form.rs`, `settings.rs`, and `keys.rs` for the actions and
bindings in the `DbTable` and `DbSql` key contexts. Widgets that need a `Window` are queued and built
in `render`. A table or SQL tab mints a `DbSessionId` when it opens and every request carries it, so
a reply for a closed tab is discarded by id; a run mints a `DbQueryId`, which is also what
Stop cancels by. The mode is registered in `ui/rail.rs`'s `modes()` straight after IDE, with
`RailMode::DB` (`state/workbench.rs`), `View::Db` (`state/nav.rs`) and the three `PanelKind` arms in
`state/dock.rs`. Its colours are seven `db_*` theme tokens, listed in
[UI and design](../tech/ui-and-design.md).

## The SQL MCP servers

Two servers, `ubiq-sql-read` and `ubiq-sql-write` (`mcp/sql/`, `D202`), let an agent use the
connections a user opened to agents. Neither is in a default set: an agent definition names them.
The read server sees every `ro` and `rw` connection and always runs read-only (the parser gate and
the engine's guard); the write server sees `rw` connections only and adds `execute`.

| Tool | What it does |
|---|---|
| `list_connections` | name, engine, database, access, default, description |
| `list_objects`, `describe_table` | databases, schemas, tables and a table's columns |
| `export_dbml` | the structure as DBML text (see below); `schema?`, `tables?[]`, `timeout_ms?` |
| `query` | one read-only statement |
| `execute` (write only) | up to 20 statements, each autocommitted, stopping at the first failure |
| `get_blob` | the rest of a cut or binary cell, by id |
| `open_editor`, `read_editor`, `edit_editor`, `run_editor` | the named query editors shared with the user |

`query` and `execute` require `max_rows` (1 to 10 000) and `max_field_size` (0 to 100 000), take
`timeout_ms` (100 to 300 000, default 30 000) and an optional `panel` of at most 64 characters,
which also shows the statements and their results to the user in the editor of that name. An
omitted `connection` is the default, else the only eligible one; on the write server an `ro`
default is an error. A connection that needs a password answers `NeedsPassword` with a hint to ask
the user to open it once.

The answer is TOON (`toon-format`, text sent as it is rather than JSON-quoted): `connection`,
`engine`, `read_only`, `duration_ms`, `row_count`, `more_rows`, `truncated_fields`,
`binary_blobs`, then `columns[N]{name,type,nullable}` and `rows[N]{...}` keyed by the column
names, with duplicates renamed (`id`, `id_2`). Several statements are `statements[N]`; a statement
that changed rows says `affected`. Null, integers, finite floats and booleans are native; every
other cell, a decimal included, is a string. A text cell over `max_field_size` characters is cut
and ends in `[blob:<id>:<total chars>]`; a binary cell is always `[blob:<id>:<bytes>]`. The whole
answer is capped at 200 000 characters: trailing rows are dropped and the statement says
`cut_by_output_cap`. A failure is an in-band error holding what ran before it.

`get_blob(id, offset, length)` counts characters for text and bytes for binary, which come back
base64. The cache is keyed `(agent, id)`, so no agent reads another's, and holds at most 1 024
blobs, 64 MiB in all and 16 MiB each; a blob expires 10 minutes after it was last read.

Each SQL `tools/call` runs on a thread of its own (`ubiq-sql-call`), at most 8 at once; a ninth is
refused as busy. The host waits `timeout_ms` plus 10 seconds, then cancels. The editors'
`agent_title` is the calling agent's name in the MCP registry.

## DBML export

A connection's structure, reverse-engineered from the catalog and written as DBML. A window asks
with `DbExportDbml` on its tree session; an agent with `export_dbml` on either SQL server, whose
answer is the DBML text itself, not TOON. Both read the catalog only, so neither needs the
read-only guard, and neither moves the session's current database. The scope is a whole database,
one schema, or a list of tables (`name` or `schema.name`). An agent's omitted `database` is the
connection's configured one.

**In the explorer.** The right-click on a connection (while connected), database, schema or table
offers `DBML: Copy to clipboard` and `DBML: Open in new file` (the menu is flat, so a prefix stands
for a submenu). The scope follows the node (`Node::dbml_scope`): connection and database
ask for the whole database (`database` set on a database node), a schema for `schema`, a table for
`schema` plus `tables: [name]`. The window mints a `DbExportId`, sends `DbExportDbml`, and keeps
`(sink, "<label>.dbml")` in `DbState::dbml_pending` until `DbDbmlReady` arrives. Copy writes the
clipboard and raises a quiet notification; open pushes an untitled editor buffer named
`<label>.dbml` (`-2`, `-3` when taken), which a save-as turns into a file; a failure raises an error
notification with the host's text. An unknown or already-answered request id is dropped.

`Connection::structure` fills `ubiq_db::dbml::Structure` with five catalog queries per engine
(`driver/structure.rs`); `to_dbml` writes it, a pure function.

| Engine | Comments | Enums | Index method | Left out |
|---|---|---|---|---|
| PostgreSQL | `obj_description`, `col_description` | `pg_enum` types | `pg_am` | partitions (the parent is written), partial-index predicates, `INCLUDE` columns |
| MySQL / MariaDB | `TABLE_COMMENT`, `COLUMN_COMMENT` | each `enum(...)` column gets `Enum <table>_<column>_enum` | `INDEX_TYPE` | functional key parts (the whole index), foreign keys to another database |
| SQLite | none | none | none | expression indexes |
| SQL Server | `MS_Description` extended properties | none | none | included columns, filter predicates |

Views, sequences, triggers and check constraints are never written. The DBML's rules:

- The default schema (`public`, `dbo`) is not written; any other prefixes the name
  (`audit.events`). MySQL and SQLite have no schema level and write no prefix.
- A single-column primary key is the column's `pk`; a composite one is `(a, b) [pk]` under
  `Indexes`. A single-column unique index is the column's `unique`, without its name.
- Column settings, in order: `pk`, `increment`, `not null`, `unique`, `default`, `note`. A default
  is a number, a `'string'`, `true`/`false`, `null` or a `` `expression` ``. A serial's `nextval`
  default is dropped for `increment`.
- An index method of `hash` is `type: hash`; `btree` is left out; any other becomes
  `note: 'using <method>'`.
- `Ref <name>: a.b > c.d [delete: …, update: …]`, composite as `t.(a, b)`. A `no action` action is
  left out, and a Ref is written only when both tables are in the export.
- Identifiers that are not plain words, or that are DBML keywords, are double-quoted; notes are
  single-quoted, triple-quoted when they span lines, with quotes and backslashes escaped.

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
| `db-agents.toml` is unreadable | Every connection reads as closed to agents, and it is logged |
| A user's editor edit started from an older `rev` | Refused; that window is sent the editor as it stands |

## Related docs

- [The workbench](./workbench.md) — the rail, opt-in modes, the dock and the settings dialog
- [Transport contract](../tech/transport-contract.md) — the database family
- [Decision register](../tech/decisions.md) — `D199`, `D200` and `D201`
- [UI and design](../tech/ui-and-design.md) — the `db_*` tokens and the kit the explorer is built from
- [Project structure](../tech/project-structure.md) — where `db.toml` and the sealed passwords sit

## Next steps

1. Per-person connections that stay out of a project-managed project's shared `db.toml`.
2. SSH tunnels, reusing the profiles the remote family files secrets for.
3. Server-side cursors for results past the row cap, in place of a cut and a truncated flag.
4. Closing idle sessions.
