//! `driver` — the contract every engine implements, and one module per engine: [`sqlite`]
//! (rusqlite), [`mysql`] (the sync `mysql` crate) and [`mssql`] (tiberius on a private
//! current-thread tokio runtime) and [`postgres`] (`sqlx-postgres` on the same pattern; the
//! `postgres` crate family cannot pass `just lock-agrees`).
//!
//! # The structure levels, per engine
//!
//! The tree is always `connection → database → [schema] → object group → object → columns`;
//! [`DbKind::has_schemas`] says whether the schema level exists, [`DbKind::object_kinds`] which
//! groups to show.
//!
//! | Engine | [`Connection::databases`] | [`Connection::schemas`] | Object kinds |
//! |---|---|---|---|
//! | PostgreSQL | every non-template database on the server | the database's schemas, system schemas (`pg_catalog`, `information_schema`, `pg_toast…`) excluded | tables, views, materialized views |
//! | MySQL / MariaDB | the server's databases (= schemas), system ones excluded | empty — database *is* the schema; call `objects(db, None)` | tables, views |
//! | SQLite | `main` plus every attached database (`PRAGMA database_list`, `temp` excluded) | empty; call `objects(db, None)` | tables, views |
//! | SQL Server | the server's databases (`sys.databases`) | the database's schemas (`sys.schemas`, fixed roles excluded) | tables, views, synonyms |
//!
//! Postgres cannot query across databases on one session: a driver asked about a database other
//! than the one it is connected to opens (and may cache) a second session for it. That is the
//! driver's business — the caller just passes the database name. To *query* another database,
//! call [`Connection::use_database`] first.
//!
//! # Threading
//!
//! Every method blocks. The UI holds a connection as `Arc<Mutex<Box<dyn Connection>>>` and calls
//! it from `cx.background_executor()`; hence `Send`. A driver that needs an async runtime
//! (tiberius) owns it privately and blocks on it inside each method.
//!
//! # Read-only (layer 2 of `_docs/readonly.md`)
//!
//! [`ExecOptions::read_only`] asks for a statement to run so that the server refuses any write.
//! A connection opened from a [`ConnectionConfig::read_only`] config forces it on every
//! [`Connection::query_with`] / [`Connection::execute_with`] (and so on `query` / `execute`); a
//! refusal is [`DbError::ReadOnly`] or the server's own error. Layer 1 — the AST check in
//! [`crate::sql`] — is the caller's job and runs *before* this; neither layer alone is enough
//! (sequence increments, files and external calls survive a rollback).
//!
//! | Engine | Per call | Connection opened `read_only` | One statement only |
//! |---|---|---|---|
//! | PostgreSQL | `BEGIN READ ONLY` → stmt → `ROLLBACK`; refused if a read-write transaction is already open | also `default_transaction_read_only=on` for every session | the statement is prepared first (Parse rejects two) |
//! | MySQL / MariaDB | `START TRANSACTION READ ONLY` → stmt → `ROLLBACK` (an open transaction is committed implicitly by the `START`) | also `SET SESSION TRANSACTION READ ONLY` at login | the statement is prepared first (`COM_STMT_PREPARE` rejects two; a statement MySQL cannot prepare is refused) |
//! | SQLite | `PRAGMA query_only = ON` for the call, and `sqlite3_stmt_readonly` must say true | opened `SQLITE_OPEN_READ_ONLY` | rusqlite refuses a second statement |
//! | SQL Server | `BEGIN TRAN` → stmt → `ROLLBACK` — a **backstop only**: there is no read-only transaction, the statement really runs, and a `COMMIT` inside the batch escapes it. The parser is the main guard here. Refused if a transaction is already open | also `ApplicationIntent=ReadOnly` at login | not enforced (T-SQL needs no `;`) — the parser's job |
//!
//! # Cancel and timeouts (layer 3)
//!
//! The UI's connection mutex is held for the whole call, so a cancel cannot go through the
//! connection: take [`Connection::cancel_handle`] *before* the call and call
//! [`CancelHandle::cancel`] from any thread. The running statement then fails with
//! [`DbError::Cancelled`]; one that outlives [`ExecOptions::timeout`] fails with
//! [`DbError::Timeout`]. Cancelling when nothing runs is a no-op (best effort: a cancel that
//! lands just as a statement ends may hit the next one on MySQL).
//!
//! | Engine | Cancel | Timeout |
//! |---|---|---|
//! | PostgreSQL | a short side session runs `pg_cancel_backend(pid)` | server-side `statement_timeout` (`SET LOCAL` in the read-only transaction, else `SET` … `RESET`) |
//! | MySQL / MariaDB | a short side connection runs `KILL QUERY <connection id>` | a client-side deadline thread that cancels the same way |
//! | SQLite | `sqlite3_interrupt` (rusqlite's `InterruptHandle`) | a client-side deadline thread that interrupts |
//! | SQL Server | a TDS Attention (tiberius `cancel_query`); the connection stays usable — if the attention itself fails the call answers `Disconnected` | the same attention, sent by a client-side deadline |
//!
//! Row limits are [`ExecOptions::row_limit`], as `query`'s `limit` always was.
//!
//! # Plans
//!
//! [`Connection::explain`] returns a portable [`Plan`] tree plus the engine's raw output.
//! `analyze = true` *executes* the statement, always inside a transaction (or savepoint) that is
//! rolled back — a read-only one on a read-only connection.
//!
//! | Engine | Estimate | `analyze` |
//! |---|---|---|
//! | PostgreSQL | `EXPLAIN (FORMAT JSON)` | `EXPLAIN (ANALYZE, FORMAT JSON)` in `BEGIN` … `ROLLBACK` (a savepoint inside an open transaction) |
//! | MySQL | `EXPLAIN FORMAT=TREE`; MariaDB / old MySQL fall back to tabular `EXPLAIN` | `EXPLAIN ANALYZE` (MySQL 8.0.18+) in `START TRANSACTION` … `ROLLBACK`; elsewhere `Unsupported` |
//! | SQLite | `EXPLAIN QUERY PLAN` (no costs) | the statement is also run inside a savepoint that is rolled back: the root gets the row count and the elapsed time |
//! | SQL Server | `SET SHOWPLAN_XML ON` | `SET STATISTICS XML ON` in `BEGIN TRAN` … `ROLLBACK` (a savepoint inside an open transaction) |
//!
//! # Approximate row counts
//!
//! [`DbObject::approx_rows`] is the planner's **estimate**, never a `count(*)`, read in the same
//! catalog query as the object list: Postgres `pg_class.reltuples` (`None` until the table is
//! first analyzed), MySQL `information_schema.TABLES.TABLE_ROWS` (InnoDB's is a rough sample),
//! SQL Server `sys.partitions.rows` (heap or clustered index), SQLite `sqlite_stat1` (`None` until
//! `ANALYZE` ran). Views are always `None`; materialized views are counted on Postgres.
//!
//! # Statement log
//!
//! Every statement run through `query_with`, `execute_with` and `explain` is logged at `info`
//! on target `ubiq_db::sql`: engine, read-only flag, elapsed time, outcome and the SQL. Catalog
//! queries are not logged.

