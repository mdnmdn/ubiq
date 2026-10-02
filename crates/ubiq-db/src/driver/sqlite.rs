//! SQLite through `rusqlite` (bundled). Opened read-write, never created: a missing file is a
//! connect error. A [`ConnectionConfig::read_only`] config (`?mode=ro`, `readonly=true`, …) opens
//! `SQLITE_OPEN_READ_ONLY`.
//!
//! Databases are `main` plus every `ATTACH`ed one; there is no schema level. SQLite is dynamically
//! typed, so a cell is converted by its storage class first and its declared column type second
//! (an `INTEGER` in a `BOOLEAN` column is a `Bool`, text in a `DATE` column is a `Date` only if it
//! validates, otherwise it stays `Text`).
//!
//! Read-only calls set `PRAGMA query_only = ON` for the call and refuse a prepared statement that
//! `sqlite3_stmt_readonly` says may write ([`DbError::ReadOnly`]). Cancel is `sqlite3_interrupt`;
//! a timeout is a deadline thread that interrupts. `approx_rows` comes from `sqlite_stat1`, so it
//! is `None` until `ANALYZE` has run.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use rusqlite::{Connection as Db, InterruptHandle, OpenFlags, Statement, types::ValueRef};

use super::{
    CancelHandle, ColumnMeta, Connection, DbError, DbObject, ExecOptions, ExecOutcome, ObjectKind,
    Plan, PlanFormat, Result, ResultSet, Structure, StructureScope, TableRef, Watchdog, canonical,
    database_or_current, log_statement, plan, strip_terminator, structure,
};
use crate::conn::{ConnectionConfig, DbKind, ParseError};
use crate::value::{DataType, Value};

pub(super) fn open(cfg: &ConnectionConfig) -> Result<Box<dyn Connection>> {
    let path = cfg
        .path
        .as_ref()
        .ok_or(DbError::Config(ParseError::Missing("path")))?;
    // `params["mode"]` for configs saved before `read_only` existed
    let read_only = cfg.read_only || cfg.params.get("mode").is_some_and(|m| m == "ro");
    let access = if read_only {
        OpenFlags::SQLITE_OPEN_READ_ONLY
    } else {
        OpenFlags::SQLITE_OPEN_READ_WRITE
    };
    let flags = access | OpenFlags::SQLITE_OPEN_URI | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let connect_err = |e: rusqlite::Error| DbError::Connect(format!("{}: {e}", path.display()));
    let db = Db::open_with_flags(path, flags).map_err(connect_err)?;
    db.busy_timeout(Duration::from_secs(5))
        .map_err(connect_err)?;
    // Opening is lazy: read the schema so "not a database" fails here, not on the first click.
    db.query_row("SELECT count(*) FROM sqlite_master", [], |_| Ok(()))
        .map_err(connect_err)?;
    if read_only {
        db.pragma_update(None, "query_only", true)
            .map_err(connect_err)?;
    }
    let cancel = Arc::new(Interrupt(db.get_interrupt_handle()));
    Ok(Box::new(Sqlite {
        db,
        current: "main".to_string(),
        read_only,
        cancel,
    }))
}

/// Create a new, empty SQLite database file at `path`. The one place a file is ever created —
/// [`open`] never does. Refuses an existing path (never overwrites) and a missing parent directory.
/// The header is written (a throw-away table, dropped), so the file is a valid database rather than
/// zero bytes; nothing else is in it.
pub fn create_sqlite_database(path: &std::path::Path) -> Result<()> {
    let fail = |msg: String| DbError::Query(format!("{}: {msg}", path.display()));
    if path.exists() {
        return Err(fail("the file already exists".into()));
    }
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty())
        && !dir.is_dir()
    {
        return Err(fail(format!("directory {} does not exist", dir.display())));
    }
    let flags = OpenFlags::SQLITE_OPEN_CREATE | OpenFlags::SQLITE_OPEN_READ_WRITE;
    let made = Db::open_with_flags(path, flags)
        .and_then(|db| db.execute_batch("CREATE TABLE _t(x); DROP TABLE _t;"));
    if let Err(e) = made {
        let _ = std::fs::remove_file(path);
        return Err(fail(e.to_string()));
    }
    Ok(())
}

struct Sqlite {
    db: Db,
    /// What `use_database` last accepted. Statements are routed by their own `db.` qualifier, so
    /// this is bookkeeping only.
    current: String,
    /// Opened `SQLITE_OPEN_READ_ONLY` (and `query_only` for good).
    read_only: bool,
    cancel: Arc<Interrupt>,
}

/// `sqlite3_interrupt` from any thread.
struct Interrupt(InterruptHandle);

impl CancelHandle for Interrupt {
    fn cancel(&self) -> Result<()> {
        self.0.interrupt();
        Ok(())
    }
}

