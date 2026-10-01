//! `sql` — parser-backed analysis of SQL text, for the editor.
//!
//! [`analyze`] splits a script into statements (by byte range), parses each one with the matching
//! `sqlparser` dialect, and reports per statement a [`StatementClass`], a `kind` label, the tables
//! it touches, and — for the first statement that does not parse — a [`SqlError`] with a position.
//! [`statement_at`] answers "which statement is under the cursor".
//!
//! Splitting is token-based (the `sqlparser` tokenizer), so `;` inside strings, comments, quoted
//! identifiers and dollar-quoted bodies never splits. When the tokenizer itself fails (typically an
//! unterminated string while typing) a small hand-written scanner splits instead, and the statements
//! are classified from their leading keywords only.
//!
//! A statement that fails to parse is still classified, from its keywords, and conservatively: a
//! broken `DELETE …` is still a write. `PRAGMA` and `DO` are never parsed (`sqlparser` rejects
//! common forms of both) and so never report a syntax error. Known limit: a `BEGIN … END` body with inner `;` splits at
//! them; recursion protection in `sqlparser` is off (it would drag in a second `object` crate).
//!
//! [`check_read_only`] is the fail-closed read-only check (layer 1 of `_docs/readonly.md`), in the
//! `readonly` submodule; its function allowlists live in `functions`.

mod functions;
mod readonly;
#[cfg(test)]
mod readonly_corpus;

pub use readonly::{ReadOnlyRule, ReadOnlyVerdict, Violation, check_read_only};

use std::ops::{ControlFlow, Range};

use sqlparser::ast::{ObjectName, Query, SetExpr, Statement, Visit, Visitor};
use sqlparser::dialect::{
    Dialect as SpDialect, GenericDialect, MsSqlDialect, MySqlDialect, PostgreSqlDialect,
    SQLiteDialect,
};
use sqlparser::parser::{Parser, ParserError};
use sqlparser::tokenizer::{Location, Token, TokenWithSpan, Tokenizer};

/// The SQL dialect to tokenize and parse with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Dialect {
    Postgres,
    MySql,
    Sqlite,
    MsSql,
    Generic,
}

impl Dialect {
    fn parser_dialect(self) -> Box<dyn SpDialect> {
        match self {
            Dialect::Postgres => Box::new(PostgreSqlDialect {}),
            Dialect::MySql => Box::new(MySqlDialect {}),
            Dialect::Sqlite => Box::new(SQLiteDialect {}),
            Dialect::MsSql => Box::new(MsSqlDialect {}),
            Dialect::Generic => Box::new(GenericDialect {}),
        }
    }
}

/// What a statement does to the database.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StatementClass {
    /// `SELECT` (incl. `WITH … SELECT`), `EXPLAIN`, `SHOW`, `DESCRIBE`, `PRAGMA`, `VALUES`.
    Read,
    /// DML: `INSERT`, `UPDATE`, `DELETE`, `MERGE`, `TRUNCATE`, `SELECT … INTO`, a CTE with DML.
    Write,
    /// `CREATE`, `ALTER`, `DROP`, `RENAME`, `GRANT`, `REVOKE`.
    Ddl,
    /// `BEGIN`, `COMMIT`, `ROLLBACK`, `SAVEPOINT`, …
    Transaction,
    /// Anything else (`SET`, `USE`, `CALL`, `EXEC`, …): not asserted read-only, not asserted a write.
    Other,
}

impl StatementClass {
    /// Whether the statement changes data or schema (`Write` or `Ddl`).
    pub fn mutates(&self) -> bool {
        matches!(self, StatementClass::Write | StatementClass::Ddl)
    }
}

/// One statement of a script.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatementInfo {
    pub class: StatementClass,
    /// A short label: `"SELECT"`, `"UPDATE"`, `"CREATE TABLE"`, `"SELECT INTO"`, …
    pub kind: String,
    /// The statement's bytes in the source, without surrounding whitespace, comments or the `;`.
    pub byte_range: Range<usize>,
    /// Tables referenced (best effort; CTE names excluded; unquoted, dot-joined, deduplicated).
    /// Empty when the statement did not parse.
    pub tables: Vec<String>,
}

/// The first syntax error of a script. `line` and `column` are 1-based (column in characters).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SqlError {
    pub message: String,
    pub line: usize,
    pub column: usize,
    pub byte_offset: usize,
}

