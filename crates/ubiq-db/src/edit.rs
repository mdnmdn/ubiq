//! `edit` — every SQL string the app runs for table browsing and row edits.
//!
//! The statements are plain text (the drivers execute one statement per call), so escaping is done
//! here, in [`literal`] and [`quote_ident`], and the same text is what the preview pane shows.
//!
//! Assumptions about the server session, all of them the defaults: Postgres
//! `standard_conforming_strings = on` (backslash is an ordinary character in `'…'`); MySQL without
//! `NO_BACKSLASH_ESCAPES` (backslash escapes in `'…'`, which [`literal`] doubles); SQLite and SQL
//! Server have no such switch for string literals.
//!
//! * [`quote_ident`] / [`qualified`] — identifiers per dialect. `qualified` follows the
//!   [`TableRef`] rule: Postgres drops the database, SQL Server writes `db.schema.name`, MySQL and
//!   SQLite write the database as the only qualifier (SQLite omits `main`).
//! * [`select_table`] / [`count_table`] — a table page. The user's filter goes after `WHERE` and the
//!   order-by after `ORDER BY` **verbatim**, each clause on its own line (so a trailing `-- note`
//!   cannot swallow the next clause). [`validate_fragment`] checks both through [`sql::analyze`]
//!   and rejects a second statement.
//! * [`RowEdit`] / [`render`] / [`render_batch`] — `UPDATE` / `INSERT` / `DELETE` of one row, keyed by
//!   [`key_for_row`]: the primary key, or every column when there is none. A keyless edit can hit
//!   duplicate rows, so [`has_unique_key`] / [`RowEdit::has_unique_key`] tell the UI to warn, and
//!   `render` adds a single-row guard where the engine has one (`LIMIT 1` on MySQL, `TOP (1)` on
//!   SQL Server, a `ctid` / `rowid` sub-select on Postgres / SQLite).

use crate::conn::DbKind;
use crate::model::{ColumnMeta, TableRef};
use crate::sql::{self, Dialect, SqlError, StatementClass};
use crate::value::{DataType, Value};
use serde::{Deserialize, Serialize};

impl From<DbKind> for Dialect {
    fn from(kind: DbKind) -> Self {
        match kind {
            DbKind::Postgres => Dialect::Postgres,
            DbKind::MySql => Dialect::MySql,
            DbKind::Sqlite => Dialect::Sqlite,
            DbKind::MsSql => Dialect::MsSql,
        }
    }
}

// ---- identifiers ------------------------------------------------------------------------------

/// `name` quoted for `kind`: `"x"` (Postgres, SQLite; `"` doubled), `` `x` `` (MySQL; `` ` `` doubled),
/// `[x]` (SQL Server; `]` doubled).
pub fn quote_ident(kind: DbKind, name: &str) -> String {
    match kind {
        DbKind::Postgres | DbKind::Sqlite => format!("\"{}\"", name.replace('"', "\"\"")),
        DbKind::MySql => format!("`{}`", name.replace('`', "``")),
        DbKind::MsSql => format!("[{}]", name.replace(']', "]]")),
    }
}

/// The table as SQL text, quoted and qualified as far as `kind` can say it. SQL Server with a
/// database but no schema writes `db..name` (the login's default schema).
pub fn qualified(kind: DbKind, table: &TableRef) -> String {
    let q = |s: &str| quote_ident(kind, s);
    let name = q(&table.name);
    match kind {
        DbKind::Postgres => match &table.schema {
            Some(s) => format!("{}.{name}", q(s)),
            None => name,
        },
        DbKind::MySql => match &table.database {
            Some(d) => format!("{}.{name}", q(d)),
            None => name,
        },
        DbKind::Sqlite => match &table.database {
            Some(d) if !d.eq_ignore_ascii_case("main") => format!("{}.{name}", q(d)),
            _ => name,
        },
        DbKind::MsSql => match (&table.database, &table.schema) {
            (Some(d), Some(s)) => format!("{}.{}.{name}", q(d), q(s)),
            (Some(d), None) => format!("{}..{name}", q(d)),
            (None, Some(s)) => format!("{}.{name}", q(s)),
            (None, None) => name,
        },
    }
}

// ---- literals ---------------------------------------------------------------------------------

/// `value` as a SQL literal for a column of `data_type`.
///
/// Text: `'…'` with `'` doubled; MySQL also doubles `\` and writes NUL as `\0`; SQL Server uses
/// `N'…'` when the text is not ASCII. NUL elsewhere: SQLite and SQL Server build the string with
/// `char(0)` / `NCHAR(0)`; Postgres cannot store NUL in text, so it is dropped. Bytes: `X'..'`
/// (MySQL, SQLite), `0x..` (SQL Server), `'\x..'::bytea` (Postgres). Booleans: `TRUE`/`FALSE`
/// (Postgres, MySQL), `1`/`0` (SQLite, SQL Server). Postgres casts uuid (`::uuid`) and leaves
/// everything else to the column's type (an unknown literal takes it, which `json` / `jsonb` /
/// enums need). A non-finite float is `'NaN'::float8`-style on Postgres, `9e999` on SQLite and
/// `NULL` where the engine has no such value. Timestamps get the form each engine reads
/// unambiguously (SQL Server `T` separator, MySQL `+00:00` for `Z`).
pub fn literal(kind: DbKind, value: &Value, data_type: &DataType) -> String {
    match value {
        Value::Null => "NULL".to_string(),
        Value::Bool(b) => match kind {
            DbKind::Postgres | DbKind::MySql => if *b { "TRUE" } else { "FALSE" }.to_string(),
            DbKind::Sqlite | DbKind::MsSql => if *b { "1" } else { "0" }.to_string(),
        },
        Value::Int(n) => n.to_string(),
        Value::Float(f) => float_literal(kind, *f),
        Value::Decimal(s) => {
            if is_plain_number(s) {
                s.clone()
            } else {
                text_literal(kind, s)
            }
        }
        Value::Text(s) => {
            if kind == DbKind::Postgres && *data_type == DataType::Uuid {
                format!("{}::uuid", text_literal(kind, s))
            } else {
                text_literal(kind, s)
            }
        }
        Value::Bytes(b) => bytes_literal(kind, b),
        Value::Uuid(s) => {
            if kind == DbKind::Postgres {
                format!("{}::uuid", text_literal(kind, s))
            } else {
                text_literal(kind, s)
            }
        }
        Value::Date(s) | Value::Time(s) | Value::Json(s) => text_literal(kind, s),
        Value::DateTime(s) => match kind {
            DbKind::MsSql => text_literal(kind, &s.replacen(' ', "T", 1)),
            _ => text_literal(kind, s),
        },
        Value::DateTimeTz(s) => match kind {
            DbKind::MsSql => text_literal(kind, &s.replacen(' ', "T", 1)),
            DbKind::MySql => {
                let s = match s.strip_suffix(['Z', 'z']) {
                    Some(head) => format!("{head}+00:00"),
                    None => s.clone(),
                };
                text_literal(kind, &s)
            }
            _ => text_literal(kind, s),
        },
    }
}

