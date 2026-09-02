//! The stores as files under the config root.
//!
//! One TOML file for the catalogue, one per project for its tasks, one per scope for view
//! state, and one per settings layer. Nothing any of them does needs a query, an index or a
//! partial read, so a whole-file rewrite of a few tens of records is microseconds and a database
//! is a cost with no matching benefit. Where volume eventually arrives is the per-project cache,
//! which is a different store behind a different trait.

use std::path::{Path, PathBuf};
use std::sync::RwLock;
use std::sync::atomic::{AtomicBool, Ordering};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use ubiq_proto::ids::ProjectId;
use ubiq_proto::projects::{ProjectRecord, Scope};
use ubiq_proto::settings::{HOST_SETTINGS_SCHEMA, HostSettings, SettingsLayer};
use ubiq_proto::work::TaskRecord;

use super::{PreferenceStore, ProjectStore, SettingsStore, StoreError, TaskStore};
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

/// The `version` at the top of a file, before anything else about it is believed.
///
/// A version above ours is not corruption. The caller leaves the file exactly as it is: overwriting
/// it with a format that cannot hold what it holds would lose what the user wrote. `None` is a file
/// whose version cannot even be read, which is the parse path's business rather than this one's.
fn version_of(raw: &str) -> Option<u32> {
    #[derive(Deserialize)]
    struct JustTheVersion {
        #[serde(default)]
        version: u32,
    }
    toml::from_str::<JustTheVersion>(raw)
        .ok()
        .map(|probe| probe.version)
}

