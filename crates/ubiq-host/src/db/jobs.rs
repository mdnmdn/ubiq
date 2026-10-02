//! What a session does: each message of the family, turned into a job and run on a connection.
//!
//! **Read-only is enforced here, in three layers.** (1) Before anything reaches a server, every
//! statement of a read-only run passes [`sql::check_read_only`] — fail closed; a refusal is
//! `ReadOnly` naming the rule. (2) The driver is handed [`ExecOptions::read_only`], which makes the
//! engine refuse a write itself. (3) A timeout, a row cap and a statement log. A session is
//! read-only when its connection is or when the message says so.

use std::time::{Duration, Instant};

use ubiq_db::conn::DbKind;
use ubiq_db::driver::Connection;
use ubiq_db::sql::{self, StatementClass};
use ubiq_db::value::Value;
use ubiq_db::{DbError, ExecOptions, ResultSet, TableRef, edit};
use ubiq_proto::db::{
    DbEditFailure, DbFailure, DbFailureKind, DbListing, DbNode, DbOutcome, DbPage, DbRun,
    DbRunOptions, RowEdit,
};
use ubiq_proto::ids::{DbQueryId, DbSessionId};
use ubiq_proto::messages::Message;

use super::agent::{Panel, StmtResult};
use super::session::Origin;

/// A page and a tree listing give up after this.
const META_TIMEOUT: Duration = Duration::from_secs(30);
/// The SQL editor's, unless the tab sets another.
const EDITOR_TIMEOUT: Duration = Duration::from_secs(5 * 60);
/// Rows a run returns when the tab does not say.
const DEFAULT_ROWS: usize = 1_000;
/// The most rows any reply carries, whatever was asked for.
const MAX_ROWS: usize = 10_000;
/// The most cell data any reply carries, so none comes near `MAX_FRAME`.
const MAX_CELL_BYTES: usize = 16 * 1024 * 1024;
/// A logged statement is cut here.
const LOG_CUT: usize = 2_000;

pub enum Job {
    /// Dial, and say where the connection stands. Answers only through `DbConnectionState`.
    Connect,
    Tree {
        node: DbNode,
    },
    Page {
        session: DbSessionId,
        query: DbQueryId,
        table: TableRef,
        filter: String,
        order_by: String,
        limit: u32,
        offset: u64,
        count: bool,
        read_only: bool,
    },
    Query {
        session: DbSessionId,
        query: DbQueryId,
        database: Option<String>,
        statements: Vec<String>,
        run: DbRun,
        opts: DbRunOptions,
    },
    Edits {
        session: DbSessionId,
        query: DbQueryId,
        table: TableRef,
        edits: Vec<RowEdit>,
    },
    /// An agent's statements, answered on `reply` rather than to a window; `panel` also shows each
    /// result, to every window, in a shared editor's session.
    Agent {
        query: DbQueryId,
        database: Option<String>,
        statements: Vec<String>,
        read_only: bool,
        timeout: Duration,
        row_limit: usize,
        panel: Option<Panel>,
        reply: flume::Sender<Vec<StmtResult>>,
    },
    /// An agent's look at the structure tree.
    AgentTree {
        node: DbNode,
        reply: flume::Sender<Result<DbListing, DbFailure>>,
    },
}

impl Job {
    /// The reply for a job that could not even start — no connection, no session. `Connect` has
    /// none: the connection state already said.
    pub fn refuse(&self, o: &Origin, failure: DbFailure) -> Option<Message> {
        let project_id = o.project;
        Some(match self {
            Job::Connect => return None,
            Job::Tree { node } => Message::DbTreeListing {
                project_id,
                conn: o.conn,
                node: node.clone(),
                result: Err(failure),
            },
            Job::Page { session, query, .. } => Message::DbTablePageResult {
                project_id,
                session: *session,
                query: *query,
                result: Err(failure),
                elapsed_ms: 0,
            },
            Job::Query { session, query, .. } => Message::DbQueryResult {
                project_id,
                session: *session,
                query: *query,
                index: 0,
                last: true,
                result: Err(failure),
                elapsed_ms: 0,
            },
            Job::Edits { session, query, .. } => Message::DbEditsApplied {
                project_id,
                session: *session,
                query: *query,
                result: Err(DbEditFailure {
                    index: 0,
                    statement: String::new(),
                    failure,
                }),
            },
            Job::Agent {
                query,
                statements,
                panel,
                reply,
                ..
            } => {
                if let Some(panel) = panel {
                    panel.show(o, *query, 0, true, 0, &Err(failure.clone()));
                }
                let _ = reply.send(vec![StmtResult {
                    sql: statements.first().cloned().unwrap_or_default(),
                    kind: String::new(),
                    elapsed_ms: 0,
                    outcome: Err(failure),
                }]);
                o.runs.end(*query);
                return None;
            }
            Job::AgentTree { reply, .. } => {
                let _ = reply.send(Err(failure));
                return None;
            }
        })
    }
}