mod mssql;
mod mysql;
mod plan;
mod postgres;
mod sqlite;

use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::{Duration, Instant};

pub use sqlite::create_sqlite_database;

use crate::conn::{ConnectionConfig, DbKind};
// The model types the engines name through `super::`; they live in `crate::model` and `crate::plan`.
use crate::model::{
    ColumnMeta, DbError, DbObject, ExecOptions, ExecOutcome, ObjectKind, Result, ResultSet,
    TableRef,
};
use crate::plan::{Plan, PlanFormat};
use crate::value::{DataType, Value, parse_cell};

/// Stops the statement running on a connection, from another thread. Taken with
/// [`Connection::cancel_handle`] before the call, since the call holds the connection.
pub trait CancelHandle: Send + Sync {
    /// Ask the server to stop the running statement; it then fails with [`DbError::Cancelled`].
    /// `Ok` means the request was sent, not that anything was running. Postgres and MySQL open a
    /// short side connection for it, so this may block for a round trip (or a connect timeout).
    fn cancel(&self) -> Result<()>;
}

/// A live session to one server or file. Blocking; see the module docs for threading.
///
/// Each method takes a single statement — splitting a script is [`crate::sql`]'s job, and the
/// caller runs the pieces one by one. `databases`, `schemas` and `objects` return entries sorted
/// by name.
pub trait Connection: Send {
    fn kind(&self) -> DbKind;

    /// A round-trip (`SELECT 1`). The dialog's "Test" is `connect` followed by `ping`.
    fn ping(&mut self) -> Result<()>;

    /// The top level of the tree — see the module table for what each engine lists.
    fn databases(&mut self) -> Result<Vec<String>>;

