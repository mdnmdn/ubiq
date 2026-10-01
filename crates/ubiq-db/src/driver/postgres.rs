//! PostgreSQL through `sqlx-postgres` (the crate directly — not the `sqlx` facade, whose sqlite
//! half would add a second `libsqlite3-sys`), on a private current-thread tokio runtime that every
//! call blocks on. Nothing async leaves this file.
//!
//! - Every statement goes through `raw_sql`, i.e. the **simple query protocol**: all values come
//!   back as text, so no `chrono`/`uuid`/`rust_decimal` feature is needed and [`cell`] converts by
//!   the column's type name. A type it does not know stays [`Value::Text`].
//! - TLS (rustls, ring): `Disable` → plain, `Prefer` → TLS if the server offers it, `Require` → TLS
//!   (certificate not checked unless `params["sslrootcert"]` names a CA file), `VerifyFull` → TLS
//!   with chain and host name verified. `params["connect_timeout"]` is seconds (default 10).
//! - One session per database: the one `connect` opened is the *current* one, and `query`,
//!   `execute` and `ping` run on it. `schemas`, `objects` and `columns` for another database open
//!   a second session on demand (a handful are kept). `use_database` makes another database's
//!   session the current one; the SQL builder drops the database part for Postgres.
//! - Read-only: the statement is prepared first (the extended protocol's Parse refuses a second
//!   statement), then runs as `BEGIN READ ONLY` → stmt → `ROLLBACK`; if `BEGIN` landed inside a
//!   read-write transaction the caller left open, the call is refused and that transaction is
//!   left alone. A read-only connection also sets `default_transaction_read_only=on` on every
//!   session it opens.
//! - Timeout: `SET LOCAL statement_timeout` inside the read-only transaction, else
//!   `SET statement_timeout` … `RESET statement_timeout`. Cancel: a short side session to the
//!   home database runs `pg_cancel_backend(pid)` on the current session's backend.
//! - `approx_rows` is `pg_class.reltuples` (`None` while it is `-1`, i.e. never analyzed).

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use futures_util::TryStreamExt;
use sqlx_core::column::Column;
use sqlx_core::connection::Connection as _;
use sqlx_core::executor::Executor;
use sqlx_core::raw_sql::raw_sql;
use sqlx_core::row::Row;
use sqlx_core::type_info::TypeInfo;
use sqlx_core::value::ValueRef;
use sqlx_postgres::{PgColumn, PgConnectOptions, PgConnection, PgRow, PgSslMode, PgValueFormat};
use tokio::runtime::Runtime;

use super::{
    CancelHandle, ColumnMeta, Connection, DbError, DbObject, ExecOptions, ExecOutcome, ObjectKind,
    Plan, PlanFormat, Result, ResultSet, TableRef, canonical, log_statement, plan,
    strip_terminator,
};
use crate::conn::{ConnectionConfig, DbKind, SslMode};
use crate::value::{DataType, Value};

/// Sessions kept besides the current one.
const EXTRA_SESSIONS: usize = 3;

pub(super) fn open(cfg: &ConnectionConfig) -> Result<Box<dyn Connection>> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| DbError::Connect(format!("cannot start the async runtime: {e}")))?;
    let db = cfg
        .database
        .as_deref()
        .filter(|d| !d.is_empty())
        .unwrap_or("postgres");
    let mut conn = rt.block_on(open_one(cfg, db))?;
    let home = rt
        .block_on(raw_sql("SELECT current_database()").fetch_all(&mut conn))
        .ok()
        .and_then(|rows| rows.first().and_then(|r| text(r, 0)))
        .unwrap_or_else(|| db.to_string());
    let mut sessions = HashMap::new();
    sessions.insert(home.clone(), conn);
    let cancel = Arc::new(PgCancel {
        cfg: cfg.clone(),
        home: home.clone(),
        pid: Mutex::new(None),
    });
    Ok(Box::new(Postgres {
        sessions,
        pids: HashMap::new(),
        current: home.clone(),
        home,
        cfg: cfg.clone(),
        cancel,
        rt,
    }))
}

struct Postgres {
    // the sessions first: they must drop while the runtime is still alive
    sessions: HashMap<String, PgConnection>,
    /// Backend pid of each session, learnt on its first statement.
    pids: HashMap<String, i32>,
    /// The database `connect` opened; its session is never evicted.
    home: String,
    /// The database `query`, `execute` and `ping` run on — `home` until `use_database`.
    current: String,
    cfg: ConnectionConfig,
    cancel: Arc<PgCancel>,
    rt: Runtime,
}

/// Cancels the backend that last ran a user statement, from a side session.
struct PgCancel {
    cfg: ConnectionConfig,
    home: String,
    pid: Mutex<Option<i32>>,
}