/// A driver error as the contract tells it.
pub fn failure(error: &DbError) -> DbFailure {
    let kind = match error {
        DbError::Config(_) => DbFailureKind::Config,
        DbError::Unsupported(_) => DbFailureKind::Unsupported,
        DbError::Connect(_) => DbFailureKind::Connect,
        DbError::Query(_) => DbFailureKind::Query,
        DbError::NotFound(_) => DbFailureKind::NotFound,
        DbError::Disconnected(_) => DbFailureKind::Disconnected,
        DbError::Cancelled => DbFailureKind::Cancelled,
        DbError::Timeout => DbFailureKind::Timeout,
        DbError::ReadOnly(_) => DbFailureKind::ReadOnly,
    };
    DbFailure::new(kind, error.to_string())
}

/// Run one job. Answers whether the connection is still good: a `Disconnected` drops it.
pub fn run(o: &Origin, conn: &mut Box<dyn Connection>, job: Job) -> bool {
    match job {
        Job::Connect => true,
        Job::Tree { node } => {
            let result = tree(conn, &node);
            let keep = keeps(&result);
            o.say(Message::DbTreeListing {
                project_id: o.project,
                conn: o.conn,
                node,
                result,
            });
            keep
        }
        Job::Page {
            session,
            query,
            table,
            filter,
            order_by,
            limit,
            offset,
            count,
            read_only,
        } => {
            let _end = EndRun::new(o, query);
            let started = Instant::now();
            let result = page(
                o, conn, query, &table, &filter, &order_by, limit, offset, count, read_only,
            );
            let keep = keeps(&result);
            o.say(Message::DbTablePageResult {
                project_id: o.project,
                session,
                query,
                result: result.map(Box::new),
                elapsed_ms: ms(started),
            });
            keep
        }
        Job::Query {
            session,
            query,
            database,
            statements,
            run,
            opts,
        } => {
            let _end = EndRun::new(o, query);
            statements_run(
                o,
                conn,
                session,
                query,
                database.as_deref(),
                &statements,
                run,
                opts,
            )
        }
        Job::Edits {
            session,
            query,
            table,
            edits,
        } => {
            let _end = EndRun::new(o, query);
            let result = apply_edits(o, conn, query, &table, &edits);
            let keep = result
                .as_ref()
                .map_or_else(|e| e.failure.kind != DbFailureKind::Disconnected, |_| true);
            o.say(Message::DbEditsApplied {
                project_id: o.project,
                session,
                query,
                result,
            });
            keep
        }
        Job::Agent {
            query,
            database,
            statements,
            read_only,
            timeout,
            row_limit,
            panel,
            reply,
        } => {
            let _end = EndRun::new(o, query);
            let exec = ExecOptions {
                read_only: read_only || conn.is_read_only(),
                timeout: Some(timeout),
                row_limit: Some(row_limit.clamp(1, MAX_ROWS)),
            };
            let (results, keep) = agent_run(
                o,
                conn,
                query,
                database.as_deref(),
                &statements,
                &exec,
                panel.as_ref(),
            );
            let _ = reply.send(results);
            keep
        }
        Job::AgentTree { node, reply } => {
            let result = tree(conn, &node);
            let keep = keeps(&result);
            let _ = reply.send(result);
            keep
        }
    }
}

fn keeps<T>(result: &Result<T, DbFailure>) -> bool {
    result
        .as_ref()
        .map_or_else(|e| e.kind != DbFailureKind::Disconnected, |_| true)
}

/// Ends a run's cancel registration when the job is over, however it ends.
struct EndRun<'a>(&'a Origin, DbQueryId);