/// The result of [`analyze`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Analysis {
    pub statements: Vec<StatementInfo>,
    pub error: Option<SqlError>,
}

impl Analysis {
    /// Any statement writes or changes the schema.
    pub fn mutates(&self) -> bool {
        self.statements.iter().any(|s| s.class.mutates())
    }

    /// Every statement is a [`StatementClass::Read`] (vacuously true for an empty script).
    pub fn is_read_only(&self) -> bool {
        self.statements
            .iter()
            .all(|s| s.class == StatementClass::Read)
    }
}

/// Analyze `sql`: statements with class/kind/tables, and the first syntax error if any.
pub fn analyze(sql: &str, dialect: Dialect) -> Analysis {
    let d = dialect.parser_dialect();
    let lines = Lines::new(sql);
    let split = split(sql, &lines, &*d, dialect);
    let mut error = split.error;
    let mut statements = Vec::with_capacity(split.pieces.len());
    for piece in &split.pieces {
        let words = words_of(&sql[piece.range.clone()]);
        let mut class = class_of_words(&words);
        let mut kind = kind_of_words(&words);
        let mut tables = Vec::new();
        // Statements `sqlparser` cannot represent (`PRAGMA t(name)`, `DO $$…$$`) are not parsed.
        let opaque = matches!(words.first().map(String::as_str), Some("PRAGMA" | "DO"));
        if let (Some(tokens), false) = (&split.tokens, opaque) {
            let chunk = tokens[piece.tokens.clone()].to_vec();
            match Parser::new(&*d)
                .with_tokens_with_locations(chunk)
                .parse_statements()
            {
                Ok(stmts) => {
                    if let Some(stmt) = stmts.first() {
                        if let Statement::Query(q) = stmt {
                            class = if query_mutates(q) {
                                StatementClass::Write
                            } else {
                                StatementClass::Read
                            };
                            kind = setexpr_kind(&q.body).to_string();
                        }
                        tables = tables_of(stmt);
                    }
                }
                Err(e) => {
                    if error.is_none() {
                        error = Some(parser_error(&e, &lines, piece.range.end));
                    }
                }
            }
        }
        statements.push(StatementInfo {
            class,
            kind,
            byte_range: piece.range.clone(),
            tables,
        });
    }
    Analysis { statements, error }
}

/// The byte range of the statement the cursor is in. A cursor right after a statement's `;`, in a
/// comment before a statement, or in the gap between statements belongs to the nearest statement
/// at or after it; past the last one it belongs to the last. `None` only when there is no statement.
pub fn statement_at(sql: &str, dialect: Dialect, byte_offset: usize) -> Option<Range<usize>> {
    let d = dialect.parser_dialect();
    let lines = Lines::new(sql);
    let split = split(sql, &lines, &*d, dialect);
    split
        .pieces
        .iter()
        .find(|p| byte_offset <= p.end)
        .or(split.pieces.last())
        .map(|p| p.range.clone())
}

// ---- splitting ------------------------------------------------------------------------------

/// One statement's place in the source.
struct Piece {
    /// Trimmed byte range (first to last significant token), `;` excluded.
    range: Range<usize>,
    /// End of the terminating `;` if there is one, else `range.end`.
    end: usize,
    /// The statement's tokens in `Split::tokens` (empty for the hand-written scanner).
    tokens: Range<usize>,
}

struct Split {
    pieces: Vec<Piece>,
    /// `None` when the tokenizer failed.
    tokens: Option<Vec<TokenWithSpan>>,
    error: Option<SqlError>,
}

fn split(sql: &str, lines: &Lines, d: &dyn SpDialect, dialect: Dialect) -> Split {
    match Tokenizer::new(d, sql).tokenize_with_location() {
        Ok(tokens) => {
            let pieces = split_tokens(&tokens, lines);
            Split {
                pieces,
                tokens: Some(tokens),
                error: None,
            }
        }
        Err(e) => {
            let offset = lines.offset(e.location);
            let (line, column) = lines.position(offset);
            let error = SqlError {
                message: e.message,
                line,
                column,
                byte_offset: offset,
            };
            Split {
                pieces: scan_split(sql, dialect),
                tokens: None,
                error: Some(error),
            }
        }
    }
}

fn is_insignificant(t: &Token) -> bool {
    matches!(t, Token::Whitespace(_) | Token::EOF)
}

