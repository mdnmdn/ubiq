//! The database family as an agent reaches it: through the SQL MCP servers, never a window.
//!
//! [`AgentDb`] is a cloneable handle on the same service [`Db`](super::Db) is, for a thread that
//! is not the coordinator's — every method **blocks** until the session answered. Its sessions are
//! keyed by the agent and the connection (`Key::Agent`), reply to [`Mailbox::nowhere`] so a
//! connection state or a password prompt never reaches a window, and are dropped once idle for
//! [`IDLE`]. Only a connection whose agent access is `ro` or `rw` is visible here; a read-only run
//! applies the parser gate *and* the driver's read-only option (`jobs`' layers 1 and 2).
//!
//! The shared editors ([`super::editors`]) are reached through it too: an agent's change is
//! broadcast as `DbEditorChanged`, and a run shown in one is announced as `DbAgentRun` with its
//! results following as `DbQueryResult`, to every window.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use ubiq_db::conn::DbKind;
use ubiq_db::sql;
use ubiq_proto::bus::Mailbox;
use ubiq_proto::db::{
    DbAgentAccess, DbAgentSettings, DbFailure, DbFailureKind, DbListing, DbNode, DbOutcome,
    StructureScope,
};
use ubiq_proto::ids::{DbConnId, DbQueryId, DbSessionId, ProjectId};
use ubiq_proto::messages::Message;

use super::editors::{EditMode, EditorReport};
use super::jobs::Job;
use super::session::Origin;
use super::store::StoredConnection;
use super::{Inner, Key};

/// An agent's session is closed after this long without a call.
pub const IDLE: Duration = Duration::from_secs(10 * 60);
/// How much longer than its own timeout a run is waited for before it is cancelled.
const GRACE: Duration = Duration::from_secs(10);

/// A connection an agent may use.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentConn {
    pub id: DbConnId,
    pub name: String,
    pub kind: DbKind,
    /// The configured database, if the connection names one.
    pub database: Option<String>,
    pub agent: DbAgentSettings,
    /// The connection itself is read-only, whatever its agent access.
    pub read_only: bool,
}

/// One run an agent asks for.
#[derive(Clone, Debug)]
pub struct AgentRun {
    pub conn: DbConnId,
    /// `None` is the connection's configured database.
    pub database: Option<String>,
    pub statements: Vec<String>,
    /// Refused on a connection the agent may only read when `false`.
    pub read_only: bool,
    pub timeout: Duration,
    /// Capped at the host's own maximum.
    pub row_limit: usize,
    /// A shared editor's session to show the results in, to every window.
    pub panel: Option<DbSessionId>,
}

/// What one statement produced.
#[derive(Clone, Debug, PartialEq)]
pub struct StmtResult {
    pub sql: String,
    /// The statement's label (`"SELECT"`, `"UPDATE"`, …); empty when it did not parse.
    pub kind: String,
    pub elapsed_ms: u64,
    pub outcome: Result<DbOutcome, DbFailure>,
}

/// Where a run shown in a shared editor sends its results.
pub struct Panel {
    pub session: DbSessionId,
    pub everyone: Mailbox,
}

impl Panel {
    pub fn show(
        &self,
        o: &Origin,
        query: DbQueryId,
        index: usize,
        last: bool,
        elapsed_ms: u64,
        result: &Result<DbOutcome, DbFailure>,
    ) {
        self.everyone.send(Message::DbQueryResult {
            project_id: o.project,
            session: self.session,
            query,
            index: u32::try_from(index).unwrap_or(u32::MAX),
            last,
            result: result.clone().map(Box::new),
            elapsed_ms,
        });
    }
}

/// The agent-facing handle. Cheap to clone; `Send + Sync`.
#[derive(Clone)]
pub struct AgentDb {
    inner: Arc<Inner>,
    everyone: Mailbox,
}

impl AgentDb {
    pub(super) fn new(inner: Arc<Inner>, everyone: Mailbox) -> Self {
        Self { inner, everyone }
    }

    /// The project's connections an agent may see: access `ro` or `rw`, in saved order.
    pub fn connections(&self, project: ProjectId, project_path: &Path) -> Vec<AgentConn> {
        self.inner
            .load(project, project_path)
            .into_iter()
            .filter(|c| c.agent.access != DbAgentAccess::None)
            .map(|c| AgentConn {
                id: c.id,
                name: c.config.name,
                kind: c.config.kind,
                database: c.config.database,
                agent: c.agent,
                read_only: c.config.read_only,
            })
            .collect()
    }