/// Digits, sign, point, exponent — safe to paste unquoted.
fn is_plain_number(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_digit() || matches!(c, '+' | '-' | '.' | 'e' | 'E'))
        && s.parse::<f64>().is_ok()
}

fn float_literal(kind: DbKind, f: f64) -> String {
    if f.is_finite() {
        return format!("{f:?}");
    }
    match kind {
        DbKind::Postgres => {
            let name = if f.is_nan() {
                "NaN"
            } else if f > 0.0 {
                "Infinity"
            } else {
                "-Infinity"
            };
            format!("'{name}'::float8")
        }
        DbKind::Sqlite if f.is_infinite() => if f > 0.0 { "9e999" } else { "-9e999" }.to_string(),
        _ => "NULL".to_string(),
    }
}

fn bytes_literal(kind: DbKind, bytes: &[u8]) -> String {
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    match kind {
        DbKind::Postgres => format!("'\\x{hex}'::bytea"),
        DbKind::MySql | DbKind::Sqlite => format!("X'{hex}'"),
        DbKind::MsSql => format!("0x{hex}"),
    }
}

fn text_literal(kind: DbKind, s: &str) -> String {
    match kind {
        DbKind::Postgres => format!("'{}'", s.replace('\0', "").replace('\'', "''")),
        DbKind::MySql => {
            let mut out = String::with_capacity(s.len() + 2);
            out.push('\'');
            for c in s.chars() {
                match c {
                    '\'' => out.push_str("''"),
                    '\\' => out.push_str("\\\\"),
                    '\0' => out.push_str("\\0"),
                    '\u{1a}' => out.push_str("\\Z"),
                    c => out.push(c),
                }
            }
            out.push('\'');
            out
        }
        DbKind::Sqlite | DbKind::MsSql => {
            let prefix = if kind == DbKind::MsSql && !s.is_ascii() {
                "N"
            } else {
                ""
            };
            let (sep, nul) = match kind {
                DbKind::Sqlite => (" || ", "char(0)"),
                _ => (" + ", "NCHAR(0)"),
            };
            let parts: Vec<String> = s
                .split('\0')
                .map(|p| format!("{prefix}'{}'", p.replace('\'', "''")))
                .collect();
            if parts.len() == 1 {
                return parts.into_iter().next().unwrap_or_default();
            }
            format!("({})", parts.join(&format!("{sep}{nul}{sep}")))
        }
    }
}

// ---- table queries ----------------------------------------------------------------------------

/// A composed SELECT and where the user's fragments sit in it (byte offsets of their trimmed text).
struct Composed {
    sql: String,
    filter_at: Option<usize>,
    order_at: Option<usize>,
}

fn compose_select(
    kind: DbKind,
    table: &TableRef,
    columns: Option<&[String]>,
    filter: &str,
    order_by: &str,
    limit: usize,
    offset: usize,
) -> Composed {
    let cols = match columns {
        Some(c) if !c.is_empty() => c
            .iter()
            .map(|c| quote_ident(kind, c))
            .collect::<Vec<_>>()
            .join(", "),
        _ => "*".to_string(),
    };
    let top = kind == DbKind::MsSql && offset == 0;
    let mut sql = if top {
        format!(
            "SELECT TOP ({limit}) {cols}\nFROM {}",
            qualified(kind, table)
        )
    } else {
        format!("SELECT {cols}\nFROM {}", qualified(kind, table))
    };
    let (filter, order_by) = (filter.trim(), order_by.trim());
    let mut filter_at = None;
    let mut order_at = None;
    if !filter.is_empty() {
        sql.push_str("\nWHERE ");
        filter_at = Some(sql.len());
        sql.push_str(filter);
    }
    if !order_by.is_empty() {
        sql.push_str("\nORDER BY ");
        order_at = Some(sql.len());
        sql.push_str(order_by);
    }
    match kind {
        DbKind::MsSql if !top => {
            if order_by.is_empty() {
                sql.push_str("\nORDER BY (SELECT NULL)");
            }
            // FETCH NEXT needs at least one row.
            sql.push_str(&format!(
                "\nOFFSET {offset} ROWS FETCH NEXT {} ROWS ONLY",
                limit.max(1)
            ));
        }
        DbKind::MsSql => {}
        _ => {
            sql.push_str(&format!("\nLIMIT {limit}"));
            if offset > 0 {
                sql.push_str(&format!(" OFFSET {offset}"));
            }
        }
    }
    Composed {
        sql,
        filter_at,
        order_at,
    }
}

/// One page of `table`: `columns` (`None` or empty = `*`), `filter` after `WHERE` and `order_by`
/// after `ORDER BY` verbatim (both may be empty), `limit` rows from `offset`. SQL Server uses
/// `TOP (n)` at offset 0 and `OFFSET … FETCH` after, with `ORDER BY (SELECT NULL)` when there is
/// no order-by; a limit of 0 is raised to 1 there when paging.
pub fn select_table(
    kind: DbKind,
    table: &TableRef,
    columns: Option<&[String]>,
    filter: &str,
    order_by: &str,
    limit: usize,
    offset: usize,
) -> String {
    compose_select(kind, table, columns, filter, order_by, limit, offset).sql
}