fn split_tokens(tokens: &[TokenWithSpan], lines: &Lines) -> Vec<Piece> {
    let mut pieces = Vec::new();
    // (first significant index, last significant index)
    let mut cur: Option<(usize, usize)> = None;
    for (i, t) in tokens.iter().enumerate() {
        if t.token == Token::SemiColon {
            if let Some((a, b)) = cur.take() {
                pieces.push(Piece {
                    range: lines.offset(tokens[a].span.start)..lines.token_end(&tokens[b]),
                    end: lines.token_end(t),
                    tokens: a..i,
                });
            }
        } else if !is_insignificant(&t.token) {
            cur = Some(match cur {
                Some((a, _)) => (a, i),
                None => (i, i),
            });
        }
    }
    if let Some((a, b)) = cur {
        let range = lines.offset(tokens[a].span.start)..lines.token_end(&tokens[b]);
        pieces.push(Piece {
            end: range.end,
            range,
            tokens: a..tokens.len(),
        });
    }
    pieces
}

/// Fallback splitter for text the tokenizer rejects. Respects `'…'`, `"…"`, `` `…` ``, `[…]`
/// (SQL Server), `--`/`#`/`/* */` comments and `$tag$…$tag$` (Postgres).
fn scan_split(sql: &str, dialect: Dialect) -> Vec<Piece> {
    let b = sql.as_bytes();
    let mut pieces = Vec::new();
    let (mut first, mut last) = (None::<usize>, 0usize);
    let mut i = 0;
    let mut close = |first: &mut Option<usize>, last: usize, end: usize| {
        if let Some(a) = first.take() {
            pieces.push(Piece {
                range: a..last,
                end,
                tokens: 0..0,
            });
        }
    };
    while i < b.len() {
        let c = b[i];
        let start = i;
        let mut mark = true;
        match c {
            b';' => {
                close(&mut first, last, i + 1);
                i += 1;
                continue;
            }
            c if c.is_ascii_whitespace() => {
                mark = false;
                i += 1;
            }
            b'-' if b.get(i + 1) == Some(&b'-') => {
                mark = false;
                i = sql[i..].find('\n').map_or(b.len(), |n| i + n);
            }
            b'#' if dialect == Dialect::MySql => {
                mark = false;
                i = sql[i..].find('\n').map_or(b.len(), |n| i + n);
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                mark = false;
                i = sql[i + 2..].find("*/").map_or(b.len(), |n| i + 2 + n + 2);
            }
            b'\'' | b'"' | b'`' => {
                i = skip_quoted(b, i, c, c == b'\'' && dialect == Dialect::MySql)
            }
            b'[' if dialect == Dialect::MsSql => i = skip_quoted(b, i, b']', false),
            b'$' if dialect == Dialect::Postgres => {
                let tag_end = b[i + 1..]
                    .iter()
                    .position(|&x| !(x.is_ascii_alphanumeric() || x == b'_'))
                    .map(|n| i + 1 + n);
                match tag_end {
                    Some(e)
                        if b[e] == b'$' && !b[i + 1..e].first().is_some_and(u8::is_ascii_digit) =>
                    {
                        let tag = &sql[i..=e];
                        i = sql[e + 1..]
                            .find(tag)
                            .map_or(b.len(), |n| e + 1 + n + tag.len());
                    }
                    _ => i += 1,
                }
            }
            _ => i += 1,
        }
        if mark {
            first.get_or_insert(start);
            last = i;
        }
    }
    close(&mut first, last, last);
    pieces
}

fn skip_quoted(b: &[u8], start: usize, close: u8, backslash: bool) -> usize {
    let mut i = start + 1;
    while i < b.len() {
        if backslash && b[i] == b'\\' {
            i += 2;
            continue;
        }
        if b[i] == close {
            if b.get(i + 1) == Some(&close) && close != b']' {
                i += 2;
                continue;
            }
            return i + 1;
        }
        i += 1;
    }
    b.len()
}

// ---- positions ------------------------------------------------------------------------------

/// Line/column (1-based, columns in characters) <-> byte offset.
struct Lines<'a> {
    src: &'a str,
    starts: Vec<usize>,
}

impl<'a> Lines<'a> {
    fn new(src: &'a str) -> Self {
        let mut starts = vec![0];
        starts.extend(src.match_indices('\n').map(|(i, _)| i + 1));
        Lines { src, starts }
    }