    /// Run statements on one connection, in order, stopping at the first failure. Blocks for at
    /// most `run.timeout` plus a grace, after which the run is cancelled and reported `Timeout`.
    pub fn run(
        &self,
        agent_key: &str,
        project: ProjectId,
        project_path: &Path,
        run: AgentRun,
    ) -> Vec<StmtResult> {
        let first = run.statements.first().cloned().unwrap_or_default();
        let fail = |failure: DbFailure| {
            vec![StmtResult {
                sql: first.clone(),
                kind: String::new(),
                elapsed_ms: 0,
                outcome: Err(failure),
            }]
        };
        let stored = match self.eligible(project, project_path, run.conn) {
            Ok(stored) => stored,
            Err(failure) => return fail(failure),
        };
        let may_write = stored.agent.access == DbAgentAccess::Rw && !stored.config.read_only;
        if !run.read_only && !may_write {
            return fail(DbFailure::new(
                DbFailureKind::ReadOnly,
                "read-only: agents may only read this connection",
            ));
        }
        let query = DbQueryId::generate();
        let session = run.panel.unwrap_or_else(DbSessionId::generate);
        self.inner.runs.begin_agent(session, query);
        if let Some(session) = run.panel {
            self.everyone.send(Message::DbAgentRun {
                project_id: project,
                session,
                query,
                statements: run.statements.clone(),
            });
        }
        let (reply, answer) = flume::bounded(1);
        let job = Job::Agent {
            query,
            database: run.database.or(stored.config.database),
            statements: run.statements,
            read_only: run.read_only || !may_write,
            timeout: run.timeout,
            row_limit: run.row_limit,
            panel: run.panel.map(|session| Panel {
                session,
                everyone: self.everyone.clone(),
            }),
            reply,
        };
        self.submit(agent_key, project, project_path, run.conn, run.panel, job);
        match answer.recv_timeout(run.timeout + GRACE) {
            Ok(results) => results,
            Err(_) => {
                self.inner.runs.cancel_agent(query);
                fail(DbFailure::new(
                    DbFailureKind::Timeout,
                    "the run did not answer in time and was cancelled",
                ))
            }
        }
    }

    /// One node of a connection's structure tree. Blocks for at most `timeout`.
    pub fn tree(
        &self,
        agent_key: &str,
        project: ProjectId,
        project_path: &Path,
        conn: DbConnId,
        node: DbNode,
        timeout: Duration,
    ) -> Result<DbListing, DbFailure> {
        self.eligible(project, project_path, conn)?;
        let (reply, answer) = flume::bounded(1);
        self.submit(
            agent_key,
            project,
            project_path,
            conn,
            None,
            Job::AgentTree { node, reply },
        );
        answer.recv_timeout(timeout).unwrap_or_else(|_| {
            Err(DbFailure::new(
                DbFailureKind::Timeout,
                "the connection did not answer in time",
            ))
        })
    }

    /// A connection's structure as DBML: `database` (`None` is the connection's configured one,
    /// else the session's current one) within `scope`. Catalog reads only, on the agent's session.
    /// Blocks for at most `timeout`.
    #[allow(clippy::too_many_arguments)]
    pub fn dbml(
        &self,
        agent_key: &str,
        project: ProjectId,
        project_path: &Path,
        conn: DbConnId,
        database: Option<String>,
        scope: StructureScope,
        timeout: Duration,
    ) -> Result<String, DbFailure> {
        let stored = self.eligible(project, project_path, conn)?;
        let (reply, answer) = flume::bounded(1);
        self.submit(
            agent_key,
            project,
            project_path,
            conn,
            None,
            Job::AgentDbml {
                database: database.or(stored.config.database),
                scope,
                reply,
            },
        );
        answer.recv_timeout(timeout).unwrap_or_else(|_| {
            Err(DbFailure::new(
                DbFailureKind::Timeout,
                "the connection did not answer in time",
            ))
        })
    }

    // ---- the shared editors ---------------------------------------------------------------------

