//! The malicious-query corpus for [`check_read_only`], per dialect.
//!
//! Extend by adding a row to the table of the dialect: `(sql, rule)` in a `DENIED_*` table (the
//! rule must be among the violations; a text that only fails to parse is `Unparseable`), or a plain
//! string in an `ALLOWED_*` table. Every table is run by one test, so a failure names the text.
//! Re-run the whole corpus when the `sqlparser` pin moves (`readonly.md`: version drift).

use super::*;
use Dialect::*;
use ReadOnlyRule as R;

/// Rows are `(sql, the rule that must be among the violations)`.
type Denied = &'static [(&'static str, ReadOnlyRule)];

fn run_denied(dialect: Dialect, table: Denied) {
    let mut bad = Vec::new();
    for (sql, rule) in table {
        match check_read_only(sql, dialect) {
            ReadOnlyVerdict::Allowed => bad.push(format!("ALLOWED but must be denied: {sql}")),
            ReadOnlyVerdict::Denied(v) if !v.iter().any(|x| x.rule == *rule) => {
                let got: Vec<_> = v.iter().map(|x| (x.rule, x.message.as_str())).collect();
                bad.push(format!("{sql}\n    expected {rule:?}, got {got:?}"));
            }
            ReadOnlyVerdict::Denied(_) => {}
        }
    }
    assert!(bad.is_empty(), "{dialect:?}:\n  {}", bad.join("\n  "));
}

fn run_allowed(dialect: Dialect, table: &[&str]) {
    let bad: Vec<_> = table
        .iter()
        .filter_map(|sql| match check_read_only(sql, dialect) {
            ReadOnlyVerdict::Allowed => None,
            v => Some(format!(
                "DENIED but must be allowed: {sql}\n    {:?}",
                v.violations()
            )),
        })
        .collect();
    assert!(bad.is_empty(), "{dialect:?}:\n  {}", bad.join("\n  "));
}