    /// Byte offset of a tokenizer location; a null location (line 0, used by EOF) is the end.
    fn offset(&self, loc: Location) -> usize {
        if loc.line == 0 {
            return self.src.len();
        }
        let line = (loc.line as usize - 1).min(self.starts.len() - 1);
        let start = self.starts[line];
        let col = (loc.column as usize).saturating_sub(1);
        self.src[start..]
            .char_indices()
            .nth(col)
            .map_or(self.src.len(), |(i, _)| start + i)
    }

    /// Byte offset just past a token.
    fn token_end(&self, t: &TokenWithSpan) -> usize {
        self.offset(t.span.end)
    }

    fn position(&self, offset: usize) -> (usize, usize) {
        let offset = offset.min(self.src.len());
        let line = self.starts.partition_point(|&s| s <= offset) - 1;
        let col = self.src[self.starts[line]..offset].chars().count() + 1;
        (line + 1, col)
    }
}

/// Convert a `sqlparser` error into a [`SqlError`]; its position is in the message text
/// (`… at Line: 1, Column: 8`). `fallback` is the offset used when there is none (EOF).
fn parser_error(e: &ParserError, lines: &Lines, fallback: usize) -> SqlError {
    let raw = match e {
        ParserError::TokenizerError(m) | ParserError::ParserError(m) => m.clone(),
        other => other.to_string(),
    };
    let raw = raw.strip_prefix("sql parser error: ").unwrap_or(&raw);
    let (message, offset) = match raw.rfind(" at Line: ") {
        Some(p) => {
            let tail = &raw[p + " at Line: ".len()..];
            let mut nums = tail.split(", Column: ").map(|s| s.trim().parse::<u64>());
            match (nums.next(), nums.next()) {
                (Some(Ok(line)), Some(Ok(column))) if line > 0 => (
                    raw[..p].to_string(),
                    lines.offset(Location { line, column }),
                ),
                _ => (raw[..p].to_string(), fallback),
            }
        }
        None => (raw.to_string(), fallback),
    };
    let (line, column) = lines.position(offset);
    SqlError {
        message,
        line,
        column,
        byte_offset: offset.min(lines.src.len()),
    }
}

// ---- classification -------------------------------------------------------------------------

/// Unquoted-ish words of a statement's text, upper-cased.
fn words_of(text: &str) -> Vec<String> {
    text.split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .filter(|w| !w.is_empty())
        .map(str::to_uppercase)
        .collect()
}

fn has(words: &[String], set: &[&str]) -> bool {
    words.iter().any(|w| set.contains(&w.as_str()))
}

fn class_of_words(words: &[String]) -> StatementClass {
    let Some(first) = words.first() else {
        return StatementClass::Other;
    };
    match first.as_str() {
        "SELECT" if has(&words[1..], &["INTO"]) => StatementClass::Write,
        "SELECT" | "SHOW" | "DESCRIBE" | "DESC" | "PRAGMA" | "VALUES" | "TABLE" => {
            StatementClass::Read
        }
        "WITH"
            if has(
                &words[1..],
                &["INSERT", "UPDATE", "DELETE", "MERGE", "INTO"],
            ) =>
        {
            StatementClass::Write
        }
        "WITH" => StatementClass::Read,
        "EXPLAIN" => {
            // `EXPLAIN ANALYZE <stmt>` runs the statement.
            let mut analyzed = false;
            for (i, w) in words.iter().enumerate().skip(1) {
                match w.as_str() {
                    "ANALYZE" | "ANALYSE" => analyzed = true,
                    "SELECT" | "WITH" | "INSERT" | "UPDATE" | "DELETE" | "MERGE" | "REPLACE"
                    | "CREATE" | "ALTER" | "DROP" | "TRUNCATE" => {
                        return if analyzed {
                            class_of_words(&words[i..])
                        } else {
                            StatementClass::Read
                        };
                    }
                    _ => {}
                }
            }
            StatementClass::Read
        }
        "INSERT" | "UPDATE" | "DELETE" | "MERGE" | "TRUNCATE" | "REPLACE" | "UPSERT" | "COPY" => {
            StatementClass::Write
        }
        "CREATE" | "ALTER" | "DROP" | "RENAME" | "GRANT" | "REVOKE" => StatementClass::Ddl,
        "BEGIN" | "START" | "COMMIT" | "ROLLBACK" | "SAVEPOINT" | "RELEASE" | "END" | "ABORT" => {
            StatementClass::Transaction
        }
        _ => StatementClass::Other,
    }
}

