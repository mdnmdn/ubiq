//! A project's saved database connections, as one TOML file: `db.toml`.
//!
//! On the shared side of the project's data directory (`ProjectData::db_connections`), so a
//! project-managed project commits it and a team shares its connection list. **It never holds a
//! secret**: the password is cleared and every parameter that names one is dropped on the way out,
//! whatever the caller handed over. Passwords live sealed in [`super::secrets`].
//!
//! `kb.rs`'s convention: `version` at the top, a missing file read as empty, written with
//! [`write_atomic`]. A file written by a newer Ubiq is [`StoreError::UnknownVersion`] and is never
//! overwritten.
//!
//! **A connection's agent settings are split across two files** ([`split`] and [`join`] are the
//! only places that know it): the `description` is shared, in `db.toml`, because a team reads the
//! same one; the `access` and `default` are this machine's, in
//! `<config root>/projects/<ulid>/local/db-agents.toml` ([`AGENTS_FILE`]), because what an agent
//! on *this* machine may touch is not something a commit should hand a teammate. Folding them back into `db.toml` is a change to
//! those two functions and [`Files`].

use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use ubiq_db::conn::ConnectionConfig;
use ubiq_proto::db::{DbAgentAccess, DbAgentSettings};
use ubiq_proto::ids::{DbConnId, ProjectId};

use crate::atomic::{preserve_aside, write_atomic};
use crate::store::StoreError;
use crate::store::project_dir::{ProjectData, ProjectDirs};

/// The format this Ubiq writes and understands, of both files.
pub const DB_VERSION: u32 = 1;

/// The machine-local half of the agent settings, under the project's `local/`.
pub const AGENTS_FILE: &str = "db-agents.toml";

/// Connection-string keys that carry secret material. Dropped, never written.
const SECRET_PARAMS: &[&str] = &["password", "pwd", "sslpassword", "sslkey"];

/// One saved connection: its stable id, its definition (`config.password` always `None`) and what
/// an agent may do with it, already normalised by [`normalise`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredConnection {
    pub id: DbConnId,
    pub config: ConnectionConfig,
    pub agent: DbAgentSettings,
}

impl StoredConnection {
    pub fn new(id: DbConnId, config: ConnectionConfig) -> Self {
        Self {
            id,
            config,
            agent: DbAgentSettings::default(),
        }
    }
}

/// The two files a project's connection list is kept in.
#[derive(Clone, Debug)]
pub struct Files {
    /// `db.toml`, shared.
    pub list: PathBuf,
    /// `<config root>/projects/<ulid>/local/db-agents.toml`, this machine's.
    pub agents: PathBuf,
}

impl Files {
    /// `db.toml` follows the project's storage mode; the agent file, like the sealed passwords,
    /// is always under the config root — never in `.ubiq/`, inside the project's folder, where the
    /// agents it governs work and could grant themselves access.
    pub fn of(root: &Path, dirs: &ProjectDirs, project: ProjectId) -> Self {
        Self {
            list: dirs.data(project).db_connections(),
            agents: ProjectData::under_config(root, project)
                .local()
                .join(AGENTS_FILE),
        }
    }
}

/// A connection as `db.toml` holds it.
#[derive(Debug, Serialize, Deserialize)]
struct SharedRow {
    id: DbConnId,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    description: String,
    #[serde(flatten)]
    config: ConnectionConfig,
}

