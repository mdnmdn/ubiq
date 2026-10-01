//! SQL Server through `tiberius` (rustls), on a private current-thread tokio runtime that every
//! call blocks on. Nothing async leaves this file.
//!
//! - Auth is SQL login only; `integrated_auth` answers [`DbError::Unsupported`].
//! - `host\instance` goes through the SQL Browser service (UDP 1434) unless a port is given.
//! - TLS: `Disable` → no encryption, `Prefer` → encrypt if the server can (certificate not
//!   verified), `Require` → must encrypt (not verified, i.e. `TrustServerCertificate=true`),
//!   `VerifyFull` → must encrypt and verify against the system roots.
//! - Every database is reachable from the one session by three-part names, so `schemas`,
//!   `objects` and `columns` never switch databases.
//! - A `limit` stops reading, but the rest of the result is drained before the next call.
//! - Read-only is a **backstop**: SQL Server has no read-only transaction, so the statement runs
//!   inside `BEGIN TRAN` … `ROLLBACK` — it really executes, a `COMMIT` in the batch escapes the
//!   wrapper, and nothing stops a second statement (T-SQL needs no `;`). The parser
//!   ([`crate::sql`]) is the main guard on this engine. A call is refused when a transaction is
//!   already open. A read-only config also logs in with `ApplicationIntent=ReadOnly` (an
//!   availability group then routes to a readable secondary).
//! - Cancel and timeout send a TDS Attention (tiberius `cancel_query`), which stops the batch and
//!   keeps the connection usable; if the attention fails the call answers `Disconnected`.
//! - `approx_rows` is `sys.partitions.rows` of the heap or clustered index, summed over
//!   partitions.

use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{NaiveDate, NaiveTime, TimeDelta, Timelike};
use futures_util::TryStreamExt;
use futures_util::future::{Either, select};
use tiberius::{
    AuthMethod, Client, ColumnData, ColumnType, Config, EncryptionLevel, Row, SqlBrowser,
};
use tiberius::{QueryItem, error::Error as TdsError};
use tokio::net::TcpStream;
use tokio::runtime::Runtime;
use tokio::sync::Notify;
use tokio_util::compat::{Compat, TokioAsyncWriteCompatExt};

use super::{
    CancelHandle, ColumnMeta, Connection, DbError, DbObject, ExecOptions, ExecOutcome, ObjectKind,
    Plan, PlanFormat, Result, ResultSet, TableRef, log_statement, plan,
};
use crate::conn::{ConnectionConfig, DbKind, ParseError, SslMode};
use crate::value::Value;

type Tds = Client<Compat<TcpStream>>;

pub(super) fn open(cfg: &ConnectionConfig) -> Result<Box<dyn Connection>> {
    if cfg.integrated_auth {
        return Err(DbError::Unsupported(
            "SQL Server integrated (Windows) authentication is not supported; use a SQL login"
                .into(),
        ));
    }
    let user = cfg
        .user
        .as_deref()
        .filter(|u| !u.is_empty())
        .ok_or(DbError::Config(ParseError::Missing("user")))?;
    let mut config = Config::new();
    config.host(cfg.effective_host());
    if let Some(port) = cfg.effective_port() {
        config.port(port);
    }
    if let Some(instance) = &cfg.instance {
        config.instance_name(instance);
    }
    if let Some(db) = cfg.database.as_deref().filter(|d| !d.is_empty()) {
        config.database(db);
    }
    config.application_name("ubiq");
    config.readonly(cfg.read_only);
    config.authentication(AuthMethod::sql_server(
        user,
        cfg.password.as_deref().unwrap_or(""),
    ));
    match cfg.ssl {
        SslMode::Disable => config.encryption(EncryptionLevel::NotSupported),
        SslMode::Prefer => {
            config.encryption(EncryptionLevel::On);
            config.trust_cert();
        }
        SslMode::Require => {
            config.encryption(EncryptionLevel::Required);
            config.trust_cert();
        }
        SslMode::VerifyFull => config.encryption(EncryptionLevel::Required),
    }
    let secs = cfg
        .params
        .get("connect_timeout")
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(10);
    let named = cfg.instance.is_some() && cfg.port.is_none();

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| DbError::Connect(format!("cannot start the async runtime: {e}")))?;
    let client = rt
        .block_on(async {
            tokio::time::timeout(Duration::from_secs(secs), async {
                let tcp = if named {
                    TcpStream::connect_named(&config).await?
                } else {
                    TcpStream::connect(config.get_addr()).await?
                };
                tcp.set_nodelay(true)?;
                Client::connect(config, tcp.compat_write()).await
            })
            .await
        })
        .map_err(|_| DbError::Connect(format!("timed out after {secs}s")))?
        .map_err(|e| DbError::Connect(message(&e)))?;
    Ok(Box::new(MsSql {
        client,
        rt,
        current: cfg.database.clone().filter(|d| !d.is_empty()),
        read_only: cfg.read_only,
        cancel: Arc::new(Attention(Notify::new())),
    }))
}

struct MsSql {
    // the client first: it must drop while its runtime is still alive
    client: Tds,
    rt: Runtime,
    /// The database last selected (`connect`'s or `use_database`'s); `None` if neither named one
    /// (the login's default database is then in effect).
    current: Option<String>,
    read_only: bool,
    cancel: Arc<Attention>,
}

