//! The model — the value types every part of the crate and its callers share: the error, the
//! structure tree's nodes, columns, result sets and the per-call options. No driver, no runtime;
//! the types a message carries between the interface and the host.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::conn::{DbKind, ParseError};
use crate::value::{DataType, Value, map_native};

/// One error type for everything a driver can report.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum DbError {
    /// The config did not validate.
    #[error("invalid connection: {0}")]
    Config(#[from] ParseError),
    /// The engine, or an operation on it, is not implemented.
    #[error("{0}")]
    Unsupported(String),
    /// Could not reach or log in to the server, or open the file.
    #[error("cannot connect: {0}")]
    Connect(String),
    /// The server rejected a statement; the message is the server's own.
    #[error("{0}")]
    Query(String),
    /// The database / schema / table asked about does not exist.
    #[error("not found: {0}")]
    NotFound(String),
    /// The connection was lost mid-call; the caller reconnects.
    #[error("connection lost: {0}")]
    Disconnected(String),
    /// The statement was stopped by `CancelHandle::cancel`.
    #[error("the statement was cancelled")]
    Cancelled,
    /// The statement ran past [`ExecOptions::timeout`] and was stopped.
    #[error("the statement timed out")]
    Timeout,
    /// The read-only guard refused the statement before the server ran it (it writes, it is more
    /// than one statement, or a transaction is in the way). Server-side refusals inside the
    /// read-only transaction come back as [`DbError::Query`] with the server's message.
    #[error("read-only: {0}")]
    ReadOnly(String),
}

pub type Result<T, E = DbError> = std::result::Result<T, E>;

/// A node kind in the structure tree. `Database` and `Schema` name the container levels;
/// `Connection::objects` returns only the rest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ObjectKind {
    Database,
    Schema,
    Table,
    View,
    /// Postgres only.
    MaterializedView,
    /// SQL Server only. Its target is in [`DbObject::target`].
    Synonym,
}

impl ObjectKind {
    /// Singular label (`"Table"`).
    pub fn label(self) -> &'static str {
        match self {
            ObjectKind::Database => "Database",
            ObjectKind::Schema => "Schema",
            ObjectKind::Table => "Table",
            ObjectKind::View => "View",
            ObjectKind::MaterializedView => "Materialized view",
            ObjectKind::Synonym => "Synonym",
        }
    }

    /// The tree's group heading (`"Tables"`).
    pub fn group_label(self) -> &'static str {
        match self {
            ObjectKind::Database => "Databases",
            ObjectKind::Schema => "Schemas",
            ObjectKind::Table => "Tables",
            ObjectKind::View => "Views",
            ObjectKind::MaterializedView => "Materialized views",
            ObjectKind::Synonym => "Synonyms",
        }
    }

    /// Whether rows can be selected from it (opens a table-data tab). A synonym counts: the
    /// server resolves it.
    pub fn is_relation(self) -> bool {
        !matches!(self, ObjectKind::Database | ObjectKind::Schema)
    }

    /// Whether row edits (UPDATE/INSERT/DELETE) are offered. Views are read-only.
    pub fn is_editable(self) -> bool {
        self == ObjectKind::Table
    }
}

impl DbKind {
    /// Whether the tree has a schema level under each database (Postgres, SQL Server).
    pub fn has_schemas(self) -> bool {
        matches!(self, DbKind::Postgres | DbKind::MsSql)
    }

    /// The schema to expand by default: `public` / `dbo`; `None` without a schema level.
    pub fn default_schema(self) -> Option<&'static str> {
        match self {
            DbKind::Postgres => Some("public"),
            DbKind::MsSql => Some("dbo"),
            DbKind::MySql | DbKind::Sqlite => None,
        }
    }

    /// The object groups the tree shows under a schema (or database), in display order.
    pub fn object_kinds(self) -> &'static [ObjectKind] {
        match self {
            DbKind::Postgres => &[
                ObjectKind::Table,
                ObjectKind::View,
                ObjectKind::MaterializedView,
            ],
            DbKind::MySql | DbKind::Sqlite => &[ObjectKind::Table, ObjectKind::View],
            DbKind::MsSql => &[ObjectKind::Table, ObjectKind::View, ObjectKind::Synonym],
        }
    }
}

/// A relation, by name as the catalog reports it — unquoted, case preserved.
///
/// Which parts a statement uses is the SQL builder's call, not the caller's: Postgres drops
/// `database` (no cross-database queries), SQL Server writes all three, MySQL writes
/// `database.name`, SQLite writes `database.name` (`main.t`, `aux.t`). Drivers fill every level
/// the engine has: `database` always, `schema` only where [`DbKind::has_schemas`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct TableRef {
    pub database: Option<String>,
    pub schema: Option<String>,
    pub name: String,
}

impl TableRef {
    pub fn new(database: Option<&str>, schema: Option<&str>, name: &str) -> Self {
        Self {
            database: database.map(str::to_string),
            schema: schema.map(str::to_string),
            name: name.to_string(),
        }
    }
}

impl std::fmt::Display for TableRef {
    /// `db.schema.name`, unquoted — for labels and tab titles only, never for SQL.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for part in [&self.database, &self.schema].into_iter().flatten() {
            write!(f, "{part}.")?;
        }
        f.write_str(&self.name)
    }
}