const DENIED_POSTGRES: Denied = &[
    // data-modifying CTEs and nested DML
    (
        "WITH d AS (DELETE FROM t RETURNING *) SELECT * FROM d",
        R::NestedDml,
    ),
    (
        "WITH u AS (UPDATE t SET a = 1 RETURNING *) SELECT * FROM u",
        R::NestedDml,
    ),
    (
        "WITH i AS (INSERT INTO t VALUES (1) RETURNING *) SELECT * FROM i",
        R::NestedDml,
    ),
    (
        "WITH a AS (WITH b AS (DELETE FROM t RETURNING 1) SELECT * FROM b) SELECT * FROM a",
        R::NestedDml,
    ),
    (
        "SELECT * FROM (WITH d AS (DELETE FROM t RETURNING 1) SELECT * FROM d) x",
        R::NestedDml,
    ),
    // SELECT INTO, locks
    ("SELECT * INTO newt FROM t", R::SelectInto),
    ("SELECT * INTO TEMP newt FROM t", R::SelectInto),
    ("SELECT * FROM t FOR UPDATE", R::Locking),
    // 0.63 cannot parse FOR NO KEY UPDATE: denied by failing closed, not by the locking rule
    ("SELECT * FROM t FOR NO KEY UPDATE", R::Unparseable),
    ("SELECT * FROM t FOR SHARE NOWAIT", R::Locking),
    (
        "SELECT * FROM t WHERE a IN (SELECT a FROM u FOR UPDATE)",
        R::Locking,
    ),
    // side-effecting functions, plain and obfuscated
    ("SELECT nextval('s')", R::Function),
    ("SELECT setval('s', 1)", R::Function),
    ("SELECT pg_sleep(10)", R::Function),
    ("SELECT pg_terminate_backend(123)", R::Function),
    ("SELECT pg_read_file('/etc/passwd')", R::Function),
    ("SELECT lo_import('/etc/passwd')", R::Function),
    ("SELECT set_config('x', 'y', false)", R::Function),
    (
        "SELECT * FROM dblink('dbname=x', 'DELETE FROM t') AS r(a text)",
        R::Function,
    ),
    ("SeLeCt PG_SLEEP(1)", R::Function),
    ("SELECT \"nextval\"('s')", R::Function),
    ("SELECT pg_catalog.pg_sleep(10)", R::Function),
    ("SELECT pg_catalog . pg_sleep ( 10 )", R::Function),
    ("SELECT public.count(*) FROM t", R::Function),
    ("SELECT \"COUNT\"(*) FROM t", R::Function),
    ("SELECT my_udf(a) FROM t", R::Function),
    ("SELECT /**/ pg_sleep /**/ (1)", R::Function),
    ("SELECT /* a /* nested */ b */ pg_sleep(1)", R::Function),
    ("SELECT 1 -- harmless\n, pg_sleep(1)", R::Function),
    // U+3000 is an identifier character in the Postgres tokenizer: a different (unlisted) name
    ("SELECT\n\t\u{3000}pg_sleep(1)", R::Function),
    (
        "SELECT 1 FROM t WHERE a = (SELECT pg_sleep(1))",
        R::Function,
    ),
    ("SELECT 1 FROM t ORDER BY pg_sleep(1)", R::Function),
    (
        "SELECT 1 FROM t a JOIN u b ON pg_sleep(1) IS NULL",
        R::Function,
    ),
    (
        "WITH c AS (SELECT nextval('s')) SELECT * FROM c",
        R::Function,
    ),
    // stacked statements
    ("SELECT 1; SELECT 2", R::StackedStatements),
    ("SELECT 1; DELETE FROM t", R::StatementKind),
    ("SELECT 1; DROP TABLE t", R::StatementKind),
    ("SELECT 1;\n-- c\nUPDATE t SET a = 1", R::StatementKind),
    // statement kinds
    ("INSERT INTO t VALUES (1)", R::StatementKind),
    ("UPDATE t SET a = 1", R::StatementKind),
    ("DELETE FROM t", R::StatementKind),
    ("TRUNCATE t", R::StatementKind),
    ("CREATE TABLE x (a int)", R::StatementKind),
    ("DROP TABLE t", R::StatementKind),
    ("ALTER TABLE t ADD COLUMN b int", R::StatementKind),
    ("COPY t TO '/tmp/x'", R::StatementKind),
    ("COPY (SELECT 1) TO PROGRAM 'id'", R::StatementKind),
    ("NOTIFY ch", R::StatementKind),
    ("LISTEN ch", R::StatementKind),
    ("VACUUM", R::StatementKind),
    ("ANALYZE t", R::StatementKind),
    ("REFRESH MATERIALIZED VIEW v", R::Unparseable), // not modelled by 0.63
    ("SET search_path = evil", R::StatementKind),
    ("CALL p()", R::StatementKind),
    ("BEGIN", R::StatementKind),
    ("COMMIT", R::StatementKind),
    ("ROLLBACK", R::StatementKind),
    ("GRANT ALL ON t TO PUBLIC", R::StatementKind),
    ("EXPLAIN ANALYZE DELETE FROM t", R::StatementKind),
    ("EXPLAIN ANALYZE SELECT pg_sleep(5)", R::Function),
    (
        "EXPLAIN (ANALYZE) WITH d AS (DELETE FROM t RETURNING 1) SELECT * FROM d",
        R::NestedDml,
    ),
    // does not parse / empty: fail closed
    ("SELEC 1", R::Unparseable),
    ("SELECT * FROM (", R::Unparseable),
    ("SELECT 'unterminated", R::Unparseable),
    ("DO $$ BEGIN PERFORM 1; END $$", R::Unparseable),
    ("", R::Empty),
    ("-- nothing here", R::Empty),
    (" ; ; ", R::Empty),
];

const ALLOWED_POSTGRES: &[&str] = &[
    "SELECT 1",
    "select 1;",
    "SELECT * FROM t WHERE a IN (SELECT b FROM u)",
    "WITH c AS (SELECT 1 AS x) SELECT * FROM c",
    "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 5) SELECT * FROM n",
    "VALUES (1), (2)",
    "SELECT * FROM t UNION ALL SELECT * FROM u",
    "SELECT count(*), max(a) FROM t GROUP BY b HAVING sum(c) > 1",
    "SELECT lower(a), now(), date_trunc('day', ts), a::text, CAST(a AS int), coalesce(a, 0) FROM t",
    "SELECT row_number() OVER (PARTITION BY a ORDER BY b) FROM t",
    "SELECT data->>'k', jsonb_array_length(data->'xs') FROM t",
    "SELECT pg_catalog.count(*) FROM t",
    "SELECT * FROM generate_series(1, 10) g",
    "SELECT * FROM t LIMIT 10 OFFSET 5",
    "SELECT current_database(), version(), pg_size_pretty(pg_total_relation_size('t'))",
    "SELECT EXTRACT(year FROM ts), SUBSTRING(a FROM 1 FOR 2), TRIM(a), POSITION('x' IN a) FROM t",
    "EXPLAIN SELECT * FROM t",
    "EXPLAIN ANALYZE SELECT * FROM t",
    "SHOW search_path",
    // text that only looks dangerous
    "SELECT 'x; DROP TABLE t'",
    "SELECT 1 -- ; DELETE FROM t",
    "SELECT $$ ; DELETE FROM t $$",
    "SELECT 'a' /* DELETE FROM t */",
    "SELECT * FROM \"delete\"",
    "SELECT 'it''s' AS \"update\"",
    "SELECT /* nextval('s') */ 1",
    "SELECT 'pg_sleep(1)' AS note",
];