fn kind_of_words(words: &[String]) -> String {
    let Some(first) = words.first() else {
        return String::new();
    };
    if matches!(first.as_str(), "CREATE" | "ALTER" | "DROP") {
        const MODIFIERS: &[&str] = &[
            "OR",
            "REPLACE",
            "TEMP",
            "TEMPORARY",
            "GLOBAL",
            "LOCAL",
            "UNIQUE",
            "UNLOGGED",
            "VIRTUAL",
            "CLUSTERED",
            "NONCLUSTERED",
            "RECURSIVE",
            "EXTERNAL",
        ];
        let mut rest = words[1..]
            .iter()
            .filter(|w| !MODIFIERS.contains(&w.as_str()));
        if let Some(obj) = rest.next() {
            if obj == "MATERIALIZED"
                && let Some(next) = rest.next()
            {
                return format!("{first} {obj} {next}");
            }
            return format!("{first} {obj}");
        }
    }
    first.clone()
}

fn query_mutates(q: &Query) -> bool {
    q.with
        .as_ref()
        .is_some_and(|w| w.cte_tables.iter().any(|c| query_mutates(&c.query)))
        || setexpr_mutates(&q.body)
}

fn setexpr_mutates(e: &SetExpr) -> bool {
    match e {
        SetExpr::Select(s) => s.into.is_some(),
        SetExpr::Query(q) => query_mutates(q),
        SetExpr::SetOperation { left, right, .. } => {
            setexpr_mutates(left) || setexpr_mutates(right)
        }
        SetExpr::Values(_) | SetExpr::Table(_) => false,
        _ => true, // Insert / Update / Delete / Merge
    }
}

fn setexpr_kind(e: &SetExpr) -> &'static str {
    match e {
        SetExpr::Select(s) if s.into.is_some() => "SELECT INTO",
        SetExpr::Select(_) => "SELECT",
        SetExpr::Query(q) => setexpr_kind(&q.body),
        SetExpr::SetOperation { left, .. } => setexpr_kind(left),
        SetExpr::Values(_) => "VALUES",
        SetExpr::Insert(_) => "INSERT",
        SetExpr::Update(_) => "UPDATE",
        SetExpr::Delete(_) => "DELETE",
        SetExpr::Merge(_) => "MERGE",
        SetExpr::Table(_) => "TABLE",
    }
}

// ---- tables ---------------------------------------------------------------------------------

#[derive(Default)]
struct TableCollector {
    tables: Vec<String>,
    ctes: Vec<String>,
}

impl Visitor for TableCollector {
    type Break = ();

    fn pre_visit_query(&mut self, query: &Query) -> ControlFlow<()> {
        if let Some(with) = &query.with {
            self.ctes
                .extend(with.cte_tables.iter().map(|c| c.alias.name.value.clone()));
        }
        ControlFlow::Continue(())
    }

    fn pre_visit_relation(&mut self, relation: &ObjectName) -> ControlFlow<()> {
        let parts: Vec<&str> = relation
            .0
            .iter()
            .filter_map(|p| p.as_ident().map(|i| i.value.as_str()))
            .collect();
        if !parts.is_empty() {
            self.tables.push(parts.join("."));
        }
        ControlFlow::Continue(())
    }
}

