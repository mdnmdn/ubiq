//! MySQL / MariaDB through the sync `mysql` crate (rustls, no OpenSSL), text protocol.
//!
//! A MySQL "database" is a schema, so there is no schema level: `databases` lists the schemas and
//! `objects(db, None)` their tables and views. The session is not tied to a database; every
//! catalog call takes the name, and a [`TableRef`] without `database` falls back to the
//! session's current one (`SELECT DATABASE()`).
//!
//! TLS: `Disable` is plain TCP, `Prefer` tries TLS (certificate not checked) and falls back to
//! plain, `Require` is TLS without verification, `VerifyFull` verifies chain and host name.
//! `params["connect_timeout"]` is in seconds (default 10).
//!
//! Cost to know: a `limit` stops *reading*, but dropping a half-read result makes the crate
//! drain the rest of it from the wire. Keep the `LIMIT` in the SQL the builder writes.
//!
//! Read-only: the statement is prepared first (`COM_STMT_PREPARE` refuses a second statement,
//! and a statement MySQL cannot prepare is refused with it), then runs as
//! `START TRANSACTION READ ONLY` → stmt → `ROLLBACK`. The `START` implicitly commits a
//! transaction the caller left open. A read-only connection also runs
//! `SET SESSION TRANSACTION READ ONLY` at login. Cancel is `KILL QUERY <connection id>` from a
//! short side connection; a timeout is a client-side deadline that cancels the same way (it
//! covers every statement, unlike `MAX_EXECUTION_TIME`). `approx_rows` is
//! `information_schema.TABLES.TABLE_ROWS` — InnoDB's is a sampled estimate.

use std::sync::Arc;
use std::time::{Duration, Instant};

use mysql::consts::{ColumnFlags, ColumnType};
use mysql::prelude::Queryable;
use mysql::{Column, Conn, Opts, OptsBuilder, SslOpts, Value as My};

use super::{
    CancelHandle, ColumnMeta, Connection, DbError, DbObject, ExecOptions, ExecOutcome, ObjectKind,
    Plan, PlanFormat, Result, ResultSet, Structure, StructureScope, TableRef, Watchdog, canonical,
    database_or_current, log_statement, plan, strip_terminator, structure,
};
use crate::conn::{ConnectionConfig, DbKind, SslMode};
use crate::edit::quote_ident;
use crate::value::{DataType, Value};

pub(super) fn open(cfg: &ConnectionConfig) -> Result<Box<dyn Connection>> {
    let timeout = cfg
        .params
        .get("connect_timeout")
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(10);
    let init: Vec<String> = if cfg.read_only {
        vec!["SET SESSION TRANSACTION READ ONLY".into()]
    } else {
        Vec::new()
    };
    let base = OptsBuilder::new()
        .init(init)
        .ip_or_hostname(Some(cfg.effective_host()))
        .tcp_port(cfg.effective_port().unwrap_or(3306))
        .user(cfg.user.as_deref())
        .pass(cfg.password.as_deref())
        .db_name(cfg.database.as_deref().filter(|d| !d.is_empty()))
        .prefer_socket(false)
        .tcp_connect_timeout(Some(Duration::from_secs(timeout)));
    let tls = |verify: bool| {
        SslOpts::default()
            .with_danger_accept_invalid_certs(!verify)
            .with_danger_skip_domain_validation(!verify)
    };
    // keep the options that worked: the cancel handle's side connection uses them
    let dial = |b: OptsBuilder| {
        let opts = Opts::from(b);
        Conn::new(opts.clone()).map(|c| (c, opts))
    };
    let (conn, opts) = match cfg.ssl {
        SslMode::Disable => dial(base),
        SslMode::Require => dial(base.ssl_opts(Some(tls(false)))),
        SslMode::VerifyFull => dial(base.ssl_opts(Some(tls(true)))),
        SslMode::Prefer => dial(base.clone().ssl_opts(Some(tls(false)))).or_else(|_| dial(base)),
    }
    .map_err(|e| DbError::Connect(e.to_string()))?;
    let cancel = Arc::new(KillQuery {
        opts,
        id: conn.connection_id(),
    });
    Ok(Box::new(MySql {
        conn,
        current: cfg.database.clone().filter(|d| !d.is_empty()),
        read_only: cfg.read_only,
        cancel,
    }))
}