/// Wakes the running call, which then sends the TDS Attention itself (the client is borrowed by
/// the call, so nothing else can).
struct Attention(Notify);

impl CancelHandle for Attention {
    fn cancel(&self) -> Result<()> {
        // only a call already waiting is woken: no permit is left for the next one
        self.0.notify_waiters();
        Ok(())
    }
}

/// Resolves when the call should stop: `Cancelled` on the handle, `Timeout` past `timeout`.
async fn stop_signal(notify: &Notify, timeout: Option<Duration>) -> DbError {
    let woken = notify.notified();
    match timeout {
        Some(t) => match tokio::time::timeout(t, woken).await {
            Ok(()) => DbError::Cancelled,
            Err(_) => DbError::Timeout,
        },
        None => {
            woken.await;
            DbError::Cancelled
        }
    }
}

/// `interruptible!(client, notify, timeout, work)` — inside an async block: await `work` (a
/// future borrowing `client`) until it ends, or until a cancel / the timeout; then drop it and
/// send a TDS Attention so the server stops and the session stays usable. A macro, not a
/// function: `work` must release its borrow of the client before the attention can use it.
macro_rules! interruptible {
    ($client:expr, $notify:expr, $timeout:expr, $work:expr) => {{
        let outcome = {
            let work = std::pin::pin!($work);
            let stop = std::pin::pin!(stop_signal($notify, $timeout));
            match select(work, stop).await {
                Either::Left((done, _)) => Ok(done),
                Either::Right((why, _)) => Err(why),
            }
        };
        match outcome {
            Ok(done) => done.map_err(err),
            Err(why) => match $client.cancel_query().await {
                Ok(()) => Err(why),
                Err(e) => Err(DbError::Disconnected(format!(
                    "the cancel request failed: {}",
                    message(&e)
                ))),
            },
        }
    }};
}

/// The first result set of `sql`, at most `limit` rows.
async fn read_first(
    client: &mut Tds,
    sql: &str,
    limit: Option<usize>,
) -> std::result::Result<ResultSet, TdsError> {
    let mut stream = client.simple_query(sql).await?;
    let mut out = ResultSet::default();
    let mut have_columns = false;
    while let Some(item) = stream.try_next().await? {
        match item {
            QueryItem::Metadata(m) => {
                if have_columns {
                    break; // only the first result set
                }
                have_columns = true;
                out.columns = m.columns().iter().map(column_meta).collect();
            }
            QueryItem::Row(row) => {
                if limit.is_some_and(|l| out.rows.len() >= l) {
                    out.truncated = true;
                    break;
                }
                out.rows.push(row.into_iter().map(cell).collect());
            }
        }
    }
    Ok(out)
}

/// Every showplan document `sql` produces: rows of the result sets whose column is the
/// `Microsoft SQL Server 2005 XML Showplan` one. Other result sets are read and dropped.
async fn showplans(client: &mut Tds, sql: &str) -> std::result::Result<Vec<String>, TdsError> {
    let mut stream = client.simple_query(sql).await?;
    let mut out = Vec::new();
    let mut take = false;
    while let Some(item) = stream.try_next().await? {
        match item {
            QueryItem::Metadata(m) => {
                take = m
                    .columns()
                    .first()
                    .is_some_and(|c| c.name().contains("Showplan"));
            }
            QueryItem::Row(row) if take => {
                if let Some(Value::Text(x)) = row.into_iter().next().map(cell) {
                    out.push(x);
                }
            }
            QueryItem::Row(_) => {}
        }
    }
    Ok(out)
}

fn message(e: &TdsError) -> String {
    match e {
        TdsError::Server(t) => t.message().to_string(),
        e => e.to_string(),
    }
}

fn err(e: TdsError) -> DbError {
    match &e {
        // 911: Database '%.*ls' does not exist
        TdsError::Server(t) if t.code() == 911 => DbError::NotFound(t.message().to_string()),
        TdsError::Server(t) => DbError::Query(t.message().to_string()),
        TdsError::Io { .. } | TdsError::Tls(_) | TdsError::Protocol(_) => {
            DbError::Disconnected(e.to_string())
        }
        _ => DbError::Query(e.to_string()),
    }
}

/// `[name]` as an identifier.
fn br(s: &str) -> String {
    format!("[{}]", s.replace(']', "]]"))
}

/// `N'text'` as a string literal.
fn lit(s: &str) -> String {
    format!("N'{}'", s.replace('\'', "''"))
}

fn text(row: &Row, i: usize) -> Option<String> {
    row.try_get::<&str, _>(i).ok().flatten().map(str::to_string)
}

fn flag(row: &Row, i: usize) -> bool {
    row.try_get::<bool, _>(i).ok().flatten().unwrap_or(false)
}

impl MsSql {
    /// A catalog query: the first result set, rows as-is.
    fn rows(&mut self, sql: &str) -> Result<Vec<Row>> {
        let Self { client, rt, .. } = self;
        rt.block_on(async { client.simple_query(sql).await?.into_first_result().await })
            .map_err(err)
    }

