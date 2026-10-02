//! The `ubiq-sql-read` and `ubiq-sql-write` servers: an agent's way into a project's databases.
//!
//! The two answer the same tools bar `execute`, which is the write server's alone. What an agent
//! may touch is the user's choice per connection (`DbAgentAccess`, set in the connection form):
//! the read server sees every `ro` and `rw` connection and always runs under the read-only guard;
//! the write server sees `rw` connections only. The host enforces both again in
//! [`crate::db::AgentDb`], so a mistake here cannot turn a read into a write.
//!
//! - `encode`: a result as TOON, with the cell rules and the output cap
//! - `blobs`: what a cut or binary cell was, kept for `get_blob`
//!
//! **Each call runs on a thread of its own** ([`super::server::handle`]), at most [`MAX_CALLS`] at
//! once, because a query waits on a server for as long as the user allowed. The slot is claimed
//! before the thread is spawned and released when the call ends.
//!
//! **Answers are text.** A tool returns `Value::String` holding TOON, which the listener sends as
//! it is. `get_blob` is the exception — JSON, because what it returns is an arbitrary text.

pub mod blobs;
pub mod encode;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use serde_json::{Map, Value, json};
use ubiq_db::conn::DbKind;
use ubiq_proto::db::{DbAgentAccess, DbFailure, DbFailureKind, DbListing, DbNode, DbOutcome};
use ubiq_proto::ids::{DbConnId, ProjectId};

use super::registry::AgentFacts;
use crate::db::agent::{AgentConn, AgentDb, AgentRun, StmtResult, split};
use crate::db::editors::{EditMode, EditedBy, EditorReport};
use blobs::Blobs;
use encode::{Header, Outcome, Statement};

/// The most SQL calls served at once; the next is refused as busy.
pub const MAX_CALLS: usize = 8;
/// The most statements an `execute` or an editor run may carry.
pub const MAX_STATEMENTS: usize = 20;
const DEFAULT_TIMEOUT_MS: u64 = 30_000;
const DEFAULT_BLOB_LENGTH: usize = 20_000;
const MAX_BLOB_LENGTH: usize = 100_000;
const MAX_NAME: usize = 64;

/// How the SQL tools reach the databases. Cheap to clone; every clone shares one blob cache and
/// one count of calls in flight.
#[derive(Clone)]
pub struct SqlReach {
    pub db: AgentDb,
    blobs: Arc<Blobs>,
    inflight: Arc<AtomicUsize>,
}

/// A claimed place among the [`MAX_CALLS`]; released on drop.
pub struct Slot(Arc<AtomicUsize>);

impl Drop for Slot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

impl SqlReach {
    pub fn new(db: AgentDb) -> Self {
        Self {
            db,
            blobs: Arc::new(Blobs::default()),
            inflight: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// A place for one more call, or `None` when [`MAX_CALLS`] are running.
    pub fn claim(&self) -> Option<Slot> {
        let taken = self.inflight.fetch_add(1, Ordering::SeqCst);
        if taken >= MAX_CALLS {
            self.inflight.fetch_sub(1, Ordering::SeqCst);
            return None;
        }
        Some(Slot(Arc::clone(&self.inflight)))
    }
}

/// Call one tool. `write` is whether this is the write server.
pub fn call(
    write: bool,
    tool: &str,
    arguments: &Value,
    facts: &AgentFacts,
    reach: &SqlReach,
) -> Result<Value, String> {
    let who = Who::new(facts)?;
    let args = Args(arguments);
    match tool {
        "list_connections" => list_connections(write, &who, reach),
        "list_objects" => list_objects(write, &who, &args, reach),
        "describe_table" => describe_table(write, &who, &args, reach),
        "query" => query(write, &who, &args, reach),
        "execute" if write => execute(&who, &args, reach),
        "get_blob" => get_blob(&who, &args, reach),
        "open_editor" => open_editor(write, &who, &args, reach),
        "read_editor" => read_editor(&who, &args, reach),
        "edit_editor" => edit_editor(&who, &args, reach),
        "run_editor" => run_editor(write, &who, &args, reach),
        other => Err(format!("unknown tool: {other}")),
    }
}

// ── who is asking ─────────────────────────────────────────────────────────────────────────────

struct Who<'a> {
    key: &'a str,
    title: &'a str,
    project: ProjectId,
    path: std::path::PathBuf,
}

impl<'a> Who<'a> {
    fn new(facts: &'a AgentFacts) -> Result<Self, String> {
        let project = facts
            .project
            .id
            .parse::<ProjectId>()
            .map_err(|_| "this agent's project is not one the host knows".to_string())?;
        Ok(Self {
            key: &facts.key,
            title: &facts.name,
            project,
            path: std::path::PathBuf::from(&facts.project.path),
        })
    }
}

// ── arguments ─────────────────────────────────────────────────────────────────────────────────

struct Args<'a>(&'a Value);