impl<'a> EndRun<'a> {
    fn new(o: &'a Origin, query: DbQueryId) -> Self {
        Self(o, query)
    }
}

impl Drop for EndRun<'_> {
    fn drop(&mut self) {
        self.0.runs.end(self.1);
    }
}

fn ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

// ---- the tree -----------------------------------------------------------------------------------

fn tree(conn: &mut Box<dyn Connection>, node: &DbNode) -> Result<DbListing, DbFailure> {
    let listing = match node {
        DbNode::Databases => conn.databases().map(DbListing::Names),
        DbNode::Schemas { database } => conn.schemas(database).map(DbListing::Names),
        DbNode::Objects { database, schema } => conn
            .objects(database, schema.as_deref())
            .map(DbListing::Objects),
        DbNode::Columns { table } => conn.columns(table).map(DbListing::Columns),
    };
    listing.map_err(|e| failure(&e))
}

// ---- statements ---------------------------------------------------------------------------------

/// Make `database` the one statements run against, if it is not already. A session is one tab's, so
/// this happens once, not before every run.
fn select_database(
    conn: &mut Box<dyn Connection>,
    database: Option<&str>,
) -> Result<(), DbFailure> {
    match database.filter(|d| !d.is_empty()) {
        Some(db) if conn.current_database().as_deref() != Some(db) => {
            conn.use_database(db).map_err(|e| failure(&e))
        }
        _ => Ok(()),
    }
}

/// Layer 1: the AST check. Fails closed, naming the first rule broken.
fn gate(sql_text: &str, kind: DbKind) -> Result<(), DbFailure> {
    match sql::check_read_only(sql_text, kind.into()) {
        sql::ReadOnlyVerdict::Allowed => Ok(()),
        sql::ReadOnlyVerdict::Denied(violations) => {
            let why = violations
                .first()
                .map_or_else(|| "refused".to_string(), |v| v.message.clone());
            Err(DbFailure::new(
                DbFailureKind::ReadOnly,
                format!("read-only: {why}"),
            ))
        }
    }
}

/// Tell the driver which handle stops this run, or that it has been stopped already.
fn arm(o: &Origin, conn: &dyn Connection, query: DbQueryId) -> Result<(), DbFailure> {
    if o.runs.arm(query, conn.cancel_handle()) {
        Err(DbFailure::new(
            DbFailureKind::Cancelled,
            DbError::Cancelled.to_string(),
        ))
    } else {
        Ok(())
    }
}

/// Layer 3's log: one line per executed statement, project and connection and session and time and
/// rows beside it, the SQL cut. No connection string and no password ever reaches here.
fn log(o: &Origin, text: &str, started: Instant, rows: Option<u64>, error: Option<&DbFailure>) {
    let cut: String = text.chars().take(LOG_CUT).collect();
    let session = o
        .session
        .map_or_else(|| "meta".to_string(), |s| s.to_string());
    tracing::info!(
        target: "ubiq_db::sql",
        project = %o.project,
        conn = %o.conn,
        session = %session,
        elapsed_ms = ms(started),
        rows = rows,
        failed = error.map(|e| e.message.as_str()),
        "{cut}"
    );
}

/// Run a row-returning statement under the three layers.
fn read(
    o: &Origin,
    conn: &mut Box<dyn Connection>,
    query: DbQueryId,
    text: &str,
    opts: &ExecOptions,
) -> Result<ResultSet, DbFailure> {
    if opts.read_only {
        gate(text, conn.kind())?;
    }
    arm(o, &**conn, query)?;
    let started = Instant::now();
    let mut result = conn.query_with(text, opts).map_err(|e| failure(&e));
    if let (Ok(rows), Some(limit)) = (&mut result, opts.row_limit) {
        cap(rows, limit);
    }
    log(
        o,
        text,
        started,
        result.as_ref().ok().map(|r| r.rows.len() as u64),
        result.as_ref().err(),
    );
    result
}