    /// The synonym's base object, when `name` is one: `[db].[schema].[name]` of the target.
    fn synonym_target(&mut self, t: &TableRef, db: &str, schema: &str) -> Result<Option<TableRef>> {
        let rows = self.rows(&format!(
            "SELECT CAST(sy.base_object_name AS nvarchar(1035)) FROM {db}.sys.synonyms sy \
             JOIN {db}.sys.schemas s ON s.schema_id = sy.schema_id \
             WHERE s.name = {} AND sy.name = {}",
            lit(schema),
            lit(&t.name),
            db = br(db)
        ))?;
        Ok(rows
            .first()
            .and_then(|r| text(r, 0))
            .and_then(|base| parse_base_name(&base, db, schema)))
    }
}

impl Connection for MsSql {
    fn kind(&self) -> DbKind {
        DbKind::MsSql
    }

    fn ping(&mut self) -> Result<()> {
        self.rows("SELECT 1").map(|_| ())
    }

    fn databases(&mut self) -> Result<Vec<String>> {
        Ok(self
            .rows(
                "SELECT CAST(name AS nvarchar(128)) FROM sys.databases \
                 WHERE state = 0 AND HAS_DBACCESS(name) = 1 ORDER BY name",
            )?
            .iter()
            .filter_map(|r| text(r, 0))
            .collect())
    }

    fn schemas(&mut self, database: &str) -> Result<Vec<String>> {
        Ok(self
            .rows(&format!(
                "SELECT CAST(name AS nvarchar(128)) FROM {}.sys.schemas \
                 WHERE schema_id < 16384 AND name NOT IN (N'sys', N'INFORMATION_SCHEMA') \
                 ORDER BY name",
                br(database)
            ))?
            .iter()
            .filter_map(|r| text(r, 0))
            .collect())
    }

    fn objects(&mut self, database: &str, schema: Option<&str>) -> Result<Vec<DbObject>> {
        let db = br(database);
        let only = |alias: &str| match schema {
            Some(s) => format!(" AND {alias}.name = {}", lit(s)),
            None => String::new(),
        };
        let rows = self.rows(&format!(
            "SELECT CAST(o.name AS nvarchar(128)), CAST(CASE o.type WHEN 'U' THEN N'table' \
                    ELSE N'view' END AS nvarchar(10)), CAST(s.name AS nvarchar(128)), \
                    CAST(ep.value AS nvarchar(4000)), CAST(NULL AS nvarchar(1035)), \
                    CAST(CASE WHEN o.type = 'U' THEN pr.n END AS bigint) \
             FROM {db}.sys.objects o JOIN {db}.sys.schemas s ON s.schema_id = o.schema_id \
             LEFT JOIN {db}.sys.extended_properties ep ON ep.class = 1 \
                    AND ep.major_id = o.object_id AND ep.minor_id = 0 \
                    AND ep.name = N'MS_Description' \
             LEFT JOIN (SELECT object_id, SUM(rows) AS n FROM {db}.sys.partitions \
                        WHERE index_id IN (0, 1) GROUP BY object_id) pr \
                    ON pr.object_id = o.object_id \
             WHERE o.type IN ('U', 'V') AND o.is_ms_shipped = 0{} \
             UNION ALL \
             SELECT CAST(sy.name AS nvarchar(128)), N'synonym', CAST(s.name AS nvarchar(128)), \
                    CAST(NULL AS nvarchar(4000)), CAST(sy.base_object_name AS nvarchar(1035)), \
                    CAST(NULL AS bigint) \
             FROM {db}.sys.synonyms sy JOIN {db}.sys.schemas s ON s.schema_id = sy.schema_id \
             WHERE 1 = 1{} \
             ORDER BY 3, 1",
            only("s"),
            only("s"),
        ))?;
        Ok(rows
            .iter()
            .filter_map(|r| {
                let name = text(r, 0)?;
                let kind = match text(r, 1)?.as_str() {
                    "table" => ObjectKind::Table,
                    "synonym" => ObjectKind::Synonym,
                    _ => ObjectKind::View,
                };
                let sch = text(r, 2)?;
                let target = text(r, 4).and_then(|b| parse_base_name(&b, database, &sch));
                Some(DbObject {
                    kind,
                    name,
                    schema: Some(sch),
                    database: Some(database.to_string()),
                    target,
                    comment: text(r, 3).filter(|c| !c.is_empty()),
                    approx_rows: r
                        .try_get::<i64, _>(5)
                        .ok()
                        .flatten()
                        .and_then(|n| u64::try_from(n).ok()),
                })
            })
            .collect())
    }