struct MySql {
    conn: Conn,
    /// The database last selected (`connect`'s or `use_database`'s); `None` if neither named one.
    current: Option<String>,
    read_only: bool,
    cancel: Arc<KillQuery>,
}

/// `KILL QUERY <id>` over a side connection.
struct KillQuery {
    opts: Opts,
    id: u32,
}

impl CancelHandle for KillQuery {
    fn cancel(&self) -> Result<()> {
        let mut side = Conn::new(self.opts.clone()).map_err(|e| DbError::Connect(e.to_string()))?;
        side.query_drop(format!("KILL QUERY {}", self.id))
            .map_err(err)
    }
}

fn err(e: mysql::Error) -> DbError {
    match &e {
        // ER_QUERY_INTERRUPTED (KILL QUERY)
        mysql::Error::MySqlError(m) if m.code == 1317 => DbError::Cancelled,
        // ER_QUERY_TIMEOUT (MAX_EXECUTION_TIME), MariaDB ER_STATEMENT_TIMEOUT
        mysql::Error::MySqlError(m) if m.code == 3024 || m.code == 1969 => DbError::Timeout,
        _ if e.is_connectivity_error() => DbError::Disconnected(e.to_string()),
        _ => DbError::Query(e.to_string()),
    }
}

/// A text-protocol cell as text, for plan rows.
fn plain(v: &My) -> Option<String> {
    match v {
        My::NULL => None,
        My::Bytes(b) => Some(String::from_utf8_lossy(b).into_owned()),
        My::Int(n) => Some(n.to_string()),
        My::UInt(n) => Some(n.to_string()),
        My::Float(x) => Some(x.to_string()),
        My::Double(x) => Some(x.to_string()),
        other => Some(format!("{other:?}")),
    }
}

/// Schemas the server owns; never shown.
const SYSTEM_SCHEMAS: &str = "('information_schema', 'mysql', 'performance_schema', 'sys')";

impl Connection for MySql {
    fn kind(&self) -> DbKind {
        DbKind::MySql
    }

    fn ping(&mut self) -> Result<()> {
        self.conn.query_drop("SELECT 1").map_err(err)
    }

    fn databases(&mut self) -> Result<Vec<String>> {
        self.conn
            .query(format!(
                "SELECT schema_name FROM information_schema.schemata \
                 WHERE schema_name NOT IN {SYSTEM_SCHEMAS} ORDER BY schema_name"
            ))
            .map_err(err)
    }

    fn schemas(&mut self, _database: &str) -> Result<Vec<String>> {
        Ok(Vec::new())
    }

    fn objects(&mut self, database: &str, _schema: Option<&str>) -> Result<Vec<DbObject>> {
        let rows: Vec<(String, String, Option<String>, Option<u64>)> = self
            .conn
            .exec(
                "SELECT table_name, table_type, table_comment, table_rows \
                 FROM information_schema.tables \
                 WHERE table_schema = ? ORDER BY table_name",
                (database,),
            )
            .map_err(err)?;
        if rows.is_empty() && !self.database_exists(database)? {
            return Err(DbError::NotFound(format!("database {database}")));
        }
        Ok(rows
            .into_iter()
            .filter(|(_, ty, _, _)| ty != "SYSTEM VIEW")
            .map(|(name, ty, comment, approx_rows)| {
                let kind = if ty == "VIEW" {
                    ObjectKind::View
                } else {
                    // BASE TABLE, and MariaDB's SYSTEM VERSIONED / SEQUENCE
                    ObjectKind::Table
                };
                DbObject {
                    kind,
                    name,
                    schema: None,
                    database: Some(database.to_string()),
                    target: None,
                    // MySQL prints "VIEW" as a view's comment
                    comment: comment.filter(|c| !c.is_empty() && c != "VIEW"),
                    approx_rows: approx_rows.filter(|_| kind == ObjectKind::Table),
                }
            })
            .collect())
    }