impl CancelHandle for PgCancel {
    fn cancel(&self) -> Result<()> {
        let Some(pid) = *self.pid.lock().unwrap_or_else(PoisonError::into_inner) else {
            return Ok(());
        };
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| DbError::Connect(format!("cannot start the async runtime: {e}")))?;
        rt.block_on(async {
            let mut side = open_one(&self.cfg, &self.home).await?;
            let sent = raw_sql(&format!("SELECT pg_cancel_backend({pid})"))
                .execute(&mut side)
                .await
                .map_err(err);
            let _ = side.close().await;
            sent.map(|_| ())
        })
    }
}

fn ssl_mode(m: SslMode) -> PgSslMode {
    match m {
        SslMode::Disable => PgSslMode::Disable,
        SslMode::Prefer => PgSslMode::Prefer,
        SslMode::Require => PgSslMode::Require,
        SslMode::VerifyFull => PgSslMode::VerifyFull,
    }
}

fn options(cfg: &ConnectionConfig, db: &str) -> PgConnectOptions {
    let port = cfg.effective_port().unwrap_or(5432);
    let host = cfg.effective_host();
    let mut o = PgConnectOptions::new_without_pgpass();
    o = if host.starts_with('/') {
        o.socket(Path::new(host).join(format!(".s.PGSQL.{port}")))
    } else {
        o.host(host).port(port)
    };
    o = o.database(db).ssl_mode(ssl_mode(cfg.ssl)).application_name(
        cfg.params
            .get("application_name")
            .map_or("dbx", String::as_str),
    );
    if let Some(u) = cfg.user.as_deref().filter(|u| !u.is_empty()) {
        o = o.username(u);
    }
    if let Some(p) = cfg.password.as_deref() {
        o = o.password(p);
    }
    if let Some(ca) = cfg.params.get("sslrootcert").filter(|c| !c.is_empty()) {
        o = o.ssl_root_cert(ca);
    }
    if cfg.read_only {
        o = o.options([("default_transaction_read_only", "on")]);
    }
    o
}

async fn open_one(cfg: &ConnectionConfig, db: &str) -> Result<PgConnection> {
    let secs = cfg
        .params
        .get("connect_timeout")
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(10);
    let opts = options(cfg, db);
    let connecting = PgConnection::connect_with(&opts);
    match tokio::time::timeout(Duration::from_secs(secs), connecting).await {
        Err(_) => Err(DbError::Connect(format!("timed out after {secs}s"))),
        Ok(Err(e)) => Err(match code(&e).as_deref() {
            // invalid_catalog_name: the database does not exist
            Some("3D000") => DbError::NotFound(message(&e)),
            _ => DbError::Connect(message(&e)),
        }),
        Ok(Ok(c)) => Ok(c),
    }
}

/// The SQLSTATE of a server error.
fn code(e: &sqlx_core::Error) -> Option<String> {
    e.as_database_error()
        .and_then(|d| d.code())
        .map(|c| c.into_owned())
}

fn message(e: &sqlx_core::Error) -> String {
    match e.as_database_error() {
        Some(d) => d.message().to_string(),
        None => e.to_string(),
    }
}

fn err(e: sqlx_core::Error) -> DbError {
    use sqlx_core::Error as E;
    match &e {
        E::Io(_) | E::Tls(_) | E::Protocol(_) | E::WorkerCrashed => {
            DbError::Disconnected(e.to_string())
        }
        // connection exceptions / admin shutdown, crash shutdown
        E::Database(_) if code(&e).is_some_and(|c| c.starts_with("08") || c.starts_with("57P")) => {
            DbError::Disconnected(message(&e))
        }
        // query_canceled: "… due to statement timeout" or "… due to user request"
        E::Database(_) if code(&e).as_deref() == Some("57014") => {
            if message(&e).contains("timeout") {
                DbError::Timeout
            } else {
                DbError::Cancelled
            }
        }
        E::Database(_) => DbError::Query(message(&e)),
        _ => DbError::Query(e.to_string()),
    }
}

