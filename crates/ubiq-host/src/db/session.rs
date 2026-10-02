//! One session: a worker thread holding one driver connection and a queue of jobs.
//!
//! Every driver call blocks, so none runs on the coordinator's thread. A session connects lazily on
//! its first job, answers through a [`Mailbox`] addressed to the window that asked, and ends when
//! its sender is dropped — jobs already queued still run, and find their run forgotten (see
//! [`Runs`]), which is how a closed tab's backlog comes to nothing.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Instant;

use ubiq_db::conn::DbKind;
use ubiq_db::driver::{self, CancelHandle, Connection};
use ubiq_proto::bus::{ClientId, Mailbox};
use ubiq_proto::db::{DbConnState, DbFailure, DbFailureKind};
use ubiq_proto::ids::{DbConnId, DbQueryId, DbSessionId, ProjectId};
use ubiq_proto::messages::Message;

use super::jobs::{self, Job};
use super::secrets::Secrets;
use super::store;

/// What a session knows about where it came from. Everything a job needs that is not the
/// connection itself.
pub struct Origin {
    pub project: ProjectId,
    pub conn: DbConnId,
    /// `None` for the connection's meta session, which serves the tree.
    pub session: Option<DbSessionId>,
    /// The window that asked; every reply goes here.
    pub reply: Mailbox,
    /// The project's folder, against which a relative SQLite path is resolved. Empty when unknown.
    pub project_path: PathBuf,
    /// The project's `db.toml` and its local agent half.
    pub files: store::Files,
    pub secrets: Arc<Secrets>,
    pub runs: Arc<Runs>,
}

impl Origin {
    pub fn say(&self, message: Message) {
        self.reply.send(message);
    }

    fn state(&self, state: DbConnState) {
        self.say(Message::DbConnectionState {
            project_id: self.project,
            conn: self.conn,
            state,
        });
    }
}

/// The runs in flight, for `DbCancel`. A run is registered when its message arrives — before it
/// has reached the front of its session's queue — so a Stop that beats the run to the connection
/// still stops it; the session arms it with the driver's cancel handle when the statement starts.
///
/// A run that is not in the table is a run that was cancelled or whose session was dropped, and a
/// session finding its run missing treats it as cancelled.
#[derive(Default)]
pub struct Runs {
    table: Mutex<HashMap<DbQueryId, Run>>,
}

struct Run {
    /// The window that started it; `None` for an agent's run, which any window may stop — the
    /// user watching an agent's editor is not the one who started what runs in it.
    client: Option<ClientId>,
    session: DbSessionId,
    handle: Option<Arc<dyn CancelHandle>>,
    cancelled: bool,
}

impl Runs {
    fn table(&self) -> MutexGuard<'_, HashMap<DbQueryId, Run>> {
        self.table.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn begin(&self, client: ClientId, session: DbSessionId, query: DbQueryId) {
        self.insert(Some(client), session, query);
    }

    /// An agent's run: `session` is the shared editor's it shows in, or one nobody knows.
    pub fn begin_agent(&self, session: DbSessionId, query: DbQueryId) {
        self.insert(None, session, query);
    }

    fn insert(&self, client: Option<ClientId>, session: DbSessionId, query: DbQueryId) {
        self.table().insert(
            query,
            Run {
                client,
                session,
                handle: None,
                cancelled: false,
            },
        );
    }

    /// Stop an agent's run whatever its session: its caller gave up waiting.
    pub fn cancel_agent(&self, query: DbQueryId) {
        let handle = match self.table().get_mut(&query) {
            Some(run) if run.client.is_none() => {
                run.cancelled = true;
                run.handle.take()
            }
            _ => None,
        };
        fire(handle);
    }

    /// The statement is about to run on a connection whose cancel handle is `handle`. Answers
    /// whether the run has been cancelled already, in which case the statement must not start.
    pub fn arm(&self, query: DbQueryId, handle: Option<Arc<dyn CancelHandle>>) -> bool {
        match self.table().get_mut(&query) {
            Some(run) if !run.cancelled => {
                run.handle = handle;
                false
            }
            _ => true,
        }
    }

    pub fn end(&self, query: DbQueryId) {
        self.table().remove(&query);
    }

    /// Stop one run. Takes the handle out and calls it on a one-off thread: a PostgreSQL cancel
    /// opens a connection of its own.
    pub fn cancel(&self, client: ClientId, session: DbSessionId, query: DbQueryId) {
        let handle = match self.table().get_mut(&query) {
            Some(run)
                if run.session == session && run.client.is_none_or(|owner| owner == client) =>
            {
                run.cancelled = true;
                run.handle.take()
            }
            _ => None,
        };
        fire(handle);
    }

    /// Stop and forget every run of a session: it is being closed.
    pub fn forget_session(&self, client: ClientId, session: DbSessionId) {
        let mut handles = Vec::new();
        self.table().retain(|_, run| {
            if run.client == Some(client) && run.session == session {
                handles.extend(run.handle.take());
                false
            } else {
                true
            }
        });
        for handle in handles {
            fire(Some(handle));
        }
    }

    #[cfg(test)]
    pub fn in_flight(&self) -> usize {
        self.table().len()
    }
}