/// Cut a result to `rows` rows and [`MAX_CELL_BYTES`] of cells, setting `truncated`.
fn cap(set: &mut ResultSet, rows: usize) {
    if set.rows.len() > rows {
        set.rows.truncate(rows);
        set.truncated = true;
    }
    let mut bytes = 0usize;
    let mut cut = None;
    for (index, row) in set.rows.iter().enumerate() {
        bytes += row.iter().map(size).sum::<usize>();
        if bytes > MAX_CELL_BYTES {
            cut = Some(index);
            break;
        }
    }
    if let Some(index) = cut {
        set.rows.truncate(index);
        set.truncated = true;
    }
}

fn size(value: &Value) -> usize {
    match value {
        Value::Null | Value::Bool(_) => 1,
        Value::Int(_) | Value::Float(_) => 8,
        Value::Bytes(bytes) => bytes.len(),
        Value::Decimal(s)
        | Value::Text(s)
        | Value::Uuid(s)
        | Value::Date(s)
        | Value::Time(s)
        | Value::DateTime(s)
        | Value::DateTimeTz(s)
        | Value::Json(s) => s.len(),
    }
}

// ---- a table page -------------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn page(
    o: &Origin,
    conn: &mut Box<dyn Connection>,
    query: DbQueryId,
    table: &TableRef,
    filter: &str,
    order_by: &str,
    limit: u32,
    offset: u64,
    count: bool,
    read_only: bool,
) -> Result<DbPage, DbFailure> {
    let kind = conn.kind();
    edit::validate_fragment(kind, filter, order_by)
        .map_err(|e| DbFailure::new(DbFailureKind::Query, e.message))?;
    select_database(conn, table.database.as_deref())?;
    let limit = (limit as usize).clamp(1, MAX_ROWS);
    let opts = ExecOptions {
        read_only: read_only || conn.is_read_only(),
        timeout: Some(META_TIMEOUT),
        row_limit: Some(limit),
    };
    let columns = conn.columns(table).map_err(|e| failure(&e))?;
    let select = edit::select_table(
        kind,
        table,
        None,
        filter,
        order_by,
        limit,
        usize::try_from(offset).unwrap_or(usize::MAX),
    );
    let rows = read(o, conn, query, &select, &opts)?;
    let exact_count = if count {
        let counted = read(
            o,
            conn,
            query,
            &edit::count_table(kind, table, filter),
            &ExecOptions {
                row_limit: Some(1),
                ..opts
            },
        )?;
        counted
            .rows
            .first()
            .and_then(|row| row.first())
            .and_then(count_of)
    } else {
        None
    };
    Ok(DbPage {
        columns,
        rows,
        exact_count,
    })
}

fn count_of(value: &Value) -> Option<u64> {
    match value {
        Value::Int(n) => u64::try_from(*n).ok(),
        Value::Decimal(s) | Value::Text(s) => s.trim().parse().ok(),
        _ => None,
    }
}

// ---- the SQL editor -----------------------------------------------------------------------------

/// Run a tab's statements in order, one reply each, stopping at the first failure.
#[allow(clippy::too_many_arguments)]
fn statements_run(
    o: &Origin,
    conn: &mut Box<dyn Connection>,
    session: DbSessionId,
    query: DbQueryId,
    database: Option<&str>,
    statements: &[String],
    run: DbRun,
    opts: DbRunOptions,
) -> bool {
    let read_only = opts.read_only || conn.is_read_only();
    let exec = ExecOptions {
        read_only,
        timeout: Some(
            opts.timeout_ms
                .map_or(EDITOR_TIMEOUT, Duration::from_millis),
        ),
        row_limit: Some(
            opts.row_limit
                .map_or(DEFAULT_ROWS, |n| n as usize)
                .clamp(1, MAX_ROWS),
        ),
    };
    let reply =
        |index: usize, last: bool, started: Instant, result: Result<DbOutcome, DbFailure>| {
            o.say(Message::DbQueryResult {
                project_id: o.project,
                session,
                query,
                index: u32::try_from(index).unwrap_or(u32::MAX),
                last,
                result: result.map(Box::new),
                elapsed_ms: ms(started),
            });
        };
    if statements.is_empty() {
        reply(
            0,
            true,
            Instant::now(),
            Err(DbFailure::new(DbFailureKind::Query, "no statement to run")),
        );
        return true;
    }
    let setup = select_database(conn, database);
    for (index, text) in statements.iter().enumerate() {
        let started = Instant::now();
        let result = match &setup {
            Err(failure) => Err(failure.clone()),
            Ok(()) => statement(o, conn, query, text, run, &exec),
        };
        let failed = result.is_err();
        let lost = result
            .as_ref()
            .err()
            .is_some_and(|e| e.kind == DbFailureKind::Disconnected);
        reply(
            index,
            failed || index + 1 == statements.len(),
            started,
            result,
        );
        if lost {
            return false;
        }
        if failed {
            break;
        }
    }
    true
}