impl Args<'_> {
    fn text(&self, name: &str) -> Result<Option<&str>, String> {
        match self.0.get(name) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(text)) => Ok(Some(text)),
            Some(_) => Err(format!("'{name}' must be a string")),
        }
    }

    fn required_text(&self, name: &str) -> Result<&str, String> {
        self.text(name)?
            .filter(|text| !text.trim().is_empty())
            .ok_or_else(|| format!("'{name}' is required"))
    }

    /// A non-empty string, or `None`.
    fn optional(&self, name: &str) -> Result<Option<String>, String> {
        Ok(self
            .text(name)?
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_string))
    }

    fn name(&self, key: &str) -> Result<String, String> {
        let name = self.required_text(key)?.trim().to_string();
        if name.chars().count() > MAX_NAME {
            return Err(format!("'{key}' is at most {MAX_NAME} characters"));
        }
        Ok(name)
    }

    fn integer(&self, name: &str, min: u64, max: u64) -> Result<Option<u64>, String> {
        match self.0.get(name) {
            None | Some(Value::Null) => Ok(None),
            Some(value) => {
                let n = value
                    .as_u64()
                    .ok_or_else(|| format!("'{name}' must be a whole number"))?;
                if !(min..=max).contains(&n) {
                    return Err(format!("'{name}' must be between {min} and {max}"));
                }
                Ok(Some(n))
            }
        }
    }

    fn required_integer(&self, name: &str, min: u64, max: u64) -> Result<u64, String> {
        self.integer(name, min, max)?
            .ok_or_else(|| format!("'{name}' is required (between {min} and {max})"))
    }
}

/// The three sizes every run takes.
struct Limits {
    max_rows: usize,
    max_field_size: usize,
    timeout: Duration,
}

impl Limits {
    fn read(args: &Args) -> Result<Self, String> {
        Ok(Self {
            max_rows: args.required_integer("max_rows", 1, 10_000)? as usize,
            max_field_size: args.required_integer("max_field_size", 0, 100_000)? as usize,
            timeout: Duration::from_millis(
                args.integer("timeout_ms", 100, 300_000)?
                    .unwrap_or(DEFAULT_TIMEOUT_MS),
            ),
        })
    }
}

// ── connections ───────────────────────────────────────────────────────────────────────────────

fn engine(kind: DbKind) -> String {
    format!("{kind:?}").to_lowercase()
}

fn access(conn: &AgentConn) -> &'static str {
    match conn.agent.access {
        DbAgentAccess::Rw => "rw",
        _ => "ro",
    }
}

/// The connections this server shows: `rw` only on the write server.
fn eligible(write: bool, who: &Who, reach: &SqlReach) -> Vec<AgentConn> {
    reach
        .db
        .connections(who.project, &who.path)
        .into_iter()
        .filter(|c| !write || c.agent.access == DbAgentAccess::Rw)
        .collect()
}