    /// Schemas of `database`; empty when [`DbKind::has_schemas`] is false.
    fn schemas(&mut self, database: &str) -> Result<Vec<String>>;

    /// Tables, views, materialized views and synonyms of `database` / `schema`. `schema` is
    /// `None` exactly when [`DbKind::has_schemas`] is false.
    fn objects(&mut self, database: &str, schema: Option<&str>) -> Result<Vec<DbObject>>;

    /// Columns of a table or view, in ordinal order, with nullability, key, default and
    /// identity facts filled. A synonym is resolved to its target.
    fn columns(&mut self, table: &TableRef) -> Result<Vec<ColumnMeta>>;

    /// Make `db` the database `query` and `execute` run against, so a table of another database
    /// can be opened from the tree. Postgres switches to that database's own session (opened on
    /// demand); MySQL and SQL Server run `USE`; SQLite accepts `main` or an attached name (the
    /// qualifier already routes the statement) and answers `NotFound` otherwise. On failure the
    /// current database is unchanged.
    fn use_database(&mut self, db: &str) -> Result<()>;

    /// The database `query` and `execute` currently run against, as far as the driver knows:
    /// `None` when `connect` named none and nothing was selected since (MySQL, SQL Server).
    fn current_database(&self) -> Option<String>;

    /// Run a row-returning statement. With `Some(limit)` the driver reads at most `limit` rows
    /// and sets `truncated` if there were more — it does not rewrite the SQL. A statement that
    /// returns no rows yields an empty `ResultSet` with no columns.
    ///
    /// `query_with` with only `row_limit` set; still read-only on a read-only connection.
    fn query(&mut self, sql: &str, limit: Option<usize>) -> Result<ResultSet> {
        self.query_with(
            sql,
            &ExecOptions {
                row_limit: limit,
                ..ExecOptions::default()
            },
        )
    }

    /// Run a statement that returns no rows (DML, DDL). Runs in autocommit.
    ///
    /// `execute_with` with default options; still read-only on a read-only connection.
    fn execute(&mut self, sql: &str) -> Result<ExecOutcome> {
        self.execute_with(sql, &ExecOptions::default())
    }

    /// [`Connection::query`] under `opts`: read-only guard, timeout, row limit (module docs).
    fn query_with(&mut self, sql: &str, opts: &ExecOptions) -> Result<ResultSet>;

    /// [`Connection::execute`] under `opts` (`row_limit` ignored).
    fn execute_with(&mut self, sql: &str, opts: &ExecOptions) -> Result<ExecOutcome>;

    /// Whether the connection was opened from a [`ConnectionConfig::read_only`] config — every
    /// call is then read-only whatever its options say.
    fn is_read_only(&self) -> bool;

    /// A handle that cancels whatever statement this connection is running. Take it before
    /// locking the connection for the call. `None` when the engine cannot cancel (all four can).
    fn cancel_handle(&self) -> Option<Arc<dyn CancelHandle>>;

    /// The plan of one statement (module docs, "Plans"). `analyze` executes it inside a
    /// transaction that is rolled back.
    fn explain(&mut self, sql: &str, analyze: bool) -> Result<Plan>;
}

/// `sql` without trailing whitespace and `;`, for wrapping in `EXPLAIN …`.
fn strip_terminator(sql: &str) -> &str {
    sql.trim_end_matches(|c: char| c.is_whitespace() || c == ';')
}

/// Log one statement on target `ubiq_db::sql` and pass its result through.
fn log_statement<T>(
    kind: DbKind,
    what: &str,
    sql: &str,
    read_only: bool,
    started: Instant,
    result: Result<T>,
) -> Result<T> {
    let ms = started.elapsed().as_secs_f64() * 1000.0;
    let ro = if read_only { " read-only" } else { "" };
    match &result {
        Ok(_) => tracing::info!(target: "ubiq_db::sql", "{kind:?} {what}{ro} ok {ms:.1}ms: {sql}"),
        Err(e) => {
            tracing::info!(target: "ubiq_db::sql", "{kind:?} {what}{ro} failed {ms:.1}ms ({e}): {sql}")
        }
    }
    result
}

/// A client-side deadline: after `timeout` it calls the cancel handle, unless dropped first.
/// For the engines whose server has no statement timeout of its own (MySQL for non-SELECTs,
/// SQLite).
struct Watchdog {
    /// (done, fired)
    state: Arc<(Mutex<(bool, bool)>, Condvar)>,
}