const DENIED_MYSQL: Denied = &[
    ("SELECT * FROM t INTO OUTFILE '/tmp/x'", R::Unparseable),
    ("SELECT * FROM t INTO DUMPFILE '/tmp/x'", R::Unparseable),
    ("SELECT 1 INTO @v", R::SelectInto),
    ("SELECT * FROM t FOR UPDATE", R::Locking),
    ("SELECT * FROM t FOR SHARE", R::Locking),
    // 0.63 does not parse LOCK IN SHARE MODE: denied by failing closed
    ("SELECT * FROM t LOCK IN SHARE MODE", R::Unparseable),
    ("SELECT SLEEP(5)", R::Function),
    ("select sleep(5)", R::Function),
    ("SELECT BENCHMARK(1000000, MD5('a'))", R::Function),
    ("SELECT LOAD_FILE('/etc/passwd')", R::Function),
    ("SELECT GET_LOCK('a', 10)", R::Function),
    ("SELECT RELEASE_LOCK('a')", R::Function),
    ("SELECT `sleep`(1)", R::Function),
    ("SELECT `count`(*) FROM t", R::Function),
    ("SELECT mydb.myfunc(1)", R::Function),
    ("SELECT udf_write(1) FROM t", R::Function),
    // executable comments and hints
    ("SELECT /*! SLEEP(1) */ 1", R::ExecutableComment),
    (
        "SELECT 1 /*!50000 UNION SELECT SLEEP(1) */",
        R::ExecutableComment,
    ),
    ("SELECT 1 /*! ; DROP TABLE t */", R::ExecutableComment),
    ("/*!50000 DELETE FROM t */", R::ExecutableComment),
    ("SELECT 1 /*M!100100 , SLEEP(1) */", R::ExecutableComment),
    (
        "SELECT /*+ SET_VAR(sort_buffer_size = 16M) */ 1",
        R::OptimizerHint,
    ),
    (
        "SELECT /*+ MAX_EXECUTION_TIME(1000) */ * FROM t",
        R::OptimizerHint,
    ),
    ("SELECT @a := 1", R::Assignment),
    // statements
    ("SELECT 1; SELECT 2", R::StackedStatements),
    ("USE other", R::StatementKind),
    ("SET @a = 1", R::StatementKind),
    ("SET GLOBAL read_only = 0", R::StatementKind),
    ("CALL p()", R::StatementKind),
    ("LOCK TABLES t WRITE", R::StatementKind),
    ("START TRANSACTION", R::StatementKind),
    ("KILL 1", R::StatementKind),
    ("FLUSH TABLES", R::StatementKind),
    ("LOAD DATA INFILE '/tmp/x' INTO TABLE t", R::Unparseable),
    ("INSERT INTO t VALUES (1)", R::StatementKind),
    ("REPLACE INTO t VALUES (1)", R::StatementKind),
    ("DELETE FROM t", R::StatementKind),
    ("WITH c AS (SELECT 1) DELETE FROM t", R::NestedDml),
    ("EXPLAIN DELETE FROM t", R::StatementKind),
    ("EXPLAIN ANALYZE SELECT SLEEP(5)", R::Function),
];

const ALLOWED_MYSQL: &[&str] = &[
    "SELECT * FROM t",
    "SELECT * FROM `t` WHERE `a` = 1 ORDER BY `b` DESC LIMIT 5",
    "SELECT IFNULL(a, 0), CONCAT(a, b), DATE_FORMAT(NOW(), '%Y') FROM t",
    "SELECT * FROM t USE INDEX (i) WHERE a = 1",
    "SELECT GROUP_CONCAT(a) FROM t",
    "SELECT COUNT(*) FROM t",
    "SHOW TABLES",
    "SHOW COLUMNS FROM t",
    "SHOW CREATE TABLE t",
    "SHOW DATABASES",
    "SHOW VARIABLES LIKE 'max%'",
    "DESCRIBE t",
    "EXPLAIN SELECT * FROM t",
    // inside a string or a line comment, `/*!` is not a comment
    "SELECT 'a /*! x */ b'",
    "SELECT 1 -- /*! x */",
    "SELECT 1 # /*! x */",
    "SELECT 1 /* plain */",
];