/// `SELECT COUNT(*)` (`COUNT_BIG(*)` on SQL Server) over the rows `filter` keeps.
pub fn count_table(kind: DbKind, table: &TableRef, filter: &str) -> String {
    let func = if kind == DbKind::MsSql {
        "COUNT_BIG(*)"
    } else {
        "COUNT(*)"
    };
    let mut sql = format!("SELECT {func}\nFROM {}", qualified(kind, table));
    let filter = filter.trim();
    if !filter.is_empty() {
        sql.push_str("\nWHERE ");
        sql.push_str(filter);
    }
    sql
}

/// Check the user's `filter` and `order_by` by analyzing the SELECT they would be part of: it must
/// parse, be a single read statement, and so a `;` that starts a second statement is rejected.
/// The error's `message` is prefixed `WHERE:` or `ORDER BY:`, and `line` / `column` / `byte_offset`
/// are relative to that fragment's trimmed text.
pub fn validate_fragment(kind: DbKind, filter: &str, order_by: &str) -> Result<(), SqlError> {
    let table = TableRef::new(None, None, "t");
    // Offset 1 so every dialect ends the query with a paging clause: a trailing `;` or an
    // unclosed comment then breaks the parse instead of ending the statement early.
    let c = compose_select(kind, &table, None, filter, order_by, 1, 1);
    let analysis = sql::analyze(&c.sql, kind.into());
    let err = if let Some(e) = analysis.error {
        e
    } else if let Some(second) = analysis.statements.get(1) {
        at_offset(
            "only one statement is allowed",
            &c.sql,
            second.byte_range.start,
        )
    } else if analysis
        .statements
        .first()
        .is_none_or(|s| s.class != StatementClass::Read)
    {
        at_offset("not a read-only query", &c.sql, 0)
    } else {
        return Ok(());
    };
    Err(relocate(err, &c, filter.trim(), order_by.trim()))
}

fn at_offset(message: &str, sql: &str, byte_offset: usize) -> SqlError {
    let (line, column) = line_col(sql, byte_offset);
    SqlError {
        message: message.to_string(),
        line,
        column,
        byte_offset,
    }
}

fn line_col(s: &str, offset: usize) -> (usize, usize) {
    let mut offset = offset.min(s.len());
    while !s.is_char_boundary(offset) {
        offset -= 1;
    }
    let head = &s[..offset];
    let line = head.matches('\n').count() + 1;
    let col = head.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
    (line, col)
}

/// Re-express an error of the composed SELECT relative to the fragment it falls in (the last one
/// starting at or before it; an error past the fragment, in `LIMIT`, pins to its end).
fn relocate(err: SqlError, c: &Composed, filter: &str, order_by: &str) -> SqlError {
    let (label, text, start) = match (c.filter_at, c.order_at) {
        (_, Some(o)) if err.byte_offset >= o => ("ORDER BY", order_by, o),
        (Some(f), _) if err.byte_offset >= f => ("WHERE", filter, f),
        (Some(f), _) => ("WHERE", filter, f),
        (None, Some(o)) => ("ORDER BY", order_by, o),
        (None, None) => return err,
    };
    let rel = err.byte_offset.saturating_sub(start).min(text.len());
    let (line, column) = line_col(text, rel);
    SqlError {
        message: format!("{label}: {}", err.message),
        line,
        column,
        byte_offset: rel,
    }
}

// ---- row edits --------------------------------------------------------------------------------

/// One row change. `key` identifies the row ([`key_for_row`] of the row as loaded); `changes` and
/// `values` pair a column with its new value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum RowEdit {
    Update {
        key: Vec<(ColumnMeta, Value)>,
        changes: Vec<(ColumnMeta, Value)>,
    },
    Insert {
        values: Vec<(ColumnMeta, Value)>,
    },
    Delete {
        key: Vec<(ColumnMeta, Value)>,
    },
}

impl RowEdit {
    /// The edit finds its row by a primary key. `false` means it matches on every column and may
    /// touch duplicate rows (the UI should say so); an `Insert` has no row to find.
    pub fn has_unique_key(&self) -> bool {
        match self {
            RowEdit::Update { key, .. } | RowEdit::Delete { key } => {
                key.iter().any(|(c, _)| c.is_pk)
            }
            RowEdit::Insert { .. } => true,
        }
    }
}

/// A [`RowEdit`] that cannot be turned into a statement.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EditError {
    #[error("no key to find the row by")]
    EmptyKey,
    #[error("nothing to update")]
    NoChanges,
    #[error("{0} is a computed column and cannot be written")]
    ComputedColumn(String),
}

/// The table has a primary key, so an edit can address exactly one row.
pub fn has_unique_key(columns: &[ColumnMeta]) -> bool {
    columns.iter().any(|c| c.is_pk)
}

/// The key of `row` (cells in `columns` order): the primary-key cells, or every cell when the table
/// has no primary key — minus the types an engine cannot compare (Postgres `json`; SQL Server
/// `ntext`/`image`/`xml`). A NULL cell is matched with `IS NULL` by [`render`].
pub fn key_for_row(columns: &[ColumnMeta], row: &[Value]) -> Vec<(ColumnMeta, Value)> {
    let pairs = columns.iter().zip(row);
    if has_unique_key(columns) {
        pairs
            .filter(|(c, _)| c.is_pk)
            .map(|(c, v)| (c.clone(), v.clone()))
            .collect()
    } else {
        pairs
            .filter(|(c, _)| !not_comparable(c))
            .map(|(c, v)| (c.clone(), v.clone()))
            .collect()
    }
}

fn not_comparable(c: &ColumnMeta) -> bool {
    matches!(
        c.native_type.trim().to_ascii_lowercase().as_str(),
        "json" | "ntext" | "image" | "xml"
    )
}

fn where_clause(kind: DbKind, key: &[(ColumnMeta, Value)]) -> String {
    key.iter()
        .map(|(c, v)| {
            let name = quote_ident(kind, &c.name);
            if v.is_null() {
                format!("{name} IS NULL")
            } else {
                format!("{name} = {}", literal(kind, v, &c.data_type))
            }
        })
        .collect::<Vec<_>>()
        .join(" AND ")
}

