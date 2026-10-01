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

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use ubiq_db::conn::ConnectionConfig;
use ubiq_proto::ids::DbConnId;

use crate::atomic::{preserve_aside, write_atomic};
use crate::store::StoreError;

/// The format this Ubiq writes and understands.
pub const DB_VERSION: u32 = 1;

/// Connection-string keys that carry secret material. Dropped, never written.
const SECRET_PARAMS: &[&str] = &["password", "pwd", "sslpassword", "sslkey"];

/// One saved connection: its stable id and its definition, `config.password` always `None`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredConnection {
    pub id: DbConnId,
    #[serde(flatten)]
    pub config: ConnectionConfig,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct DbFile {
    version: u32,
    #[serde(default, rename = "connection", skip_serializing_if = "Vec::is_empty")]
    connections: Vec<StoredConnection>,
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
/// `project_root` resolves a SQLite path that was saved relative to it.
pub fn load(path: &Path, project_root: Option<&Path>) -> Result<Vec<StoredConnection>, StoreError> {
    let raw = match std::fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => {
            return Err(StoreError::Io {
                path: path.to_path_buf(),
                source,
            });
        }
    };
    check_version(path, &raw)?;
    match toml::from_str::<DbFile>(&raw) {
        Ok(file) => Ok(file
            .connections
            .into_iter()
            .map(|mut connection| {
                // A hand-edited file may carry a password; it is not honoured.
                scrub(&mut connection.config);
                absolutise(&mut connection.config, project_root);
                connection
            })
            .collect()),
        Err(error) => Err(StoreError::Parse {
            path: path.to_path_buf(),
            preserved_as: preserve_aside(path, chrono::Utc::now()).ok(),
            message: error.to_string(),
        }),
    }
}

/// Write a project's whole connection list, atomically, scrubbed. Refuses to replace a file a
/// newer Ubiq wrote.
pub fn save(
    path: &Path,
    project_root: Option<&Path>,
    connections: &[StoredConnection],
) -> Result<(), StoreError> {
    if let Ok(raw) = std::fs::read_to_string(path) {
        check_version(path, &raw)?;
    }
    let file = DbFile {
        version: DB_VERSION,
        connections: connections
            .iter()
            .map(|connection| {
                let mut connection = connection.clone();
                scrub(&mut connection.config);
                relativise(&mut connection.config, project_root);
                connection
            })
            .collect(),
    };
    let body = toml::to_string_pretty(&file).map_err(|error| StoreError::Io {
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
        StoredConnection {
            id: DbConnId::generate(),
            config,
        }
    }

    fn sqlite(path: &Path) -> StoredConnection {
        let mut config = ConnectionConfig::new(DbKind::Sqlite);
        config.name = "local cache".into();
        config.path = Some(path.to_path_buf());
        StoredConnection {
            id: DbConnId::generate(),
            config,
        }
    }

    #[test]
    fn a_list_round_trips_and_a_missing_file_is_empty() {
        let dir = tempfile::tempdir().expect("dir");
        let path = dir.path().join("db.toml");
        assert!(load(&path, None).expect("missing").is_empty());

        let list = vec![postgres(), sqlite(Path::new(":memory:"))];
        save(&path, None, &list).expect("save");
        assert_eq!(load(&path, None).expect("load"), list);
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

        save(&path, None, &[connection]).expect("save");
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
        let loaded = load(&path, None).expect("load");
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

        save(&path, Some(&root), &[inside.clone(), outside.clone()]).expect("save");
        let raw = std::fs::read_to_string(&path).expect("read");
        assert!(raw.contains("path = \"data/cache.sqlite\""), "{raw}");
        assert!(raw.contains("/elsewhere/other.sqlite"));

        // A clone elsewhere resolves it against its own root.
        let moved = dir.path().join("clone");
        let loaded = load(&path, Some(&moved)).expect("load");
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
            load(&path, None),
            Err(StoreError::UnknownVersion { found: 9, .. })
        ));
        assert!(matches!(
            save(&path, None, &[postgres()]),
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
        let Err(StoreError::Parse { preserved_as, .. }) = load(&path, None) else {
            panic!("expected a parse error");
        };
        assert!(preserved_as.is_some_and(|p| p.exists()));
    }
}