    /// Open (or create) the agent's editor `name` on `conn`; `sql`, when given, replaces its text.
    /// Broadcasts `DbEditorChanged` when anything changed, revealing the tab when it is new.
    #[allow(clippy::too_many_arguments)]
    pub fn open_editor(
        &self,
        agent_key: &str,
        agent_title: &str,
        project: ProjectId,
        name: &str,
        conn: DbConnId,
        database: Option<String>,
        sql: Option<String>,
    ) -> EditorReport {
        let (report, touched) =
            self.inner
                .editors
                .open(agent_key, agent_title, project, name, conn, database, sql);
        if touched {
            self.changed(project, &report, report.created);
        }
        report
    }

    /// The agent's editor `name`, if it has one.
    pub fn read_editor(
        &self,
        agent_key: &str,
        project: ProjectId,
        name: &str,
    ) -> Option<EditorReport> {
        self.inner.editors.read(agent_key, project, name)
    }

    /// Replace the text of the agent's editor `name` under `mode`. `NotFound` when it has none.
    pub fn edit_editor(
        &self,
        agent_key: &str,
        agent_title: &str,
        project: ProjectId,
        name: &str,
        sql: String,
        mode: EditMode,
    ) -> Result<EditorReport, DbFailure> {
        let (report, touched) = self
            .inner
            .editors
            .edit(agent_key, agent_title, project, name, sql, mode)
            .ok_or_else(|| no_editor(name))?;
        if touched {
            self.changed(project, &report, false);
        }
        Ok(report)
    }

    /// Run the agent's editor `name` as it now stands, shown in its tab: its text split into
    /// statements, on its connection and database.
    #[allow(clippy::too_many_arguments)]
    pub fn run_editor(
        &self,
        agent_key: &str,
        project: ProjectId,
        project_path: &Path,
        name: &str,
        read_only: bool,
        timeout: Duration,
        row_limit: usize,
    ) -> Result<Vec<StmtResult>, DbFailure> {
        let editor = self
            .inner
            .editors
            .get(agent_key, project, name)
            .ok_or_else(|| no_editor(name))?;
        let kind = self
            .eligible(project, project_path, editor.conn)?
            .config
            .kind;
        Ok(self.run(
            agent_key,
            project,
            project_path,
            AgentRun {
                conn: editor.conn,
                database: editor.database,
                statements: split(&editor.text, kind),
                read_only,
                timeout,
                row_limit,
                panel: Some(editor.session),
            },
        ))
    }

    fn changed(&self, project: ProjectId, report: &EditorReport, reveal: bool) {
        self.everyone.send(Message::DbEditorChanged {
            project_id: project,
            editor: Box::new(report.editor.clone()),
            reveal,
        });
    }

    /// The saved connection, if an agent may use it at all.
    fn eligible(
        &self,
        project: ProjectId,
        project_path: &Path,
        conn: DbConnId,
    ) -> Result<StoredConnection, DbFailure> {
        self.inner
            .load(project, project_path)
            .into_iter()
            .find(|c| c.id == conn && c.agent.access != DbAgentAccess::None)
            .ok_or_else(|| {
                DbFailure::new(
                    DbFailureKind::NotFound,
                    "no connection open to agents has that id",
                )
            })
    }

    fn submit(
        &self,
        agent_key: &str,
        project: ProjectId,
        project_path: &Path,
        conn: DbConnId,
        panel: Option<DbSessionId>,
        job: Job,
    ) {
        self.inner
            .drop_where(|key, held| matches!(key, Key::Agent(..)) && held.used.elapsed() > IDLE);
        self.inner.submit_to(
            &Mailbox::nowhere(),
            project_path,
            Key::Agent(agent_key.to_string(), conn),
            project,
            conn,
            panel,
            job,
        );
    }
}

fn no_editor(name: &str) -> DbFailure {
    DbFailure::new(
        DbFailureKind::NotFound,
        format!("no query editor is called {name:?}"),
    )
}

/// A script cut into its statements, as the parser sees them.
pub fn split(text: &str, kind: DbKind) -> Vec<String> {
    sql::analyze(text, kind.into())
        .statements
        .into_iter()
        .map(|s| text[s.byte_range].to_string())
        .filter(|s| !s.trim().is_empty())
        .collect()
}

/// The MCP threads share one handle.
const _: fn() = || {
    fn check<T: Send + Sync>() {}
    check::<AgentDb>();
};