    fn columns(&mut self, table: &TableRef) -> Result<Vec<ColumnMeta>> {
        let mut table = table.clone();
        // a synonym stands for its target (and a target may itself be a synonym)
        for _ in 0..4 {
            let db = match table.database.clone() {
                Some(d) => d,
                None => text(&self.rows("SELECT CAST(DB_NAME() AS nvarchar(128))")?[0], 0)
                    .ok_or_else(|| DbError::NotFound("no database selected".into()))?,
            };
            let schema = table.schema.clone().unwrap_or_else(|| "dbo".into());
            table = TableRef::new(Some(&db), Some(&schema), &table.name);
            match self.synonym_target(&table, &db, &schema)? {
                Some(target) => table = target,
                None => break,
            }
        }
        let (db, schema) = (
            table.database.as_deref().unwrap_or(""),
            table.schema.as_deref().unwrap_or("dbo"),
        );
        let qdb = br(db);
        let rows = self.rows(&format!(
            "SELECT CAST(c.name AS nvarchar(128)), \
              CAST(CASE \
               WHEN ty.name IN (N'varchar', N'char', N'varbinary', N'binary') THEN ty.name + N'(' \
                + CASE WHEN c.max_length = -1 THEN N'max' ELSE CAST(c.max_length AS nvarchar(10)) END + N')' \
               WHEN ty.name IN (N'nvarchar', N'nchar') THEN ty.name + N'(' \
                + CASE WHEN c.max_length = -1 THEN N'max' ELSE CAST(c.max_length / 2 AS nvarchar(10)) END + N')' \
               WHEN ty.name IN (N'decimal', N'numeric') THEN ty.name + N'(' \
                + CAST(c.precision AS nvarchar(5)) + N',' + CAST(c.scale AS nvarchar(5)) + N')' \
               WHEN ty.name IN (N'datetime2', N'datetimeoffset', N'time') THEN ty.name + N'(' \
                + CAST(c.scale AS nvarchar(5)) + N')' \
               ELSE ty.name END AS nvarchar(200)), \
              CAST(c.is_nullable AS bit), CAST(c.is_identity AS bit), CAST(c.is_computed AS bit), \
              CAST(dc.definition AS nvarchar(4000)), \
              CAST(CASE WHEN EXISTS (SELECT 1 FROM {qdb}.sys.indexes i \
                 JOIN {qdb}.sys.index_columns ic ON ic.object_id = i.object_id AND ic.index_id = i.index_id \
                 WHERE i.is_primary_key = 1 AND ic.object_id = c.object_id AND ic.column_id = c.column_id) \
                THEN 1 ELSE 0 END AS bit) \
             FROM {qdb}.sys.columns c \
             JOIN {qdb}.sys.types ty ON ty.user_type_id = c.user_type_id \
             LEFT JOIN {qdb}.sys.default_constraints dc ON dc.object_id = c.default_object_id \
             WHERE c.object_id = OBJECT_ID({}) ORDER BY c.column_id",
            lit(&format!("{qdb}.{}.{}", br(schema), br(&table.name)))
        ))?;
        if rows.is_empty() {
            return Err(DbError::NotFound(format!("table {table}")));
        }
        Ok(rows
            .iter()
            .map(|r| {
                let mut c = ColumnMeta::new(
                    DbKind::MsSql,
                    &text(r, 0).unwrap_or_default(),
                    &text(r, 1).unwrap_or_default(),
                );
                c.nullable = flag(r, 2);
                c.auto_increment = flag(r, 3);
                c.computed = flag(r, 4);
                c.default = text(r, 5);
                c.is_pk = flag(r, 6);
                c
            })
            .collect())
    }

    fn use_database(&mut self, db: &str) -> Result<()> {
        self.rows(&format!("USE {}", br(db)))?;
        self.current = Some(db.to_string());
        Ok(())
    }

    fn current_database(&self) -> Option<String> {
        self.current.clone()
    }

    fn query_with(&mut self, sql: &str, opts: &ExecOptions) -> Result<ResultSet> {
        let (limit, timeout) = (opts.row_limit, opts.timeout);
        self.guarded("query", sql, opts, |me| {
            let Self {
                client, rt, cancel, ..
            } = me;
            rt.block_on(async {
                interruptible!(client, &cancel.0, timeout, read_first(client, sql, limit))
            })
        })
    }

    fn execute_with(&mut self, sql: &str, opts: &ExecOptions) -> Result<ExecOutcome> {
        let timeout = opts.timeout;
        self.guarded("execute", sql, opts, |me| {
            let Self {
                client, rt, cancel, ..
            } = me;
            let done = rt.block_on(async {
                interruptible!(client, &cancel.0, timeout, client.execute(sql, &[]))
            })?;
            Ok(ExecOutcome {
                rows_affected: done.total(),
            })
        })
    }

    fn is_read_only(&self) -> bool {
        self.read_only
    }

    fn cancel_handle(&self) -> Option<Arc<dyn CancelHandle>> {
        Some(self.cancel.clone())
    }

    fn explain(&mut self, sql: &str, analyze: bool) -> Result<Plan> {
        let started = Instant::now();
        let result = (|| {
            let xml = if analyze {
                // STATISTICS XML executes the batch: inside a transaction (or, in the caller's
                // open one, a savepoint) that is rolled back
                let open = self.trancount()? > 0;
                self.rows(if open {
                    "SAVE TRANSACTION dbx_explain"
                } else {
                    "BEGIN TRAN"
                })?;
                let r = self
                    .rows("SET STATISTICS XML ON")
                    .and_then(|_| self.plans_of(sql));
                let _ = self.rows("SET STATISTICS XML OFF");
                let _ = self.rows(if open {
                    "ROLLBACK TRANSACTION dbx_explain"
                } else {
                    "IF @@TRANCOUNT > 0 ROLLBACK TRAN"
                });
                r?
            } else {
                // SHOWPLAN_XML must be alone in its batch; the statement is planned, not run
                self.rows("SET SHOWPLAN_XML ON")?;
                let r = self.plans_of(sql);
                let _ = self.rows("SET SHOWPLAN_XML OFF");
                r?
            };
            let raw = xml.join("\n");
            let root = plan::from_mssql_xml(&raw).map_err(DbError::Query)?;
            Ok(Plan {
                root,
                raw,
                format: PlanFormat::Xml,
            })
        })();
        let what = if analyze {
            "explain analyze"
        } else {
            "explain"
        };
        log_statement(DbKind::MsSql, what, sql, self.read_only, started, result)
    }
}