fn statement(
    o: &Origin,
    conn: &mut Box<dyn Connection>,
    query: DbQueryId,
    text: &str,
    run: DbRun,
    opts: &ExecOptions,
) -> Result<DbOutcome, DbFailure> {
    match run {
        DbRun::Query => {
            let class = sql::analyze(text, conn.kind().into())
                .statements
                .first()
                .map(|s| s.class);
            // The proof's rule: a read-only run goes down the query path, where the guard is; a
            // writing statement on a read-write run is executed and counted.
            let rows = matches!(
                class,
                None | Some(StatementClass::Read) | Some(StatementClass::Other)
            ) || opts.read_only;
            if rows {
                return read(o, conn, query, text, opts).map(DbOutcome::Rows);
            }
            arm(o, &**conn, query)?;
            let started = Instant::now();
            let outcome = conn.execute_with(text, opts).map_err(|e| failure(&e));
            log(
                o,
                text,
                started,
                outcome.as_ref().ok().map(|r| r.rows_affected),
                outcome.as_ref().err(),
            );
            outcome.map(|r| DbOutcome::Affected(r.rows_affected))
        }
        DbRun::Explain { analyze } => {
            // `analyze` executes the statement; a plain plan does not.
            if analyze && opts.read_only {
                gate(text, conn.kind())?;
            }
            arm(o, &**conn, query)?;
            let started = Instant::now();
            let plan = conn.explain(text, analyze).map_err(|e| failure(&e));
            log(o, text, started, None, plan.as_ref().err());
            plan.map(DbOutcome::Plan)
        }
    }
}

// ---- an agent's statements ----------------------------------------------------------------------

/// Run an agent's statements in order, stopping at the first failure. The database is selected on
/// every run: the session is shared by every call the agent makes on the connection, and
/// `use_database` is sticky. Answers the results and whether the connection is still good.
fn agent_run(
    o: &Origin,
    conn: &mut Box<dyn Connection>,
    query: DbQueryId,
    database: Option<&str>,
    statements: &[String],
    exec: &ExecOptions,
    panel: Option<&Panel>,
) -> (Vec<StmtResult>, bool) {
    if statements.is_empty() {
        let failure = DbFailure::new(DbFailureKind::Query, "no statement to run");
        if let Some(panel) = panel {
            panel.show(o, query, 0, true, 0, &Err(failure.clone()));
        }
        let result = StmtResult {
            sql: String::new(),
            kind: String::new(),
            elapsed_ms: 0,
            outcome: Err(failure),
        };
        return (vec![result], true);
    }
    let setup = select_database(conn, database);
    let mut results = Vec::with_capacity(statements.len());
    for (index, text) in statements.iter().enumerate() {
        let started = Instant::now();
        let kind = sql::analyze(text, conn.kind().into())
            .statements
            .first()
            .map(|s| s.kind.clone())
            .unwrap_or_default();
        let outcome = match &setup {
            Err(failure) => Err(failure.clone()),
            Ok(()) => statement(o, conn, query, text, DbRun::Query, exec),
        };
        let elapsed_ms = ms(started);
        let failed = outcome.is_err();
        let lost = outcome
            .as_ref()
            .err()
            .is_some_and(|e| e.kind == DbFailureKind::Disconnected);
        if let Some(panel) = panel {
            let last = failed || index + 1 == statements.len();
            panel.show(o, query, index, last, elapsed_ms, &outcome);
        }
        results.push(StmtResult {
            sql: text.clone(),
            kind,
            elapsed_ms,
            outcome,
        });
        if lost {
            return (results, false);
        }
        if failed {
            break;
        }
    }
    (results, true)
}

// ---- row edits ----------------------------------------------------------------------------------