/// Which connection a call means: the one named (by name, then id), else the default, else the
/// only one there is. On the write server an `ro` default is an error rather than a quiet fallback
/// to something else.
fn resolve(
    write: bool,
    who: &Who,
    reach: &SqlReach,
    named: Option<String>,
) -> Result<AgentConn, String> {
    let all = reach.db.connections(who.project, &who.path);
    let usable = |c: &AgentConn| !write || c.agent.access == DbAgentAccess::Rw;
    let names = |list: &[&AgentConn]| {
        list.iter()
            .map(|c| c.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    };
    if let Some(wanted) = named {
        let found = all
            .iter()
            .find(|c| c.name == wanted)
            .or_else(|| all.iter().find(|c| c.id.to_string() == wanted));
        return match found {
            Some(conn) if usable(conn) => Ok(conn.clone()),
            Some(conn) => Err(format!(
                "connection '{}' is read-only for agents; this server writes only to read-write \
                 connections. Use the ubiq-sql-read server to read it",
                conn.name
            )),
            None => {
                let available: Vec<&AgentConn> = all.iter().filter(|c| usable(c)).collect();
                Err(format!(
                    "no connection called '{wanted}'. Available: {}",
                    if available.is_empty() {
                        "none".to_string()
                    } else {
                        names(&available)
                    }
                ))
            }
        };
    }
    if let Some(default) = all.iter().find(|c| c.agent.default) {
        return if usable(default) {
            Ok(default.clone())
        } else {
            Err(format!(
                "the default connection '{}' is read-only for agents; name a read-write \
                 connection with 'connection'",
                default.name
            ))
        };
    }
    let available: Vec<&AgentConn> = all.iter().filter(|c| usable(c)).collect();
    match available.as_slice() {
        [only] => Ok((*only).clone()),
        [] => Err(
            "no database connection is open to agents here; the user enables one in the \
             connection's form"
                .to_string(),
        ),
        many => Err(format!(
            "there is no default connection and several are available; name one with \
             'connection': {}",
            names(many)
        )),
    }
}

/// A failure as the sentence an agent gets, with what to do about it where there is something.
fn failure_text(failure: &DbFailure) -> String {
    let hint = match failure.kind {
        DbFailureKind::NeedsPassword => Some(
            "ask the user to open this connection once in Ubiq's database panel and enter its \
             password, then retry",
        ),
        DbFailureKind::Timeout => Some("raise timeout_ms (at most 300000) or narrow the query"),
        DbFailureKind::ReadOnly => {
            Some("this is a read-only run; writes go through execute on the ubiq-sql-write server")
        }
        DbFailureKind::Connect | DbFailureKind::Disconnected => {
            Some("the connection could not be reached; tell the user rather than retrying at once")
        }
        _ => None,
    };
    match hint {
        Some(hint) => format!("{} ({hint})", failure.message),
        None => failure.message.clone(),
    }
}

// ── tools: structure ──────────────────────────────────────────────────────────────────────────

fn toon_of(value: &Value) -> Result<Value, String> {
    toon_format::encode(value, &toon_format::EncodeOptions::default())
        .map(Value::String)
        .map_err(|error| format!("the answer could not be encoded: {error}"))
}

fn list_connections(write: bool, who: &Who, reach: &SqlReach) -> Result<Value, String> {
    let rows: Vec<Value> = eligible(write, who, reach)
        .iter()
        .map(|c| {
            json!({
                "name": c.name,
                "engine": engine(c.kind),
                "database": c.database,
                "access": access(c),
                "default": c.agent.default,
                "description": c.agent.description,
            })
        })
        .collect();
    toon_of(&json!({"connections": rows}))
}

fn has_schemas(kind: DbKind) -> bool {
    matches!(kind, DbKind::Postgres | DbKind::MsSql)
}

fn list_objects(write: bool, who: &Who, args: &Args, reach: &SqlReach) -> Result<Value, String> {
    let conn = resolve(write, who, reach, args.optional("connection")?)?;
    let database = args.optional("database")?;
    let schema = args.optional("schema")?;
    let node = match (
        database.or_else(|| schema.as_ref().and(conn.database.clone())),
        schema,
    ) {
        (None, _) => DbNode::Databases,
        (Some(database), None) if has_schemas(conn.kind) => DbNode::Schemas { database },
        (Some(database), schema) => DbNode::Objects {
            database,
            schema: schema.filter(|_| has_schemas(conn.kind)),
        },
    };
    let listing = reach
        .db
        .tree(
            who.key,
            who.project,
            &who.path,
            conn.id,
            node,
            Duration::from_secs(60),
        )
        .map_err(|failure| failure_text(&failure))?;
    let value = match listing {
        DbListing::Names(names) => json!({"connection": conn.name, "names": names}),
        DbListing::Objects(objects) => {
            let rows: Vec<Value> = objects
                .iter()
                .map(|o| {
                    json!({
                        "kind": format!("{:?}", o.kind).to_lowercase(),
                        "schema": o.schema,
                        "name": o.name,
                        "approx_rows": o.approx_rows,
                        "comment": o.comment,
                    })
                })
                .collect();
            json!({"connection": conn.name, "objects": rows})
        }
        DbListing::Columns(_) => return Err("unexpected listing".to_string()),
    };
    toon_of(&value)
}

fn describe_table(write: bool, who: &Who, args: &Args, reach: &SqlReach) -> Result<Value, String> {
    let conn = resolve(write, who, reach, args.optional("connection")?)?;
    let table = args.required_text("table")?.trim().to_string();
    let table = ubiq_db::TableRef {
        database: args.optional("database")?.or_else(|| conn.database.clone()),
        schema: args.optional("schema")?,
        name: table,
    };
    let listing = reach
        .db
        .tree(
            who.key,
            who.project,
            &who.path,
            conn.id,
            DbNode::Columns { table },
            Duration::from_secs(60),
        )
        .map_err(|failure| failure_text(&failure))?;
    let DbListing::Columns(columns) = listing else {
        return Err("unexpected listing".to_string());
    };
    let rows: Vec<Value> = columns
        .iter()
        .map(|c| {
            json!({
                "name": c.name,
                "type": c.native_type,
                "nullable": c.nullable,
                "pk": c.is_pk,
                "default": c.default,
                "auto_increment": c.auto_increment,
                "computed": c.computed,
            })
        })
        .collect();
    toon_of(&json!({"connection": conn.name, "columns": rows}))
}

// ── tools: running SQL ────────────────────────────────────────────────────────────────────────

/// A statement's outcome, as the encoder takes it.
fn statement_of(result: StmtResult) -> Statement {
    let outcome = match result.outcome {
        Ok(DbOutcome::Rows(set)) => Outcome::Rows {
            columns: set.columns,
            rows: set.rows,
            more_rows: set.truncated,
        },
        Ok(DbOutcome::Affected(n)) => Outcome::Affected(n),
        Ok(DbOutcome::Plan(plan)) => Outcome::Rows {
            columns: vec![ubiq_db::ColumnMeta::new(DbKind::Postgres, "plan", "text")],
            rows: vec![vec![ubiq_db::value::Value::Text(plan.raw)]],
            more_rows: false,
        },
        Err(failure) => Outcome::Failed(failure_text(&failure)),
    };
    Statement {
        sql: result.sql,
        elapsed_ms: result.elapsed_ms,
        outcome,
    }
}

/// The run as TOON. A failed statement makes the whole thing an in-band error — with everything
/// that did run in it, so a half-applied `execute` is never silent.
fn answer(
    conn: &AgentConn,
    read_only: bool,
    results: Vec<StmtResult>,
    limits: &Limits,
    who: &Who,
    reach: &SqlReach,
) -> Result<Value, String> {
    let statements: Vec<Statement> = results.into_iter().map(statement_of).collect();
    let failed = statements
        .iter()
        .any(|s| matches!(s.outcome, Outcome::Failed(_)));
    let header = Header {
        connection: conn.name.clone(),
        engine: engine(conn.kind),
        read_only,
    };
    let text = encode::encode_run(
        &header,
        &statements,
        limits.max_field_size,
        &reach.blobs,
        who.key,
    );
    if failed {
        Err(text)
    } else {
        Ok(Value::String(text))
    }
}

/// The shared editor a `panel` names, created or retargeted and holding `sql`. Its session is where
/// the user sees the results.
fn panel_session(
    who: &Who,
    reach: &SqlReach,
    panel: &str,
    conn: &AgentConn,
    database: Option<String>,
    sql: String,
) -> ubiq_proto::ids::DbSessionId {
    reach
        .db
        .open_editor(
            who.key,
            who.title,
            who.project,
            panel,
            conn.id,
            database,
            Some(sql),
        )
        .editor
        .session
}

fn query(write: bool, who: &Who, args: &Args, reach: &SqlReach) -> Result<Value, String> {
    let sql = args.required_text("sql")?.to_string();
    let limits = Limits::read(args)?;
    let panel = args.optional("panel")?;
    if let Some(panel) = &panel
        && panel.chars().count() > MAX_NAME
    {
        return Err(format!("'panel' is at most {MAX_NAME} characters"));
    }
    let conn = resolve(write, who, reach, args.optional("connection")?)?;
    let database = args.optional("database")?;
    let panel =
        panel.map(|name| panel_session(who, reach, &name, &conn, database.clone(), sql.clone()));
    let results = reach.db.run(
        who.key,
        who.project,
        &who.path,
        AgentRun {
            conn: conn.id,
            database,
            statements: vec![sql],
            read_only: true,
            timeout: limits.timeout,
            row_limit: limits.max_rows,
            panel,
        },
    );
    answer(&conn, true, results, &limits, who, reach)
}

fn execute(who: &Who, args: &Args, reach: &SqlReach) -> Result<Value, String> {
    let statements: Vec<String> = match args.0.get("statements") {
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| match item {
                Value::String(s) if !s.trim().is_empty() => Ok(s.clone()),
                _ => Err("'statements' must be non-empty strings".to_string()),
            })
            .collect::<Result<_, _>>()?,
        _ => return Err("'statements' is required: a list of SQL statements".to_string()),
    };
    if statements.is_empty() || statements.len() > MAX_STATEMENTS {
        return Err(format!(
            "'statements' takes between 1 and {MAX_STATEMENTS} statements, not {}",
            statements.len()
        ));
    }
    let limits = Limits::read(args)?;
    let panel = args.optional("panel")?;
    if let Some(panel) = &panel
        && panel.chars().count() > MAX_NAME
    {
        return Err(format!("'panel' is at most {MAX_NAME} characters"));
    }
    let conn = resolve(true, who, reach, args.optional("connection")?)?;
    let database = args.optional("database")?;
    let panel = panel.map(|name| {
        let text = statements
            .iter()
            .map(|s| format!("{s};"))
            .collect::<Vec<_>>()
            .join("\n");
        panel_session(who, reach, &name, &conn, database.clone(), text)
    });
    let results = reach.db.run(
        who.key,
        who.project,
        &who.path,
        AgentRun {
            conn: conn.id,
            database,
            statements,
            read_only: false,
            timeout: limits.timeout,
            row_limit: limits.max_rows,
            panel,
        },
    );
    answer(&conn, false, results, &limits, who, reach)
}