impl MsSql {
    fn trancount(&mut self) -> Result<i32> {
        Ok(self
            .rows("SELECT @@TRANCOUNT")?
            .first()
            .and_then(|r| r.try_get::<i32, _>(0).ok().flatten())
            .unwrap_or(0))
    }

    fn plans_of(&mut self, sql: &str) -> Result<Vec<String>> {
        let Self { client, rt, .. } = self;
        rt.block_on(showplans(client, sql)).map_err(err)
    }

    /// Run `body` under the read-only backstop, and log it.
    fn guarded<T>(
        &mut self,
        what: &str,
        sql: &str,
        opts: &ExecOptions,
        body: impl FnOnce(&mut Self) -> Result<T>,
    ) -> Result<T> {
        let ro = opts.read_only || self.read_only;
        let started = Instant::now();
        let result = (|| {
            if ro {
                if self.trancount()? > 0 {
                    return Err(DbError::ReadOnly(
                        "a transaction is open on this connection; commit or roll it back first"
                            .into(),
                    ));
                }
                self.rows("BEGIN TRAN")?;
            }
            let r = body(self);
            if ro {
                let _ = self.rows("IF @@TRANCOUNT > 0 ROLLBACK TRAN");
            }
            r
        })();
        log_statement(DbKind::MsSql, what, sql, ro, started, result)
    }
}

/// `[server].[db].[schema].[name]` (any suffix of it, brackets optional) → the target relation.
/// `None` for a linked-server (four-part) name, which cannot be browsed from here.
fn parse_base_name(base: &str, db: &str, schema: &str) -> Option<TableRef> {
    let mut parts: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut chars = base.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '[' if !quoted => quoted = true,
            ']' if quoted && chars.peek() == Some(&']') => {
                cur.push(']');
                chars.next();
            }
            ']' if quoted => quoted = false,
            '.' if !quoted => parts.push(std::mem::take(&mut cur)),
            c => cur.push(c),
        }
    }
    parts.push(cur);
    let mut it = parts.into_iter().rev();
    let name = it.next()?;
    let sch = it.next();
    let dbn = it.next();
    if it.next().is_some() {
        return None;
    }
    Some(TableRef {
        database: Some(dbn.unwrap_or_else(|| db.to_string())),
        schema: Some(
            sch.filter(|s| !s.is_empty())
                .unwrap_or_else(|| schema.to_string()),
        ),
        name,
    })
}

/// A result column, typed from the TDS type token (no length or precision on the wire here).
fn column_meta(col: &tiberius::Column) -> ColumnMeta {
    use ColumnType::*;
    let native = match col.column_type() {
        Bit | Bitn => "bit",
        Int1 => "tinyint",
        Int2 => "smallint",
        Int4 | Intn => "int",
        Int8 => "bigint",
        Float4 => "real",
        Float8 | Floatn => "float",
        Money => "money",
        Money4 => "smallmoney",
        Decimaln | Numericn => "decimal",
        Datetime | Datetimen => "datetime",
        Datetime4 => "smalldatetime",
        Daten => "date",
        Timen => "time",
        Datetime2 => "datetime2",
        DatetimeOffsetn => "datetimeoffset",
        Guid => "uniqueidentifier",
        BigVarBin | BigBinary => "varbinary",
        Image => "image",
        BigVarChar => "varchar",
        BigChar => "char",
        NVarchar => "nvarchar",
        NChar => "nchar",
        Text => "text",
        NText => "ntext",
        Xml => "xml",
        Udt => "udt",
        SSVariant => "sql_variant",
        Null => "",
    };
    ColumnMeta::new(DbKind::MsSql, col.name(), native)
}

