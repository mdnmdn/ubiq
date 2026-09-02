//! What Ubiq writes down, behind four traits.
//!
//! The catalogue, a project's tasks, the interface's view state and application settings have
//! different durability rules, and the differences are the point:
//!
//! - **The catalogue is the host's to understand.** It parses it, acts on it, and reports when it
//!   cannot be written. A corrupt one is preserved rather than truncated.
//! - **Tasks are the catalogue's class, not view state's.** The host parses them, acts on them,
//!   and reports when they cannot be written; a corrupt file is preserved. What differs is scale:
//!   they are per project, so they are per file.
//! - **View state is opaque.** The host stores a string it never reads, on the same discipline
//!   that keeps terminal bytes uninterpreted — the interface owns that schema, so the interface
//!   versions it. A failed write is a log line, not an error anybody has to read.
//! - **Settings split in two.** The Ui layer is opaque, like view state. The Host layer is the
//!   host's to parse, like the catalogue: a corrupt file is preserved and reported.

pub mod file;
pub mod memory;

use std::path::PathBuf;

use ubiq_proto::ids::ProjectId;
use ubiq_proto::projects::{ProjectRecord, Scope};
use ubiq_proto::settings::SettingsLayer;
use ubiq_proto::work::TaskRecord;

/// What can go wrong reaching a store.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// The file could not be parsed. It has already been moved aside, and `preserved_as` says
    /// where, so the reply to the user can name it.
    #[error("{path} could not be read{}: {message}", match preserved_as {
        Some(p) => format!(" and was kept as {}", p.display()),
        None => String::new(),
    })]
    Parse {
        path: PathBuf,
        preserved_as: Option<PathBuf>,
        message: String,
    },

    /// Written by a newer Ubiq than this one. Deliberately **not** treated as corruption: the file
    /// is left exactly as it is rather than being overwritten with a format that would lose data.
    #[error("{path} is version {found}, and this Ubiq understands {supported}")]
    UnknownVersion {
        path: PathBuf,
        found: u32,
        supported: u32,
    },

    /// A previous write already failed. Mutations still apply in memory for the rest of the
    /// session, so the user is told once and not on every keystroke afterwards.
    #[error("the catalogue is not durable: an earlier write failed")]
    NotDurable,
}

/// The project catalogue.
///
/// Three methods, because that is every mutation the catalogue makes. A file store rewrites the
/// whole file for each; a SQL store maps each to a statement. Neither shape leaks into the caller.
pub trait ProjectStore: Send + Sync {
    fn load(&self) -> Result<Vec<ProjectRecord>, StoreError>;
    fn upsert(&self, record: &ProjectRecord) -> Result<(), StoreError>;
    fn remove(&self, id: ProjectId) -> Result<(), StoreError>;
}

/// A project's tasks. The caller holds the list and hands the whole of it back, because the order
/// is the user's and a partial write cannot preserve it.
pub trait TaskStore: Send + Sync {
    /// `None` means never written, which is not the same as no tasks — the distinction the
    /// seeding rule turns on, and the one `PreferenceStore::get` already draws.
    fn load(&self, project: ProjectId) -> Result<Option<Vec<TaskRecord>>, StoreError>;
    fn save(&self, project: ProjectId, tasks: &[TaskRecord]) -> Result<(), StoreError>;
    fn clear(&self, project: ProjectId) -> Result<(), StoreError>;
}

/// The interface's view state, which the host holds and never reads.
pub trait PreferenceStore: Send + Sync {
    fn get(&self, scope: &Scope) -> Result<Option<String>, StoreError>;
    fn set(&self, scope: &Scope, value: &str) -> Result<(), StoreError>;
    fn clear(&self, scope: &Scope) -> Result<(), StoreError>;
}

/// Application settings, one file per layer under the config root.
///
/// The Ui layer's value is opaque — the host writes it and hands it back. The Host layer's value
/// is JSON on the wire and TOML on disk; the file store translates, and a blob it cannot read is
/// an error rather than a discarded default.
pub trait SettingsStore: Send + Sync {
    fn get(&self, layer: SettingsLayer) -> Result<Option<String>, StoreError>;
    fn set(&self, layer: SettingsLayer, value: &str) -> Result<(), StoreError>;
    fn clear(&self, layer: SettingsLayer) -> Result<(), StoreError>;
}