/// `'text'` as a string literal (default `standard_conforming_strings`).
fn lit(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// A column of a catalog row as text; `None` for NULL.
fn text(row: &PgRow, i: usize) -> Option<String> {
    let raw = row.try_get_raw(i).ok()?;
    if raw.is_null() {
        return None;
    }
    raw.as_str().ok().map(str::to_string)
}

fn flag(row: &PgRow, i: usize) -> bool {
    text(row, i).is_some_and(|t| t == "t")
}

impl Postgres {
    /// The session for `db` (`None` = the current one), opened on first use.
    fn sess(&mut self, db: Option<&str>) -> Result<(&Runtime, &mut PgConnection)> {
        let Self {
            sessions,
            pids,
            home,
            current,
            cfg,
            rt,
            ..
        } = self;
        let db = db.filter(|d| !d.is_empty()).unwrap_or(current.as_str());
        if !sessions.contains_key(db) {
            if sessions.len() > EXTRA_SESSIONS
                && let Some(old) = sessions
                    .keys()
                    .find(|k| *k != home && *k != current)
                    .cloned()
            {
                sessions.remove(&old);
                pids.remove(&old);
            }
            let conn = rt.block_on(open_one(cfg, db))?;
            sessions.insert(db.to_string(), conn);
        }
        let conn = sessions.get_mut(db).expect("session just ensured");
        Ok((rt, conn))
    }

    /// A catalog query on `db`'s session; rows as text.
    fn rows(&mut self, db: Option<&str>, sql: &str) -> Result<Vec<PgRow>> {
        let (rt, conn) = self.sess(db)?;
        rt.block_on(raw_sql(sql).fetch_all(&mut *conn)).map_err(err)
    }

    /// Point the cancel handle at the current session's backend (pid learnt once per session).
    fn arm_cancel(&mut self) -> Result<()> {
        let pid = match self.pids.get(&self.current) {
            Some(p) => Some(*p),
            None => {
                let pid = self
                    .rows(None, "SELECT pg_backend_pid()::text")?
                    .first()
                    .and_then(|r| text(r, 0))
                    .and_then(|t| t.parse::<i32>().ok());
                if let Some(p) = pid {
                    self.pids.insert(self.current.clone(), p);
                }
                pid
            }
        };
        *self
            .cancel
            .pid
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = pid;
        Ok(())
    }

    /// Run `body` on the current session under the read-only guard and the timeout, and log it.
    fn guarded<T>(
        &mut self,
        what: &str,
        sql: &str,
        opts: &ExecOptions,
        body: impl FnOnce(&Runtime, &mut PgConnection) -> Result<T>,
    ) -> Result<T> {
        let ro = opts.read_only || self.cfg.read_only;
        let started = Instant::now();
        let result = (|| {
            self.arm_cancel()?;
            let (rt, conn) = self.sess(None)?;
            let ms = opts.timeout.map(|t| t.as_millis().max(1));
            if ro {
                // Parse takes exactly one statement: `SELECT 1; DELETE …` is refused here
                rt.block_on((&mut *conn).prepare(sql)).map_err(err)?;
                let rows = rt
                    .block_on(
                        raw_sql("BEGIN READ ONLY; SELECT current_setting('transaction_read_only')")
                            .fetch_all(&mut *conn),
                    )
                    .map_err(err)?;
                if rows.first().and_then(|r| text(r, 0)).as_deref() != Some("on") {
                    // BEGIN was a no-op inside the caller's read-write transaction
                    return Err(DbError::ReadOnly(
                        "a read-write transaction is open on this connection; \
                         commit or roll it back first"
                            .into(),
                    ));
                }
            }
            if let Some(ms) = ms {
                let set = if ro { "SET LOCAL" } else { "SET" };
                let r = rt
                    .block_on(
                        raw_sql(&format!("{set} statement_timeout = {ms}")).execute(&mut *conn),
                    )
                    .map_err(err);
                if let Err(e) = r {
                    if ro {
                        let _ = rt.block_on(raw_sql("ROLLBACK").execute(&mut *conn));
                    }
                    return Err(e);
                }
            }
            let r = body(rt, conn);
            let cleanup = match (ro, ms) {
                (true, _) => Some("ROLLBACK"),
                (false, Some(_)) => Some("RESET statement_timeout"),
                (false, None) => None,
            };
            if let Some(c) = cleanup {
                let _ = rt.block_on(raw_sql(c).execute(&mut *conn));
            }
            r
        })();
        log_statement(DbKind::Postgres, what, sql, ro, started, result)
    }
}

impl Connection for Postgres {
    fn kind(&self) -> DbKind {
        DbKind::Postgres
    }

    fn ping(&mut self) -> Result<()> {
        self.rows(None, "SELECT 1").map(|_| ())
    }

    fn databases(&mut self) -> Result<Vec<String>> {
        Ok(self
            .rows(
                None,
                "SELECT datname::text FROM pg_database \
                 WHERE NOT datistemplate AND datallowconn \
                   AND has_database_privilege(datname, 'CONNECT') ORDER BY datname",
            )?
            .iter()
            .filter_map(|r| text(r, 0))
            .collect())
    }

    fn schemas(&mut self, database: &str) -> Result<Vec<String>> {
        Ok(self
            .rows(
                Some(database),
                "SELECT nspname::text FROM pg_namespace \
                 WHERE nspname <> 'information_schema' AND nspname !~ '^pg_' ORDER BY nspname",
            )?
            .iter()
            .filter_map(|r| text(r, 0))
            .collect())
    }

    fn objects(&mut self, database: &str, schema: Option<&str>) -> Result<Vec<DbObject>> {
        let scope = match schema {
            Some(s) => format!("n.nspname = {}", lit(s)),
            None => "n.nspname <> 'information_schema' AND n.nspname !~ '^pg_'".to_string(),
        };
        let rows = self.rows(
            Some(database),
            &format!(
                "SELECT c.relname::text, c.relkind::text, n.nspname::text, \
                        obj_description(c.oid, 'pg_class'), \
                        CASE WHEN c.relkind IN ('r', 'p', 'm') AND c.reltuples >= 0 \
                             THEN c.reltuples::bigint::text END \
                 FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
                 WHERE c.relkind IN ('r', 'p', 'v', 'm', 'f') AND {scope} \
                 ORDER BY n.nspname, c.relname"
            ),
        )?;
        Ok(rows
            .iter()
            .filter_map(|r| {
                let kind = match text(r, 1)?.as_str() {
                    "v" => ObjectKind::View,
                    "m" => ObjectKind::MaterializedView,
                    _ => ObjectKind::Table, // tables, partitioned tables, foreign tables
                };
                Some(DbObject {
                    kind,
                    name: text(r, 0)?,
                    schema: Some(text(r, 2)?),
                    database: Some(database.to_string()),
                    target: None,
                    comment: text(r, 3).filter(|c| !c.is_empty()),
                    approx_rows: text(r, 4).and_then(|n| n.parse().ok()),
                })
            })
            .collect())
    }

    fn columns(&mut self, table: &TableRef) -> Result<Vec<ColumnMeta>> {
        let schema = table.schema.as_deref().unwrap_or("public");
        let rows = self.rows(
            table.database.as_deref(),
            &format!(
                "SELECT a.attname::text, format_type(a.atttypid, a.atttypmod), \
                   NOT a.attnotnull, \
                   EXISTS (SELECT 1 FROM pg_index i WHERE i.indrelid = a.attrelid \
                           AND i.indisprimary AND a.attnum = ANY (i.indkey)), \
                   pg_get_expr(d.adbin, d.adrelid), a.attidentity::text, a.attgenerated::text, \
                   (SELECT string_agg(e.enumlabel, chr(31) ORDER BY e.enumsortorder) \
                    FROM pg_enum e WHERE e.enumtypid = a.atttypid) \
                 FROM pg_attribute a \
                 JOIN pg_class c ON c.oid = a.attrelid \
                 JOIN pg_namespace n ON n.oid = c.relnamespace \
                 LEFT JOIN pg_attrdef d ON d.adrelid = a.attrelid AND d.adnum = a.attnum \
                 WHERE n.nspname = {} AND c.relname = {} AND a.attnum > 0 AND NOT a.attisdropped \
                 ORDER BY a.attnum",
                lit(schema),
                lit(&table.name)
            ),
        )?;
        if rows.is_empty() {
            return Err(DbError::NotFound(format!("table {table}")));
        }
        Ok(rows
            .iter()
            .map(|r| {
                let mut c = ColumnMeta::new(
                    DbKind::Postgres,
                    &text(r, 0).unwrap_or_default(),
                    &text(r, 1).unwrap_or_default(),
                );
                c.nullable = flag(r, 2);
                c.is_pk = flag(r, 3);
                c.default = text(r, 4);
                c.computed = text(r, 6).is_some_and(|g| g == "s");
                c.auto_increment = text(r, 5).is_some_and(|i| i == "a" || i == "d")
                    || c.default
                        .as_deref()
                        .is_some_and(|d| d.starts_with("nextval("));
                if c.computed {
                    c.default = None; // the generation expression is not a default
                }
                if let Some(labels) = text(r, 7) {
                    c.data_type =
                        DataType::Enum(labels.split('\u{1f}').map(String::from).collect());
                }
                c
            })
            .collect())
    }

    fn use_database(&mut self, db: &str) -> Result<()> {
        self.sess(Some(db))?; // opens (or reuses) the session; fails before `current` moves
        self.current = db.to_string();
        Ok(())
    }

    fn current_database(&self) -> Option<String> {
        Some(self.current.clone())
    }

    fn query_with(&mut self, sql: &str, opts: &ExecOptions) -> Result<ResultSet> {
        let limit = opts.row_limit;
        self.guarded("query", sql, opts, |rt, conn| {
            rt.block_on(async {
                let mut out = ResultSet::default();
                {
                    let mut stream = raw_sql(sql).fetch(&mut *conn);
                    while let Some(row) = stream.try_next().await? {
                        if out.columns.is_empty() {
                            out.columns = row.columns().iter().map(column_meta).collect();
                        }
                        if limit.is_some_and(|l| out.rows.len() >= l) {
                            out.truncated = true;
                            break; // the unread rest is discarded by the next call
                        }
                        out.rows.push(cells(&row, &out.columns));
                    }
                }
                if out.columns.is_empty()
                    && let Ok(d) = (&mut *conn).describe(sql).await
                {
                    // no rows came back, so there was no row to read the columns from
                    out.columns = d.columns().iter().map(column_meta).collect();
                }
                Ok(out)
            })
            .map_err(err)
        })
    }

    fn execute_with(&mut self, sql: &str, opts: &ExecOptions) -> Result<ExecOutcome> {
        self.guarded("execute", sql, opts, |rt, conn| {
            let done = rt.block_on(raw_sql(sql).execute(&mut *conn)).map_err(err)?;
            Ok(ExecOutcome {
                rows_affected: done.rows_affected(),
            })
        })
    }

    fn is_read_only(&self) -> bool {
        self.cfg.read_only
    }

    fn cancel_handle(&self) -> Option<Arc<dyn CancelHandle>> {
        Some(self.cancel.clone())
    }

    fn explain(&mut self, sql: &str, analyze: bool) -> Result<Plan> {
        let ro = self.cfg.read_only;
        let started = Instant::now();
        let body = strip_terminator(sql);
        let result = (|| {
            self.arm_cancel()?;
            let (rt, conn) = self.sess(None)?;
            // one statement only: `EXPLAIN SELECT 1; DELETE …` would run the DELETE
            rt.block_on((&mut *conn).prepare(body)).map_err(err)?;
            let mut undo = None;
            if analyze {
                // ANALYZE executes: inside the caller's transaction a savepoint, else our own
                // transaction, rolled back either way
                match rt.block_on(raw_sql("SAVEPOINT dbx_explain").execute(&mut *conn)) {
                    Ok(_) => {
                        undo =
                            Some("ROLLBACK TO SAVEPOINT dbx_explain; RELEASE SAVEPOINT dbx_explain")
                    }
                    // no_active_sql_transaction
                    Err(e) if code(&e).as_deref() == Some("25P01") => {
                        let begin = if ro { "BEGIN READ ONLY" } else { "BEGIN" };
                        rt.block_on(raw_sql(begin).execute(&mut *conn))
                            .map_err(err)?;
                        undo = Some("ROLLBACK");
                    }
                    Err(e) => return Err(err(e)),
                }
            }
            let stmt = if analyze {
                format!("EXPLAIN (ANALYZE, FORMAT JSON) {body}")
            } else {
                format!("EXPLAIN (FORMAT JSON) {body}")
            };
            let rows = rt.block_on(raw_sql(&stmt).fetch_all(&mut *conn));
            if let Some(u) = undo {
                let _ = rt.block_on(raw_sql(u).execute(&mut *conn));
            }
            let raw = rows
                .map_err(err)?
                .iter()
                .filter_map(|r| text(r, 0))
                .collect::<Vec<_>>()
                .join("\n");
            let root = plan::from_pg_json(&raw).map_err(DbError::Query)?;
            Ok(Plan {
                root,
                raw,
                format: PlanFormat::Json,
            })
        })();
        let what = if analyze {
            "explain analyze"
        } else {
            "explain"
        };
        log_statement(DbKind::Postgres, what, sql, ro, started, result)
    }
}

/// A result column, typed from the type name the server announced.
fn column_meta(col: &PgColumn) -> ColumnMeta {
    ColumnMeta::new(
        DbKind::Postgres,
        col.name(),
        &col.type_info().name().to_ascii_lowercase(),
    )
}

fn cells(row: &PgRow, columns: &[ColumnMeta]) -> Vec<Value> {
    columns
        .iter()
        .enumerate()
        .map(|(i, meta)| match row.try_get_raw(i) {
            Ok(raw) if !raw.is_null() => {
                if raw.format() == PgValueFormat::Binary {
                    return Value::Text(format!("<{}>", meta.native_type));
                }
                match raw.as_str() {
                    Ok(s) => cell(s.to_string(), meta),
                    Err(_) => Value::Bytes(raw.as_bytes().map(<[u8]>::to_vec).unwrap_or_default()),
                }
            }
            _ => Value::Null,
        })
        .collect()
}

/// A text-format cell, in the canonical form of [`Value`] for its column. What does not convert
/// (`infinity` dates, `timetz`, arrays, ranges, …) stays `Value::Text`.
fn cell(s: String, meta: &ColumnMeta) -> Value {
    match &meta.data_type {
        DataType::Bool => match s.as_str() {
            "t" => Value::Bool(true),
            "f" => Value::Bool(false),
            _ => Value::Text(s),
        },
        DataType::Int { .. } => s.parse::<i64>().map(Value::Int).unwrap_or(Value::Text(s)),
        DataType::Float => s.parse::<f64>().map(Value::Float).unwrap_or(Value::Text(s)),
        DataType::Decimal { .. } => Value::Decimal(s),
        DataType::Bytes => match s.strip_prefix("\\x").and_then(unhex) {
            Some(b) => Value::Bytes(b),
            None => Value::Text(s),
        },
        _ => canonical(s, meta),
    }
}

/// `bytea`'s hex output without the `\x`.
fn unhex(h: &str) -> Option<Vec<u8>> {
    if !h.len().is_multiple_of(2) || !h.is_ascii() {
        return None;
    }
    (0..h.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&h[i..i + 2], 16).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conn::parse_connection_string;
    use crate::driver::connect;

    fn col(native: &str) -> ColumnMeta {
        ColumnMeta::new(DbKind::Postgres, "c", native)
    }

    fn go(native: &str, text: &str) -> Value {
        cell(text.to_string(), &col(native))
    }

    #[test]
    fn text_cells_follow_the_column_type() {
        assert_eq!(go("bool", "t"), Value::Bool(true));
        assert_eq!(go("bool", "f"), Value::Bool(false));
        assert_eq!(go("int2", "-7"), Value::Int(-7));
        assert_eq!(go("int4", "42"), Value::Int(42));
        assert_eq!(go("int8", "9223372036854775807"), Value::Int(i64::MAX));
        assert_eq!(go("float4", "1.5"), Value::Float(1.5));
        assert_eq!(go("float8", "-0.25"), Value::Float(-0.25));
        assert_eq!(go("float8", "Infinity"), Value::Float(f64::INFINITY));
        assert!(matches!(go("float8", "NaN"), Value::Float(x) if x.is_nan()));
        assert_eq!(go("numeric", "12.50"), Value::Decimal("12.50".into()));
        assert_eq!(go("numeric", "NaN"), Value::Decimal("NaN".into()));
        assert_eq!(go("text", "héllo"), Value::Text("héllo".into()));
        assert_eq!(go("varchar", "a b"), Value::Text("a b".into()));
        assert_eq!(go("bpchar", "ab "), Value::Text("ab ".into()));
        assert_eq!(go("name", "pg_class"), Value::Text("pg_class".into()));
    }

    #[test]
    fn temporal_uuid_json_and_bytes() {
        assert_eq!(go("date", "2024-02-29"), Value::Date("2024-02-29".into()));
        assert_eq!(go("date", "infinity"), Value::Text("infinity".into()));
        assert_eq!(go("time", "03:04:05.5"), Value::Time("03:04:05.5".into()));
        assert_eq!(
            go("timestamp", "2020-01-02 03:04:05.678"),
            Value::DateTime("2020-01-02 03:04:05.678".into())
        );
        assert_eq!(
            go("timestamptz", "2020-01-02 03:04:05+00"),
            Value::DateTimeTz("2020-01-02 03:04:05+00".into())
        );
        assert_eq!(
            go("timestamptz", "2020-01-02 03:04:05.5-05:30"),
            Value::DateTimeTz("2020-01-02 03:04:05.5-05:30".into())
        );
        assert_eq!(
            go("uuid", "A0EEBC99-9C0B-4EF8-BB6D-6BB9BD380A11"),
            Value::Uuid("a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11".into())
        );
        assert_eq!(go("jsonb", "{\"a\": 1}"), Value::Json("{\"a\": 1}".into()));
        assert_eq!(go("bytea", "\\x00ff"), Value::Bytes(vec![0, 255]));
        assert_eq!(go("bytea", "\\x"), Value::Bytes(vec![]));
        assert_eq!(go("bytea", "\\xzz"), Value::Text("\\xzz".into()));
    }

    #[test]
    fn unknown_types_and_arrays_stay_text() {
        assert_eq!(go("_int4", "{1,2}"), Value::Text("{1,2}".into()));
        assert_eq!(go("int4[]", "{1,2}"), Value::Text("{1,2}".into()));
        assert_eq!(go("interval", "1 day"), Value::Text("1 day".into()));
        assert_eq!(go("money", "$1.00"), Value::Text("$1.00".into()));
        assert_eq!(go("mood", "happy"), Value::Text("happy".into()));
        assert_eq!(go("int4", "oops"), Value::Text("oops".into()));
    }

    #[test]
    fn ssl_modes_map_one_to_one() {
        assert!(matches!(ssl_mode(SslMode::Disable), PgSslMode::Disable));
        assert!(matches!(ssl_mode(SslMode::Prefer), PgSslMode::Prefer));
        assert!(matches!(ssl_mode(SslMode::Require), PgSslMode::Require));
        assert!(matches!(
            ssl_mode(SslMode::VerifyFull),
            PgSslMode::VerifyFull
        ));
    }

    #[test]
    fn literals_and_hex() {
        assert_eq!(lit("o'neil"), "'o''neil'");
        assert_eq!(unhex("0aFF"), Some(vec![10, 255]));
        assert_eq!(unhex("abc"), None);
    }

    #[test]
    fn options_follow_the_config() {
        let cfg =
            parse_connection_string("postgres://u:p@db.example:6543/shop?sslmode=require").unwrap();
        let o = options(&cfg, "shop");
        assert_eq!(o.get_host(), "db.example");
        assert_eq!(o.get_port(), 6543);
        assert_eq!(o.get_username(), "u");
        assert_eq!(o.get_database(), Some("shop"));
        assert!(matches!(o.get_ssl_mode(), PgSslMode::Require));
    }

    #[test]
    fn refused_connection_is_a_connect_error() {
        let cfg = parse_connection_string("postgres://u@127.0.0.1:1/db?sslmode=disable").unwrap();
        assert!(matches!(connect(&cfg), Err(DbError::Connect(_))));
    }

    /// Needs a server: `DBX_TEST_PG_URL=postgres://postgres:pw@localhost/postgres`. Creates and
    /// drops a schema named `dbx_test` in that database.
    #[test]
    #[ignore = "needs DBX_TEST_PG_URL"]
    fn live_server() {
        let url = std::env::var("DBX_TEST_PG_URL").expect("DBX_TEST_PG_URL");
        let cfg = parse_connection_string(&url).unwrap();
        let mut c = connect(&cfg).unwrap();
        c.ping().unwrap();
        c.execute("DROP SCHEMA IF EXISTS dbx_test CASCADE").unwrap();
        c.execute("CREATE SCHEMA dbx_test").unwrap();
        c.execute("CREATE TYPE dbx_test.mood AS ENUM ('sad', 'ok')")
            .unwrap();
        c.execute(
            "CREATE TABLE dbx_test.person (id serial PRIMARY KEY, \
             name varchar(40) NOT NULL, price numeric(10,2) DEFAULT 0, born date, \
             seen timestamp(3), at timestamptz, flag boolean, feel dbx_test.mood, doc jsonb, \
             photo bytea, tags text[], ident bigint GENERATED ALWAYS AS IDENTITY, \
             twice int GENERATED ALWAYS AS (id * 2) STORED)",
        )
        .unwrap();
        c.execute("COMMENT ON TABLE dbx_test.person IS 'people'")
            .unwrap();
        c.execute("CREATE VIEW dbx_test.v AS SELECT id, name FROM dbx_test.person")
            .unwrap();
        c.execute("CREATE MATERIALIZED VIEW dbx_test.mv AS SELECT id FROM dbx_test.person")
            .unwrap();
        let ins = c
            .execute(
                "INSERT INTO dbx_test.person (name, price, born, seen, at, flag, feel, doc, photo, tags) \
                 VALUES ('Ada', 12.5, '1815-12-10', '2020-01-02 03:04:05.678', \
                 '2020-01-02 03:04:05+00', true, 'ok', '{\"a\": 1}', '\\x00ff', '{x,y}'), \
                 ('Bob', NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL)",
            )
            .unwrap();
        assert_eq!(ins.rows_affected, 2);

        let dbs = c.databases().unwrap();
        assert!(!dbs.is_empty() && !dbs.iter().any(|d| d.starts_with("template")));
        let home = dbs
            .iter()
            .find(|d| cfg.database.as_deref().is_none_or(|n| n == d.as_str()))
            .unwrap()
            .clone();
        let schemas = c.schemas(&home).unwrap();
        assert!(schemas.iter().any(|s| s == "dbx_test"));
        assert!(
            !schemas
                .iter()
                .any(|s| s == "pg_catalog" || s == "information_schema")
        );
        let objs = c.objects(&home, Some("dbx_test")).unwrap();
        let kinds: Vec<_> = objs.iter().map(|o| (o.name.as_str(), o.kind)).collect();
        assert_eq!(
            kinds,
            [
                ("mv", ObjectKind::MaterializedView),
                ("person", ObjectKind::Table),
                ("v", ObjectKind::View)
            ]
        );
        assert_eq!(objs[1].comment.as_deref(), Some("people"));

        let cols = c.columns(&objs[1].table_ref()).unwrap();
        assert!(cols[0].is_pk && cols[0].auto_increment && !cols[0].nullable);
        assert_eq!(cols[1].native_type, "character varying(40)");
        assert_eq!(cols[2].native_type, "numeric(10,2)");
        assert_eq!(
            cols[7].data_type,
            DataType::Enum(vec!["sad".into(), "ok".into()])
        );
        assert!(cols[11].auto_increment);
        assert!(cols[12].computed && cols[12].default.is_none());

        let rs = c
            .query(
                "SELECT id, name, price, born, seen, at, flag, feel, doc, photo, tags \
                 FROM dbx_test.person ORDER BY id",
                None,
            )
            .unwrap();
        assert_eq!(rs.rows.len(), 2);
        assert_eq!(rs.rows[0][0], Value::Int(1));
        assert_eq!(rs.rows[0][2], Value::Decimal("12.50".into()));
        assert_eq!(rs.rows[0][3], Value::Date("1815-12-10".into()));
        assert_eq!(
            rs.rows[0][4],
            Value::DateTime("2020-01-02 03:04:05.678".into())
        );
        assert!(matches!(rs.rows[0][5], Value::DateTimeTz(_)));
        assert_eq!(rs.rows[0][6], Value::Bool(true));
        assert_eq!(rs.rows[0][7], Value::Text("ok".into()));
        assert_eq!(rs.rows[0][9], Value::Bytes(vec![0, 255]));
        assert_eq!(rs.rows[0][10], Value::Text("{x,y}".into()));
        assert_eq!(rs.rows[1][2], Value::Null);
        let rs = c.query("SELECT id FROM dbx_test.person", Some(1)).unwrap();
        assert!(rs.truncated && rs.rows.len() == 1);
        let rs = c
            .query("SELECT id, name FROM dbx_test.person WHERE false", None)
            .unwrap();
        assert!(rs.rows.is_empty() && rs.columns.len() == 2);
        c.ping().unwrap(); // the abandoned result above did not poison the session
        assert!(matches!(c.query("SELEC 1", None), Err(DbError::Query(_))));
        assert!(matches!(
            c.columns(&TableRef::new(Some(&home), Some("dbx_test"), "ghost")),
            Err(DbError::NotFound(_))
        ));
        assert!(matches!(
            c.schemas("dbx_no_such_database"),
            Err(DbError::NotFound(_))
        ));
        c.use_database(&home).unwrap();
        assert!(matches!(
            c.use_database("dbx_no_such_database"),
            Err(DbError::NotFound(_))
        ));
        assert_eq!(c.current_database().as_deref(), Some(home.as_str()));

        // read-only, per call and per connection
        let ro = ExecOptions::read_only();
        let count = |c: &mut Box<dyn Connection>| {
            c.query("SELECT count(*) FROM dbx_test.person", None)
                .unwrap()
                .rows[0][0]
                .clone()
        };
        assert!(
            c.execute_with("INSERT INTO dbx_test.person (name) VALUES ('x')", &ro)
                .is_err()
        );
        assert!(
            c.query_with("SELECT 1; DELETE FROM dbx_test.person", &ro)
                .is_err()
        );
        assert!(
            c.query_with(
                "WITH d AS (DELETE FROM dbx_test.person RETURNING *) SELECT * FROM d",
                &ro
            )
            .is_err()
        );
        assert_eq!(c.query_with("SELECT 1", &ro).unwrap().rows.len(), 1);
        c.execute("BEGIN").unwrap();
        assert!(matches!(
            c.query_with("SELECT 1", &ro),
            Err(DbError::ReadOnly(_))
        ));
        c.execute("ROLLBACK").unwrap();
        assert_eq!(count(&mut c), Value::Int(2));
        let mut ro_cfg = cfg.clone();
        ro_cfg.read_only = true;
        let mut rc = connect(&ro_cfg).unwrap();
        assert!(rc.is_read_only());
        assert!(rc.execute("DELETE FROM dbx_test.person").is_err());
        assert_eq!(count(&mut rc), Value::Int(2));

        // timeout and cancel
        let t = ExecOptions {
            timeout: Some(Duration::from_millis(200)),
            ..ExecOptions::default()
        };
        assert_eq!(
            c.query_with("SELECT pg_sleep(5)", &t),
            Err(DbError::Timeout)
        );
        assert_eq!(
            c.query_with(
                "SELECT pg_sleep(5)",
                &ExecOptions {
                    read_only: true,
                    ..t
                }
            ),
            Err(DbError::Timeout)
        );
        let h = c.cancel_handle().unwrap();
        let th = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            h.cancel().unwrap();
        });
        assert_eq!(
            c.query("SELECT pg_sleep(10)", None),
            Err(DbError::Cancelled)
        );
        th.join().unwrap();
        c.ping().unwrap();

        // plans
        let p = c
            .explain("SELECT * FROM dbx_test.person WHERE id > 0", false)
            .unwrap();
        assert_eq!(p.format, PlanFormat::Json);
        assert!(p.root.est_cost.is_some() && p.root.actual_rows.is_none());
        let p = c.explain("DELETE FROM dbx_test.person", true).unwrap();
        assert!(p.root.actual_ms.is_some());
        assert_eq!(count(&mut c), Value::Int(2));
        assert!(
            c.explain("SELECT 1; DELETE FROM dbx_test.person", false)
                .is_err()
        );

        // approximate counts (`None` before ANALYZE on 14+, `Some(0)` on older servers)
        c.execute("ANALYZE dbx_test.person").unwrap();
        let objs = c.objects(&home, Some("dbx_test")).unwrap();
        assert_eq!(objs[1].approx_rows, Some(2));
        assert_eq!(objs[2].approx_rows, None); // the view

        c.execute("DROP SCHEMA dbx_test CASCADE").unwrap();
    }
}