/// The statement for one edit, ending without `;`. An update or delete with an empty key is an
/// error (it would hit the whole table). Computed columns are an error in an update and left out of
/// an insert, as are auto-increment columns given NULL (the engine generates them). Without a
/// primary key a single-row guard is added; see the module docs.
pub fn render(kind: DbKind, table: &TableRef, edit: &RowEdit) -> Result<String, EditError> {
    let t = qualified(kind, table);
    match edit {
        RowEdit::Insert { values } => {
            let cols: Vec<&(ColumnMeta, Value)> = values
                .iter()
                .filter(|(c, v)| !c.computed && !(c.auto_increment && v.is_null()))
                .collect();
            if cols.is_empty() {
                return Ok(match kind {
                    DbKind::MySql => format!("INSERT INTO {t} () VALUES ()"),
                    _ => format!("INSERT INTO {t} DEFAULT VALUES"),
                });
            }
            let names: Vec<String> = cols
                .iter()
                .map(|(c, _)| quote_ident(kind, &c.name))
                .collect();
            let vals: Vec<String> = cols
                .iter()
                .map(|(c, v)| literal(kind, v, &c.data_type))
                .collect();
            Ok(format!(
                "INSERT INTO {t} ({}) VALUES ({})",
                names.join(", "),
                vals.join(", ")
            ))
        }
        RowEdit::Update { key, changes } => {
            if key.is_empty() {
                return Err(EditError::EmptyKey);
            }
            if changes.is_empty() {
                return Err(EditError::NoChanges);
            }
            if let Some((c, _)) = changes.iter().find(|(c, _)| c.computed) {
                return Err(EditError::ComputedColumn(c.name.clone()));
            }
            let set = changes
                .iter()
                .map(|(c, v)| {
                    format!(
                        "{} = {}",
                        quote_ident(kind, &c.name),
                        literal(kind, v, &c.data_type)
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            let unique = edit.has_unique_key();
            let verb = if kind == DbKind::MsSql && !unique {
                "UPDATE TOP (1)"
            } else {
                "UPDATE"
            };
            Ok(format!(
                "{verb} {t} SET {set} {}",
                guarded_where(kind, &t, key, unique)
            ))
        }
        RowEdit::Delete { key } => {
            if key.is_empty() {
                return Err(EditError::EmptyKey);
            }
            let unique = edit.has_unique_key();
            let verb = if kind == DbKind::MsSql && !unique {
                "DELETE TOP (1) FROM"
            } else {
                "DELETE FROM"
            };
            Ok(format!(
                "{verb} {t} {}",
                guarded_where(kind, &t, key, unique)
            ))
        }
    }
}

/// `WHERE …`, with the single-row guard when the key is not unique (SQL Server's is `TOP (1)`, in
/// the verb).
fn guarded_where(kind: DbKind, t: &str, key: &[(ColumnMeta, Value)], unique: bool) -> String {
    let w = where_clause(kind, key);
    if unique {
        return format!("WHERE {w}");
    }
    match kind {
        DbKind::Postgres => format!("WHERE ctid IN (SELECT ctid FROM {t} WHERE {w} LIMIT 1)"),
        DbKind::Sqlite => format!("WHERE rowid IN (SELECT rowid FROM {t} WHERE {w} LIMIT 1)"),
        DbKind::MySql => format!("WHERE {w} LIMIT 1"),
        DbKind::MsSql => format!("WHERE {w}"),
    }
}

/// [`render`] each edit in order; the first failure stops the batch.
pub fn render_batch(
    kind: DbKind,
    table: &TableRef,
    edits: &[RowEdit],
) -> Result<Vec<String>, EditError> {
    edits.iter().map(|e| render(kind, table, e)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [DbKind; 4] = DbKind::ALL;

    fn col(kind: DbKind, name: &str, native: &str, pk: bool) -> ColumnMeta {
        let mut c = ColumnMeta::new(kind, name, native);
        c.is_pk = pk;
        c
    }

    fn lit(kind: DbKind, v: Value) -> String {
        literal(kind, &v, &DataType::Other(String::new()))
    }

    fn t(db: Option<&str>, schema: Option<&str>, name: &str) -> TableRef {
        TableRef::new(db, schema, name)
    }

    #[test]
    fn quote_ident_per_dialect() {
        assert_eq!(quote_ident(DbKind::Postgres, "a\"b"), "\"a\"\"b\"");
        assert_eq!(quote_ident(DbKind::Sqlite, "user"), "\"user\"");
        assert_eq!(quote_ident(DbKind::MySql, "a`b"), "`a``b`");
        assert_eq!(quote_ident(DbKind::MsSql, "a]b"), "[a]]b]");
        assert_eq!(quote_ident(DbKind::MsSql, "a b"), "[a b]");
    }

    #[test]
    fn qualified_follows_the_qualifier_rule() {
        let full = t(Some("d"), Some("s"), "n");
        assert_eq!(qualified(DbKind::Postgres, &full), "\"s\".\"n\"");
        assert_eq!(qualified(DbKind::MsSql, &full), "[d].[s].[n]");
        assert_eq!(
            qualified(DbKind::MySql, &t(Some("d"), None, "n")),
            "`d`.`n`"
        );
        assert_eq!(
            qualified(DbKind::Sqlite, &t(Some("aux"), None, "n")),
            "\"aux\".\"n\""
        );
        assert_eq!(
            qualified(DbKind::Sqlite, &t(Some("main"), None, "n")),
            "\"n\""
        );
        assert_eq!(
            qualified(DbKind::MsSql, &t(Some("d"), None, "n")),
            "[d]..[n]"
        );
        assert_eq!(
            qualified(DbKind::Postgres, &t(Some("d"), None, "n")),
            "\"n\""
        );
        for k in ALL {
            assert!(qualified(k, &t(None, None, "n")).contains('n'));
        }
    }

    #[test]
    fn text_quotes_are_doubled_everywhere() {
        for k in ALL {
            assert_eq!(lit(k, Value::Text("it's".into())), "'it''s'");
            assert_eq!(
                lit(k, Value::Text("'; DROP TABLE t; --".into())),
                "'''; DROP TABLE t; --'"
            );
            assert_eq!(lit(k, Value::Text(String::new())), "''");
        }
    }

    #[test]
    fn backslashes_are_literal_except_in_mysql() {
        let v = || Value::Text(r"a\b\'c".into());
        assert_eq!(lit(DbKind::MySql, v()), r"'a\\b\\''c'");
        assert_eq!(lit(DbKind::Postgres, v()), r"'a\b\''c'");
        assert_eq!(lit(DbKind::Sqlite, v()), r"'a\b\''c'");
        assert_eq!(lit(DbKind::MsSql, v()), r"'a\b\''c'");
    }

    #[test]
    fn mysql_trailing_backslash_cannot_escape_the_closing_quote() {
        assert_eq!(lit(DbKind::MySql, Value::Text("x\\".into())), r"'x\\'");
    }

    #[test]
    fn unicode_and_sql_server_n_prefix() {
        assert_eq!(
            lit(DbKind::MsSql, Value::Text("héllo 日本".into())),
            "N'héllo 日本'"
        );
        assert_eq!(lit(DbKind::MsSql, Value::Text("plain".into())), "'plain'");
        assert_eq!(
            lit(DbKind::Postgres, Value::Text("héllo 日本".into())),
            "'héllo 日本'"
        );
        assert_eq!(lit(DbKind::MySql, Value::Text("😀".into())), "'😀'");
    }

    #[test]
    fn nul_handling_per_dialect() {
        let v = || Value::Text("a\0b".into());
        assert_eq!(lit(DbKind::Postgres, v()), "'ab'");
        assert_eq!(lit(DbKind::MySql, v()), r"'a\0b'");
        assert_eq!(lit(DbKind::Sqlite, v()), "('a' || char(0) || 'b')");
        assert_eq!(lit(DbKind::MsSql, v()), "('a' + NCHAR(0) + 'b')");
        assert_eq!(
            lit(DbKind::MsSql, Value::Text("é\0'".into())),
            "(N'é' + NCHAR(0) + N'''')"
        );
    }

    #[test]
    fn mysql_control_z_is_escaped() {
        assert_eq!(
            lit(DbKind::MySql, Value::Text("a\u{1a}b".into())),
            r"'a\Zb'"
        );
    }

    #[test]
    fn newlines_pass_through() {
        for k in ALL {
            assert_eq!(lit(k, Value::Text("a\nb\r\n".into())), "'a\nb\r\n'");
        }
    }

    #[test]
    fn null_bool_int() {
        for k in ALL {
            assert_eq!(lit(k, Value::Null), "NULL");
            assert_eq!(lit(k, Value::Int(-42)), "-42");
            assert_eq!(lit(k, Value::Int(i64::MIN)), i64::MIN.to_string());
        }
        assert_eq!(lit(DbKind::Postgres, Value::Bool(true)), "TRUE");
        assert_eq!(lit(DbKind::MySql, Value::Bool(false)), "FALSE");
        assert_eq!(lit(DbKind::Sqlite, Value::Bool(true)), "1");
        assert_eq!(lit(DbKind::MsSql, Value::Bool(false)), "0");
    }

    #[test]
    fn bytes_per_dialect() {
        let v = || Value::Bytes(vec![0xde, 0xad, 0x00, 0x0f]);
        assert_eq!(lit(DbKind::Postgres, v()), "'\\xdead000f'::bytea");
        assert_eq!(lit(DbKind::MySql, v()), "X'dead000f'");
        assert_eq!(lit(DbKind::Sqlite, v()), "X'dead000f'");
        assert_eq!(lit(DbKind::MsSql, v()), "0xdead000f");
        assert_eq!(lit(DbKind::Sqlite, Value::Bytes(vec![])), "X''");
    }

    #[test]
    fn decimal_and_float() {
        assert_eq!(
            lit(DbKind::Postgres, Value::Decimal("-12.50".into())),
            "-12.50"
        );
        // not a number: quoted, never pasted raw
        assert_eq!(
            lit(DbKind::MySql, Value::Decimal("1; DROP TABLE t".into())),
            "'1; DROP TABLE t'"
        );
        assert_eq!(lit(DbKind::Sqlite, Value::Decimal("inf".into())), "'inf'");
        assert_eq!(lit(DbKind::MsSql, Value::Float(1.5)), "1.5");
        assert_eq!(lit(DbKind::MsSql, Value::Float(2.0)), "2.0");
        assert_eq!(
            lit(DbKind::Postgres, Value::Float(f64::NAN)),
            "'NaN'::float8"
        );
        assert_eq!(
            lit(DbKind::Postgres, Value::Float(f64::NEG_INFINITY)),
            "'-Infinity'::float8"
        );
        assert_eq!(lit(DbKind::Sqlite, Value::Float(f64::INFINITY)), "9e999");
        assert_eq!(lit(DbKind::MySql, Value::Float(f64::NAN)), "NULL");
    }

    #[test]
    fn temporal_literals() {
        let dt = || Value::DateTime("2024-05-01 10:20:30".into());
        assert_eq!(lit(DbKind::Postgres, dt()), "'2024-05-01 10:20:30'");
        assert_eq!(lit(DbKind::MsSql, dt()), "'2024-05-01T10:20:30'");
        assert_eq!(
            lit(DbKind::MySql, Value::Date("2024-05-01".into())),
            "'2024-05-01'"
        );
        assert_eq!(lit(DbKind::Sqlite, Value::Time("10:20".into())), "'10:20'");
        let tz = || Value::DateTimeTz("2024-05-01 10:20:30Z".into());
        assert_eq!(lit(DbKind::MySql, tz()), "'2024-05-01 10:20:30+00:00'");
        assert_eq!(lit(DbKind::MsSql, tz()), "'2024-05-01T10:20:30Z'");
    }

    #[test]
    fn uuid_and_json() {
        let u = "123e4567-e89b-12d3-a456-426614174000";
        assert_eq!(
            lit(DbKind::Postgres, Value::Uuid(u.into())),
            format!("'{u}'::uuid")
        );
        assert_eq!(lit(DbKind::MsSql, Value::Uuid(u.into())), format!("'{u}'"));
        assert_eq!(
            literal(DbKind::Postgres, &Value::Text(u.into()), &DataType::Uuid),
            format!("'{u}'::uuid")
        );
        // json takes the column's type: no cast
        assert_eq!(
            lit(DbKind::Postgres, Value::Json(r#"{"a":"it's"}"#.into())),
            r#"'{"a":"it''s"}'"#
        );
        assert_eq!(
            lit(DbKind::MySql, Value::Json(r#"{"a":"\n"}"#.into())),
            r#"'{"a":"\\n"}'"#
        );
    }

    #[test]
    fn select_basic_per_dialect() {
        let tb = t(Some("d"), Some("s"), "users");
        assert_eq!(
            select_table(DbKind::Postgres, &tb, None, "", "", 100, 0),
            "SELECT *\nFROM \"s\".\"users\"\nLIMIT 100"
        );
        assert_eq!(
            select_table(DbKind::MySql, &tb, None, "", "", 100, 200),
            "SELECT *\nFROM `d`.`users`\nLIMIT 100 OFFSET 200"
        );
        assert_eq!(
            select_table(
                DbKind::Sqlite,
                &t(Some("main"), None, "users"),
                None,
                "",
                "",
                10,
                0
            ),
            "SELECT *\nFROM \"users\"\nLIMIT 10"
        );
    }

    #[test]
    fn select_filter_order_and_columns() {
        let tb = t(None, Some("public"), "users");
        let cols = vec!["id".to_string(), "na\"me".to_string()];
        let q = select_table(
            DbKind::Postgres,
            &tb,
            Some(&cols),
            "  age > 18 ",
            " id DESC ",
            50,
            0,
        );
        assert_eq!(
            q,
            "SELECT \"id\", \"na\"\"me\"\nFROM \"public\".\"users\"\nWHERE age > 18\nORDER BY id DESC\nLIMIT 50"
        );
        // empty column slice means *
        assert!(select_table(DbKind::MySql, &tb, Some(&[]), "", "", 1, 0).starts_with("SELECT *"));
    }

    #[test]
    fn line_comment_in_filter_cannot_swallow_the_next_clause() {
        let q = select_table(
            DbKind::Sqlite,
            &t(None, None, "t"),
            None,
            "a = 1 -- only a",
            "a",
            5,
            0,
        );
        assert_eq!(
            q,
            "SELECT *\nFROM \"t\"\nWHERE a = 1 -- only a\nORDER BY a\nLIMIT 5"
        );
    }

    #[test]
    fn mssql_top_and_offset_fetch() {
        let tb = t(Some("db"), Some("dbo"), "T");
        assert_eq!(
            select_table(DbKind::MsSql, &tb, None, "x > 1", "", 100, 0),
            "SELECT TOP (100) *\nFROM [db].[dbo].[T]\nWHERE x > 1"
        );
        assert_eq!(
            select_table(DbKind::MsSql, &tb, None, "", "", 100, 200),
            "SELECT *\nFROM [db].[dbo].[T]\nORDER BY (SELECT NULL)\nOFFSET 200 ROWS FETCH NEXT 100 ROWS ONLY"
        );
        assert_eq!(
            select_table(DbKind::MsSql, &tb, None, "", "id", 100, 200),
            "SELECT *\nFROM [db].[dbo].[T]\nORDER BY id\nOFFSET 200 ROWS FETCH NEXT 100 ROWS ONLY"
        );
        assert!(select_table(DbKind::MsSql, &tb, None, "", "", 0, 5).contains("FETCH NEXT 1 ROWS"));
    }

    #[test]
    fn count_per_dialect() {
        let tb = t(None, Some("public"), "u");
        assert_eq!(
            count_table(DbKind::Postgres, &tb, ""),
            "SELECT COUNT(*)\nFROM \"public\".\"u\""
        );
        assert_eq!(
            count_table(DbKind::MsSql, &t(None, Some("dbo"), "u"), " a = 1 "),
            "SELECT COUNT_BIG(*)\nFROM [dbo].[u]\nWHERE a = 1"
        );
    }

    #[test]
    fn composed_selects_parse_in_every_dialect() {
        for k in ALL {
            for (limit, offset) in [(10, 0), (10, 30)] {
                for (f, o) in [
                    ("", ""),
                    ("id > 3", ""),
                    ("", "id"),
                    ("id > 3 AND n IS NOT NULL", "id DESC, n"),
                ] {
                    let q = select_table(k, &t(None, None, "t"), None, f, o, limit, offset);
                    let a = sql::analyze(&q, k.into());
                    assert!(a.error.is_none(), "{k:?}: {q}: {:?}", a.error);
                    assert_eq!(a.statements.len(), 1, "{q}");
                }
            }
            let a = sql::analyze(&count_table(k, &t(None, None, "t"), "x = 1"), k.into());
            assert!(a.error.is_none());
        }
    }

    #[test]
    fn fragments_valid() {
        for k in ALL {
            assert!(validate_fragment(k, "", "").is_ok());
            assert!(validate_fragment(k, "a = 1 AND b LIKE 'x;y%'", "a DESC").is_ok());
            assert!(validate_fragment(k, "a = 'it''s; fine' -- trailing", "").is_ok());
        }
    }

    #[test]
    fn fragment_syntax_error_is_located_in_the_fragment() {
        let e = validate_fragment(DbKind::Postgres, "a = = 1", "").unwrap_err();
        assert!(e.message.starts_with("WHERE:"), "{}", e.message);
        assert_eq!(e.line, 1);
        assert!(e.byte_offset <= "a = = 1".len());
        let e = validate_fragment(DbKind::Sqlite, "a = 1", "a DESC DESC DESC,").unwrap_err();
        assert!(e.message.starts_with("ORDER BY:"), "{}", e.message);
    }

    #[test]
    fn fragment_rejects_second_statement() {
        for k in ALL {
            let e = validate_fragment(k, "1=1; DROP TABLE t", "").unwrap_err();
            assert!(
                e.message.contains("one statement") || e.message.starts_with("WHERE"),
                "{k:?} {e:?}"
            );
            assert!(
                validate_fragment(k, "1=1", "id; DELETE FROM t").is_err(),
                "{k:?}"
            );
            assert!(validate_fragment(k, "1=1;", "").is_err(), "{k:?}");
            assert!(validate_fragment(k, "1=1; --", "").is_err(), "{k:?}");
        }
    }

    #[test]
    fn fragment_cannot_comment_out_the_rest() {
        for k in ALL {
            // an open block comment would eat the ORDER BY / LIMIT and everything after
            assert!(validate_fragment(k, "a = 1 /*", "").is_err(), "{k:?}");
            assert!(validate_fragment(k, "a = 'open", "").is_err(), "{k:?}");
        }
    }

    #[test]
    fn fragment_unbalanced_paren_is_rejected() {
        assert!(validate_fragment(DbKind::MySql, "(a = 1", "").is_err());
        assert!(validate_fragment(DbKind::MySql, "a = 1)", "").is_err());
    }

    #[test]
    fn fragment_double_limit_is_rejected() {
        assert!(validate_fragment(DbKind::Postgres, "", "id LIMIT 5").is_err());
    }

    fn users(kind: DbKind) -> Vec<ColumnMeta> {
        vec![
            col(kind, "id", "int", true),
            col(kind, "name", "varchar(20)", false),
            col(kind, "note", "varchar(20)", false),
        ]
    }

    fn row() -> Vec<Value> {
        vec![Value::Int(7), Value::Text("Ann".into()), Value::Null]
    }

    #[test]
    fn key_is_the_primary_key() {
        let cols = users(DbKind::Postgres);
        let k = key_for_row(&cols, &row());
        assert_eq!(k.len(), 1);
        assert_eq!(k[0].0.name, "id");
        assert_eq!(k[0].1, Value::Int(7));
        assert!(has_unique_key(&cols));
    }

    #[test]
    fn key_without_pk_is_every_comparable_column() {
        let mut cols = users(DbKind::Postgres);
        cols[0].is_pk = false;
        cols.push(col(DbKind::Postgres, "doc", "json", false));
        let mut r = row();
        r.push(Value::Json("{}".into()));
        let k = key_for_row(&cols, &r);
        assert_eq!(k.len(), 3);
        assert!(!has_unique_key(&cols));
    }

    #[test]
    fn update_by_pk_all_dialects() {
        let tb = t(Some("d"), Some("s"), "users");
        for k in ALL {
            let cols = users(k);
            let edit = RowEdit::Update {
                key: key_for_row(&cols, &row()),
                changes: vec![
                    (cols[1].clone(), Value::Text("O'Neil".into())),
                    (cols[2].clone(), Value::Null),
                ],
            };
            assert!(edit.has_unique_key());
            let sql = render(k, &tb, &edit).unwrap();
            let expect = format!(
                "UPDATE {} SET {} = 'O''Neil', {} = NULL WHERE {} = 7",
                qualified(k, &tb),
                quote_ident(k, "name"),
                quote_ident(k, "note"),
                quote_ident(k, "id")
            );
            assert_eq!(sql, expect);
            assert!(sql::analyze(&sql, k.into()).error.is_none(), "{sql}");
        }
    }

    #[test]
    fn mysql_update_escapes_backslash() {
        let cols = users(DbKind::MySql);
        let edit = RowEdit::Update {
            key: key_for_row(&cols, &row()),
            changes: vec![(cols[1].clone(), Value::Text(r"C:\tmp".into()))],
        };
        assert_eq!(
            render(DbKind::MySql, &t(Some("d"), None, "u"), &edit).unwrap(),
            r"UPDATE `d`.`u` SET `name` = 'C:\\tmp' WHERE `id` = 7"
        );
    }

    #[test]
    fn update_with_null_key_uses_is_null() {
        let kind = DbKind::Postgres;
        let mut cols = users(kind);
        cols[0].is_pk = false; // no pk: key is every column, `note` is NULL
        let edit = RowEdit::Update {
            key: key_for_row(&cols, &row()),
            changes: vec![(cols[1].clone(), Value::Text("Bob".into()))],
        };
        assert!(!edit.has_unique_key());
        let sql = render(kind, &t(None, Some("public"), "u"), &edit).unwrap();
        assert_eq!(
            sql,
            "UPDATE \"public\".\"u\" SET \"name\" = 'Bob' WHERE ctid IN (SELECT ctid FROM \"public\".\"u\" \
             WHERE \"id\" = 7 AND \"name\" = 'Ann' AND \"note\" IS NULL LIMIT 1)"
        );
        assert!(sql::analyze(&sql, kind.into()).error.is_none());
    }

    #[test]
    fn keyless_guard_per_dialect() {
        for k in ALL {
            let mut cols = users(k);
            cols[0].is_pk = false;
            let key = key_for_row(&cols, &row());
            let tb = t(None, Some("s"), "u");
            let up = render(
                k,
                &tb,
                &RowEdit::Update {
                    key: key.clone(),
                    changes: vec![(cols[1].clone(), Value::Null)],
                },
            )
            .unwrap();
            let del = render(k, &tb, &RowEdit::Delete { key }).unwrap();
            match k {
                DbKind::Postgres => {
                    assert!(up.contains("ctid IN") && del.contains("ctid IN"));
                }
                DbKind::Sqlite => {
                    assert!(up.contains("rowid IN") && del.contains("rowid IN"));
                }
                DbKind::MySql => {
                    assert!(up.ends_with("LIMIT 1") && del.ends_with("LIMIT 1"));
                }
                DbKind::MsSql => {
                    assert!(
                        up.starts_with("UPDATE TOP (1) ")
                            && del.starts_with("DELETE TOP (1) FROM ")
                    );
                }
            }
            // sqlparser has no `UPDATE TOP (n)` / `DELETE TOP (n)`; T-SQL does
            if k != DbKind::MsSql {
                for s in [&up, &del] {
                    assert!(sql::analyze(s, k.into()).error.is_none(), "{k:?}: {s}");
                }
            }
        }
    }

    #[test]
    fn delete_by_pk() {
        let cols = users(DbKind::MsSql);
        let edit = RowEdit::Delete {
            key: key_for_row(&cols, &row()),
        };
        assert_eq!(
            render(DbKind::MsSql, &t(Some("d"), Some("dbo"), "users"), &edit).unwrap(),
            "DELETE FROM [d].[dbo].[users] WHERE [id] = 7"
        );
        assert_eq!(
            render(
                DbKind::Sqlite,
                &t(None, None, "users"),
                &RowEdit::Delete {
                    key: key_for_row(&users(DbKind::Sqlite), &row())
                }
            )
            .unwrap(),
            "DELETE FROM \"users\" WHERE \"id\" = 7"
        );
    }

    #[test]
    fn composite_key() {
        let mut cols = users(DbKind::MySql);
        cols[1].is_pk = true;
        let edit = RowEdit::Delete {
            key: key_for_row(&cols, &row()),
        };
        assert_eq!(
            render(DbKind::MySql, &t(Some("d"), None, "u"), &edit).unwrap(),
            "DELETE FROM `d`.`u` WHERE `id` = 7 AND `name` = 'Ann'"
        );
    }

    #[test]
    fn insert_all_dialects() {
        let tb = t(Some("d"), Some("s"), "users");
        for k in ALL {
            let cols = users(k);
            let edit = RowEdit::Insert {
                values: vec![
                    (cols[0].clone(), Value::Int(1)),
                    (cols[1].clone(), Value::Text("a'b".into())),
                    (cols[2].clone(), Value::Null),
                ],
            };
            let sql = render(k, &tb, &edit).unwrap();
            assert_eq!(
                sql,
                format!(
                    "INSERT INTO {} ({}, {}, {}) VALUES (1, 'a''b', NULL)",
                    qualified(k, &tb),
                    quote_ident(k, "id"),
                    quote_ident(k, "name"),
                    quote_ident(k, "note")
                )
            );
            assert!(sql::analyze(&sql, k.into()).error.is_none(), "{sql}");
        }
    }

    #[test]
    fn insert_skips_computed_and_null_auto_increment() {
        let k = DbKind::MySql;
        let mut id = col(k, "id", "int", true);
        id.auto_increment = true;
        let mut calc = col(k, "total", "int", false);
        calc.computed = true;
        let name = col(k, "name", "varchar(9)", false);
        let edit = RowEdit::Insert {
            values: vec![
                (id.clone(), Value::Null),
                (calc, Value::Int(3)),
                (name.clone(), Value::Text("x".into())),
            ],
        };
        assert_eq!(
            render(k, &t(Some("d"), None, "u"), &edit).unwrap(),
            "INSERT INTO `d`.`u` (`name`) VALUES ('x')"
        );
        // an explicit id is written
        let edit = RowEdit::Insert {
            values: vec![(id, Value::Int(9))],
        };
        assert_eq!(
            render(k, &t(Some("d"), None, "u"), &edit).unwrap(),
            "INSERT INTO `d`.`u` (`id`) VALUES (9)"
        );
    }

    #[test]
    fn insert_with_nothing_uses_defaults() {
        let edit = RowEdit::Insert { values: vec![] };
        let tb = t(None, None, "u");
        assert_eq!(
            render(DbKind::Sqlite, &tb, &edit).unwrap(),
            "INSERT INTO \"u\" DEFAULT VALUES"
        );
        assert_eq!(
            render(DbKind::MySql, &tb, &edit).unwrap(),
            "INSERT INTO `u` () VALUES ()"
        );
    }

    #[test]
    fn unsafe_edits_are_errors() {
        let cols = users(DbKind::Sqlite);
        let tb = t(None, None, "u");
        let ch = vec![(cols[1].clone(), Value::Null)];
        assert_eq!(
            render(
                DbKind::Sqlite,
                &tb,
                &RowEdit::Update {
                    key: vec![],
                    changes: ch
                }
            ),
            Err(EditError::EmptyKey)
        );
        assert_eq!(
            render(DbKind::Sqlite, &tb, &RowEdit::Delete { key: vec![] }),
            Err(EditError::EmptyKey)
        );
        assert_eq!(
            render(
                DbKind::Sqlite,
                &tb,
                &RowEdit::Update {
                    key: key_for_row(&cols, &row()),
                    changes: vec![]
                }
            ),
            Err(EditError::NoChanges)
        );
        let mut calc = cols[1].clone();
        calc.computed = true;
        assert_eq!(
            render(
                DbKind::Sqlite,
                &tb,
                &RowEdit::Update {
                    key: key_for_row(&cols, &row()),
                    changes: vec![(calc, Value::Null)]
                }
            ),
            Err(EditError::ComputedColumn("name".into()))
        );
    }

    #[test]
    fn injection_in_values_and_names_stays_quoted() {
        for k in ALL {
            let mut cols = users(k);
            cols[1].name = "na\"me]`".to_string();
            let edit = RowEdit::Update {
                key: key_for_row(&cols, &row()),
                changes: vec![(cols[1].clone(), Value::Text("x'; DROP TABLE u; --".into()))],
            };
            let sql = render(k, &t(None, None, "u"), &edit).unwrap();
            let a = sql::analyze(&sql, k.into());
            assert!(a.error.is_none(), "{k:?}: {sql}");
            assert_eq!(a.statements.len(), 1, "{k:?}: {sql}");
        }
    }

    #[test]
    fn batch_renders_in_order_and_stops_on_error() {
        let cols = users(DbKind::Sqlite);
        let tb = t(None, None, "u");
        let ok = RowEdit::Delete {
            key: key_for_row(&cols, &row()),
        };
        let bad = RowEdit::Delete { key: vec![] };
        let out = render_batch(DbKind::Sqlite, &tb, &[ok.clone(), ok.clone()]).unwrap();
        assert_eq!(out.len(), 2);
        assert_eq!(
            render_batch(DbKind::Sqlite, &tb, &[ok, bad]),
            Err(EditError::EmptyKey)
        );
    }

    #[test]
    fn dialect_from_kind() {
        assert_eq!(Dialect::from(DbKind::MsSql), Dialect::MsSql);
        assert_eq!(Dialect::from(DbKind::Postgres), Dialect::Postgres);
    }
}