/// An entry of `Connection::objects`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DbObject {
    /// Never `Database` or `Schema`.
    pub kind: ObjectKind,
    pub name: String,
    pub schema: Option<String>,
    pub database: Option<String>,
    /// Synonym target, as far as the server resolves it; `None` for other kinds or when the
    /// target is not a relation.
    pub target: Option<TableRef>,
    /// The object's comment / description, when the engine stores one.
    pub comment: Option<String>,
    /// The engine's **approximate** row count (statistics, not `count(*)`) for tables and
    /// materialized views; `None` for views, synonyms, and tables never analyzed. See the module
    /// docs, "Approximate row counts".
    pub approx_rows: Option<u64>,
}

impl DbObject {
    /// The [`TableRef`] for `Connection::columns` and the SQL builder.
    pub fn table_ref(&self) -> TableRef {
        TableRef {
            database: self.database.clone(),
            schema: self.schema.clone(),
            name: self.name.clone(),
        }
    }
}

/// One column — of a table (`Connection::columns`) or of a result ([`ResultSet::columns`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ColumnMeta {
    pub name: String,
    /// The engine's type as declared, with its arguments: `varchar(255)`, `numeric(10,2)`,
    /// `int unsigned`, `enum('a','b')`, `nvarchar(max)`. Shown to the user as-is.
    pub native_type: String,
    /// [`map_native`] of `native_type` — what validation and the SQL builder use. A driver may
    /// refine it (Postgres enums: `native_type` is the type name, the driver fills
    /// `DataType::Enum` with the labels from `pg_enum`).
    pub data_type: DataType,
    /// Result-set columns whose nullability is unknown say `true`.
    pub nullable: bool,
    /// Part of the primary key. Edits are keyed by these columns, or by every column when a
    /// table has none. Always `false` in a result set.
    pub is_pk: bool,
    /// The default expression as the catalog prints it (`nextval('t_id_seq'::regclass)`, `0`,
    /// `CURRENT_TIMESTAMP`); `None` when there is none.
    pub default: Option<String>,
    /// Identity / `AUTO_INCREMENT` / `serial` / SQLite `INTEGER PRIMARY KEY`: an INSERT may leave
    /// it out.
    pub auto_increment: bool,
    /// Computed / generated column: never written by UPDATE or INSERT.
    pub computed: bool,
}

impl ColumnMeta {
    /// A column with `data_type` mapped from `native_type`, nullable, not a key, no default.
    pub fn new(kind: DbKind, name: &str, native_type: &str) -> Self {
        Self {
            name: name.to_string(),
            native_type: native_type.to_string(),
            data_type: map_native(kind, native_type),
            nullable: true,
            is_pk: false,
            default: None,
            auto_increment: false,
            computed: false,
        }
    }

    /// The length limit of a text, char or binary type, if any.
    pub fn max_len(&self) -> Option<u32> {
        self.data_type.max_len()
    }
}

/// Rows of a query.
///
/// Every cell is the [`Value`] variant that matches its column's `data_type`, in the text forms
/// [`crate::value::parse_cell`] produces (dates `YYYY-MM-DD`, datetimes `YYYY-MM-DD HH:MM:SS[.f]`,
/// uuids lowercase hyphenated); a cell the driver cannot convert goes out as `Value::Text`.
/// Every row has exactly `columns.len()` cells.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ResultSet {
    pub columns: Vec<ColumnMeta>,
    pub rows: Vec<Vec<Value>>,
    /// The server had more rows than the `limit` passed to `Connection::query`.
    pub truncated: bool,
}

/// Result of a statement that returns no rows.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecOutcome {
    /// Rows inserted/updated/deleted; `0` for DDL and for engines that do not report it.
    pub rows_affected: u64,
}

/// How `Connection::query_with` / `Connection::execute_with` run one statement. The default
/// is what `query(sql, None)` / `execute(sql)` do: read-write, no timeout, every row.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ExecOptions {
    /// Run under the engine's read-only guard (the `driver` module docs, "Read-only"). Forced on by a
    /// connection opened read-only.
    pub read_only: bool,
    /// Stop the statement after this long ([`DbError::Timeout`]).
    pub timeout: Option<Duration>,
    /// Read at most this many rows; `truncated` says there were more. `execute_with` ignores it.
    pub row_limit: Option<usize>,
}

impl ExecOptions {
    /// Read-only, no timeout, every row.
    pub fn read_only() -> Self {
        Self {
            read_only: true,
            ..Self::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exec_options() {
        assert_eq!(
            ExecOptions::default(),
            ExecOptions {
                read_only: false,
                timeout: None,
                row_limit: None
            }
        );
        assert!(ExecOptions::read_only().read_only);
    }

    #[test]
    fn table_ref_display() {
        assert_eq!(
            TableRef::new(Some("db"), Some("dbo"), "T").to_string(),
            "db.dbo.T"
        );
        assert_eq!(TableRef::new(None, None, "t").to_string(), "t");
    }

    #[test]
    fn model_serde_round_trip() {
        let rs = ResultSet {
            columns: vec![ColumnMeta::new(DbKind::Sqlite, "id", "INTEGER")],
            rows: vec![vec![Value::Int(1)], vec![Value::Null]],
            truncated: true,
        };
        let json = serde_json::to_string(&rs).unwrap();
        assert_eq!(serde_json::from_str::<ResultSet>(&json).unwrap(), rs);
    }
}