fn get_blob(who: &Who, args: &Args, reach: &SqlReach) -> Result<Value, String> {
    let id = args.required_text("id")?.trim();
    let offset = args.integer("offset", 0, u64::MAX / 2)?.unwrap_or(0) as usize;
    let length = args
        .integer("length", 1, MAX_BLOB_LENGTH as u64)?
        .map(|n| n as usize)
        .unwrap_or(DEFAULT_BLOB_LENGTH);
    let slice = reach.blobs.slice(who.key, id, offset, length).ok_or_else(|| {
        format!(
            "no blob '{id}': it expired or never existed. Blobs are kept a few minutes after you \
             last read them; run the query again to get a fresh id"
        )
    })?;
    Ok(json!({
        "id": id,
        "encoding": if slice.binary { "base64" } else { "text" },
        "total": slice.total,
        "offset": slice.offset,
        "returned": slice.returned,
        "more": slice.offset + slice.returned < slice.total,
        "data": slice.data,
    }))
}

// ── tools: the shared editors ─────────────────────────────────────────────────────────────────

fn editor_value(conn_name: &str, report: &EditorReport) -> Map<String, Value> {
    let mut out = Map::new();
    out.insert("name".into(), json!(report.editor.name));
    out.insert("connection".into(), json!(conn_name));
    out.insert("database".into(), json!(report.editor.database));
    out.insert("rev".into(), json!(report.editor.rev));
    out.insert("created".into(), json!(report.created));
    out.insert(
        "changed_by_others".into(),
        json!(report.changed_since_last_write),
    );
    out.insert(
        "last_edited_by".into(),
        match &report.last_edited_by {
            None => Value::Null,
            Some(EditedBy::User) => json!("user"),
            Some(EditedBy::Agent(_)) => json!("agent"),
        },
    );
    out.insert("text".into(), json!(report.editor.text));
    out
}

