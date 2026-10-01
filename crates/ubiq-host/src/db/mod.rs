//! A project's databases, host side: the saved connection list, the sealed passwords, and the
//! sessions that run SQL against them.
//!
//! [`Db`] is a concrete service, [`crate::kb::Kb`]'s shape: the coordinator owns one and hands it
//! each message of the family with the addresses to answer to. **It never blocks the coordinator.**
//! A tab's work goes to that tab's own session thread ([`session`]); the connection list, which
//! reads files and the keychain, goes to one ordered admin thread; a Test and a file creation get
//! one-off threads. Replies leave through a [`Mailbox`] addressed to the asking window.
//!
//! - `store` — `db.toml`, no secrets
//! - `secrets` — the sealed passwords
//! - `session` — one worker thread per table or SQL tab, and one meta session per
//!   `(window, connection)` for the tree; the cancel registry
//! - `jobs` — each message turned into a job, and the three read-only layers
//!
//! See `_docs/wip/db-explorer.md`.

pub mod jobs;
pub mod secrets;
pub mod session;
pub mod store;

#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, Once};

use ubiq_db::conn::ConnectionConfig;
use ubiq_db::driver;
use ubiq_proto::bus::{ClientId, Mailbox};
use ubiq_proto::db::{DbConnState, DbConnection, DbFailure, DbFailureKind, SecretEdit};
use ubiq_proto::ids::{DbConnId, DbProbeId, DbSessionId, ProjectId};
use ubiq_proto::messages::Message;

use crate::connectors::store::Store;
use crate::store::project_dir::ProjectDirs;
use jobs::Job;
use secrets::{KeySource, Secrets};
use session::{Origin, Runs, Session};
use store::StoredConnection;

/// Where to answer, and what the project's folder is: everything the coordinator knows that a
/// message of the family needs.
pub struct Ctx {
    pub client: ClientId,
    /// The window that asked.
    pub asker: Mailbox,
    /// Every window.
    pub everyone: Mailbox,
    /// The project's folder, or empty when the catalogue does not know it.
    pub project_path: PathBuf,
}

/// A session is a tab's (`Tab`) or a connection's tree (`Meta`), per window.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Key {
    Tab(ClientId, DbSessionId),
    Meta(ClientId, DbConnId),
}

type Admin = Box<dyn FnOnce(&Inner) + Send>;

/// The database family, as the coordinator holds it.
pub struct Db {
    inner: Arc<Inner>,
    /// The connection list's queue. One thread, so a save followed by a delete land in order.
    admin: flume::Sender<Admin>,
}

struct Inner {
    dirs: ProjectDirs,
    secrets: Arc<Secrets>,
    sessions: Mutex<HashMap<Key, Session>>,
    runs: Arc<Runs>,
}

impl Db {
    /// The service under `root`, its key kept in the OS keychain through the connector family's
    /// secret store. Constructing does no I/O.
    pub fn new(root: PathBuf) -> Self {
        let keys: Arc<dyn KeySource> = Arc::new(Store::open(&root));
        Self::with_keys(root, keys)
    }

    /// The same, with the key kept somewhere else — a test's, in memory.
    pub fn with_keys(root: PathBuf, keys: Arc<dyn KeySource>) -> Self {
        install_crypto_provider();
        let inner = Arc::new(Inner {
            dirs: ProjectDirs::new(root.clone()),
            secrets: Arc::new(Secrets::new(root, keys)),
            sessions: Mutex::new(HashMap::new()),
            runs: Arc::new(Runs::default()),
        });
        let (admin, queue) = flume::unbounded::<Admin>();
        let worker = inner.clone();
        let started = std::thread::Builder::new()
            .name("ubiq-db-admin".into())
            .spawn(move || {
                for job in queue {
                    job(&worker);
                }
            });
        if let Err(error) = started {
            tracing::warn!("the database connection list has no thread: {error}");
        }
        Self { inner, admin }
    }