/// Render the batch and run it between `BEGIN` and `COMMIT`; the first error rolls the whole of it
/// back and names the statement.
fn apply_edits(
    o: &Origin,
    conn: &mut Box<dyn Connection>,
    query: DbQueryId,
    table: &TableRef,
    edits: &[RowEdit],
) -> Result<u64, DbEditFailure> {
    let at = |index: usize, statement: &str, failure: DbFailure| DbEditFailure {
        index: u32::try_from(index).unwrap_or(u32::MAX),
        statement: statement.to_string(),
        failure,
    };
    if conn.is_read_only() {
        return Err(at(
            0,
            "",
            DbFailure::new(
                DbFailureKind::ReadOnly,
                "read-only: this connection is read-only",
            ),
        ));
    }
    let kind = conn.kind();
    let statements = edit::render_batch(kind, table, edits).map_err(|e| {
        let index = edits
            .iter()
            .position(|edit| edit::render(kind, table, edit).is_err())
            .unwrap_or(0);
        at(
            index,
            "",
            DbFailure::new(DbFailureKind::Query, e.to_string()),
        )
    })?;
    if statements.is_empty() {
        return Ok(0);
    }
    select_database(conn, table.database.as_deref()).map_err(|f| at(0, "", f))?;

    let begin = match kind {
        DbKind::MsSql => "BEGIN TRANSACTION",
        DbKind::MySql => "START TRANSACTION",
        DbKind::Postgres | DbKind::Sqlite => "BEGIN",
    };
    conn.execute(begin).map_err(|e| at(0, begin, failure(&e)))?;

    let opts = ExecOptions {
        read_only: false,
        timeout: Some(META_TIMEOUT),
        row_limit: None,
    };
    let mut affected = 0u64;
    for (index, text) in statements.iter().enumerate() {
        let started = Instant::now();
        let outcome = arm(o, &**conn, query)
            .and_then(|()| conn.execute_with(text, &opts).map_err(|e| failure(&e)));
        log(
            o,
            text,
            started,
            outcome.as_ref().ok().map(|r| r.rows_affected),
            outcome.as_ref().err(),
        );
        match outcome {
            Ok(done) => affected += done.rows_affected,
            Err(error) => {
                rollback(conn);
                return Err(at(index, text, error));
            }
        }
    }
    if let Err(error) = conn.execute("COMMIT") {
        rollback(conn);
        return Err(at(statements.len() - 1, "COMMIT", failure(&error)));
    }
    Ok(affected)
}

fn rollback(conn: &mut Box<dyn Connection>) {
    if let Err(error) = conn.execute("ROLLBACK") {
        tracing::warn!("a rollback failed: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ubiq_db::conn::ConnectionConfig;
    use ubiq_db::driver;

    fn file_db() -> (tempfile::TempDir, Box<dyn Connection>) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.sqlite");
        driver::create_sqlite_database(&path).unwrap();
        let mut config = ConnectionConfig::new(DbKind::Sqlite);
        config.path = Some(path);
        let mut conn = driver::connect(&config).unwrap();
        conn.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)")
            .unwrap();
        (dir, conn)
    }

    /// Layer 2 on its own: the driver is handed `read_only`, so a write that got past layer 1 is
    /// refused by the engine.
    #[test]
    fn the_engine_refuses_a_write_that_got_past_the_parser() {
        let (_dir, mut conn) = file_db();
        let refused = conn
            .query_with("INSERT INTO t VALUES (1)", &ExecOptions::read_only())
            .unwrap_err();
        assert!(matches!(refused, DbError::ReadOnly(_) | DbError::Query(_)));
        assert_eq!(conn.query("SELECT * FROM t", None).unwrap().rows.len(), 0);
    }

    #[test]
    fn a_result_is_cut_to_its_row_cap() {
        let mut set = ResultSet {
            columns: Vec::new(),
            rows: vec![vec![Value::Int(1)]; 5],
            truncated: false,
        };
        cap(&mut set, 3);
        assert_eq!((set.rows.len(), set.truncated), (3, true));
    }

    #[test]
    fn the_parser_gate_names_the_rule() {
        let refused = gate("DELETE FROM t", DbKind::Sqlite).unwrap_err();
        assert_eq!(refused.kind, DbFailureKind::ReadOnly);
        assert!(gate("SELECT 1", DbKind::Sqlite).is_ok());
    }
}