fn query_err(e: rusqlite::Error) -> DbError {
    match e {
        rusqlite::Error::SqliteFailure(f, _)
            if f.code == rusqlite::ErrorCode::OperationInterrupted =>
        {
            DbError::Cancelled
        }
        rusqlite::Error::SqliteFailure(f, _)
            if matches!(
                f.code,
                rusqlite::ErrorCode::DatabaseCorrupt | rusqlite::ErrorCode::NotADatabase
            ) =>
        {
            DbError::Disconnected(e.to_string())
        }
        e => DbError::Query(e.to_string()),
    }
}

/// `"name"` as an identifier.
fn quote_ident(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

/// `'text'` as a string literal.
fn quote_lit(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

impl Sqlite {
    fn database_names(&mut self) -> Result<Vec<String>> {
        let mut stmt = self.db.prepare("PRAGMA database_list").map_err(query_err)?;
        let mut names = stmt
            .query_map([], |r| r.get::<_, String>(1))
            .map_err(query_err)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(query_err)?;
        names.retain(|n| n != "temp");
        names.sort();
        Ok(names)
    }

    /// Table → approximate rows from `database`'s `sqlite_stat1` (the first number of each row
    /// is the table's or the index's row count); empty when `ANALYZE` never ran. Best effort.
    fn approx_rows(&self, database: &str) -> HashMap<String, u64> {
        let mut out = HashMap::new();
        let db = quote_ident(database);
        let has_stats = self
            .db
            .query_row(
                &format!(
                    "SELECT count(*) FROM {db}.sqlite_master \
                     WHERE type = 'table' AND name = 'sqlite_stat1'"
                ),
                [],
                |r| r.get::<_, i64>(0),
            )
            .is_ok_and(|n| n > 0);
        if !has_stats {
            return out;
        }
        let Ok(mut stmt) = self
            .db
            .prepare(&format!("SELECT tbl, stat FROM {db}.sqlite_stat1"))
        else {
            return out;
        };
        let Ok(rows) = stmt.query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
        }) else {
            return out;
        };
        for (tbl, stat) in rows.flatten() {
            if let Some(n) = stat
                .as_deref()
                .and_then(|s| s.split_whitespace().next())
                .and_then(|n| n.parse::<u64>().ok())
            {
                let e = out.entry(tbl).or_insert(n);
                *e = (*e).max(n);
            }
        }
        out
    }

    /// Prepare and run one statement under `opts`: `query_only` and the `stmt_readonly` check
    /// when read-only, the deadline when timed, the statement log always.
    fn run<T>(
        &mut self,
        what: &str,
        sql: &str,
        opts: &ExecOptions,
        body: impl FnOnce(&Db, &mut Statement<'_>) -> Result<T>,
    ) -> Result<T> {
        let ro = opts.read_only || self.read_only;
        let started = Instant::now();
        // a read-only connection keeps `query_only` on for good
        let toggle = ro && !self.read_only;
        let result = (|| {
            if toggle {
                self.db
                    .pragma_update(None, "query_only", true)
                    .map_err(query_err)?;
            }
            let mut stmt = self.db.prepare(sql).map_err(query_err)?;
            if ro && !stmt.readonly() {
                return Err(DbError::ReadOnly(
                    "the statement may write to the database".into(),
                ));
            }
            let cancel: Arc<dyn CancelHandle> = self.cancel.clone();
            let wd = Watchdog::arm(opts.timeout, Some(cancel));
            let r = body(&self.db, &mut stmt);
            Watchdog::finish(wd, r)
        })();
        if toggle {
            let _ = self.db.pragma_update(None, "query_only", false);
        }
        log_statement(DbKind::Sqlite, what, sql, ro, started, result)
    }
}

impl Connection for Sqlite {
    fn kind(&self) -> DbKind {
        DbKind::Sqlite
    }

    fn ping(&mut self) -> Result<()> {
        self.db
            .query_row("SELECT 1", [], |_| Ok(()))
            .map_err(query_err)
    }

    fn databases(&mut self) -> Result<Vec<String>> {
        self.database_names()
    }

    fn schemas(&mut self, _database: &str) -> Result<Vec<String>> {
        Ok(Vec::new())
    }

    fn objects(&mut self, database: &str, _schema: Option<&str>) -> Result<Vec<DbObject>> {
        if !self.database_names()?.iter().any(|d| d == database) {
            return Err(DbError::NotFound(format!("database {database}")));
        }
        let sql = format!(
            "SELECT name, type FROM {}.sqlite_master \
             WHERE type IN ('table', 'view') AND name NOT LIKE 'sqlite\\_%' ESCAPE '\\' \
             ORDER BY name",
            quote_ident(database)
        );
        let mut stmt = self.db.prepare(&sql).map_err(query_err)?;
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            .map_err(query_err)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(query_err)?;
        drop(stmt);
        let counts = self.approx_rows(database);
        Ok(rows
            .into_iter()
            .map(|(name, ty)| {
                let kind = if ty == "view" {
                    ObjectKind::View
                } else {
                    ObjectKind::Table
                };
                DbObject {
                    approx_rows: (kind == ObjectKind::Table)
                        .then(|| counts.get(&name).copied())
                        .flatten(),
                    kind,
                    name,
                    schema: None,
                    database: Some(database.to_string()),
                    target: None,
                    comment: None,
                }
            })
            .collect())
    }

    fn columns(&mut self, table: &TableRef) -> Result<Vec<ColumnMeta>> {
        let db = table.database.as_deref().unwrap_or("main");
        // table_xinfo (unlike table_info) also lists generated columns, flagged in `hidden`.
        let sql = format!(
            "PRAGMA {}.table_xinfo({})",
            quote_ident(db),
            quote_lit(&table.name)
        );
        let mut stmt = self.db.prepare(&sql).map_err(query_err)?;
        // cid, name, type, notnull, dflt_value, pk, hidden
        let raw = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, i64>(3)? != 0,
                    r.get::<_, Option<String>>(4)?,
                    r.get::<_, i64>(5)?,
                    r.get::<_, i64>(6)?,
                ))
            })
            .map_err(query_err)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(query_err)?;
        // `hidden` 1 is a virtual-table column nobody can select.
        let raw: Vec<_> = raw.into_iter().filter(|c| c.5 != 1).collect();
        if raw.is_empty() {
            return Err(DbError::NotFound(format!("table {table}")));
        }
        let pk_count = raw.iter().filter(|c| c.4 > 0).count();
        Ok(raw
            .into_iter()
            .map(|(name, ty, notnull, default, pk, hidden)| {
                let mut c = ColumnMeta::new(DbKind::Sqlite, &name, &ty);
                c.is_pk = pk > 0;
                // The one rowid alias: a lone `INTEGER PRIMARY KEY`.
                c.auto_increment = pk_count == 1 && pk > 0 && ty.eq_ignore_ascii_case("integer");
                // SQLite lets a non-INTEGER key column hold NULL; editing treats keys as required.
                c.nullable = !notnull && !c.is_pk;
                c.default = default;
                c.computed = hidden >= 2;
                c
            })
            .collect())
    }

    fn use_database(&mut self, db: &str) -> Result<()> {
        if !self.database_names()?.iter().any(|d| d == db) {
            return Err(DbError::NotFound(format!("database {db}")));
        }
        self.current = db.to_string();
        Ok(())
    }

    fn current_database(&self) -> Option<String> {
        Some(self.current.clone())
    }

    fn query_with(&mut self, sql: &str, opts: &ExecOptions) -> Result<ResultSet> {
        let limit = opts.row_limit;
        self.run("query", sql, opts, |_, stmt| {
            if stmt.column_count() == 0 {
                stmt.execute([]).map_err(query_err)?;
                return Ok(ResultSet::default());
            }
            let columns: Vec<ColumnMeta> = stmt
                .columns()
                .iter()
                .map(|c| ColumnMeta::new(DbKind::Sqlite, c.name(), c.decl_type().unwrap_or("")))
                .collect();
            let mut out = ResultSet {
                columns,
                ..ResultSet::default()
            };
            let mut rows = stmt.query([]).map_err(query_err)?;
            while let Some(row) = rows.next().map_err(query_err)? {
                if limit.is_some_and(|l| out.rows.len() >= l) {
                    out.truncated = true;
                    break;
                }
                let cells = out
                    .columns
                    .iter()
                    .enumerate()
                    .map(|(i, meta)| Ok(cell(row.get_ref(i)?, meta)))
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .map_err(query_err)?;
                out.rows.push(cells);
            }
            Ok(out)
        })
    }

    fn execute_with(&mut self, sql: &str, opts: &ExecOptions) -> Result<ExecOutcome> {
        self.run("execute", sql, opts, |db, stmt| {
            let before = db.total_changes();
            let n = stmt.execute([]).map_err(query_err)?;
            // `changes()` is stale after DDL; a statement that changed nothing reports 0.
            let rows_affected = if db.total_changes() == before {
                0
            } else {
                n as u64
            };
            Ok(ExecOutcome { rows_affected })
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
            let mut stmt = self
                .db
                .prepare(&format!("EXPLAIN QUERY PLAN {body}"))
                .map_err(query_err)?;
            // id, parent, notused, detail
            let rows = stmt
                .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get(1)?, r.get(3)?)))
                .map_err(query_err)?
                .collect::<rusqlite::Result<Vec<(i64, i64, String)>>>()
                .map_err(query_err)?;
            drop(stmt);
            let mut root = plan::from_sqlite_eqp(&rows);
            let raw = rows
                .iter()
                .map(|(id, parent, detail)| format!("{id}\t{parent}\t{detail}"))
                .collect::<Vec<_>>()
                .join("\n");
            if analyze {
                // run it for real inside a savepoint (works in or out of a transaction), undo it
                self.db
                    .execute_batch("SAVEPOINT dbx_explain")
                    .map_err(query_err)?;
                let measured = (|| {
                    let mut stmt = self.db.prepare(body).map_err(query_err)?;
                    if self.read_only && !stmt.readonly() {
                        return Err(DbError::ReadOnly(
                            "the statement may write to the database".into(),
                        ));
                    }
                    let t = Instant::now();
                    let n = if stmt.column_count() == 0 {
                        stmt.execute([]).map_err(query_err)?
                    } else {
                        let mut rows = stmt.query([]).map_err(query_err)?;
                        let mut n = 0;
                        while rows.next().map_err(query_err)?.is_some() {
                            n += 1;
                        }
                        n
                    };
                    Ok((n, t.elapsed()))
                })();
                let _ = self
                    .db
                    .execute_batch("ROLLBACK TO dbx_explain; RELEASE dbx_explain");
                let (n, took) = measured?;
                root.actual_rows = Some(n as f64);
                root.actual_ms = Some(took.as_secs_f64() * 1000.0);
            }
            Ok(Plan {
                root,
                raw,
                format: PlanFormat::Table,
            })
        })();
        let what = if analyze {
            "explain analyze"
        } else {
            "explain"
        };
        log_statement(DbKind::Sqlite, what, sql, self.read_only, started, result)
    }

    fn structure(&mut self, database: &str, scope: &StructureScope) -> Result<Structure> {
        let db = database_or_current(Some(self.current.clone()), database)?;
        if !self.database_names()?.iter().any(|d| *d == db) {
            return Err(DbError::NotFound(format!("database {db}")));
        }
        structure::introspect(DbKind::Sqlite, &db, scope, |sql| {
            let mut stmt = self.db.prepare(sql).map_err(query_err)?;
            let width = stmt.column_count();
            stmt.query_map([], |r| {
                (0..width)
                    .map(|i| {
                        Ok(match r.get_ref(i)? {
                            ValueRef::Null => None,
                            ValueRef::Integer(n) => Some(n.to_string()),
                            ValueRef::Real(x) => Some(x.to_string()),
                            ValueRef::Text(t) | ValueRef::Blob(t) => {
                                Some(String::from_utf8_lossy(t).into_owned())
                            }
                        })
                    })
                    .collect()
            })
            .map_err(query_err)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(query_err)
        })
    }
}