    /// Answer one message of the family. Never blocks.
    pub fn handle(&self, ctx: Ctx, message: Message) {
        match message {
            Message::DbConnections { project_id } => self.on_admin(move |db| {
                ctx.asker.send(db.listed(project_id, &ctx.project_path));
            }),
            Message::SaveDbConnection {
                project_id,
                id,
                config,
                password,
                remember,
            } => self.on_admin(move |db| {
                db.save(project_id, &ctx.project_path, id, *config, password, remember);
                ctx.everyone.send(db.listed(project_id, &ctx.project_path));
            }),
            Message::DeleteDbConnection { project_id, id } => self.on_admin(move |db| {
                db.delete(project_id, &ctx.project_path, id);
                ctx.everyone.send(db.listed(project_id, &ctx.project_path));
            }),
            Message::TestDbConnection {
                project_id,
                probe,
                id,
                config,
                password,
            } => self.test(ctx, project_id, probe, id, *config, password),
            Message::DbPassword {
                project_id,
                conn,
                password,
                remember,
            } => self.on_admin(move |db| {
                let kept = db
                    .secrets
                    .remember(project_id, conn, password.expose(), remember);
                if let Err(error) = kept {
                    tracing::warn!("a database password could not be kept: {error}");
                    let _ = db
                        .secrets
                        .remember(project_id, conn, password.expose(), false);
                }
                let key = Key::Meta(ctx.client, conn);
                db.submit(&ctx, key, project_id, conn, None, Job::Connect);
            }),
            Message::DbTree {
                project_id,
                conn,
                node,
            } => self.inner.submit(
                &ctx,
                Key::Meta(ctx.client, conn),
                project_id,
                conn,
                None,
                Job::Tree { node },
            ),
            Message::DbTablePage {
                project_id,
                conn,
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
                self.inner.runs.begin(ctx.client, session, query);
                self.inner.submit(
                    &ctx,
                    Key::Tab(ctx.client, session),
                    project_id,
                    conn,
                    Some(session),
                    Job::Page {
                        session,
                        query,
                        table: *table,
                        filter,
                        order_by,
                        limit,
                        offset,
                        count,
                        read_only,
                    },
                );
            }
            Message::DbQuery {
                project_id,
                conn,
                session,
                query,
                database,
                statements,
                run,
                opts,
            } => {
                self.inner.runs.begin(ctx.client, session, query);
                self.inner.submit(
                    &ctx,
                    Key::Tab(ctx.client, session),
                    project_id,
                    conn,
                    Some(session),
                    Job::Query {
                        session,
                        query,
                        database,
                        statements,
                        run,
                        opts,
                    },
                );
            }
            Message::DbApplyEdits {
                project_id,
                conn,
                session,
                query,
                table,
                edits,
            } => {
                self.inner.runs.begin(ctx.client, session, query);
                self.inner.submit(
                    &ctx,
                    Key::Tab(ctx.client, session),
                    project_id,
                    conn,
                    Some(session),
                    Job::Edits {
                        session,
                        query,
                        table: *table,
                        edits,
                    },
                );
            }
            Message::DbCancel {
                session, query, ..
            } => self.inner.runs.cancel(ctx.client, session, query),
            Message::DbCloseSession { session, .. } => {
                let client = ctx.client;
                self.inner
                    .drop_where(|key, _| *key == Key::Tab(client, session));
            }
            Message::DbDisconnect { project_id, conn } => {
                let client = ctx.client;
                self.inner.drop_where(|key, held| {
                    held.conn == conn
                        && match key {
                            Key::Tab(owner, _) | Key::Meta(owner, _) => *owner == client,
                        }
                });
                ctx.asker.send(Message::DbConnectionState {
                    project_id,
                    conn,
                    state: DbConnState::Idle,
                });
            }
            Message::CreateDbFile { project_id, path } => create_file(ctx, project_id, path),
            other => tracing::warn!("the database family was sent {other:?}"),
        }
    }

    /// A window has gone: its sessions go, and what they were running is stopped.
    pub fn client_gone(&self, client: ClientId) {
        self.inner.drop_where(|key, _| match key {
            Key::Tab(owner, _) | Key::Meta(owner, _) => *owner == client,
        });
    }