/// A connection as `db-agents.toml` holds it.
#[derive(Debug, Serialize, Deserialize)]
struct LocalRow {
    id: DbConnId,
    #[serde(default)]
    access: DbAgentAccess,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    default: bool,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct DbFile {
    version: u32,
    #[serde(default, rename = "connection", skip_serializing_if = "Vec::is_empty")]
    connections: Vec<SharedRow>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct AgentsFile {
    version: u32,
    #[serde(default, rename = "agent", skip_serializing_if = "Vec::is_empty")]
    agents: Vec<LocalRow>,
}

/// The one place that says which half of a connection goes in which file.
fn split(connection: &StoredConnection) -> (SharedRow, Option<LocalRow>) {
    let agent = &connection.agent;
    let local = (agent.access != DbAgentAccess::None || agent.default).then_some(LocalRow {
        id: connection.id,
        access: agent.access,
        default: agent.default,
    });
    let shared = SharedRow {
        id: connection.id,
        description: agent.description.clone(),
        config: connection.config.clone(),
    };
    (shared, local)
}

/// [`split`]'s inverse. A connection the local file does not name has no agent access.
fn join(shared: SharedRow, local: Option<&LocalRow>) -> StoredConnection {
    StoredConnection {
        id: shared.id,
        config: shared.config,
        agent: DbAgentSettings {
            access: local.map_or(DbAgentAccess::None, |l| l.access),
            default: local.is_some_and(|l| l.default),
            description: shared.description,
        },
    }
}

/// The agent settings' invariants, applied on every load and every save: a read-only connection is
/// never `rw`, a connection agents cannot see is never the default, and a project has at most one
/// default — the first, when a hand-edited file names several.
pub fn normalise(list: &mut [StoredConnection]) {
    let mut seen_default = false;
    for connection in list {
        let agent = &mut connection.agent;
        if connection.config.read_only && agent.access == DbAgentAccess::Rw {
            agent.access = DbAgentAccess::Ro;
        }
        if agent.access == DbAgentAccess::None {
            agent.default = false;
        }
        if agent.default {
            agent.default = !seen_default;
            seen_default = true;
        }
    }
}

/// Take every secret out of a config and return the password, if it had one — a connection string
/// pasted with a password in it puts the password into the form's field, not the file. The other
/// secret parameters have no home and are dropped.
pub fn scrub(config: &mut ConnectionConfig) -> Option<String> {
    config
        .params
        .retain(|key, _| !SECRET_PARAMS.iter().any(|s| key.eq_ignore_ascii_case(s)));
    config
        .password
        .take()
        .filter(|password| !password.is_empty())
}

/// A project's connections. A missing file is no connections; one that does not parse is moved
/// aside and reported, one from a newer Ubiq is left alone and reported.
///
/// `project_root` resolves a SQLite path that was saved relative to it. The local agent file is
/// best-effort: one that cannot be read leaves every connection without agent access, which is the
/// safe reading, and is reported.
pub fn load(
    files: &Files,
    project_root: Option<&Path>,
) -> Result<Vec<StoredConnection>, StoreError> {
    let shared = read::<DbFile>(&files.list)?.unwrap_or_default();
    let local = read::<AgentsFile>(&files.agents)
        .unwrap_or_else(|error| {
            tracing::warn!("the database agent settings are unreadable: {error}");
            None
        })
        .unwrap_or_default();
    let mut list: Vec<StoredConnection> = shared
        .connections
        .into_iter()
        .map(|mut row| {
            // A hand-edited file may carry a password; it is not honoured.
            scrub(&mut row.config);
            absolutise(&mut row.config, project_root);
            let id = row.id;
            join(row, local.agents.iter().find(|l| l.id == id))
        })
        .collect();
    normalise(&mut list);
    Ok(list)
}

/// Write a project's whole connection list, atomically, scrubbed and normalised: `db.toml`, then
/// the local agent file. Refuses to replace a file a newer Ubiq wrote.
pub fn save(
    files: &Files,
    project_root: Option<&Path>,
    connections: &[StoredConnection],
) -> Result<(), StoreError> {
    let mut connections = connections.to_vec();
    normalise(&mut connections);
    let mut shared = DbFile {
        version: DB_VERSION,
        connections: Vec::new(),
    };
    let mut local = AgentsFile {
        version: DB_VERSION,
        agents: Vec::new(),
    };
    for mut connection in connections {
        scrub(&mut connection.config);
        relativise(&mut connection.config, project_root);
        let (row, agent) = split(&connection);
        shared.connections.push(row);
        local.agents.extend(agent);
    }
    write(&files.list, &shared)?;
    write(&files.agents, &local)
}

/// A TOML file of ours. A missing one is `None`; one that does not parse is moved aside.
fn read<T: DeserializeOwned>(path: &Path) -> Result<Option<T>, StoreError> {
    let raw = match std::fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(StoreError::Io {
                path: path.to_path_buf(),
                source,
            });
        }
    };
    check_version(path, &raw)?;
    toml::from_str::<T>(&raw)
        .map(Some)
        .map_err(|error| StoreError::Parse {
            path: path.to_path_buf(),
            preserved_as: preserve_aside(path, chrono::Utc::now()).ok(),
            message: error.to_string(),
        })
}

fn write<T: Serialize>(path: &Path, file: &T) -> Result<(), StoreError> {
    if let Ok(raw) = std::fs::read_to_string(path) {
        check_version(path, &raw)?;
    }
    let body = toml::to_string_pretty(file).map_err(|error| StoreError::Io {
        path: path.to_path_buf(),
        source: std::io::Error::other(error),
    })?;
    write_atomic(path, body.as_bytes()).map_err(|source| StoreError::Io {
        path: path.to_path_buf(),
        source,
    })
}

fn check_version(path: &Path, raw: &str) -> Result<(), StoreError> {
    #[derive(Deserialize)]
    struct Probe {
        #[serde(default)]
        version: u32,
    }
    match toml::from_str::<Probe>(raw) {
        Ok(Probe { version }) if version > DB_VERSION => Err(StoreError::UnknownVersion {
            path: path.to_path_buf(),
            found: version,
            supported: DB_VERSION,
        }),
        _ => Ok(()),
    }
}

