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

use super::project_dir::ProjectDirs;
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

/// Every TOML key a `[[project]]` row can hold today — `ProjectRecord`'s own field names, after
/// `#[serde(rename)]`. [`merge_unknown_fields`]'s `known_fields` for the catalogue: kept in step
/// with `ProjectRecord` by hand, the same obligation `mission.rs`'s own probe carries.
const PROJECT_RECORD_FIELDS: &[&str] = &[
    "id",
    "name",
    "path",
    "colour",
    "custom_colour",
    "temporary",
    "created_at",
    "last_opened_at",
    "search_excludes",
    "index",
    "mission_term",
    "managed_repos",
    "tools",
    "lanes",
    "runs_on",
    "initials",
    "storage",
];

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

/// `D179`: a parsed store round-trips a field it does not itself know, rather than dropping it —
/// the mechanism [`crate::store::mission`] already uses for one record per file, generalised here
/// to a list of them, keyed by `id`.
///
/// `known_fields` is every TOML key this build's record type can itself emit (its field names,
/// after `#[serde(rename)]`). A key in the old row that is **not** one of them is somebody else's —
/// a newer Ubiq's, or a Studio sidecar's — and is copied onto the matching fresh row untouched. A
/// key that **is** one of ours is never copied forward even if the fresh row omits it, because an
/// omitted `Option` field is a value this save cleared, not one it forgot; copying it forward would
/// resurrect it (`mission.rs`'s own reasoning).
fn merge_unknown_fields(
    mut fresh: Vec<toml::Value>,
    existing_raw: &str,
    array_key: &str,
    known_fields: &[&str],
) -> Vec<toml::Value> {
    let Some(toml::Value::Array(existing_rows)) = existing_raw
        .parse::<toml::Table>()
        .ok()
        .and_then(|table| table.get(array_key).cloned())
    else {
        return fresh;
    };

    for row in &mut fresh {
        let toml::Value::Table(table) = row else {
            continue;
        };
        let Some(id) = table.get("id").cloned() else {
            continue;
        };
        let old = existing_rows.iter().find_map(|value| match value {
            toml::Value::Table(old) if old.get("id") == Some(&id) => Some(old),
            _ => None,
        });
        let Some(old) = old else { continue };
        for (key, value) in old {
            if known_fields.contains(&key.as_str()) {
                continue;
            }
            table.entry(key.clone()).or_insert_with(|| value.clone());
        }
    }
    fresh
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
        let records = self
            .records
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone();

        let mut rows = Vec::with_capacity(records.len());
        for record in &records {
            let value = toml::Value::try_from(record).map_err(|error| StoreError::Parse {
                path: self.path.clone(),
                preserved_as: None,
                message: error.to_string(),
            })?;
            rows.push(value);
        }
        // Every project's own keys survive the read/modify/write cycle; anything else on its row —
        // a Studio key, or a field a newer Ubiq wrote — comes along for the ride (`D179`).
        if let Ok(existing_raw) = std::fs::read_to_string(&self.path) {
            rows = merge_unknown_fields(rows, &existing_raw, "project", PROJECT_RECORD_FIELDS);
        }

        let mut table = toml::Table::new();
        table.insert(
            "version".to_string(),
            toml::Value::Integer(i64::from(CATALOGUE_VERSION)),
        );
        if !rows.is_empty() {
            table.insert("project".to_string(), toml::Value::Array(rows));
        }

        let body = toml::to_string_pretty(&table).map_err(|error| StoreError::Parse {
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

/// Every TOML key a `[[task]]` row can hold today — `TaskRecord`'s own field names, after
/// `#[serde(rename)]`. [`merge_unknown_fields`]'s `known_fields` for tasks; see
/// [`PROJECT_RECORD_FIELDS`] for the same obligation on the catalogue side.
const TASK_RECORD_FIELDS: &[&str] = &[
    "id",
    "session",
    "status",
    "priority",
    "shape",
    "kind",
    "level",
    "parent",
    "reference",
    "prerequisite",
    "attachment",
    "complexity",
    "assigned_to",
    "key",
    "link",
    "label",
    "colour",
    "title",
    "description",
    "step",
    "comment",
    "created_at",
    "updated_at",
];

/// A project's tasks, one file per project under the config root.
///
/// Deliberately unlike [`FileProjectStore`]: no in-memory copy of the list and no `durable` flag.
/// The service above holds the authoritative list and hands the whole of it back on every save, so
/// a cache here would be a second copy of the same truth; and the told-once flag belongs where it
/// can be kept per project rather than for the store as a whole.
pub struct FileTaskStore {
    /// Where each project's data directory is. Tasks are the user's own data and are shared with
    /// whoever clones a project-managed one, so this is the store that has to ask rather than
    /// composing a path under the config root — see [`crate::store::project_dir`].
    dirs: ProjectDirs,
}

impl FileTaskStore {
    pub fn new(root: PathBuf) -> Self {
        Self {
            dirs: ProjectDirs::new(root),
        }
    }

    /// One file per project, for `FilePreferenceStore::path`'s reason: a task edit must not rewrite
    /// the catalogue the user may be hand-editing. Under the project's data directory, so Forget
    /// and the orphan collector already cover a Ubiq-managed project's copy and a project-managed
    /// one lands where the project itself can carry it.
    pub fn path(&self, project: ProjectId) -> PathBuf {
        self.dirs.data(project).tasks()
    }

    /// Where a project's archived tasks live — beside the live file under the same `tasks/`, so
    /// Forget and the orphan collector cover this the same way they already cover that file.
    fn archive_dir(&self, project: ProjectId) -> PathBuf {
        self.dirs.data(project).tasks_archive()
    }

    /// Every archive page already on disk, oldest first, named by the page number that decides the
    /// order — `0001.toml`, `0002.toml`, … — rather than by read order, which a directory listing
    /// does not promise.
    fn archive_pages(&self, project: ProjectId) -> Vec<(u32, PathBuf)> {
        let mut pages: Vec<(u32, PathBuf)> = std::fs::read_dir(self.archive_dir(project))
            .into_iter()
            .flatten()
            .filter_map(|entry| entry.ok())
            .filter_map(|entry| {
                let path = entry.path();
                let number: u32 = path.file_stem()?.to_str()?.parse().ok()?;
                Some((number, path))
            })
            .collect();
        pages.sort_by_key(|(number, _)| *number);
        pages
    }
}

/// One archive page holds at most this many tasks (`T-190`), so a project archiving for years
/// never asks a reader to open a file that grows without bound.
pub const ARCHIVE_PAGE_SIZE: usize = 100;

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

        let mut rows = Vec::with_capacity(tasks.len());
        for task in tasks {
            let value = toml::Value::try_from(task).map_err(|error| StoreError::Parse {
                path: path.clone(),
                preserved_as: None,
                message: error.to_string(),
            })?;
            rows.push(value);
        }
        // Every task's own keys survive the read/modify/write cycle; anything else on its row —
        // a Studio key, or a field a newer Ubiq wrote — comes along for the ride (`D179`).
        if let Ok(existing_raw) = std::fs::read_to_string(&path) {
            rows = merge_unknown_fields(rows, &existing_raw, "task", TASK_RECORD_FIELDS);
        }

        let mut table = toml::Table::new();
        table.insert(
            "version".to_string(),
            toml::Value::Integer(i64::from(TASKS_VERSION)),
        );
        if !rows.is_empty() {
            table.insert("task".to_string(), toml::Value::Array(rows));
        }

        let body = toml::to_string_pretty(&table).map_err(|error| StoreError::Parse {
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

    fn archive(&self, project: ProjectId, tasks: &[TaskRecord]) -> Result<(), StoreError> {
        if tasks.is_empty() {
            return Ok(());
        }
        let pages = self.archive_pages(project);
        let mut remaining = tasks;
        let mut next_number = pages.last().map_or(1, |(number, _)| number + 1);

        // Top up the last page before opening a new one, so a page short of `ARCHIVE_PAGE_SIZE`
        // is filled rather than left short forever.
        if let Some((_, path)) = pages.last() {
            let raw = std::fs::read_to_string(path).map_err(|source| StoreError::Io {
                path: path.clone(),
                source,
            })?;
            let mut file: TasksFile = toml::from_str(&raw).map_err(|error| StoreError::Parse {
                path: path.clone(),
                preserved_as: None,
                message: error.message().to_string(),
            })?;
            let room = ARCHIVE_PAGE_SIZE.saturating_sub(file.tasks.len());
            if room > 0 {
                let take = room.min(remaining.len());
                file.tasks.extend_from_slice(&remaining[..take]);
                remaining = &remaining[take..];
                write_tasks_file(path, &file)?;
            }
        }

        for chunk in remaining.chunks(ARCHIVE_PAGE_SIZE) {
            let path = self
                .archive_dir(project)
                .join(format!("{next_number:04}.toml"));
            write_tasks_file(
                &path,
                &TasksFile {
                    version: TASKS_VERSION,
                    tasks: chunk.to_vec(),
                },
            )?;
            next_number += 1;
        }
        Ok(())
    }
}

/// Write one archive page whole — every page is small enough that a partial rewrite buys nothing
/// [`FileTaskStore::save`] doesn't already get from doing the same for the live file.
fn write_tasks_file(path: &Path, file: &TasksFile) -> Result<(), StoreError> {
    let body = toml::to_string_pretty(file).map_err(|error| StoreError::Parse {
        path: path.to_path_buf(),
        preserved_as: None,
        message: error.to_string(),
    })?;
    write_atomic(path, body.as_bytes()).map_err(|source| StoreError::Io {
        path: path.to_path_buf(),
        source,
    })
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
            Scope::Project(id) => {
                super::project_dir::ProjectData::under_config(&self.root, *id).view()
            }
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