const DENIED_SQLITE: Denied = &[
    ("PRAGMA journal_mode = 2", R::Pragma),
    ("PRAGMA journal_mode = DELETE", R::Pragma),
    ("PRAGMA journal_mode(wal)", R::Pragma),
    (
        "PRAGMA table_info(t) ; PRAGMA writable_schema = 1",
        R::StackedStatements,
    ),
    ("PRAGMA writable_schema = 1", R::Pragma),
    ("PRAGMA user_version = 5", R::Pragma),
    ("PRAGMA foreign_keys(0)", R::Pragma),
    ("PRAGMA query_only = OFF", R::Pragma),
    ("PRAGMA user_version(5)", R::Pragma),
    ("PRAGMA query_only = 0", R::Pragma),
    ("PRAGMA a.b.table_info(1)", R::Pragma),
    ("SELECT load_extension('/tmp/evil.so')", R::Function),
    ("SELECT writefile('/tmp/x', 'y')", R::Function),
    ("SELECT readfile('/etc/passwd')", R::Function),
    ("SELECT fts3_tokenizer('x')", R::Function),
    ("ATTACH DATABASE 'x.db' AS y", R::StatementKind),
    ("VACUUM INTO 'x.db'", R::Unparseable),
    ("VACUUM", R::StatementKind),
    ("INSERT INTO t VALUES (1)", R::StatementKind),
    ("REPLACE INTO t VALUES (1)", R::StatementKind),
    ("CREATE TEMP TABLE x (a)", R::StatementKind),
    ("DROP TABLE t", R::StatementKind),
    ("BEGIN", R::StatementKind),
    ("SELECT 1; PRAGMA journal_mode = WAL", R::StackedStatements),
    ("WITH c AS (SELECT 1) DELETE FROM t", R::NestedDml),
];

const ALLOWED_SQLITE: &[&str] = &[
    "SELECT * FROM t",
    "SELECT * FROM sqlite_master WHERE type = 'table'",
    "SELECT sqlite_version(), typeof(a), json_extract(a, '$.b'), ifnull(a, 0) FROM t",
    "SELECT * FROM pragma_table_info('t')",
    "PRAGMA table_info(t)",
    "PRAGMA table_info('t')",
    "PRAGMA main.index_list(t)",
    "PRAGMA foreign_key_list(t)",
    "PRAGMA database_list",
    "PRAGMA user_version",
    "EXPLAIN QUERY PLAN SELECT * FROM t",
    "SELECT 'PRAGMA x = 1; DROP TABLE t'",
];

const DENIED_MSSQL: Denied = &[
    ("SELECT * FROM t WITH (UPDLOCK)", R::TableHint),
    ("SELECT * FROM t WITH (XLOCK, ROWLOCK)", R::TableHint),
    ("SELECT * FROM t WITH (HOLDLOCK)", R::TableHint),
    ("SELECT * FROM t WITH (TABLOCKX)", R::TableHint),
    ("SELECT * FROM t WITH (NOLOCK, UPDLOCK)", R::TableHint),
    ("SELECT * INTO newt FROM t", R::SelectInto),
    ("SELECT * INTO #tmp FROM t", R::SelectInto),
    ("EXEC xp_cmdshell 'dir'", R::StatementKind),
    ("EXECUTE sp_who", R::StatementKind),
    ("SELECT [xp_cmdshell]('dir')", R::Function),
    ("SELECT dbo.my_func(1)", R::Function),
    (
        "SELECT * FROM OPENROWSET('SQLNCLI', 'Server=x;', 'SELECT 1') AS r",
        R::Function,
    ),
    (
        "SELECT * FROM OPENQUERY(linked, 'DELETE FROM t')",
        R::Function,
    ),
    (
        "SELECT * FROM OPENDATASOURCE('a', 'b').db.dbo.t",
        R::Unparseable,
    ),
    // a CTE feeding an UPDATE/DELETE, in T-SQL syntax
    (
        "WITH c AS (SELECT a FROM t) UPDATE c SET a = 2",
        R::NestedDml,
    ),
    ("WITH c AS (SELECT * FROM t) DELETE FROM c", R::NestedDml),
    ("USE other", R::StatementKind),
    ("SET NOCOUNT ON", R::StatementKind),
    ("WAITFOR DELAY '00:00:10'", R::StatementKind),
    ("BEGIN TRAN", R::StatementKind),
    ("DECLARE @x int", R::StatementKind),
    ("SELECT 1; EXEC sp_who", R::StackedStatements),
    ("SELECT 1 SELECT 2", R::Unparseable), // a T-SQL batch without `;`: 0.63 refuses it
    ("INSERT INTO t VALUES (1)", R::StatementKind),
    ("TRUNCATE TABLE t", R::StatementKind),
];