impl Watchdog {
    fn arm(timeout: Option<Duration>, cancel: Option<Arc<dyn CancelHandle>>) -> Option<Self> {
        let (timeout, cancel) = (timeout?, cancel?);
        let state = Arc::new((Mutex::new((false, false)), Condvar::new()));
        let shared = state.clone();
        std::thread::Builder::new()
            .name("dbx-deadline".into())
            .spawn(move || {
                let (lock, cv) = &*shared;
                let guard = lock.lock().unwrap_or_else(PoisonError::into_inner);
                let (mut st, _) = cv
                    .wait_timeout_while(guard, timeout, |st| !st.0)
                    .unwrap_or_else(PoisonError::into_inner);
                if st.0 {
                    return;
                }
                st.1 = true;
                drop(st);
                let _ = cancel.cancel();
            })
            .ok()?;
        Some(Self { state })
    }

    fn fired(&self) -> bool {
        self.state
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .1
    }

    /// Disarm. Once it fired the call is [`DbError::Timeout`] whatever it returned: an
    /// interrupted statement may also come back "successful" (MySQL's `SLEEP()` answers 1).
    fn finish<T>(wd: Option<Self>, result: Result<T>) -> Result<T> {
        let Some(wd) = wd else { return result };
        let fired = wd.fired();
        drop(wd);
        if fired { Err(DbError::Timeout) } else { result }
    }
}

impl Drop for Watchdog {
    fn drop(&mut self) {
        let (lock, cv) = &*self.state;
        lock.lock().unwrap_or_else(PoisonError::into_inner).0 = true;
        cv.notify_all();
    }
}

/// Open a connection for `cfg`, after [`ConnectionConfig::validate`].
pub fn connect(cfg: &ConnectionConfig) -> Result<Box<dyn Connection>> {
    cfg.validate()?;
    match cfg.kind {
        DbKind::Sqlite => sqlite::open(cfg),
        DbKind::MySql => mysql::open(cfg),
        DbKind::MsSql => mssql::open(cfg),
        DbKind::Postgres => postgres::open(cfg),
    }
}

/// Text from the engine for a column of a temporal / uuid / json type, in the canonical form of
/// [`Value`]; text that does not validate (MySQL's `0000-00-00`, free-form SQLite dates) stays
/// `Value::Text`. Any other type is `Value::Text` unchanged.
fn canonical(text: String, meta: &ColumnMeta) -> Value {
    let wanted = matches!(
        meta.data_type,
        DataType::Date
            | DataType::Time
            | DataType::DateTime
            | DataType::DateTimeTz
            | DataType::Uuid
            | DataType::Json
    );
    if wanted
        && !text.is_empty()
        && let Ok(v) = parse_cell(&text, meta)
    {
        return v;
    }
    Value::Text(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conn::ParseError;

    #[test]
    fn connect_validates() {
        let mut cfg = ConnectionConfig::new(DbKind::Postgres);
        cfg.port = Some(0);
        assert!(matches!(connect(&cfg), Err(DbError::Config(_))));
        let cfg = ConnectionConfig::new(DbKind::Sqlite);
        assert!(matches!(
            connect(&cfg),
            Err(DbError::Config(ParseError::Missing("path")))
        ));
    }

    struct Count(std::sync::atomic::AtomicU32);

    impl CancelHandle for Count {
        fn cancel(&self) -> Result<()> {
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        }
    }

    #[test]
    fn watchdog_fires_only_past_the_deadline() {
        let count = Arc::new(Count(Default::default()));
        let fired = || count.0.load(std::sync::atomic::Ordering::SeqCst);
        let h = || Some(count.clone() as Arc<dyn CancelHandle>);
        assert!(Watchdog::arm(None, h()).is_none());
        assert!(Watchdog::arm(Some(Duration::from_millis(1)), None).is_none());
        // disarmed in time: never fires
        let wd = Watchdog::arm(Some(Duration::from_millis(300)), h());
        assert_eq!(Watchdog::finish(wd, Ok(1)), Ok(1));
        std::thread::sleep(Duration::from_millis(400));
        assert_eq!(fired(), 0);
        // past the deadline: cancels once, and the call is a timeout
        let wd = Watchdog::arm(Some(Duration::from_millis(20)), h());
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(Watchdog::finish(wd, Ok(1)), Err(DbError::Timeout));
        assert_eq!(fired(), 1);
    }

    #[test]
    fn terminator_is_stripped() {
        assert_eq!(strip_terminator("SELECT 1 ; \n;"), "SELECT 1");
    }
}