/// A file database inside the project is written relative to its root, with `/` so a teammate's
/// clone on another platform resolves it.
fn relativise(config: &mut ConnectionConfig, root: Option<&Path>) {
    let (Some(root), Some(path)) = (root, config.path.as_ref()) else {
        return;
    };
    if let Ok(inside) = path.strip_prefix(root) {
        let text = inside.to_string_lossy().replace('\\', "/");
        config.path = Some(PathBuf::from(text));
    }
}

/// Resolve a relative SQLite path against the project's folder.
pub(super) fn absolutise(config: &mut ConnectionConfig, root: Option<&Path>) {
    let (Some(root), Some(path)) = (root, config.path.as_ref()) else {
        return;
    };
    if path.is_relative() && path != Path::new(":memory:") {
        config.path = Some(root.join(path));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ubiq_db::conn::DbKind;

    fn postgres() -> StoredConnection {
        let mut config = ConnectionConfig::new(DbKind::Postgres);
        config.name = "orders (staging)".into();
        config.host = Some("db.staging.internal".into());
        config.database = Some("orders".into());
        config.user = Some("reader".into());
        config.read_only = true;
        config.port = Some(5433);
        config
            .params
            .insert("application_name".into(), "ubiq".into());
        StoredConnection::new(DbConnId::generate(), config)
    }

    fn sqlite(path: &Path) -> StoredConnection {
        let mut config = ConnectionConfig::new(DbKind::Sqlite);
        config.name = "local cache".into();
        config.path = Some(path.to_path_buf());
        StoredConnection::new(DbConnId::generate(), config)
    }

    /// `db.toml` at `path`, the agent file in a `local/` beside it.
    fn files(path: &Path) -> Files {
        Files {
            list: path.to_path_buf(),
            agents: path.with_file_name("local").join(AGENTS_FILE),
        }
    }

    fn agent(access: DbAgentAccess, default: bool, description: &str) -> DbAgentSettings {
        DbAgentSettings {
            access,
            default,
            description: description.into(),
        }
    }

    #[test]
    fn agent_settings_split_across_the_two_files_and_join_again() {
        let dir = tempfile::tempdir().expect("dir");
        let path = dir.path().join("db.toml");
        let mut cache = sqlite(Path::new(":memory:"));
        cache.agent = agent(DbAgentAccess::Rw, true, "the local cache");
        let plain = sqlite(Path::new("/elsewhere/plain.sqlite"));
        save(&files(&path), None, &[cache.clone(), plain.clone()]).expect("save");

        let shared = std::fs::read_to_string(&path).expect("shared");
        assert!(
            shared.contains("description = \"the local cache\""),
            "{shared}"
        );
        assert!(
            !shared.contains("access") && !shared.contains("default"),
            "{shared}"
        );
        let local = std::fs::read_to_string(&files(&path).agents).expect("local");
        assert!(
            local.contains("access = \"rw\"") && local.contains("default = true"),
            "{local}"
        );
        assert!(
            !local.contains("cache") && !local.contains(&plain.id.to_string()),
            "{local}"
        );

        assert_eq!(
            load(&files(&path), None).expect("load"),
            vec![cache.clone(), plain]
        );

        // Without the local half, the description stays and the access goes.
        std::fs::remove_file(files(&path).agents).expect("rm");
        let loaded = load(&files(&path), None).expect("load");
        assert_eq!(
            loaded[0].agent,
            agent(DbAgentAccess::None, false, "the local cache")
        );
    }

    #[test]
    fn the_agent_invariants_hold_on_load_and_on_save() {
        let dir = tempfile::tempdir().expect("dir");
        let path = dir.path().join("db.toml");
        let mut ro = postgres(); // read-only
        ro.agent = agent(DbAgentAccess::Rw, false, "");
        let mut hidden = sqlite(Path::new(":memory:"));
        hidden.agent = agent(DbAgentAccess::None, true, "");
        let mut first = sqlite(Path::new("/a.sqlite"));
        first.agent = agent(DbAgentAccess::Ro, true, "");
        let mut second = sqlite(Path::new("/b.sqlite"));
        second.agent = agent(DbAgentAccess::Rw, true, "");

        save(&files(&path), None, &[ro, hidden, first, second]).expect("save");
        let loaded = load(&files(&path), None).expect("load");
        let got: Vec<_> = loaded
            .iter()
            .map(|c| (c.agent.access, c.agent.default))
            .collect();
        assert_eq!(
            got,
            [
                (DbAgentAccess::Ro, false),
                (DbAgentAccess::None, false),
                (DbAgentAccess::Ro, true),
                (DbAgentAccess::Rw, false),
            ]
        );

        // A hand-edited local file naming two defaults: the first in the connection list wins,
        // whatever order the local file has them in.
        let ids: Vec<_> = loaded.iter().map(|c| c.id).collect();
        std::fs::write(
            files(&path).agents,
            format!(
                "version = 1\n[[agent]]\nid = \"{}\"\naccess = \"ro\"\ndefault = true\n\
                 [[agent]]\nid = \"{}\"\naccess = \"ro\"\ndefault = true\n",
                ids[3], ids[2]
            ),
        )
        .expect("write");
        let loaded = load(&files(&path), None).expect("load");
        assert!(loaded[2].agent.default && !loaded[3].agent.default);
    }

    #[test]
    fn a_list_round_trips_and_a_missing_file_is_empty() {
        let dir = tempfile::tempdir().expect("dir");
        let path = dir.path().join("db.toml");
        assert!(load(&files(&path), None).expect("missing").is_empty());

        let list = vec![postgres(), sqlite(Path::new(":memory:"))];
        save(&files(&path), None, &list).expect("save");
        assert_eq!(load(&files(&path), None).expect("load"), list);
        assert!(
            std::fs::read_to_string(&path)
                .expect("read")
                .starts_with("version = 1")
        );
    }

    #[test]
    fn no_password_is_ever_written() {
        let dir = tempfile::tempdir().expect("dir");
        let path = dir.path().join("db.toml");
        let mut connection = postgres();
        connection.config.password = Some("hunter2-needle".into());
        for key in ["password", "PWD", "sslpassword", "sslkey"] {
            connection
                .config
                .params
                .insert(key.into(), "needle-param".into());
        }

        save(&files(&path), None, &[connection]).expect("save");
        let raw = std::fs::read_to_string(&path).expect("read");
        assert!(!raw.contains("needle"), "a secret reached the file:\n{raw}");
        assert!(raw.contains("application_name"), "an ordinary param stays");

        // A hand-edited file carrying one is not honoured on the way in either.
        std::fs::write(
            &path,
            "version = 1\n[[connection]]\nid = \"01JZ0000000000000000000000\"\nname = \"x\"\n\
             kind = \"postgres\"\npassword = \"needle\"\n[connection.params]\npwd = \"needle\"\n",
        )
        .expect("write");
        let loaded = load(&files(&path), None).expect("load");
        assert_eq!(loaded[0].config.password, None);
        assert!(loaded[0].config.params.is_empty());
    }

    #[test]
    fn scrub_returns_the_password_a_pasted_string_carried() {
        let mut config = ConnectionConfig::new(DbKind::Postgres);
        config.password = Some("s3cret".into());
        assert_eq!(scrub(&mut config).as_deref(), Some("s3cret"));
        assert_eq!(config.password, None);
    }

    #[test]
    fn a_path_inside_the_project_is_stored_relative() {
        let dir = tempfile::tempdir().expect("dir");
        let root = dir.path().join("project");
        let path = dir.path().join("db.toml");
        let inside = sqlite(&root.join("data").join("cache.sqlite"));
        let outside = sqlite(Path::new("/elsewhere/other.sqlite"));

        save(
            &files(&path),
            Some(&root),
            &[inside.clone(), outside.clone()],
        )
        .expect("save");
        let raw = std::fs::read_to_string(&path).expect("read");
        assert!(raw.contains("path = \"data/cache.sqlite\""), "{raw}");
        assert!(raw.contains("/elsewhere/other.sqlite"));

        // A clone elsewhere resolves it against its own root.
        let moved = dir.path().join("clone");
        let loaded = load(&files(&path), Some(&moved)).expect("load");
        assert_eq!(
            loaded[0].config.path.as_deref(),
            Some(moved.join("data/cache.sqlite").as_path())
        );
        assert_eq!(loaded[1], outside);
    }

    #[test]
    fn a_newer_file_is_reported_and_left_alone() {
        let dir = tempfile::tempdir().expect("dir");
        let path = dir.path().join("db.toml");
        std::fs::write(&path, "version = 9\n").expect("write");
        assert!(matches!(
            load(&files(&path), None),
            Err(StoreError::UnknownVersion { found: 9, .. })
        ));
        assert!(matches!(
            save(&files(&path), None, &[postgres()]),
            Err(StoreError::UnknownVersion { .. })
        ));
        assert_eq!(
            std::fs::read_to_string(&path).expect("read"),
            "version = 9\n"
        );
    }

    #[test]
    fn garbage_is_kept_aside() {
        let dir = tempfile::tempdir().expect("dir");
        let path = dir.path().join("db.toml");
        std::fs::write(&path, "version = 1\n[[connection]]\nid = 7\n").expect("write");
        let Err(StoreError::Parse { preserved_as, .. }) = load(&files(&path), None) else {
            panic!("expected a parse error");
        };
        assert!(preserved_as.is_some_and(|p| p.exists()));
    }
}