    fn columns(&mut self, table: &TableRef) -> Result<Vec<ColumnMeta>> {
        let db = match &table.database {
            Some(d) => d.clone(),
            None => self
                .conn
                .query_first::<Option<String>, _>("SELECT DATABASE()")
                .map_err(err)?
                .flatten()
                .ok_or_else(|| DbError::NotFound("no database selected".into()))?,
        };
        // name, type, nullable, key, default, extra
        let rows: Vec<(String, String, String, String, Option<String>, String)> = self
            .conn
            .exec(
                "SELECT column_name, column_type, is_nullable, column_key, column_default, extra \
                 FROM information_schema.columns \
                 WHERE table_schema = ? AND table_name = ? ORDER BY ordinal_position",
                (&db, &table.name),
            )
            .map_err(err)?;
        if rows.is_empty() {
            return Err(DbError::NotFound(format!("table {db}.{}", table.name)));
        }
        Ok(rows
            .into_iter()
            .map(|(name, ty, nullable, key, default, extra)| {
                let extra = extra.to_ascii_lowercase();
                let mut c = ColumnMeta::new(DbKind::MySql, &name, &ty);
                c.nullable = nullable.eq_ignore_ascii_case("YES");
                c.is_pk = key == "PRI";
                // MariaDB spells "no default" as the string NULL
                c.default = default.filter(|d| !d.eq_ignore_ascii_case("null"));
                c.auto_increment = extra.contains("auto_increment");
                // not DEFAULT_GENERATED, which is an expression default
                c.computed = extra.contains("virtual generated")
                    || extra.contains("stored generated")
                    || extra.contains("persistent");
                c
            })
            .collect())
    }

    fn use_database(&mut self, db: &str) -> Result<()> {
        self.conn
            .query_drop(format!("USE {}", quote_ident(DbKind::MySql, db)))
            .map_err(err)?;
        self.current = Some(db.to_string());
        Ok(())
    }

    fn current_database(&self) -> Option<String> {
        self.current.clone()
    }

    fn query_with(&mut self, sql: &str, opts: &ExecOptions) -> Result<ResultSet> {
        let limit = opts.row_limit;
        self.guarded("query", sql, opts, |conn| {
            let mut result = conn.query_iter(sql).map_err(err)?;
            let cols: Vec<Column> = result.columns().as_ref().to_vec();
            if cols.is_empty() {
                return Ok(ResultSet::default());
            }
            let mut out = ResultSet {
                columns: cols.iter().map(column_meta).collect(),
                ..ResultSet::default()
            };
            for row in result.by_ref() {
                if limit.is_some_and(|l| out.rows.len() >= l) {
                    out.truncated = true;
                    break;
                }
                let row = row.map_err(err)?;
                out.rows.push(
                    row.unwrap()
                        .into_iter()
                        .zip(&out.columns)
                        .map(|(v, meta)| cell(v, meta))
                        .collect(),
                );
            }
            Ok(out)
        })
    }