fn cell(v: ValueRef<'_>, meta: &ColumnMeta) -> Value {
    match v {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(n) => match meta.data_type {
            DataType::Bool if n == 0 || n == 1 => Value::Bool(n == 1),
            DataType::Decimal { .. } => Value::Decimal(n.to_string()),
            _ => Value::Int(n),
        },
        ValueRef::Real(x) => match meta.data_type {
            DataType::Decimal { .. } if x.is_finite() => Value::Decimal(x.to_string()),
            _ => Value::Float(x),
        },
        ValueRef::Text(t) => match std::str::from_utf8(t) {
            Ok(s) => canonical(s.to_string(), meta),
            Err(_) => Value::Bytes(t.to_vec()),
        },
        ValueRef::Blob(b) => Value::Bytes(b.to_vec()),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};

    use super::*;
    use crate::driver::connect;

    /// A database file in the temp dir, removed on drop.
    struct TempDb(PathBuf);

    impl TempDb {
        fn new() -> Self {
            static N: AtomicU32 = AtomicU32::new(0);
            let n = N.fetch_add(1, Ordering::Relaxed);
            let path =
                std::env::temp_dir().join(format!("dbx-sqlite-test-{}-{n}.db", std::process::id()));
            let _ = std::fs::remove_file(&path);
            // Create the file out of band: the driver never creates.
            Db::open(&path).unwrap().execute_batch("SELECT 1").unwrap();
            Self(path)
        }

        fn cfg(&self) -> ConnectionConfig {
            let mut cfg = ConnectionConfig::new(DbKind::Sqlite);
            cfg.path = Some(self.0.clone());
            cfg
        }
    }

    impl Drop for TempDb {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    fn fixture() -> (TempDb, Box<dyn Connection>) {
        let tmp = TempDb::new();
        let mut c = connect(&tmp.cfg()).unwrap();
        for sql in [
            "CREATE TABLE person (id INTEGER PRIMARY KEY, name VARCHAR(40) NOT NULL, \
             age INT DEFAULT 18, active BOOLEAN, born DATE, seen DATETIME, token UUID, \
             doc JSON, photo BLOB, price DECIMAL(10,2), total INT GENERATED ALWAYS AS (age * 2))",
            "CREATE TABLE link (a INT, b INT, note TEXT, PRIMARY KEY (a, b))",
            "CREATE VIEW adults AS SELECT id, name FROM person WHERE age >= 18",
        ] {
            c.execute(sql).unwrap();
        }
        (tmp, c)
    }

    #[test]
    fn missing_file_is_a_connect_error_and_nothing_is_created() {
        let mut cfg = ConnectionConfig::new(DbKind::Sqlite);
        let path = std::env::temp_dir().join(format!("dbx-nope-{}.db", std::process::id()));
        cfg.path = Some(path.clone());
        assert!(matches!(connect(&cfg), Err(DbError::Connect(_))));
        assert!(!path.exists());
    }

    #[test]
    fn create_makes_a_valid_empty_database_and_refuses_an_existing_file() {
        let tmp = TempDb::new();
        std::fs::remove_file(&tmp.0).unwrap();
        create_sqlite_database(&tmp.0).unwrap();
        assert!(
            std::fs::metadata(&tmp.0).unwrap().len() > 0,
            "header written"
        );
        let mut c = connect(&tmp.cfg()).unwrap();
        c.ping().unwrap();
        drop(c);
        assert!(create_sqlite_database(&tmp.0).is_err(), "never overwrites");
        let nested = tmp.0.with_extension("nodir").join("x.db");
        assert!(create_sqlite_database(&nested).is_err(), "missing parent");
    }

    #[test]
    fn not_a_database_is_a_connect_error() {
        let tmp = TempDb::new();
        std::fs::write(&tmp.0, b"this is not a sqlite database at all, just text").unwrap();
        assert!(matches!(connect(&tmp.cfg()), Err(DbError::Connect(_))));
    }

    #[test]
    fn introspection() {
        let (tmp, mut c) = fixture();
        c.ping().unwrap();
        assert_eq!(c.kind(), DbKind::Sqlite);
        assert_eq!(c.databases().unwrap(), ["main"]);
        assert!(c.schemas("main").unwrap().is_empty());

        let objs = c.objects("main", None).unwrap();
        let summary: Vec<_> = objs.iter().map(|o| (o.name.as_str(), o.kind)).collect();
        assert_eq!(
            summary,
            [
                ("adults", ObjectKind::View),
                ("link", ObjectKind::Table),
                ("person", ObjectKind::Table)
            ]
        );
        assert!(matches!(c.objects("nope", None), Err(DbError::NotFound(_))));

        let cols = c.columns(&objs[2].table_ref()).unwrap();
        let names: Vec<_> = cols.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "id", "name", "age", "active", "born", "seen", "token", "doc", "photo", "price",
                "total"
            ]
        );
        let id = &cols[0];
        assert!(id.is_pk && id.auto_increment && !id.nullable);
        assert_eq!(
            id.data_type,
            DataType::Int {
                bits: 64,
                unsigned: false
            }
        );
        let name = &cols[1];
        assert!(!name.nullable && !name.is_pk);
        assert_eq!(name.native_type, "VARCHAR(40)");
        assert_eq!(name.max_len(), Some(40));
        assert_eq!(cols[2].default.as_deref(), Some("18"));
        assert_eq!(cols[3].data_type, DataType::Bool);
        assert!(cols[10].computed);
        assert!(!cols[2].computed);

        let link = c
            .columns(&TableRef::new(Some("main"), None, "link"))
            .unwrap();
        assert!(link[0].is_pk && link[1].is_pk && !link[2].is_pk);
        assert!(!link[0].auto_increment);

        let view = c.columns(&objs[0].table_ref()).unwrap();
        assert_eq!(view.len(), 2);
        assert!(matches!(
            c.columns(&TableRef::new(None, None, "ghost")),
            Err(DbError::NotFound(_))
        ));
        drop(tmp);
    }

    #[test]
    fn structure_as_dbml() {
        let tmp = TempDb::new();
        let mut c = connect(&tmp.cfg()).unwrap();
        for sql in [
            "CREATE TABLE author (id INTEGER PRIMARY KEY, email TEXT NOT NULL UNIQUE, \
             name VARCHAR(40) DEFAULT 'anon', joined DATETIME DEFAULT CURRENT_TIMESTAMP)",
            "CREATE TABLE book (author_id INT NOT NULL REFERENCES author(id) ON DELETE CASCADE, \
             n INT NOT NULL, title TEXT, PRIMARY KEY (author_id, n))",
            "CREATE INDEX book_title ON book (title, n)",
            "CREATE INDEX book_lower ON book (lower(title))",
            "CREATE TABLE tag (book_author INT, book_n INT, label, \
             FOREIGN KEY (book_author, book_n) REFERENCES book (author_id, n))",
            "CREATE VIEW v AS SELECT 1",
        ] {
            c.execute(sql).unwrap();
        }
        let all = c.structure("main", &StructureScope::default()).unwrap();
        assert_eq!(
            crate::dbml::to_dbml(&all),
            "Table author {
  id INTEGER [pk, increment]
  email TEXT [not null, unique]
  name VARCHAR(40) [default: 'anon']
  joined DATETIME [default: `CURRENT_TIMESTAMP`]
}

Table book {
  author_id INT [not null]
  n INT [not null]
  title TEXT

  Indexes {
    (author_id, n) [pk]
    (title, n) [name: 'book_title']
  }
}

Table tag {
  book_author INT
  book_n INT
  label any
}

Ref: book.author_id > author.id [delete: cascade]
Ref: tag.(book_author, book_n) > book.(author_id, n)
"
        );
        // one table: the reference to a table left out goes with it
        let scope = StructureScope {
            schema: None,
            tables: vec!["book".into()],
        };
        let one = c.structure("", &scope).unwrap();
        assert_eq!(one.tables.len(), 1);
        assert!(one.refs.is_empty());
        assert!(matches!(
            c.structure("nope", &scope),
            Err(DbError::NotFound(_))
        ));
    }

    #[test]
    fn attached_databases_are_listed_and_queryable() {
        let (_tmp, mut c) = fixture();
        c.execute("ATTACH DATABASE ':memory:' AS aux").unwrap();
        c.execute("CREATE TABLE aux.extra (x INTEGER)").unwrap();
        assert_eq!(c.databases().unwrap(), ["aux", "main"]);
        let objs = c.objects("aux", None).unwrap();
        assert_eq!(objs.len(), 1);
        assert_eq!(objs[0].table_ref().database.as_deref(), Some("aux"));
        let cols = c.columns(&objs[0].table_ref()).unwrap();
        assert_eq!(cols.len(), 1);
    }

    #[test]
    fn use_database_accepts_main_and_attached_only() {
        let (_tmp, mut c) = fixture();
        assert_eq!(c.current_database().as_deref(), Some("main"));
        c.use_database("main").unwrap();
        assert!(matches!(c.use_database("aux"), Err(DbError::NotFound(_))));
        c.execute("ATTACH DATABASE ':memory:' AS aux").unwrap();
        c.use_database("aux").unwrap();
        assert_eq!(c.current_database().as_deref(), Some("aux"));
        // a rejected switch leaves the current database alone
        assert!(c.use_database("nope").is_err());
        assert_eq!(c.current_database().as_deref(), Some("aux"));
        c.ping().unwrap();
    }

    #[test]
    fn query_values_and_truncation() {
        let (_tmp, mut c) = fixture();
        let out = c
            .execute(
                "INSERT INTO person (name, age, active, born, seen, token, doc, photo, price) \
                 VALUES ('Ada', 36, 1, '1815-12-10', '1843-07-01 10:30:00', \
                 '6F9619FF-8B86-D011-B42D-00C04FC964FF', '{\"a\":1}', x'00ff', 12.5), \
                 ('Bob', NULL, 0, 'not a date', NULL, NULL, NULL, NULL, 3), \
                 ('Cy', 5, NULL, NULL, NULL, NULL, NULL, NULL, NULL)",
            )
            .unwrap();
        assert_eq!(out.rows_affected, 3);

        let rs = c
            .query(
                "SELECT id, name, age, active, born, seen, token, doc, photo, price, total \
                 FROM person ORDER BY id",
                None,
            )
            .unwrap();
        assert!(!rs.truncated);
        assert_eq!(rs.columns.len(), 11);
        assert!(rs.rows.iter().all(|r| r.len() == 11));
        assert_eq!(
            rs.rows[0],
            [
                Value::Int(1),
                Value::Text("Ada".into()),
                Value::Int(36),
                Value::Bool(true),
                Value::Date("1815-12-10".into()),
                Value::DateTime("1843-07-01 10:30:00".into()),
                Value::Uuid("6f9619ff-8b86-d011-b42d-00c04fc964ff".into()),
                Value::Json("{\"a\":1}".into()),
                Value::Bytes(vec![0, 255]),
                Value::Decimal("12.5".into()),
                Value::Int(72),
            ]
        );
        // an unvalidatable date stays text; NULLs stay NULL; an integer in DECIMAL is Decimal
        assert_eq!(rs.rows[1][2], Value::Null);
        assert_eq!(rs.rows[1][3], Value::Bool(false));
        assert_eq!(rs.rows[1][4], Value::Text("not a date".into()));
        assert_eq!(rs.rows[1][9], Value::Decimal("3".into()));
        assert_eq!(rs.rows[2][3], Value::Null);

        let rs = c
            .query("SELECT id FROM person ORDER BY id", Some(2))
            .unwrap();
        assert_eq!(rs.rows.len(), 2);
        assert!(rs.truncated);
        let rs = c.query("SELECT id FROM person", Some(3)).unwrap();
        assert_eq!((rs.rows.len(), rs.truncated), (3, false));
        let rs = c
            .query("SELECT 1 + 1 AS two, 'x' || 'y', 1.5", None)
            .unwrap();
        assert_eq!(rs.columns[0].name, "two");
        assert_eq!(
            rs.rows[0],
            [Value::Int(2), Value::Text("xy".into()), Value::Float(1.5)]
        );
    }

    #[test]
    fn execute_counts_and_errors() {
        let (_tmp, mut c) = fixture();
        c.execute("INSERT INTO link (a, b) VALUES (1, 1), (1, 2), (2, 1)")
            .unwrap();
        assert_eq!(
            c.execute("UPDATE link SET note = 'x' WHERE a = 1")
                .unwrap()
                .rows_affected,
            2
        );
        assert_eq!(
            c.execute("DELETE FROM link WHERE a = 99")
                .unwrap()
                .rows_affected,
            0
        );
        // DDL right after DML must not repeat the DML count
        assert_eq!(c.execute("CREATE TABLE t2 (x)").unwrap().rows_affected, 0);
        assert_eq!(c.execute("DELETE FROM link").unwrap().rows_affected, 3);

        let e = c.execute("INSERT INTO nope VALUES (1)").unwrap_err();
        assert!(matches!(e, DbError::Query(m) if m.contains("no such table")));
        let e = c.query("SELEC 1", None).unwrap_err();
        assert!(matches!(e, DbError::Query(_)));
        // a statement without rows passed to query() yields an empty result set
        let rs = c
            .query("INSERT INTO link (a, b) VALUES (7, 7)", None)
            .unwrap();
        assert!(rs.columns.is_empty() && rs.rows.is_empty());
        assert_eq!(
            c.query("SELECT count(*) FROM link", None).unwrap().rows[0][0],
            Value::Int(1)
        );
    }

    const ENDLESS: &str =
        "WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM n) SELECT count(*) FROM n";

    fn person_count(c: &mut Box<dyn Connection>) -> Value {
        c.query("SELECT count(*) FROM person", None).unwrap().rows[0][0].clone()
    }

    #[test]
    fn read_only_per_call() {
        let (_tmp, mut c) = fixture();
        c.execute("INSERT INTO person (name) VALUES ('Ada')")
            .unwrap();
        assert!(!c.is_read_only());
        let ro = ExecOptions::read_only();
        for sql in [
            "DELETE FROM person",
            "INSERT INTO person (name) VALUES ('x')",
            "UPDATE person SET name = 'y'",
            "CREATE TABLE z (a)",
            "DROP TABLE link",
        ] {
            assert!(
                matches!(c.query_with(sql, &ro), Err(DbError::ReadOnly(_))),
                "{sql}"
            );
            assert!(
                matches!(c.execute_with(sql, &ro), Err(DbError::ReadOnly(_))),
                "{sql}"
            );
        }
        // a second statement is refused by the binding, never run
        assert!(c.query_with("SELECT 1; DELETE FROM person", &ro).is_err());
        assert_eq!(person_count(&mut c), Value::Int(1));
        // reads still work, with the row limit
        let rs = c
            .query_with(
                "SELECT name FROM person",
                &ExecOptions {
                    row_limit: Some(0),
                    ..ro
                },
            )
            .unwrap();
        assert!(rs.truncated && rs.rows.is_empty());
        // query_only was reset: the connection writes again
        c.execute("DELETE FROM person").unwrap();
        assert_eq!(person_count(&mut c), Value::Int(0));
    }

    #[test]
    fn read_only_connection() {
        let (tmp, mut rw) = fixture();
        rw.execute("INSERT INTO person (name) VALUES ('Ada')")
            .unwrap();
        let mut cfg = tmp.cfg();
        cfg.read_only = true;
        let mut c = connect(&cfg).unwrap();
        assert!(c.is_read_only());
        // forced on the plain calls too
        assert!(matches!(
            c.execute("DELETE FROM person"),
            Err(DbError::ReadOnly(_))
        ));
        assert!(matches!(
            c.query("INSERT INTO person (name) VALUES ('x') RETURNING id", None),
            Err(DbError::ReadOnly(_))
        ));
        // even the pragma cannot open a write path: the file is opened read-only
        let _ = c.execute("PRAGMA query_only = OFF");
        assert!(c.execute("DELETE FROM person").is_err());
        assert_eq!(person_count(&mut c), Value::Int(1));
        assert_eq!(c.objects("main", None).unwrap().len(), 3);
        // the legacy `params["mode"]` still opens read-only
        let mut legacy = tmp.cfg();
        legacy.params.insert("mode".into(), "ro".into());
        assert!(connect(&legacy).unwrap().is_read_only());
    }

    #[test]
    fn cancel_from_another_thread() {
        let (_tmp, mut c) = fixture();
        let handle = c.cancel_handle().unwrap();
        // nothing running: a no-op
        handle.cancel().unwrap();
        c.ping().unwrap();
        let t = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(200));
            handle.cancel().unwrap();
        });
        let started = Instant::now();
        assert_eq!(c.query(ENDLESS, None), Err(DbError::Cancelled));
        assert!(started.elapsed() < Duration::from_secs(10));
        t.join().unwrap();
        // the connection survives
        c.ping().unwrap();
        assert_eq!(person_count(&mut c), Value::Int(0));
    }

    #[test]
    fn timeout() {
        let (_tmp, mut c) = fixture();
        let opts = ExecOptions {
            timeout: Some(Duration::from_millis(150)),
            ..ExecOptions::default()
        };
        assert_eq!(c.query_with(ENDLESS, &opts), Err(DbError::Timeout));
        // a fast statement under the same timeout is untouched, and nothing fires later
        c.query_with("SELECT 1", &opts).unwrap();
        std::thread::sleep(Duration::from_millis(250));
        c.ping().unwrap();
    }

    #[test]
    fn explain_plans() {
        let (_tmp, mut c) = fixture();
        c.execute("INSERT INTO link (a, b) VALUES (1, 1), (1, 2), (2, 1)")
            .unwrap();
        c.execute("INSERT INTO person (name) VALUES ('Ada'), ('Bob')")
            .unwrap();
        let p = c
            .explain(
                "SELECT p.name FROM person p JOIN link l ON l.a = p.id ORDER BY p.name;",
                false,
            )
            .unwrap();
        assert_eq!(p.format, PlanFormat::Table);
        assert_eq!(p.root.label, "QUERY PLAN");
        assert!(p.root.children.len() >= 2, "{p:?}");
        assert!(
            p.root
                .children
                .iter()
                .any(|n| n.label.starts_with("SCAN") || n.label.starts_with("SEARCH")),
            "{p:?}"
        );
        assert!(p.raw.contains('\t'));
        assert_eq!(p.root.actual_rows, None);

        let p = c.explain("SELECT * FROM link", true).unwrap();
        assert_eq!(p.root.actual_rows, Some(3.0));
        assert!(p.root.actual_ms.is_some());
        // analyze runs a write, and rolls it back
        let p = c.explain("DELETE FROM link WHERE a = 1", true).unwrap();
        assert_eq!(p.root.actual_rows, Some(2.0));
        assert_eq!(
            c.query("SELECT count(*) FROM link", None).unwrap().rows[0][0],
            Value::Int(3)
        );
        // inside the caller's own transaction too, which stays open
        c.execute("BEGIN").unwrap();
        c.execute("DELETE FROM link WHERE a = 2").unwrap();
        c.explain("DELETE FROM link", true).unwrap();
        assert_eq!(
            c.query("SELECT count(*) FROM link", None).unwrap().rows[0][0],
            Value::Int(2)
        );
        c.execute("ROLLBACK").unwrap();
        assert_eq!(
            c.query("SELECT count(*) FROM link", None).unwrap().rows[0][0],
            Value::Int(3)
        );
        assert!(matches!(
            c.explain("SELEC 1", false),
            Err(DbError::Query(_))
        ));
    }

    #[test]
    fn approx_rows_from_stats() {
        let (_tmp, mut c) = fixture();
        let objs = c.objects("main", None).unwrap();
        assert!(objs.iter().all(|o| o.approx_rows.is_none()));
        c.execute("INSERT INTO link (a, b) VALUES (1, 1), (1, 2), (2, 1)")
            .unwrap();
        c.execute("INSERT INTO person (name) VALUES ('Ada'), ('Bob')")
            .unwrap();
        c.execute("ANALYZE").unwrap();
        let objs = c.objects("main", None).unwrap();
        let rows: Vec<_> = objs
            .iter()
            .map(|o| (o.name.as_str(), o.approx_rows))
            .collect();
        assert_eq!(
            rows,
            [("adults", None), ("link", Some(3)), ("person", Some(2))]
        );
    }
}