/// The host-settings file names its format `schema`, the same field the record on the wire carries.
fn schema_of(raw: &str) -> Option<u32> {
    #[derive(Deserialize)]
    struct JustTheSchema {
        #[serde(default)]
        schema: u32,
    }
    toml::from_str::<JustTheSchema>(raw)
        .ok()
        .map(|probe| probe.schema)
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

        if let Some(version) = version_of(&raw)
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

/// The task format this Ubiq writes and understands.
pub const TASKS_VERSION: u32 = 1;

/// The whole file, mirroring [`CatalogueFile`]: `version` at the top so a future migration has a
/// hook to read.
#[derive(Debug, Default, Serialize, Deserialize)]
struct TasksFile {
    version: u32,
    #[serde(default, rename = "task", skip_serializing_if = "Vec::is_empty")]
    tasks: Vec<TaskRecord>,
}

/// A project's tasks, one file per project under the config root.
///
/// Deliberately unlike [`FileProjectStore`]: no in-memory copy of the list and no `durable` flag.
/// The service above holds the authoritative list and hands the whole of it back on every save, so
/// a cache here would be a second copy of the same truth; and the told-once flag belongs where it
/// can be kept per project rather than for the store as a whole.
pub struct FileTaskStore {
    root: PathBuf,
}

impl FileTaskStore {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// One file per project, for `FilePreferenceStore::path`'s reason: a task edit must not rewrite
    /// the catalogue the user may be hand-editing. Under the project's own directory, so Forget and
    /// the orphan collector already cover it.
    pub fn path(&self, project: ProjectId) -> PathBuf {
        self.root
            .join("projects")
            .join(project.to_string())
            .join("tasks.toml")
    }
}

impl TaskStore for FileTaskStore {
    fn load(&self, project: ProjectId) -> Result<Option<Vec<TaskRecord>>, StoreError> {
        let path = self.path(project);
        let raw = match std::fs::read_to_string(&path) {
            Ok(raw) => raw,
            // The seeding hook. No file yet is a project whose tasks were never written, which is
            // not the same as a project with none: the caller may seed the first, never the second.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(source) => return Err(StoreError::Io { path, source }),
        };

        if let Some(version) = version_of(&raw)
            && version > TASKS_VERSION
        {
            return Err(StoreError::UnknownVersion {
                path,
                found: version,
                supported: TASKS_VERSION,
            });
        }

        match toml::from_str::<TasksFile>(&raw) {
            Ok(file) => Ok(Some(file.tasks)),
            Err(error) => {
                // Preserved, never truncated. The user's tasks are worth as much as the catalogue.
                let preserved_as = preserve_aside(&path, Utc::now()).ok();
                Err(StoreError::Parse {
                    path,
                    preserved_as,
                    message: error.message().to_string(),
                })
            }
        }
    }

    fn save(&self, project: ProjectId, tasks: &[TaskRecord]) -> Result<(), StoreError> {
        let path = self.path(project);
        let file = TasksFile {
            version: TASKS_VERSION,
            tasks: tasks.to_vec(),
        };
        let body = toml::to_string_pretty(&file).map_err(|error| StoreError::Parse {
            path: path.clone(),
            preserved_as: None,
            message: error.to_string(),
        })?;
        // The variant that creates the directories above it: a project that never had view state
        // has no directory of its own yet, and must still be able to write a task.
        write_atomic(&path, body.as_bytes()).map_err(|source| StoreError::Io { path, source })
    }

    fn clear(&self, project: ProjectId) -> Result<(), StoreError> {
        let path = self.path(project);
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(source) => Err(StoreError::Io { path, source }),
        }
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

/// The envelope a Ui-layer settings blob is stored in. `value` is opaque: the host writes it and
/// hands it back, and never looks inside. The Host layer does not use this — that file *is* the
/// record.
#[derive(Debug, Serialize, Deserialize)]
struct SettingsEnvelope {
    version: u32,
    updated_at: chrono::DateTime<Utc>,
    value: String,
}

/// The Ui-layer envelope format. Not the interface's schema, which lives inside `value`.
pub const SETTINGS_ENVELOPE_VERSION: u32 = 1;

/// Application settings, one file per layer under the config root.
pub struct FileSettingsStore {
    root: PathBuf,
}

impl FileSettingsStore {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn path(&self, layer: SettingsLayer) -> PathBuf {
        match layer {
            SettingsLayer::Ui => self.root.join("ui-settings.toml"),
            SettingsLayer::Host => self.root.join("host-settings.toml"),
        }
    }
}

impl SettingsStore for FileSettingsStore {
    fn get(&self, layer: SettingsLayer) -> Result<Option<String>, StoreError> {
        match layer {
            SettingsLayer::Ui => self.get_ui(),
            SettingsLayer::Host => self.get_host(),
        }
    }

    fn set(&self, layer: SettingsLayer, value: &str) -> Result<(), StoreError> {
        match layer {
            SettingsLayer::Ui => self.set_ui(value),
            SettingsLayer::Host => self.set_host(value),
        }
    }

    fn clear(&self, layer: SettingsLayer) -> Result<(), StoreError> {
        let path = self.path(layer);
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(source) => Err(StoreError::Io { path, source }),
        }
    }
}

impl FileSettingsStore {
    fn get_ui(&self) -> Result<Option<String>, StoreError> {
        let path = self.path(SettingsLayer::Ui);
        let raw = match std::fs::read_to_string(&path) {
            Ok(raw) => raw,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(source) => return Err(StoreError::Io { path, source }),
        };

        // Opaque, so unreadable is discarded rather than preserved: the host never read the
        // value, and a checkbox the window will reopen on its default is not a catalogue.
        match toml::from_str::<SettingsEnvelope>(&raw) {
            Ok(file) => Ok(Some(file.value)),
            Err(error) => {
                tracing::warn!(
                    "discarding unreadable ui settings at {}: {error}",
                    path.display()
                );
                Ok(None)
            }
        }
    }

    fn set_ui(&self, value: &str) -> Result<(), StoreError> {
        let path = self.path(SettingsLayer::Ui);
        let file = SettingsEnvelope {
            version: SETTINGS_ENVELOPE_VERSION,
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

    fn get_host(&self) -> Result<Option<String>, StoreError> {
        let path = self.path(SettingsLayer::Host);
        let raw = match std::fs::read_to_string(&path) {
            Ok(raw) => raw,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(source) => return Err(StoreError::Io { path, source }),
        };

        if let Some(schema) = schema_of(&raw)
            && schema > HOST_SETTINGS_SCHEMA
        {
            return Err(StoreError::UnknownVersion {
                path,
                found: schema,
                supported: HOST_SETTINGS_SCHEMA,
            });
        }

        match toml::from_str::<HostSettings>(&raw) {
            Ok(settings) => {
                serde_json::to_string(&settings)
                    .map(Some)
                    .map_err(|error| StoreError::Parse {
                        path,
                        preserved_as: None,
                        message: error.to_string(),
                    })
            }
            Err(error) => {
                let preserved_as = preserve_aside(&path, Utc::now()).ok();
                Err(StoreError::Parse {
                    path,
                    preserved_as,
                    message: error.message().to_string(),
                })
            }
        }
    }

    fn set_host(&self, value: &str) -> Result<(), StoreError> {
        let path = self.path(SettingsLayer::Host);
        let settings: HostSettings =
            serde_json::from_str(value).map_err(|error| StoreError::Parse {
                path: path.clone(),
                preserved_as: None,
                message: error.to_string(),
            })?;
        if settings.schema > HOST_SETTINGS_SCHEMA {
            return Err(StoreError::UnknownVersion {
                path,
                found: settings.schema,
                supported: HOST_SETTINGS_SCHEMA,
            });
        }
        let body = toml::to_string_pretty(&settings).map_err(|error| StoreError::Parse {
            path: path.clone(),
            preserved_as: None,
            message: error.to_string(),
        })?;
        write_atomic(&path, body.as_bytes()).map_err(|source| StoreError::Io { path, source })
    }
}