fn fire(handle: Option<Arc<dyn CancelHandle>>) {
    if let Some(handle) = handle {
        let spawned = std::thread::Builder::new()
            .name("ubiq-db-cancel".into())
            .spawn(move || {
                if let Err(error) = handle.cancel() {
                    tracing::warn!("a database cancel failed: {error}");
                }
            });
        if let Err(error) = spawned {
            tracing::warn!("a database cancel could not start a thread: {error}");
        }
    }
}

/// The handle a [`Db`](super::Db) keeps for a session's worker.
pub struct Session {
    pub conn: DbConnId,
    /// When a job was last queued on it: an agent's session is dropped once idle long enough.
    pub used: Instant,
    jobs: flume::Sender<Job>,
}

impl Session {
    pub fn spawn(origin: Origin) -> Self {
        let (tx, rx) = flume::unbounded();
        let conn = origin.conn;
        let worker = Worker {
            origin,
            conn: None,
            server: String::new(),
        };
        let started = std::thread::Builder::new()
            .name("ubiq-db-session".into())
            .spawn(move || worker.run(rx));
        if let Err(error) = started {
            // The receiver went with the closure; the first `send` fails and says so.
            tracing::warn!("a database session could not start a thread: {error}");
        }
        Self {
            conn,
            used: Instant::now(),
            jobs: tx,
        }
    }

    /// Queue a job. A session whose thread has gone gives the job back.
    pub fn send(&self, job: Job) -> Result<(), Box<Job>> {
        self.jobs.send(job).map_err(|e| Box::new(e.into_inner()))
    }
}

struct Worker {
    origin: Origin,
    conn: Option<Box<dyn Connection>>,
    /// The server's version string, as of the last connect.
    server: String,
}

impl Worker {
    fn run(mut self, jobs: flume::Receiver<Job>) {
        for job in jobs {
            let (mut conn, fresh) = match self.conn.take() {
                Some(conn) => (conn, false),
                None => match self.connect() {
                    Ok(conn) => (conn, true),
                    Err(failure) => {
                        if let Some(message) = job.refuse(&self.origin, failure) {
                            self.origin.say(message);
                        }
                        continue;
                    }
                },
            };
            if matches!(job, Job::Connect) {
                if !fresh {
                    self.origin.state(DbConnState::Connected {
                        server: self.server.clone(),
                    });
                }
                self.conn = Some(conn);
                continue;
            }
            // A connection that reports itself lost is dropped; the next job dials again.
            if jobs::run(&self.origin, &mut conn, job) {
                self.conn = Some(conn);
            }
        }
    }

    /// Dial the connection, saying where it stands. A password the host does not hold is a prompt,
    /// not an error: `NeedsPassword`.
    fn connect(&mut self) -> Result<Box<dyn Connection>, DbFailure> {
        let o = &self.origin;
        o.state(DbConnState::Connecting);
        let result = dial(o);
        match &result {
            Ok((_, server)) => {
                self.server = server.clone();
                o.state(DbConnState::Connected {
                    server: server.clone(),
                });
            }
            Err(failure) if failure.kind == DbFailureKind::NeedsPassword => {
                o.state(DbConnState::NeedsPassword)
            }
            Err(failure) => o.state(DbConnState::Failed(failure.clone())),
        }
        result.map(|(conn, _)| conn)
    }
}

fn dial(o: &Origin) -> Result<(Box<dyn Connection>, String), DbFailure> {
    let root = Some(o.project_path.as_path()).filter(|p| !p.as_os_str().is_empty());
    let stored = store::load(&o.files, root)
        .map_err(|e| DbFailure::new(DbFailureKind::Config, e.to_string()))?
        .into_iter()
        .find(|c| c.id == o.conn)
        .ok_or_else(|| {
            DbFailure::new(DbFailureKind::NotFound, "the connection no longer exists")
        })?;
    let mut config = stored.config;
    match o.secrets.password(o.project, o.conn) {
        Some(password) => config.password = Some(password),
        None if o.secrets.state(o.project, o.conn) == ubiq_proto::db::PasswordState::Missing => {
            return Err(DbFailure::new(
                DbFailureKind::NeedsPassword,
                "the saved password cannot be opened; it needs to be typed again",
            ));
        }
        None => {}
    }
    let asked_none =
        config.password.is_none() && config.kind != DbKind::Sqlite && !config.integrated_auth;
    let mut conn = driver::connect(&config).map_err(|e| {
        let mut failure = jobs::failure(&e);
        // A server that wants a password the host never held is a prompt, not a dead end.
        if asked_none
            && failure.kind == DbFailureKind::Connect
            && failure.message.to_lowercase().contains("password")
        {
            failure.kind = DbFailureKind::NeedsPassword;
        }
        failure
    })?;
    let server = version(&mut *conn);
    Ok((conn, server))
}

/// The server's version string, first line of it; empty when it will not say.
pub fn version(conn: &mut dyn Connection) -> String {
    let sql = match conn.kind() {
        DbKind::Sqlite => "SELECT sqlite_version()",
        DbKind::Postgres => "SHOW server_version",
        DbKind::MySql => "SELECT VERSION()",
        DbKind::MsSql => "SELECT @@VERSION",
    };
    conn.query(sql, Some(1))
        .ok()
        .and_then(|rows| rows.rows.into_iter().next())
        .and_then(|row| row.into_iter().next())
        .map(|cell| cell.to_string())
        .and_then(|text| text.lines().next().map(|line| line.trim().to_string()))
        .unwrap_or_default()
}