    fn on_admin(&self, job: impl FnOnce(&Inner) + Send + 'static) {
        if self.admin.send(Box::new(job)).is_err() {
            tracing::warn!("the database connection list has no thread to run on");
        }
    }

    /// A Test dials on a thread of its own: it may take as long as a connect does, and must not
    /// queue the connection list behind it.
    fn test(
        &self,
        ctx: Ctx,
        project: ProjectId,
        probe: DbProbeId,
        id: Option<DbConnId>,
        mut config: ConnectionConfig,
        edit: SecretEdit,
    ) {
        let secrets = self.inner.secrets.clone();
        let spawned = std::thread::Builder::new()
            .name("ubiq-db-test".into())
            .spawn(move || {
                let pasted = store::scrub(&mut config);
                config.password = match edit {
                    SecretEdit::Set(secret) => Some(secret.expose().to_string()),
                    SecretEdit::Keep => id
                        .and_then(|id| secrets.password(project, id))
                        .or(pasted),
                    SecretEdit::Clear => None,
                };
                let root = Some(ctx.project_path.as_path()).filter(|p| !p.as_os_str().is_empty());
                store::absolutise(&mut config, root);
                let result = driver::connect(&config)
                    .and_then(|mut conn| conn.ping().map(|()| session::version(&mut *conn)))
                    .map_err(|e| jobs::failure(&e));
                ctx.asker.send(Message::DbTested {
                    project_id: project,
                    probe,
                    result,
                });
            });
        if let Err(error) = spawned {
            tracing::warn!("a database test could not start a thread: {error}");
        }
    }

    #[cfg(test)]
    fn session_count(&self) -> usize {
        self.inner.sessions().len()
    }
}

/// Create an empty SQLite file at a host path, on a thread of its own: it touches disk.
fn create_file(ctx: Ctx, project_id: ProjectId, path: String) {
    let spawned = std::thread::Builder::new()
        .name("ubiq-db-create".into())
        .spawn(move || {
            let reply = match driver::create_sqlite_database(Path::new(&path)) {
                Ok(()) => Message::DbFileCreated { project_id, path },
                Err(error) => Message::DbFileError {
                    project_id,
                    path,
                    message: error.to_string(),
                },
            };
            ctx.asker.send(reply);
        });
    if let Err(error) = spawned {
        tracing::warn!("a database file could not start a thread: {error}");
    }
}

impl Inner {
    fn sessions(&self) -> MutexGuard<'_, HashMap<Key, Session>> {
        self.sessions.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Queue a job on a session, starting the session if it is not there. A session whose thread
    /// has died is replaced once; past that the asker is told rather than left waiting.
    fn submit(
        &self,
        ctx: &Ctx,
        key: Key,
        project: ProjectId,
        conn: DbConnId,
        session: Option<DbSessionId>,
        mut job: Job,
    ) {
        let origin = |this: &Self| Origin {
            project,
            conn,
            session,
            reply: ctx.asker.clone(),
            project_path: ctx.project_path.clone(),
            list: this.dirs.data(project).db_connections(),
            secrets: this.secrets.clone(),
            runs: this.runs.clone(),
        };
        for _ in 0..2 {
            let sent = {
                let mut sessions = self.sessions();
                if sessions.get(&key).is_some_and(|held| held.conn != conn) {
                    sessions.remove(&key);
                }
                sessions
                    .entry(key)
                    .or_insert_with(|| Session::spawn(origin(self)))
                    .send(job)
            };
            match sent {
                Ok(()) => return,
                Err(back) => {
                    self.sessions().remove(&key);
                    job = *back;
                }
            }
        }
        if let Some(message) = job.refuse(
            &origin(self),
            DbFailure::new(
                DbFailureKind::Disconnected,
                "the database session could not be started",
            ),
        ) {
            ctx.asker.send(message);
        }
    }