fn conn_name(who: &Who, reach: &SqlReach, id: DbConnId) -> String {
    reach
        .db
        .connections(who.project, &who.path)
        .into_iter()
        .find(|c| c.id == id)
        .map(|c| c.name)
        .unwrap_or_else(|| "(no longer open to agents)".to_string())
}

fn open_editor(write: bool, who: &Who, args: &Args, reach: &SqlReach) -> Result<Value, String> {
    let name = args.name("name")?;
    let conn = resolve(write, who, reach, args.optional("connection")?)?;
    let text = args.text("text")?.map(str::to_string);
    let report = reach.db.open_editor(
        who.key,
        who.title,
        who.project,
        &name,
        conn.id,
        args.optional("database")?,
        text,
    );
    toon_of(&Value::Object(editor_value(&conn.name, &report)))
}

fn read_editor(who: &Who, args: &Args, reach: &SqlReach) -> Result<Value, String> {
    let name = args.name("name")?;
    let report = reach
        .db
        .read_editor(who.key, who.project, &name)
        .ok_or_else(|| no_editor(&name))?;
    let conn = conn_name(who, reach, report.editor.conn);
    toon_of(&Value::Object(editor_value(&conn, &report)))
}

fn no_editor(name: &str) -> String {
    format!("you have no editor called '{name}'; open_editor creates it")
}

