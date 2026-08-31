//! The stores as files under the config root.
//!
//! One TOML file for the catalogue, and one per scope for view state. Nothing the catalogue does
//! needs a query, an index or a partial read, so a whole-file rewrite of a few tens of records is
//! microseconds and a database is a cost with no matching benefit. Where volume eventually arrives
//! is the per-project cache, which is a different store behind a different trait.

use std::path::{Path, PathBuf};
use std::sync::RwLock;
use std::sync::atomic::{AtomicBool, Ordering};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use ubiq_proto::ids::ProjectId;
use ubiq_proto::projects::{ProjectRecord, Scope};

use super::{PreferenceStore, ProjectStore, StoreError};
use crate::atomic::{preserve_aside, write_atomic};

/// The catalogue format this Ubiq writes and understands.
pub const CATALOGUE_VERSION: u32 = 1;

/// The whole file. `version` is at the top so a future migration has a hook to read.
#[derive(Debug, Default, Serialize, Deserialize)]
struct CatalogueFile {
    version: u32,
    #[serde(default, rename = "project", skip_serializing_if = "Vec::is_empty")]
    projects: Vec<ProjectRecord>,
}

/// The catalogue, as one TOML file.
pub struct FileProjectStore {
    path: PathBuf,
    /// The live catalogue. Mutations land here first, so an unwritable store still answers.
    records: RwLock<Vec<ProjectRecord>>,
    /// Cleared by the first failed write. Everything after it answers [`StoreError::NotDurable`],
    /// which is how the user is told once rather than on every change.
    durable: AtomicBool,
}

impl FileProjectStore {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            records: RwLock::new(Vec::new()),
            durable: AtomicBool::new(true),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Rewrite the file from what is in memory.
    fn flush(&self) -> Result<(), StoreError> {
        let records = self.records.read().unwrap_or_else(|e| e.into_inner());
        let file = CatalogueFile {
            version: CATALOGUE_VERSION,
            projects: records.clone(),
        };
        drop(records);

        let body = toml::to_string_pretty(&file).map_err(|error| StoreError::Parse {
            path: self.path.clone(),
            preserved_as: None,
            message: error.to_string(),
        })?;

        match write_atomic(&self.path, body.as_bytes()) {
            Ok(()) => Ok(()),
            Err(source) => {
                // The session carries on from memory; it is simply no longer durable.
                self.durable.store(false, Ordering::Relaxed);
                Err(StoreError::Io {
                    path: self.path.clone(),
                    source,
                })
            }
        }
    }

    /// Apply a change in memory, then try to make it durable.
    fn mutate(&self, change: impl FnOnce(&mut Vec<ProjectRecord>)) -> Result<(), StoreError> {
        {
            let mut records = self.records.write().unwrap_or_else(|e| e.into_inner());
            change(&mut records);
            // Written in id order, which is creation order, so the file reads chronologically.
            records.sort_by_key(|record| record.id);
        }
        if !self.durable.load(Ordering::Relaxed) {
            return Err(StoreError::NotDurable);
        }
        self.flush()
    }
}

impl ProjectStore for FileProjectStore {
    fn load(&self) -> Result<Vec<ProjectRecord>, StoreError> {
        let raw = match std::fs::read_to_string(&self.path) {
            Ok(raw) => raw,
            // No catalogue yet is the ordinary first run, not a failure.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                *self.records.write().unwrap_or_else(|e| e.into_inner()) = Vec::new();
                return Ok(Vec::new());
            }
            Err(source) => {
                return Err(StoreError::Io {
                    path: self.path.clone(),
                    source,
                });
            }
        };