    /// Close the sessions `pred` picks, and stop what a tab among them was running.
    fn drop_where(&self, mut pred: impl FnMut(&Key, &Session) -> bool) {
        let gone: Vec<Key> = {
            let mut sessions = self.sessions();
            let keys: Vec<Key> = sessions
                .iter()
                .filter(|(key, held)| pred(key, held))
                .map(|(key, _)| *key)
                .collect();
            for key in &keys {
                sessions.remove(key);
            }
            keys
        };
        for key in gone {
            if let Key::Tab(client, session) = key {
                self.runs.forget_session(client, session);
            }
        }
    }

    /// The project's connections, each with where its password stands.
    fn listed(&self, project: ProjectId, project_path: &Path) -> Message {
        let stored = self.load(project, project_path);
        let ids: Vec<DbConnId> = stored.iter().map(|c| c.id).collect();
        let states = self.secrets.states(project, &ids);
        Message::DbConnectionsListed {
            project_id: project,
            connections: stored
                .into_iter()
                .zip(states)
                .map(|(stored, password)| DbConnection {
                    id: stored.id,
                    config: stored.config,
                    password,
                })
                .collect(),
            keystore: self.secrets.keystore(),
        }
    }

    fn load(&self, project: ProjectId, project_path: &Path) -> Vec<StoredConnection> {
        let path = self.dirs.data(project).db_connections();
        store::load(&path, root_of(project_path)).unwrap_or_else(|error| {
            tracing::warn!("the database connections of project {project} are unreadable: {error}");
            Vec::new()
        })
    }

    fn write(&self, project: ProjectId, project_path: &Path, list: &[StoredConnection]) {
        let path = self.dirs.data(project).db_connections();
        if let Err(error) = store::save(&path, root_of(project_path), list) {
            // The list the window is answered with is what it sees whether or not it is durable,
            // the knowledge base's own degradation.
            tracing::warn!("the database connections of project {project} could not be written: {error}");
        }
    }

    fn save(
        &self,
        project: ProjectId,
        project_path: &Path,
        id: Option<DbConnId>,
        mut config: ConnectionConfig,
        password: SecretEdit,
        remember: bool,
    ) {
        let existing = id.is_some();
        let id = id.unwrap_or_else(DbConnId::generate);
        // A connection string pasted with a password in it: the password goes where passwords go.
        let pasted = store::scrub(&mut config);
        let mut list = self.load(project, project_path);
        match list.iter_mut().find(|c| c.id == id) {
            Some(held) => held.config = config,
            None => list.push(StoredConnection { id, config }),
        }
        self.write(project, project_path, &list);
        let typed = match password {
            SecretEdit::Set(secret) => Some(secret.expose().to_string()),
            SecretEdit::Keep => pasted,
            SecretEdit::Clear => {
                if let Err(error) = self.secrets.forget(project, id) {
                    tracing::warn!("a database password could not be forgotten: {error}");
                }
                None
            }
        };
        if let Some(typed) = typed.filter(|p| !p.is_empty()) {
            let kept = self.secrets.remember(project, id, &typed, remember);
            if let Err(error) = kept {
                tracing::warn!("a database password could not be kept: {error}");
                let _ = self.secrets.remember(project, id, &typed, false);
            }
        }
        if existing {
            self.drop_where(|_, held| held.conn == id);
        }
    }

    fn delete(&self, project: ProjectId, project_path: &Path, id: DbConnId) {
        self.drop_where(|_, held| held.conn == id);
        if let Err(error) = self.secrets.forget(project, id) {
            tracing::warn!("a database password could not be forgotten: {error}");
        }
        let mut list = self.load(project, project_path);
        list.retain(|c| c.id != id);
        self.write(project, project_path, &list);
    }
}

fn root_of(project_path: &Path) -> Option<&Path> {
    Some(project_path).filter(|p| !p.as_os_str().is_empty())
}

/// `mysql` calls `ClientConfig::builder()`, which panics when two providers are compiled in and
/// none is installed; `tiberius` honours an installed default. Install `ring`, once. Every other
/// rustls site in the host names its provider explicitly, so none of them changes.
fn install_crypto_provider() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        // `Err` is "already installed", which is as good.
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}