    fn execute_with(&mut self, sql: &str, opts: &ExecOptions) -> Result<ExecOutcome> {
        self.guarded("execute", sql, opts, |conn| {
            conn.query_drop(sql).map_err(err)?;
            Ok(ExecOutcome {
                rows_affected: conn.affected_rows(),
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
        let body = strip_terminator(sql);
        let result = (|| {
            self.one_statement(body)?;
            if analyze {
                // EXPLAIN ANALYZE executes: in a transaction of our own, rolled back
                let begin = if self.read_only {
                    "START TRANSACTION READ ONLY"
                } else {
                    "START TRANSACTION"
                };
                self.conn.query_drop(begin).map_err(err)?;
                let r = self.explain_rows(&format!("EXPLAIN ANALYZE {body}"));
                let _ = self.conn.query_drop("ROLLBACK");
                return match r {
                    Ok(out) => Ok(plan_of(out)),
                    Err(DbError::Query(m)) => Err(DbError::Unsupported(format!(
                        "EXPLAIN ANALYZE needs MySQL 8.0.18 or later: {m}"
                    ))),
                    Err(e) => Err(e),
                };
            }
            match self.explain_rows(&format!("EXPLAIN FORMAT=TREE {body}")) {
                Ok(out) => Ok(plan_of(out)),
                // MariaDB, MySQL before 8.0.16: the tabular form
                Err(DbError::Query(_)) => {
                    self.explain_rows(&format!("EXPLAIN {body}")).map(plan_of)
                }
                Err(e) => Err(e),
            }
        })();
        let what = if analyze {
            "explain analyze"
        } else {
            "explain"
        };
        log_statement(DbKind::MySql, what, sql, self.read_only, started, result)
    }

    fn structure(&mut self, database: &str, scope: &StructureScope) -> Result<Structure> {
        let db = database_or_current(self.current.clone(), database)?;
        if !self.database_exists(&db)? {
            return Err(DbError::NotFound(format!("database {db}")));
        }
        structure::introspect(DbKind::MySql, &db, scope, |sql| {
            let rows: Vec<mysql::Row> = self.conn.query(sql).map_err(err)?;
            Ok(rows
                .into_iter()
                .map(|r| r.unwrap().iter().map(plain).collect())
                .collect())
        })
    }
}

/// Column names and text cells of an `EXPLAIN` statement.
type ExplainRows = (Vec<String>, Vec<Vec<Option<String>>>);

/// `EXPLAIN` output as a plan: one `EXPLAIN` column is the tree text, anything else the table.
fn plan_of((columns, rows): ExplainRows) -> Plan {
    if columns.len() == 1 && columns[0].eq_ignore_ascii_case("EXPLAIN") {
        let raw = rows
            .into_iter()
            .filter_map(|r| r.into_iter().next().flatten())
            .collect::<Vec<_>>()
            .join("\n");
        return Plan {
            root: plan::from_mysql_tree(&raw),
            raw,
            format: PlanFormat::Text,
        };
    }
    let mut raw = columns.join("\t");
    for r in &rows {
        raw.push('\n');
        let cells: Vec<&str> = r.iter().map(|c| c.as_deref().unwrap_or("NULL")).collect();
        raw.push_str(&cells.join("\t"));
    }
    Plan {
        root: plan::from_mysql_table(&columns, &rows),
        raw,
        format: PlanFormat::Table,
    }
}

impl MySql {
    /// `COM_STMT_PREPARE` takes exactly one statement: `SELECT 1; DELETE …` is refused here.
    fn one_statement(&mut self, sql: &str) -> Result<()> {
        let stmt = self.conn.prep(sql).map_err(err)?;
        let _ = self.conn.close(stmt);
        Ok(())
    }

    /// Run `body` under the read-only guard and the deadline, and log it.
    fn guarded<T>(
        &mut self,
        what: &str,
        sql: &str,
        opts: &ExecOptions,
        body: impl FnOnce(&mut Conn) -> Result<T>,
    ) -> Result<T> {
        let ro = opts.read_only || self.read_only;
        let started = Instant::now();
        let result = (|| {
            if ro {
                self.one_statement(sql)?;
                self.conn
                    .query_drop("START TRANSACTION READ ONLY")
                    .map_err(err)?;
            }
            let cancel: Arc<dyn CancelHandle> = self.cancel.clone();
            let wd = Watchdog::arm(opts.timeout, Some(cancel));
            let r = Watchdog::finish(wd, body(&mut self.conn));
            if ro {
                let _ = self.conn.query_drop("ROLLBACK");
            }
            r
        })();
        log_statement(DbKind::MySql, what, sql, ro, started, result)
    }

    fn explain_rows(&mut self, sql: &str) -> Result<ExplainRows> {
        let mut result = self.conn.query_iter(sql).map_err(err)?;
        let columns: Vec<String> = result
            .columns()
            .as_ref()
            .iter()
            .map(|c| c.name_str().into_owned())
            .collect();
        let mut rows = Vec::new();
        for row in result.by_ref() {
            rows.push(row.map_err(err)?.unwrap().iter().map(plain).collect());
        }
        Ok((columns, rows))
    }
    fn database_exists(&mut self, database: &str) -> Result<bool> {
        let n: Option<u64> = self
            .conn
            .exec_first(
                "SELECT count(*) FROM information_schema.schemata WHERE schema_name = ?",
                (database,),
            )
            .map_err(err)?;
        Ok(n.unwrap_or(0) > 0)
    }
}

/// The result column's type as DDL would spell it, so [`ColumnMeta::new`] maps it.
fn column_meta(col: &Column) -> ColumnMeta {
    use ColumnType::*;
    let unsigned = col.flags().contains(ColumnFlags::UNSIGNED_FLAG);
    let binary = col.character_set() == 63;
    let flags = col.flags();
    let (base, int) = match col.column_type() {
        MYSQL_TYPE_TINY if col.column_length() == 1 && !unsigned => ("tinyint(1)", false),
        MYSQL_TYPE_TINY => ("tinyint", true),
        MYSQL_TYPE_SHORT => ("smallint", true),
        MYSQL_TYPE_INT24 => ("mediumint", true),
        MYSQL_TYPE_LONG => ("int", true),
        MYSQL_TYPE_LONGLONG => ("bigint", true),
        MYSQL_TYPE_FLOAT => ("float", false),
        MYSQL_TYPE_DOUBLE => ("double", false),
        MYSQL_TYPE_DECIMAL | MYSQL_TYPE_NEWDECIMAL => ("decimal", false),
        MYSQL_TYPE_DATE | MYSQL_TYPE_NEWDATE => ("date", false),
        MYSQL_TYPE_TIME | MYSQL_TYPE_TIME2 => ("time", false),
        MYSQL_TYPE_DATETIME | MYSQL_TYPE_DATETIME2 => ("datetime", false),
        MYSQL_TYPE_TIMESTAMP | MYSQL_TYPE_TIMESTAMP2 => ("timestamp", false),
        MYSQL_TYPE_JSON => ("json", false),
        MYSQL_TYPE_BIT => ("bit(1)", false),
        MYSQL_TYPE_VARCHAR | MYSQL_TYPE_VAR_STRING if binary => ("varbinary", false),
        MYSQL_TYPE_VARCHAR | MYSQL_TYPE_VAR_STRING => ("varchar", false),
        MYSQL_TYPE_STRING if flags.contains(ColumnFlags::ENUM_FLAG) => ("enum", false),
        MYSQL_TYPE_STRING if flags.contains(ColumnFlags::SET_FLAG) => ("set", false),
        MYSQL_TYPE_STRING if binary => ("binary", false),
        MYSQL_TYPE_STRING => ("char", false),
        MYSQL_TYPE_ENUM => ("enum", false),
        MYSQL_TYPE_SET => ("set", false),
        MYSQL_TYPE_TINY_BLOB | MYSQL_TYPE_MEDIUM_BLOB | MYSQL_TYPE_LONG_BLOB | MYSQL_TYPE_BLOB
            if binary =>
        {
            ("blob", false)
        }
        MYSQL_TYPE_TINY_BLOB => ("tinytext", false),
        MYSQL_TYPE_MEDIUM_BLOB => ("mediumtext", false),
        MYSQL_TYPE_LONG_BLOB => ("longtext", false),
        MYSQL_TYPE_BLOB => ("text", false),
        MYSQL_TYPE_YEAR => ("year", false),
        MYSQL_TYPE_GEOMETRY => ("geometry", false),
        _ => ("", false),
    };
    let native = if int && unsigned {
        format!("{base} unsigned")
    } else {
        base.to_string()
    };
    ColumnMeta::new(DbKind::MySql, &col.name_str(), &native)
}

fn cell(v: My, meta: &ColumnMeta) -> Value {
    match v {
        My::NULL => Value::Null,
        My::Bytes(b) => from_text(b, meta),
        My::Int(n) => Value::Int(n),
        My::UInt(n) => i64::try_from(n).map_or_else(|_| Value::Decimal(n.to_string()), Value::Int),
        My::Float(x) => Value::Float(f64::from(x)),
        My::Double(x) => Value::Float(x),
        // Only the binary protocol sends these; the text protocol we use sends bytes.
        My::Date(y, mo, d, h, mi, s, us) => {
            let date = format!("{y:04}-{mo:02}-{d:02}");
            if meta.data_type == DataType::Date {
                canonical(date, meta)
            } else {
                canonical(format!("{date} {}", clock(h, mi, s, us)), meta)
            }
        }
        My::Time(neg, days, h, mi, s, us) => Value::Text(format!(
            "{}{}",
            if neg { "-" } else { "" },
            clock(u32::from(h) + days * 24, mi, s, us)
        )),
    }
}

fn clock(h: impl std::fmt::Display, mi: u8, s: u8, us: u32) -> String {
    if us == 0 {
        format!("{h:02}:{mi:02}:{s:02}")
    } else {
        format!("{h:02}:{mi:02}:{s:02}.{us:06}")
    }
}

/// A text-protocol cell: every value arrives as bytes, the column says what they mean.
fn from_text(b: Vec<u8>, meta: &ColumnMeta) -> Value {
    if meta.data_type == DataType::Bytes {
        return Value::Bytes(b);
    }
    // BIT(1) is one raw byte, 0 or 1.
    if meta.data_type == DataType::Bool && b.len() == 1 && b[0] <= 1 {
        return Value::Bool(b[0] == 1);
    }
    let s = match String::from_utf8(b) {
        Ok(s) => s,
        // geometry and other binary payloads
        Err(e) => return Value::Bytes(e.into_bytes()),
    };
    match &meta.data_type {
        DataType::Bool => match s.as_str() {
            "0" => Value::Bool(false),
            "1" => Value::Bool(true),
            _ => Value::Text(s),
        },
        DataType::Int { .. } => s
            .parse::<i64>()
            .map(Value::Int)
            .or_else(|_| s.parse::<u64>().map(|n| Value::Decimal(n.to_string())))
            .unwrap_or(Value::Text(s)),
        DataType::Float => s.parse::<f64>().map(Value::Float).unwrap_or(Value::Text(s)),
        DataType::Decimal { .. } => Value::Decimal(s),
        _ => canonical(s, meta),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conn::parse_connection_string;
    use crate::driver::connect;

    #[test]
    fn text_cells_follow_the_column_type() {
        let col = |native: &str| ColumnMeta::new(DbKind::MySql, "c", native);
        let go = |native: &str, v: &str| cell(My::Bytes(v.as_bytes().to_vec()), &col(native));
        assert_eq!(go("int", "-5"), Value::Int(-5));
        assert_eq!(
            go("bigint unsigned", "18446744073709551615"),
            Value::Decimal("18446744073709551615".into())
        );
        assert_eq!(go("tinyint(1)", "1"), Value::Bool(true));
        assert_eq!(cell(My::Bytes(vec![0]), &col("bit(1)")), Value::Bool(false));
        assert_eq!(go("decimal(10,2)", "12.50"), Value::Decimal("12.50".into()));
        assert_eq!(go("double", "1.5"), Value::Float(1.5));
        assert_eq!(go("date", "2024-02-29"), Value::Date("2024-02-29".into()));
        assert_eq!(go("date", "0000-00-00"), Value::Text("0000-00-00".into()));
        assert_eq!(
            go("datetime", "2024-02-29 01:02:03.5"),
            Value::DateTime("2024-02-29 01:02:03.5".into())
        );
        assert_eq!(go("json", "{\"a\": 1}"), Value::Json("{\"a\": 1}".into()));
        assert_eq!(go("enum('a','b')", "a"), Value::Text("a".into()));
        assert_eq!(
            cell(My::Bytes(vec![0, 255]), &col("blob")),
            Value::Bytes(vec![0, 255])
        );
        assert_eq!(cell(My::NULL, &col("int")), Value::Null);
    }

    /// Needs a server: `DBX_TEST_MYSQL_URL=mysql://root:pw@localhost/mysql`. Creates and drops
    /// a schema named `dbx_test`.
    #[test]
    #[ignore = "needs DBX_TEST_MYSQL_URL"]
    fn live_server() {
        let url = std::env::var("DBX_TEST_MYSQL_URL").expect("DBX_TEST_MYSQL_URL");
        let cfg = parse_connection_string(&url).unwrap();
        let mut c = connect(&cfg).unwrap();
        c.ping().unwrap();
        c.execute("DROP DATABASE IF EXISTS dbx_test").unwrap();
        c.execute("CREATE DATABASE dbx_test").unwrap();
        c.execute(
            "CREATE TABLE dbx_test.person (id INT UNSIGNED AUTO_INCREMENT PRIMARY KEY, \
             name VARCHAR(40) NOT NULL, price DECIMAL(10,2) DEFAULT 0, born DATE, \
             seen DATETIME(3), flag TINYINT(1), kind ENUM('a','b'), doc JSON, photo BLOB, \
             twice INT AS (id * 2) VIRTUAL) COMMENT 'people'",
        )
        .unwrap();
        c.execute("CREATE VIEW dbx_test.v AS SELECT id, name FROM dbx_test.person")
            .unwrap();
        let ins = c
            .execute(
                "INSERT INTO dbx_test.person (name, price, born, seen, flag, kind, doc, photo) \
                 VALUES ('Ada', 12.5, '1815-12-10', '2020-01-02 03:04:05.678', 1, 'b', \
                 '{\"a\": 1}', x'00ff'), ('Bob', NULL, NULL, NULL, NULL, NULL, NULL, NULL)",
            )
            .unwrap();
        assert_eq!(ins.rows_affected, 2);

        let dbs = c.databases().unwrap();
        assert!(dbs.iter().any(|d| d == "dbx_test") && !dbs.iter().any(|d| d == "mysql"));
        assert!(c.schemas("dbx_test").unwrap().is_empty());
        let objs = c.objects("dbx_test", None).unwrap();
        assert_eq!(objs.len(), 2);
        assert_eq!(objs[0].kind, ObjectKind::Table);
        assert_eq!(objs[0].comment.as_deref(), Some("people"));
        assert_eq!(objs[1].kind, ObjectKind::View);
        assert_eq!(objs[1].comment, None);

        let cols = c.columns(&objs[0].table_ref()).unwrap();
        assert!(cols[0].is_pk && cols[0].auto_increment && !cols[0].nullable);
        assert_eq!(cols[0].native_type, "int unsigned");
        assert_eq!(cols[1].native_type, "varchar(40)");
        assert!(cols.last().unwrap().computed);

        let scope = crate::dbml::StructureScope::default();
        let dbml = crate::dbml::to_dbml(&c.structure("dbx_test", &scope).unwrap());
        for want in [
            "Enum person_kind_enum {",
            "kind person_kind_enum",
            "id \"int unsigned\" [pk, increment]",
            "Note: 'people'",
        ] {
            assert!(dbml.contains(want), "{want} in {dbml}");
        }

        let rs = c
            .query(
                "SELECT id, name, price, born, seen, flag, kind, doc, photo \
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
        assert_eq!(rs.rows[0][5], Value::Bool(true));
        assert_eq!(rs.rows[0][6], Value::Text("b".into()));
        assert_eq!(rs.rows[0][8], Value::Bytes(vec![0, 255]));
        assert_eq!(rs.rows[1][2], Value::Null);
        let rs = c.query("SELECT id FROM dbx_test.person", Some(1)).unwrap();
        assert!(rs.truncated && rs.rows.len() == 1);
        c.ping().unwrap();
        assert!(matches!(c.query("SELEC 1", None), Err(DbError::Query(_))));
        assert!(matches!(
            c.columns(&TableRef::new(Some("dbx_test"), None, "ghost")),
            Err(DbError::NotFound(_))
        ));
        c.use_database("dbx_test").unwrap();
        assert_eq!(c.current_database().as_deref(), Some("dbx_test"));
        assert_eq!(
            c.query("SELECT id FROM person", None).unwrap().rows.len(),
            2
        );

        // read-only, per call and per connection
        let ro = ExecOptions::read_only();
        assert!(
            c.execute_with("INSERT INTO person (name) VALUES ('x')", &ro)
                .is_err()
        );
        assert!(c.query_with("SELECT 1; DELETE FROM person", &ro).is_err());
        assert_eq!(c.query_with("SELECT 1", &ro).unwrap().rows.len(), 1);
        assert_eq!(
            c.query("SELECT id FROM person", None).unwrap().rows.len(),
            2
        );
        let mut ro_cfg = cfg.clone();
        ro_cfg.read_only = true;
        let mut rc = connect(&ro_cfg).unwrap();
        assert!(rc.is_read_only());
        assert!(rc.execute("DELETE FROM dbx_test.person").is_err());

        // timeout and cancel
        let t = ExecOptions {
            timeout: Some(Duration::from_millis(300)),
            ..ExecOptions::default()
        };
        assert_eq!(c.query_with("SELECT SLEEP(5)", &t), Err(DbError::Timeout));
        let h = c.cancel_handle().unwrap();
        let th = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            h.cancel().unwrap();
        });
        // SLEEP() answers 1 when KILL QUERY interrupts it, rather than failing
        let r = c.query("SELECT SLEEP(10), (SELECT count(*) FROM person)", None);
        assert!(matches!(&r, Err(DbError::Cancelled)) || r.is_ok(), "{r:?}");
        th.join().unwrap();
        c.ping().unwrap();

        // plans
        let p = c
            .explain("SELECT * FROM person WHERE id > 0", false)
            .unwrap();
        assert!(p.root.count() >= 1, "{p:?}");
        assert!(c.explain("SELECT 1; DELETE FROM person", false).is_err());
        if let Ok(p) = c.explain("DELETE FROM person", true) {
            assert!(p.root.count() >= 1);
        }
        assert_eq!(
            c.query("SELECT id FROM person", None).unwrap().rows.len(),
            2
        );

        // approximate counts
        c.execute("ANALYZE TABLE person").unwrap();
        let objs = c.objects("dbx_test", None).unwrap();
        assert!(objs[0].approx_rows.is_some());
        assert_eq!(objs[1].approx_rows, None);

        c.execute("DROP DATABASE dbx_test").unwrap();
    }
}