        // A version above ours is not corruption. The file is left exactly as it is: overwriting
        // it with a format that cannot hold what it holds would lose the user's catalogue.
        #[derive(Deserialize)]
        struct JustTheVersion {
            #[serde(default)]
            version: u32,
        }
        if let Ok(JustTheVersion { version }) = toml::from_str::<JustTheVersion>(&raw)
            && version > CATALOGUE_VERSION
        {
            self.durable.store(false, Ordering::Relaxed);
            return Err(StoreError::UnknownVersion {
                path: self.path.clone(),
                found: version,
                supported: CATALOGUE_VERSION,
            });
        }

        match toml::from_str::<CatalogueFile>(&raw) {
            Ok(file) => {
                *self.records.write().unwrap_or_else(|e| e.into_inner()) = file.projects.clone();
                Ok(file.projects)
            }
            Err(error) => {
                // Preserved, never truncated. The session starts empty and says so.
                let preserved_as = preserve_aside(&self.path, Utc::now()).ok();
                *self.records.write().unwrap_or_else(|e| e.into_inner()) = Vec::new();
                Err(StoreError::Parse {
                    path: self.path.clone(),
                    preserved_as,
                    message: error.message().to_string(),
                })
            }
        }
    }

    fn upsert(&self, record: &ProjectRecord) -> Result<(), StoreError> {
        self.mutate(
            |records| match records.iter_mut().find(|r| r.id == record.id) {
                Some(existing) => *existing = record.clone(),
                None => records.push(record.clone()),
            },
        )
    }

    fn remove(&self, id: ProjectId) -> Result<(), StoreError> {
        self.mutate(|records| records.retain(|record| record.id != id))
    }
}

/// The envelope a view blob is stored in. `value` is opaque: the host writes it and hands it back,
/// and never looks inside.
#[derive(Debug, Serialize, Deserialize)]
struct PreferenceFile {
    version: u32,
    updated_at: chrono::DateTime<Utc>,
    value: String,
}

/// The preference envelope format. Not the interface's schema, which lives inside `value`.
pub const PREFERENCE_VERSION: u32 = 1;

/// View state, one file per scope under the config root.
pub struct FilePreferenceStore {
    root: PathBuf,
}

impl FilePreferenceStore {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// One file per project rather than a section in the catalogue: otherwise every panel drag
    /// rewrites the list the user may be hand-editing.
    pub fn path(&self, scope: &Scope) -> PathBuf {
        match scope {
            Scope::Interface => self.root.join("preferences.toml"),
            Scope::Project(id) => self
                .root
                .join("projects")
                .join(id.to_string())
                .join("view.toml"),
        }
    }
}

impl PreferenceStore for FilePreferenceStore {
    fn get(&self, scope: &Scope) -> Result<Option<String>, StoreError> {
        let path = self.path(scope);
        let raw = match std::fs::read_to_string(&path) {
            Ok(raw) => raw,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(source) => return Err(StoreError::Io { path, source }),
        };

        // A blob the host cannot even find in its envelope is discarded, not preserved: the host
        // never read the value, so it cannot say anything useful about it, and the window opening
        // on defaults is the whole recovery.
        match toml::from_str::<PreferenceFile>(&raw) {
            Ok(file) => Ok(Some(file.value)),
            Err(error) => {
                tracing::warn!(
                    "discarding unreadable view state at {}: {error}",
                    path.display()
                );
                Ok(None)
            }
        }
    }

    fn set(&self, scope: &Scope, value: &str) -> Result<(), StoreError> {
        let path = self.path(scope);
        let file = PreferenceFile {
            version: PREFERENCE_VERSION,
            updated_at: Utc::now(),
            value: value.to_string(),
        };
        let body = toml::to_string_pretty(&file).map_err(|error| StoreError::Parse {
            path: path.clone(),
            preserved_as: None,
            message: error.to_string(),
        })?;
        write_atomic(&path, body.as_bytes()).map_err(|source| StoreError::Io { path, source })
    }

    fn clear(&self, scope: &Scope) -> Result<(), StoreError> {
        let path = self.path(scope);
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(source) => Err(StoreError::Io { path, source }),
        }
    }
}