const ALLOWED_MSSQL: &[&str] = &[
    "SELECT TOP 10 * FROM t WITH (NOLOCK)",
    "SELECT ISNULL(a, 0), LEN(b), GETDATE(), DATEDIFF(day, a, b) FROM dbo.t",
    "SELECT [name] FROM [sys].[tables]",
    "SELECT CONVERT(varchar(10), a, 120), CAST(b AS int) FROM t",
    "WITH c AS (SELECT 1 AS x) SELECT * FROM c",
    "SELECT a FROM t ORDER BY a OFFSET 5 ROWS FETCH NEXT 5 ROWS ONLY",
    "SELECT OBJECT_ID('t'), DB_NAME()",
    "SELECT '[xp_cmdshell]', 'EXEC x'",
];

#[test]
fn postgres_corpus() {
    run_denied(Postgres, DENIED_POSTGRES);
    run_allowed(Postgres, ALLOWED_POSTGRES);
}

#[test]
fn mysql_corpus() {
    run_denied(MySql, DENIED_MYSQL);
    run_allowed(MySql, ALLOWED_MYSQL);
}

#[test]
fn sqlite_corpus() {
    run_denied(Sqlite, DENIED_SQLITE);
    run_allowed(Sqlite, ALLOWED_SQLITE);
}

#[test]
fn mssql_corpus() {
    run_denied(MsSql, DENIED_MSSQL);
    run_allowed(MsSql, ALLOWED_MSSQL);
}

#[test]
fn generic_dialect_is_as_strict() {
    run_denied(
        Generic,
        &[
            ("SELECT pg_sleep(1)", R::Function),
            ("SELECT sleep(1)", R::Function),
            ("SELECT load_extension('x')", R::Function),
            ("DELETE FROM t", R::StatementKind),
            ("SELECT /*! 1 */ 1", R::ExecutableComment),
        ],
    );
    run_allowed(Generic, &["SELECT lower(a), count(*) FROM t GROUP BY a"]);
}

#[test]
fn violations_carry_positions_and_texts() {
    let sql = "SELECT 1 FROM t WHERE x = pg_sleep(2)";
    let ReadOnlyVerdict::Denied(v) = check_read_only(sql, Postgres) else {
        panic!("must be denied");
    };
    assert_eq!(v.len(), 1);
    assert_eq!(v[0].rule, R::Function);
    assert!(v[0].message.contains("pg_sleep"), "{}", v[0].message);
    assert_eq!(&sql[v[0].byte_range.clone().unwrap()], "pg_sleep");

    // a statement-level violation points at its statement
    let sql = "  DELETE FROM t  ";
    let v = check_read_only(sql, Postgres);
    let r = v.violations()[0].byte_range.clone().unwrap();
    assert_eq!(&sql[r], "DELETE FROM t");

    // the executable comment's own bytes
    let sql = "SELECT 1 /*! x */";
    let v = check_read_only(sql, MySql);
    let c = v
        .violations()
        .iter()
        .find(|x| x.rule == R::ExecutableComment)
        .unwrap();
    assert_eq!(&sql[c.byte_range.clone().unwrap()], "/*! x */");
}

#[test]
fn every_violation_is_reported_not_just_the_first() {
    let v = check_read_only(
        "SELECT nextval('s'), pg_sleep(1) FROM t FOR UPDATE",
        Postgres,
    );
    let rules: Vec<_> = v.violations().iter().map(|x| x.rule).collect();
    assert_eq!(rules.iter().filter(|r| **r == R::Function).count(), 2);
    assert!(rules.contains(&R::Locking));
}

#[test]
fn verdict_helpers() {
    let ok = check_read_only("SELECT 1", Sqlite);
    assert!(ok.is_allowed() && ok.violations().is_empty());
    let no = check_read_only("DROP TABLE t", Sqlite);
    assert!(!no.is_allowed() && !no.violations().is_empty());
}