fn cell(d: ColumnData<'static>) -> Value {
    fn opt<T>(v: Option<T>, f: impl FnOnce(T) -> Value) -> Value {
        v.map_or(Value::Null, f)
    }
    match d {
        ColumnData::U8(v) => opt(v, |n| Value::Int(i64::from(n))),
        ColumnData::I16(v) => opt(v, |n| Value::Int(i64::from(n))),
        ColumnData::I32(v) => opt(v, |n| Value::Int(i64::from(n))),
        ColumnData::I64(v) => opt(v, Value::Int),
        ColumnData::F32(v) => opt(v, |x| Value::Float(f64::from(x))),
        ColumnData::F64(v) => opt(v, Value::Float),
        ColumnData::Bit(v) => opt(v, Value::Bool),
        ColumnData::String(v) => opt(v, |s| Value::Text(s.into_owned())),
        ColumnData::Guid(v) => opt(v, |u| Value::Uuid(u.to_string())),
        ColumnData::Binary(v) => opt(v, |b| Value::Bytes(b.into_owned())),
        ColumnData::Numeric(v) => opt(v, |n| Value::Decimal(decimal(n.value(), n.scale()))),
        ColumnData::Xml(v) => opt(v, |x| Value::Text(x.into_owned().into_string())),
        ColumnData::DateTime(v) => opt(v, |t| {
            let date = base_1900().checked_add_signed(TimeDelta::days(i64::from(t.days())));
            let frag = i64::from(t.seconds_fragments()); // 1/300 s
            let (secs, ms) = (frag / 300, (frag % 300 * 1000 + 150) / 300);
            date.map_or_else(
                || Value::Text(format!("{} days, {frag}/300 s", t.days())),
                |d| {
                    Value::DateTime(stamp(
                        d,
                        (secs / 3600) as u32,
                        (secs / 60 % 60) as u32,
                        (secs % 60) as u32,
                        ms as u32 * 1_000_000,
                    ))
                },
            )
        }),
        ColumnData::SmallDateTime(v) => opt(v, |t| {
            let date = base_1900().checked_add_signed(TimeDelta::days(i64::from(t.days())));
            let mins = u32::from(t.seconds_fragments()); // minutes since midnight
            date.map_or_else(
                || Value::Text(format!("{} days, {mins} min", t.days())),
                |d| Value::DateTime(stamp(d, mins / 60, mins % 60, 0, 0)),
            )
        }),
        ColumnData::Date(v) => opt(v, |d| {
            date_of(d.days()).map_or_else(
                || Value::Text(format!("day {}", d.days())),
                |d| Value::Date(d.format("%Y-%m-%d").to_string()),
            )
        }),
        ColumnData::Time(v) => opt(v, |t| Value::Time(time_of(t.increments(), t.scale()))),
        ColumnData::DateTime2(v) => opt(v, |t| {
            date_of(t.date().days()).map_or_else(
                || Value::Text(format!("day {}", t.date().days())),
                |d| {
                    Value::DateTime(format!(
                        "{} {}",
                        d.format("%Y-%m-%d"),
                        time_of(t.time().increments(), t.time().scale())
                    ))
                },
            )
        }),
        ColumnData::DateTimeOffset(v) => opt(v, |t| {
            // the stored date and time are UTC; the offset (minutes) says how to show them
            let dt = t.datetime2();
            let offset = i64::from(t.offset());
            let local = date_of(dt.date().days()).map(|d| {
                let nanos = nanos_of(dt.time().increments(), dt.time().scale());
                d.and_time(NaiveTime::default())
                    + TimeDelta::nanoseconds(nanos)
                    + TimeDelta::minutes(offset)
            });
            local.map_or_else(
                || Value::Text(format!("day {}", dt.date().days())),
                |l| {
                    let sign = if offset < 0 { '-' } else { '+' };
                    Value::DateTimeTz(format!(
                        "{} {}{sign}{:02}:{:02}",
                        l.date().format("%Y-%m-%d"),
                        time_of(
                            nanos_of_day(&l),
                            9 // nanosecond increments
                        ),
                        offset.abs() / 60,
                        offset.abs() % 60
                    ))
                },
            )
        }),
    }
}

fn base_1900() -> NaiveDate {
    NaiveDate::from_ymd_opt(1900, 1, 1).expect("valid date")
}

/// Days since 0001-01-01 → date.
fn date_of(days: u32) -> Option<NaiveDate> {
    NaiveDate::from_num_days_from_ce_opt(i32::try_from(days).ok()? + 1)
}

fn stamp(d: NaiveDate, h: u32, m: u32, s: u32, nanos: u32) -> String {
    let frac = fraction(u64::from(nanos), 9);
    format!("{} {h:02}:{m:02}:{s:02}{frac}", d.format("%Y-%m-%d"))
}

/// `increments` of 10^-`scale` seconds since midnight → nanoseconds.
fn nanos_of(increments: u64, scale: u8) -> i64 {
    (increments * 10u64.pow(9 - u32::from(scale.min(9)))) as i64
}

fn nanos_of_day(t: &chrono::NaiveDateTime) -> u64 {
    u64::from(t.num_seconds_from_midnight()) * 1_000_000_000 + u64::from(t.nanosecond())
}

/// `HH:MM:SS[.fraction]` for `increments` of 10^-`scale` seconds since midnight.
fn time_of(increments: u64, scale: u8) -> String {
    let scale = u32::from(scale.min(9));
    let unit = 10u64.pow(scale);
    let secs = increments / unit;
    let frac = fraction(increments % unit, scale);
    format!(
        "{:02}:{:02}:{:02}{frac}",
        secs / 3600 % 24,
        secs / 60 % 60,
        secs % 60
    )
}

/// `.123` for `v` of 10^-`digits`, trailing zeros trimmed; empty when zero.
fn fraction(v: u64, digits: u32) -> String {
    if v == 0 {
        return String::new();
    }
    let s = format!("{v:0width$}", width = digits as usize);
    format!(".{}", s.trim_end_matches('0'))
}