fn tables_of(stmt: &Statement) -> Vec<String> {
    let mut c = TableCollector::default();
    let _ = stmt.visit(&mut c);
    let mut out: Vec<String> = Vec::new();
    for t in c.tables {
        if !c.ctes.contains(&t) && !out.contains(&t) {
            out.push(t);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use Dialect::*;
    use StatementClass::*;

    fn one(sql: &str, d: Dialect) -> StatementInfo {
        let a = analyze(sql, d);
        assert_eq!(a.error, None, "{sql}");
        assert_eq!(a.statements.len(), 1, "{sql}");
        a.statements.into_iter().next().unwrap()
    }

    fn class(sql: &str, d: Dialect) -> StatementClass {
        one(sql, d).class
    }

    fn texts<'a>(sql: &'a str, a: &Analysis) -> Vec<&'a str> {
        a.statements
            .iter()
            .map(|s| &sql[s.byte_range.clone()])
            .collect()
    }

    #[test]
    fn empty_and_comment_only() {
        for sql in [
            "",
            "   \n\t",
            "-- just a comment",
            "/* a; b */",
            "-- x\n/* y */ ;;",
        ] {
            let a = analyze(sql, Generic);
            assert!(a.statements.is_empty(), "{sql:?}");
            assert_eq!(a.error, None);
            assert!(a.is_read_only() && !a.mutates());
            assert_eq!(statement_at(sql, Generic, 0), None);
        }
    }

    #[test]
    fn selects_are_read() {
        let s = one("SELECT a, b FROM t WHERE a > 1", Postgres);
        assert_eq!((s.class, s.kind.as_str()), (Read, "SELECT"));
        assert_eq!(s.tables, ["t"]);
        assert_eq!(class("SELECT 1 UNION SELECT 2", Sqlite), Read);
        assert_eq!(class("VALUES (1), (2)", Postgres), Read);
    }

    #[test]
    fn dml_is_write() {
        for (sql, kind) in [
            ("INSERT INTO t (a) VALUES (1)", "INSERT"),
            ("UPDATE t SET a = 1 WHERE b = 2", "UPDATE"),
            ("DELETE FROM t WHERE a = 1", "DELETE"),
            ("TRUNCATE TABLE t", "TRUNCATE"),
        ] {
            let s = one(sql, Postgres);
            assert_eq!((s.class, s.kind.as_str()), (Write, kind), "{sql}");
            assert!(s.class.mutates());
        }
        assert_eq!(
            class(
                "MERGE INTO t USING s ON t.id = s.id WHEN MATCHED THEN DELETE",
                MsSql
            ),
            Write
        );
    }

    #[test]
    fn ddl_kinds() {
        for (sql, kind) in [
            ("CREATE TABLE t (id INT)", "CREATE TABLE"),
            ("CREATE OR REPLACE VIEW v AS SELECT 1", "CREATE VIEW"),
            ("CREATE UNIQUE INDEX i ON t (id)", "CREATE INDEX"),
            ("ALTER TABLE t ADD COLUMN c INT", "ALTER TABLE"),
            ("DROP TABLE IF EXISTS t", "DROP TABLE"),
            (
                "CREATE MATERIALIZED VIEW m AS SELECT 1",
                "CREATE MATERIALIZED VIEW",
            ),
        ] {
            let s = one(sql, Postgres);
            assert_eq!((s.class, s.kind.as_str()), (Ddl, kind), "{sql}");
            assert!(s.class.mutates());
        }
    }

    #[test]
    fn transactions() {
        for sql in ["BEGIN", "COMMIT", "ROLLBACK", "START TRANSACTION"] {
            let s = one(sql, Postgres);
            assert_eq!(s.class, Transaction, "{sql}");
            assert!(!s.class.mutates());
        }
    }

    #[test]
    fn cte_read_and_write() {
        let r = one("WITH x AS (SELECT 1 AS n) SELECT n FROM x", Postgres);
        assert_eq!((r.class, r.kind.as_str()), (Read, "SELECT"));
        assert!(
            r.tables.is_empty(),
            "CTE name is not a table: {:?}",
            r.tables
        );
        let w = one(
            "WITH d AS (DELETE FROM t WHERE a = 1 RETURNING *) SELECT * FROM d",
            Postgres,
        );
        assert_eq!(w.class, Write);
        let w = one(
            "WITH s AS (SELECT 1) DELETE FROM t WHERE a IN (SELECT * FROM s)",
            Postgres,
        );
        assert_eq!((w.class, w.kind.as_str()), (Write, "DELETE"));
    }

    #[test]
    fn select_into_is_write() {
        let s = one("SELECT a INTO t2 FROM t", MsSql);
        assert_eq!((s.class, s.kind.as_str()), (Write, "SELECT INTO"));
        assert_eq!(class("SELECT a INTO t2 FROM t", Postgres), Write);
    }

    #[test]
    fn explain_show_describe_pragma_are_read() {
        assert_eq!(class("EXPLAIN SELECT * FROM t", Postgres), Read);
        assert_eq!(class("EXPLAIN DELETE FROM t", MySql), Read);
        assert_eq!(class("SHOW TABLES", MySql), Read);
        assert_eq!(class("DESCRIBE t", MySql), Read);
        assert_eq!(class("PRAGMA table_info(t)", Sqlite), Read);
        assert_eq!(class("EXPLAIN QUERY PLAN SELECT 1", Sqlite), Read);
    }

    #[test]
    fn explain_analyze_runs_the_statement() {
        assert_eq!(class("EXPLAIN ANALYZE DELETE FROM t", Postgres), Write);
        assert_eq!(
            class("EXPLAIN (ANALYZE, VERBOSE) UPDATE t SET a = 1", Postgres),
            Write
        );
        assert_eq!(class("EXPLAIN ANALYZE SELECT 1", Postgres), Read);
    }

    #[test]
    fn other_class() {
        let a = analyze("SET search_path = public", Postgres);
        assert_eq!(a.statements[0].class, Other);
        assert!(!a.mutates() && !a.is_read_only());
    }

    #[test]
    fn mssql_top_and_brackets() {
        let s = one(
            "SELECT TOP 10 [My Col], [t].[id] FROM [dbo].[My Table] AS [t]",
            MsSql,
        );
        assert_eq!(s.class, Read);
        assert_eq!(s.tables, ["dbo.My Table"]);
    }

    #[test]
    fn mysql_backticks() {
        let s = one("SELECT `a` FROM `db`.`order` WHERE `x` = 'y;z'", MySql);
        assert_eq!(s.tables, ["db.order"]);
        assert_eq!(
            analyze("UPDATE `t` SET `a` = 1; SELECT 1", MySql)
                .statements
                .len(),
            2
        );
    }

    #[test]
    fn postgres_params_casts_dollar_quotes() {
        assert_eq!(
            class(
                "SELECT $1::int + $2::bigint, '1'::text FROM t WHERE id = $3",
                Postgres
            ),
            Read
        );
        let sql = "DO $$ BEGIN PERFORM 1; PERFORM 2; END $$; SELECT $tag$a;b$tag$";
        let a = analyze(sql, Postgres);
        assert_eq!(a.error, None);
        assert_eq!(
            texts(sql, &a),
            [
                "DO $$ BEGIN PERFORM 1; PERFORM 2; END $$",
                "SELECT $tag$a;b$tag$"
            ]
        );
    }

    #[test]
    fn multi_statement_ranges() {
        let sql = "SELECT 1;\n  -- note; here\n  UPDATE t SET a = 'x;y' /* ; */;\nDELETE FROM t";
        let a = analyze(sql, Postgres);
        assert_eq!(a.error, None);
        assert_eq!(
            texts(sql, &a),
            ["SELECT 1", "UPDATE t SET a = 'x;y'", "DELETE FROM t"]
        );
        let classes: Vec<_> = a.statements.iter().map(|s| s.class).collect();
        assert_eq!(classes, [Read, Write, Write]);
        assert!(a.mutates() && !a.is_read_only());
        let r = analyze("SELECT 1; SELECT 2;", Generic);
        assert!(!r.mutates() && r.is_read_only());
    }

    #[test]
    fn error_has_position() {
        let sql = "SELECT 1;\nSELECT a b c FROM t;";
        let a = analyze(sql, Postgres);
        let e = a.error.expect("error");
        assert_eq!(e.line, 2, "{e:?}");
        assert_eq!(&sql[e.byte_offset..e.byte_offset + 1], "c", "{e:?}");
        assert_eq!(e.column, 12);
        assert!(!e.message.contains("Line:") && !e.message.is_empty());
        assert_eq!(a.statements.len(), 2, "good and bad statements both listed");
    }

    #[test]
    fn error_at_eof_points_to_end() {
        let sql = "SELECT * FROM";
        let e = analyze(sql, Postgres).error.expect("error");
        assert_eq!((e.line, e.byte_offset), (1, sql.len()));
        assert_eq!(e.column, sql.len() + 1);
    }

    #[test]
    fn broken_write_is_still_a_write() {
        let a = analyze("DELETE FROM t WHERE (", Postgres);
        assert!(a.error.is_some());
        assert_eq!(a.statements[0].class, Write);
        assert!(a.mutates());
    }

    #[test]
    fn unterminated_string_reports_and_splits() {
        let sql = "SELECT 1; DELETE FROM t WHERE a = 'oops;\nSELECT 2";
        let a = analyze(sql, Postgres);
        let e = a.error.clone().expect("tokenizer error");
        assert_eq!(e.line, 2 - 1, "{e:?}");
        assert_eq!(sql.as_bytes()[e.byte_offset], b'\'', "{e:?}");
        assert_eq!(
            texts(sql, &a),
            ["SELECT 1", "DELETE FROM t WHERE a = 'oops;\nSELECT 2"]
        );
        assert!(a.mutates());
    }

    #[test]
    fn statement_at_picks_the_statement() {
        let sql = "SELECT 1;\n\nUPDATE t SET a = 1;\nSELECT 3";
        let at = |o| statement_at(sql, Postgres, o).map(|r| sql[r].to_string());
        assert_eq!(at(0).as_deref(), Some("SELECT 1"));
        assert_eq!(at(7).as_deref(), Some("SELECT 1"));
        assert_eq!(at(9).as_deref(), Some("SELECT 1"), "right after the ;");
        assert_eq!(
            at(10).as_deref(),
            Some("UPDATE t SET a = 1"),
            "blank gap goes to the next"
        );
        assert_eq!(at(20).as_deref(), Some("UPDATE t SET a = 1"));
        assert_eq!(at(sql.len()).as_deref(), Some("SELECT 3"));
        assert_eq!(
            at(10_000).as_deref(),
            Some("SELECT 3"),
            "past the end is the last"
        );
    }

    #[test]
    fn statement_at_survives_parse_errors() {
        let sql = "SELEC 1; SELECT 'a;b' FROM";
        assert_eq!(
            statement_at(sql, Postgres, 2).map(|r| &sql[r]),
            Some("SELEC 1")
        );
        assert_eq!(
            statement_at(sql, Postgres, 20).map(|r| &sql[r]),
            Some("SELECT 'a;b' FROM")
        );
    }

    #[test]
    fn statement_at_fallback_scanner() {
        // Tokenizer fails (unterminated quoted identifier); the scanner still splits.
        let sql = "SELECT [a;b] FROM t; SELECT \"oops; x";
        for d in [MsSql, Postgres] {
            let r = statement_at(sql, d, 25).map(|r| &sql[r]);
            assert_eq!(r, Some("SELECT \"oops; x"), "{d:?}");
        }
        assert_eq!(
            statement_at(sql, MsSql, 3).map(|r| &sql[r]),
            Some("SELECT [a;b] FROM t")
        );
    }

    #[test]
    fn scanner_respects_comments_and_dollar_quotes() {
        let sql = "SELECT 1 -- ; \n; /* ; */ SELECT $x$ ; $x$, 'unterminated";
        let p = scan_split(sql, Postgres);
        let t: Vec<_> = p.iter().map(|p| &sql[p.range.clone()]).collect();
        assert_eq!(t, ["SELECT 1", "SELECT $x$ ; $x$, 'unterminated"]);
    }

    #[test]
    fn unicode_offsets_are_bytes() {
        let sql = "SELECT 'é€' AS a; SELEC 'ü'";
        let a = analyze(sql, Postgres);
        let e = a.error.clone().expect("error");
        assert_eq!(&sql[e.byte_offset..e.byte_offset + 5], "SELEC", "{e:?}");
        assert_eq!(e.column, 19);
        assert_eq!(texts(sql, &a), ["SELECT 'é€' AS a", "SELEC 'ü'"]);
    }

    #[test]
    fn tables_best_effort() {
        let s = one(
            "SELECT * FROM a JOIN s.b ON a.id = b.id, c WHERE a.x IN (SELECT x FROM a)",
            Postgres,
        );
        assert_eq!(s.tables, ["a", "s.b", "c"]);
        assert_eq!(one("UPDATE \"My T\" SET x = 1", Postgres).tables, ["My T"]);
        assert_eq!(one("INSERT INTO t (a) VALUES (1)", Sqlite).tables, ["t"]);
        assert_eq!(one("DELETE FROM t", MySql).tables, ["t"]);
    }

    #[test]
    fn crlf_lines() {
        let sql = "SELECT 1;\r\nSELEC 2;";
        let e = analyze(sql, Generic).error.expect("error");
        assert_eq!((e.line, e.column), (2, 1));
        assert_eq!(&sql[e.byte_offset..e.byte_offset + 5], "SELEC");
    }
}