fn edit_editor(who: &Who, args: &Args, reach: &SqlReach) -> Result<Value, String> {
    let name = args.name("name")?;
    let text = args
        .text("text")?
        .ok_or_else(|| "'text' is required".to_string())?
        .to_string();
    let mode = match args.required_text("mode")? {
        "force" => EditMode::Force,
        "overwrite_return_previous" => EditMode::OverwriteReturnPrevious,
        "keep_if_user_changed" => EditMode::KeepIfUserChanged,
        other => {
            return Err(format!(
                "unknown mode '{other}': use force, overwrite_return_previous or keep_if_user_changed"
            ));
        }
    };
    let report = reach
        .db
        .edit_editor(who.key, who.title, who.project, &name, text, mode)
        .map_err(|_| no_editor(&name))?;
    let mut out = Map::new();
    out.insert("rev".into(), json!(report.editor.rev));
    match mode {
        EditMode::Force => {
            out.insert(
                "changed_by_others".into(),
                json!(report.changed_since_last_write),
            );
        }
        EditMode::OverwriteReturnPrevious => {
            if let Some(previous) = report.previous_text {
                out.insert("changed_by_others".into(), json!(true));
                out.insert("previous_text".into(), json!(previous));
            }
        }
        EditMode::KeepIfUserChanged => {
            out.insert("kept".into(), json!(report.kept));
            if report.kept {
                out.insert("current_text".into(), json!(report.editor.text));
            }
        }
    }
    toon_of(&Value::Object(out))
}

fn run_editor(write: bool, who: &Who, args: &Args, reach: &SqlReach) -> Result<Value, String> {
    let name = args.name("name")?;
    let limits = Limits::read(args)?;
    let report = reach
        .db
        .read_editor(who.key, who.project, &name)
        .ok_or_else(|| no_editor(&name))?;
    let conn = reach
        .db
        .connections(who.project, &who.path)
        .into_iter()
        .find(|c| c.id == report.editor.conn)
        .ok_or_else(|| "that editor's connection is no longer open to agents".to_string())?;
    let read_only = !(write && conn.agent.access == DbAgentAccess::Rw);
    if split(&report.editor.text, conn.kind).len() > MAX_STATEMENTS {
        return Err(format!(
            "an editor run takes at most {MAX_STATEMENTS} statements"
        ));
    }
    let results = reach
        .db
        .run_editor(
            who.key,
            who.project,
            &who.path,
            &name,
            read_only,
            limits.timeout,
            limits.max_rows,
        )
        .map_err(|failure| failure_text(&failure))?;
    answer(&conn, read_only, results, &limits, who, reach)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn at_most_eight_calls_hold_a_slot_and_a_dropped_slot_frees_one() {
        let inflight = Arc::new(AtomicUsize::new(0));
        let claim = || {
            let taken = inflight.fetch_add(1, Ordering::SeqCst);
            if taken >= MAX_CALLS {
                inflight.fetch_sub(1, Ordering::SeqCst);
                return None;
            }
            Some(Slot(Arc::clone(&inflight)))
        };
        let held: Vec<Slot> = (0..MAX_CALLS).map(|_| claim().unwrap()).collect();
        assert!(claim().is_none());
        drop(held);
        assert!(claim().is_some());
        assert_eq!(inflight.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn sizes_are_required_and_bounded() {
        let ok = json!({"max_rows": 10, "max_field_size": 100});
        let limits = Limits::read(&Args(&ok)).unwrap();
        assert_eq!(limits.timeout, Duration::from_millis(30_000));
        assert!(Limits::read(&Args(&json!({"max_field_size": 1}))).is_err());
        assert!(Limits::read(&Args(&json!({"max_rows": 1}))).is_err());
        assert!(Limits::read(&Args(&json!({"max_rows": 0, "max_field_size": 1}))).is_err());
        assert!(Limits::read(&Args(&json!({"max_rows": 10_001, "max_field_size": 1}))).is_err());
        assert!(
            Limits::read(&Args(
                &json!({"max_rows": 1, "max_field_size": 1, "timeout_ms": 5})
            ))
            .is_err()
        );
    }
}