/// A scaled integer as plain decimal text (`-1250`, scale 2 → `-12.50`).
fn decimal(value: i128, scale: u8) -> String {
    let neg = value < 0;
    let digits = value.unsigned_abs().to_string();
    let scale = usize::from(scale);
    let padded = format!("{digits:0>width$}", width = scale + 1);
    let (int, frac) = padded.split_at(padded.len() - scale);
    let sign = if neg { "-" } else { "" };
    if scale == 0 {
        format!("{sign}{int}")
    } else {
        format!("{sign}{int}.{frac}")
    }
}

#[cfg(test)]
mod tests {
    use chrono::Datelike;

    use super::*;
    use crate::conn::parse_connection_string;
    use crate::driver::connect;

    #[test]
    fn decimals() {
        assert_eq!(decimal(-1250, 2), "-12.50");
        assert_eq!(decimal(5, 3), "0.005");
        assert_eq!(decimal(42, 0), "42");
        assert_eq!(decimal(0, 4), "0.0000");
    }

    #[test]
    fn times_and_dates() {
        assert_eq!(time_of(36_000_000_000, 7), "01:00:00");
        assert_eq!(time_of(1_234_567_890, 7), "00:02:03.456789");
        assert_eq!(date_of(0).unwrap().to_string(), "0001-01-01");
        assert_eq!(date_of(738_000).unwrap().year(), 2021);
        let dt = ColumnData::DateTime(Some(tiberius::time::DateTime::new(0, 300 * 3661 + 2)));
        assert_eq!(cell(dt), Value::DateTime("1900-01-01 01:01:01.007".into()));
        let dto = tiberius::time::DateTimeOffset::new(
            tiberius::time::DateTime2::new(
                tiberius::time::Date::new(738_000),
                tiberius::time::Time::new(23 * 3600 * 10_000_000, 7),
            ),
            90,
        );
        // 23:00 UTC at +01:30 is 00:30 the next day
        let Value::DateTimeTz(s) = cell(ColumnData::DateTimeOffset(Some(dto))) else {
            panic!("not a DateTimeTz")
        };
        assert!(s.ends_with("00:30:00+01:30"), "{s}");
    }

    #[test]
    fn base_names() {
        let t = parse_base_name("[srv].[Sales].[dbo].[Orders]", "x", "y");
        assert_eq!(t, None);
        let t = parse_base_name("[Sales].[dbo].[Or]]ders]", "x", "y").unwrap();
        assert_eq!(t, TableRef::new(Some("Sales"), Some("dbo"), "Or]ders"));
        let t = parse_base_name("[dbo].[T]", "Cur", "y").unwrap();
        assert_eq!(t, TableRef::new(Some("Cur"), Some("dbo"), "T"));
        let t = parse_base_name("T", "Cur", "sch").unwrap();
        assert_eq!(t, TableRef::new(Some("Cur"), Some("sch"), "T"));
    }

    #[test]
    fn integrated_auth_is_unsupported() {
        let mut cfg = ConnectionConfig::new(DbKind::MsSql);
        cfg.integrated_auth = true;
        assert!(matches!(connect(&cfg), Err(DbError::Unsupported(_))));
    }

    /// Needs a server: `DBX_TEST_MSSQL_URL=sqlserver://sa:pw@localhost/master?trustServerCertificate=true`.
    #[test]
    #[ignore = "needs DBX_TEST_MSSQL_URL"]
    fn live_server() {
        let url = std::env::var("DBX_TEST_MSSQL_URL").expect("DBX_TEST_MSSQL_URL");
        let cfg = parse_connection_string(&url).unwrap();
        let mut c = connect(&cfg).unwrap();
        c.ping().unwrap();
        let dbs = c.databases().unwrap();
        assert!(dbs.iter().any(|d| d == "master"), "{dbs:?}");
        let schemas = c.schemas("master").unwrap();
        assert!(schemas.iter().any(|s| s == "dbo"), "{schemas:?}");

        let t = format!("dbx_t_{}", std::process::id());
        let _ = c.execute(&format!("DROP SYNONYM dbo.{t}_syn"));
        let _ = c.execute(&format!("DROP VIEW dbo.{t}_v"));
        let _ = c.execute(&format!("DROP TABLE dbo.{t}"));
        c.execute(&format!(
            "CREATE TABLE dbo.{t} (id int IDENTITY PRIMARY KEY, name nvarchar(40) NOT NULL, \
             price decimal(10,2) NULL DEFAULT 0, born date NULL, seen datetime2(3) NULL, \
             at datetimeoffset NULL, token uniqueidentifier NULL, flag bit NULL, \
             twice AS (id * 2))"
        ))
        .unwrap();
        c.execute(&format!(
            "CREATE VIEW dbo.{t}_v AS SELECT id, name FROM dbo.{t}"
        ))
        .unwrap();
        c.execute(&format!("CREATE SYNONYM dbo.{t}_syn FOR dbo.{t}"))
            .unwrap();
        let ins = c
            .execute(&format!(
                "INSERT INTO dbo.{t} (name, price, born, seen, at, token, flag) VALUES \
                 (N'Ada', 12.5, '1815-12-10', '2020-01-02 03:04:05.678', \
                 '2020-01-02 03:04:05 +01:30', '6F9619FF-8B86-D011-B42D-00C04FC964FF', 1), \
                 (N'Bob', NULL, NULL, NULL, NULL, NULL, NULL)"
            ))
            .unwrap();
        assert_eq!(ins.rows_affected, 2);

        let objs = c.objects("master", Some("dbo")).unwrap();
        let find = |n: &str| objs.iter().find(|o| o.name == n).cloned();
        assert_eq!(find(&t).unwrap().kind, ObjectKind::Table);
        assert_eq!(find(&format!("{t}_v")).unwrap().kind, ObjectKind::View);
        let syn = find(&format!("{t}_syn")).unwrap();
        assert_eq!(syn.kind, ObjectKind::Synonym);
        assert_eq!(syn.target.unwrap().name, t);

        let cols = c
            .columns(&TableRef::new(
                Some("master"),
                Some("dbo"),
                &format!("{t}_syn"),
            ))
            .unwrap();
        assert_eq!(cols[0].name, "id");
        assert!(cols[0].is_pk && cols[0].auto_increment && !cols[0].nullable);
        assert_eq!(cols[1].native_type, "nvarchar(40)");
        assert!(cols[2].default.is_some());
        assert!(cols.last().unwrap().computed);

        let rs = c
            .query(
                &format!(
                    "SELECT id, name, price, born, seen, token, flag FROM dbo.{t} ORDER BY id"
                ),
                None,
            )
            .unwrap();
        assert_eq!(rs.rows.len(), 2);
        assert_eq!(rs.rows[0][1], Value::Text("Ada".into()));
        assert_eq!(rs.rows[0][2], Value::Decimal("12.50".into()));
        assert_eq!(rs.rows[0][3], Value::Date("1815-12-10".into()));
        assert_eq!(
            rs.rows[0][4],
            Value::DateTime("2020-01-02 03:04:05.678".into())
        );
        assert_eq!(
            rs.rows[0][5],
            Value::Uuid("6f9619ff-8b86-d011-b42d-00c04fc964ff".into())
        );
        assert_eq!(rs.rows[0][6], Value::Bool(true));
        assert_eq!(rs.rows[1][2], Value::Null);
        let rs = c
            .query(&format!("SELECT id FROM dbo.{t}"), Some(1))
            .unwrap();
        assert!(rs.truncated && rs.rows.len() == 1);
        // the session survives a half-read result
        c.ping().unwrap();
        assert!(matches!(c.query("SELEC 1", None), Err(DbError::Query(_))));

        // read-only backstop: the write runs and is rolled back
        let count = |c: &mut Box<dyn Connection>| {
            c.query(&format!("SELECT count(*) FROM dbo.{t}"), None)
                .unwrap()
                .rows[0][0]
                .clone()
        };
        let ro = ExecOptions::read_only();
        c.execute_with(&format!("DELETE FROM dbo.{t}"), &ro)
            .unwrap();
        assert_eq!(count(&mut c), Value::Int(2));
        c.execute("BEGIN TRAN").unwrap();
        assert!(matches!(
            c.query_with("SELECT 1", &ro),
            Err(DbError::ReadOnly(_))
        ));
        c.execute("ROLLBACK TRAN").unwrap();

        // timeout and cancel through an Attention; the session stays usable
        let to = ExecOptions {
            timeout: Some(Duration::from_millis(300)),
            ..ExecOptions::default()
        };
        assert_eq!(
            c.execute_with("WAITFOR DELAY '00:00:05'", &to),
            Err(DbError::Timeout)
        );
        c.ping().unwrap();
        let h = c.cancel_handle().unwrap();
        let th = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            h.cancel().unwrap();
        });
        assert_eq!(
            c.query("WAITFOR DELAY '00:00:10'; SELECT 1", None),
            Err(DbError::Cancelled)
        );
        th.join().unwrap();
        c.ping().unwrap();

        // plans
        let p = c
            .explain(&format!("SELECT * FROM dbo.{t} WHERE id > 0"), false)
            .unwrap();
        assert_eq!(p.format, PlanFormat::Xml);
        assert!(p.root.est_rows.is_some(), "{p:?}");
        let p = c.explain(&format!("DELETE FROM dbo.{t}"), true).unwrap();
        assert!(p.root.count() >= 1);
        assert_eq!(count(&mut c), Value::Int(2));

        // approximate counts
        let objs = c.objects("master", Some("dbo")).unwrap();
        let find = |n: &str| objs.iter().find(|o| o.name == n).cloned().unwrap();
        assert_eq!(find(&t).approx_rows, Some(2));
        assert_eq!(find(&format!("{t}_v")).approx_rows, None);

        c.execute(&format!("DROP SYNONYM dbo.{t}_syn")).unwrap();
        c.execute(&format!("DROP VIEW dbo.{t}_v")).unwrap();
        c.execute(&format!("DROP TABLE dbo.{t}")).unwrap();

        c.use_database("master").unwrap();
        assert_eq!(c.current_database().as_deref(), Some("master"));
        let rs = c.query("SELECT DB_NAME()", None).unwrap();
        assert_eq!(rs.rows[0][0], Value::Text("master".into()));
    }
}
