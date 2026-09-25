//! A mission on disk: one directory per mission, under the project that owns it.
//!
//! ```text
//! <config root>/projects/<ProjectId>/missions/<TaskId>/
//!     mission.toml     the record
//!     docs/            the documents beyond the plan
//!     journal.jsonl    append-only, what happened
//! ```
//!
//! Under the project's own directory, exactly as [`super::plan::FilePlanStore`] is and for its
//! reason: Forget and the orphan collector already cover everything there, so a mission never
//! outlives the project it belongs to. The plan stays where it is — `plans/<TaskId>.md` — because
//! `ubiq-plan` addresses it there and a mission is not a reason to move it.
//!
//! A directory rather than a file, because two of the three things a mission keeps are not the
//! record: the documents are markdown with sidecars of their own, and the journal is append-only
//! and grows. This stage writes only `mission.toml`; the other two are made by the code that first
//! needs them, and the layout is settled here so it never has to move.
//!
//! **TOML, and unknown keys survive.** `mission.toml` is a small record a user may reasonably open
//! in an editor, so it follows [`super::file`]'s format rather than the plan sidecar's JSON. A save
//! merges what it writes *into* the table already on disk rather than replacing it, so a key this
//! build does not know — one a newer Ubiq wrote, or one the user added — is still there afterwards.
//! That is the same instinct [`crate::atomic::preserve_aside`] serves on the parse path, at the
//! other end: this build never silently drops what it could not read.
//!
//! **A store with exactly one implementation is a concrete type instead** — `store/harness.rs`'s
//! own line, which is why this is a plain struct with no trait and no memory twin.

use std::path::PathBuf;

use chrono::Utc;
use ubiq_proto::ids::{ProjectId, TaskId};
use ubiq_proto::mission::{JournalEntry, MissionRecord};

use super::StoreError;
use crate::atomic::{preserve_aside, write_atomic};

/// The mission format this Ubiq writes and understands.
pub const MISSION_VERSION: u32 = 1;

/// One project's missions, one directory each.
pub struct MissionStore {
    root: PathBuf,
}

impl MissionStore {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// The mission's own directory — the record, the documents and the journal together.
    pub fn dir(&self, project: ProjectId, task: TaskId) -> PathBuf {
        self.root
            .join("projects")
            .join(project.to_string())
            .join("missions")
            .join(task.to_string())
    }

    pub fn path(&self, project: ProjectId, task: TaskId) -> PathBuf {
        self.dir(project, task).join("mission.toml")
    }

    /// Where the mission's documents beyond the plan live (M8). Nothing creates it yet.
    pub fn docs_dir(&self, project: ProjectId, task: TaskId) -> PathBuf {
        self.dir(project, task).join("docs")
    }

    /// The append-only journal (M12).
    pub fn journal_path(&self, project: ProjectId, task: TaskId) -> PathBuf {
        self.dir(project, task).join("journal.jsonl")
    }

    /// The documents in `docs/`, by bare name without the `.md`, sorted.
    ///
    /// A directory that is not there is an empty list rather than an error — a mission nobody has
    /// written a document for is the ordinary case, and so is a name that is not markdown, which
    /// is skipped rather than reported: `docs/` is Ubiq's directory but the user's to open.
    pub fn documents(&self, project: ProjectId, task: TaskId) -> Vec<String> {
        let Ok(entries) = std::fs::read_dir(self.docs_dir(project, task)) else {
            return Vec::new();
        };
        let mut names: Vec<String> = entries
            .flatten()
            .filter_map(|entry| {
                let name = entry.file_name().to_str()?.to_string();
                name.strip_suffix(".md").map(str::to_string)
            })
            .collect();
        names.sort();
        names
    }

    /// Append one line, stamped with the sequence it lands at — the number of lines already
    /// there, which is the line's own index and the cursor [`read_journal`] pages back through.
    ///
    /// **Not atomic-written.** [`crate::atomic::write_atomic`] replaces a file, and this one is
    /// append-only by design: a journal rewritten on every line would lose everything a crash
    /// caught mid-write, which is the opposite of what an append-only log is for. One `O_APPEND`
    /// write of one line is what a journal is.
    ///
    /// [`read_journal`]: Self::read_journal
    pub fn append_journal(
        &self,
        project: ProjectId,
        task: TaskId,
        mut entry: JournalEntry,
    ) -> Result<JournalEntry, StoreError> {
        use std::io::Write;

        let path = self.journal_path(project, task);
        entry.seq = self.journal_len(project, task);
        let mut line = serde_json::to_string(&entry).map_err(|error| StoreError::Parse {
            path: path.clone(),
            preserved_as: None,
            message: error.to_string(),
        })?;
        line.push('\n');

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| StoreError::Io {
                path: path.clone(),
                source,
            })?;
        }
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|source| StoreError::Io {
                path: path.clone(),
                source,
            })?;
        file.write_all(line.as_bytes())
            .map_err(|source| StoreError::Io { path, source })?;
        Ok(entry)
    }

    /// How many lines the journal holds — the next sequence, and the count the panel shows.
    ///
    /// A line that will not parse still counts: the sequence is a position in the file, so
    /// skipping one would make two lines claim the same number forever after.
    pub fn journal_len(&self, project: ProjectId, task: TaskId) -> u64 {
        std::fs::read_to_string(self.journal_path(project, task))
            .map(|raw| raw.lines().filter(|line| !line.trim().is_empty()).count() as u64)
            .unwrap_or(0)
    }

    /// One page of the journal, **newest first**, plus whether anything older than it exists.
    ///
    /// `before` is a sequence exclusive upper bound — the oldest `seq` of the page before this
    /// one — and `None` asks for the newest page. A line that will not parse is dropped with a
    /// warning rather than failing the page: one bad line must not take the whole history with it.
    pub fn read_journal(
        &self,
        project: ProjectId,
        task: TaskId,
        before: Option<u64>,
        limit: usize,
    ) -> (Vec<JournalEntry>, bool) {
        let path = self.journal_path(project, task);
        let Ok(raw) = std::fs::read_to_string(&path) else {
            return (Vec::new(), false);
        };
        let mut kept: Vec<JournalEntry> = raw
            .lines()
            .filter(|line| !line.trim().is_empty())
            .filter_map(|line| match serde_json::from_str::<JournalEntry>(line) {
                Ok(entry) => Some(entry),
                Err(error) => {
                    tracing::warn!(?path, "a journal line would not read: {error}");
                    None
                }
            })
            .filter(|entry| before.is_none_or(|before| entry.seq < before))
            .collect();
        // Newest first is the order every reader wants: the panel shows the last three and the
        // Activity tab scrolls back from now.
        kept.reverse();
        let more = kept.len() > limit;
        kept.truncate(limit);
        (kept, more)
    }

    /// Whether anything has been journaled — half of the question a demotion is refused on, the
    /// other half being the roster on the record itself. A file that exists but is empty counts as
    /// nothing having happened.
    pub fn has_journal(&self, project: ProjectId, task: TaskId) -> bool {
        std::fs::metadata(self.journal_path(project, task))
            .map(|meta| meta.len() > 0)
            .unwrap_or(false)
    }

    /// A mission's record, or `None` for a task that has never had one — which is every mission
    /// until the first time one is opened, listed or touched (M3), and is not an error.
    pub fn load(
        &self,
        project: ProjectId,
        task: TaskId,
    ) -> Result<Option<MissionRecord>, StoreError> {
        let path = self.path(project, task);
        let raw = match std::fs::read_to_string(&path) {
            Ok(raw) => raw,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(source) => return Err(StoreError::Io { path, source }),
        };

        let table = match raw.parse::<toml::Table>() {
            Ok(table) => table,
            Err(error) => {
                // Preserved, never truncated: a record that will not parse is still the only copy
                // of what the user and their agents did.
                let preserved_as = preserve_aside(&path, Utc::now()).ok();
                return Err(StoreError::Parse {
                    path,
                    preserved_as,
                    message: error.message().to_string(),
                });
            }
        };

        // A version above ours is not corruption, and the file is left exactly as it is —
        // `store/file.rs`'s rule, for its reason.
        let version = table
            .get("version")
            .and_then(|value| value.as_integer())
            .unwrap_or(0) as u32;
        if version > MISSION_VERSION {
            return Err(StoreError::UnknownVersion {
                path,
                found: version,
                supported: MISSION_VERSION,
            });
        }

        match toml::Value::Table(table).try_into::<MissionRecord>() {
            Ok(record) => Ok(Some(record)),
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

    /// Write a mission's record, merging it into whatever is already on disk so that a key this
    /// build does not know survives the save — see the module doc.
    pub fn save(&self, record: &MissionRecord) -> Result<(), StoreError> {
        let path = self.path(record.project_id, record.task_id);

        let mut table = match toml::Value::try_from(record) {
            Ok(toml::Value::Table(table)) => table,
            Ok(_) => unreachable!("a record serialises as a table"),
            Err(error) => {
                return Err(StoreError::Parse {
                    path,
                    preserved_as: None,
                    message: error.to_string(),
                });
            }
        };
        table.insert(
            "version".to_string(),
            toml::Value::Integer(i64::from(MISSION_VERSION)),
        );
        if let Ok(raw) = std::fs::read_to_string(&path)
            && let Ok(existing) = raw.parse::<toml::Table>()
        {
            // **Only keys that are not ours.** A cleared `Option` writes no key at all in TOML, so
            // carrying over every key the file already had would resurrect the coordinator or the
            // pending request the caller just cleared — the save would silently refuse to clear
            // anything. `our_keys` is what a record with all of its optional fields present emits.
            let ours = our_keys();
            for (key, value) in existing {
                if !ours.contains(key.as_str()) {
                    table.entry(key).or_insert(value);
                }
            }
        }

        let body = toml::to_string_pretty(&table).map_err(|error| StoreError::Parse {
            path: path.clone(),
            preserved_as: None,
            message: error.to_string(),
        })?;
        // The variant that creates the directories above it: a mission's directory is made by its
        // first save.
        write_atomic(&path, body.as_bytes()).map_err(|source| StoreError::Io { path, source })
    }

    /// Drop a mission, whole — the record, the documents and the journal with it. Not an error
    /// when there was none: the caller asked for it to be gone, and it already was.
    pub fn delete(&self, project: ProjectId, task: TaskId) -> Result<(), StoreError> {
        let path = self.dir(project, task);
        match std::fs::remove_dir_all(&path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(source) => Err(StoreError::Io { path, source }),
        }
    }

    /// Every mission this project has a record for, in no particular order — the caller orders
    /// them, because what "most recently active" means is not this store's to decide.
    ///
    /// A directory that will not read at all is an empty list rather than an error: a project with
    /// no missions has no such directory, which is the ordinary case. A single record that will
    /// not parse **is** reported, because that is one the user would miss.
    pub fn list(&self, project: ProjectId) -> Result<Vec<MissionRecord>, StoreError> {
        let dir = self
            .root
            .join("projects")
            .join(project.to_string())
            .join("missions");
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(source) => return Err(StoreError::Io { path: dir, source }),
        };

        let mut records = Vec::new();
        for entry in entries.flatten() {
            let Some(task) = entry
                .file_name()
                .to_str()
                .and_then(|name| name.parse::<TaskId>().ok())
            else {
                continue;
            };
            if let Some(record) = self.load(project, task)? {
                records.push(record);
            }
        }
        Ok(records)
    }
}

/// Every key a [`MissionRecord`] writes, whatever its optional fields hold.
///
/// Read off a probe record with all three of them present rather than listed by hand, so a field
/// added to the record is covered without anybody remembering to come here — the cost being that
/// the probe has to keep setting every `Option` the record has.
fn our_keys() -> std::collections::BTreeSet<String> {
    let mut probe = MissionRecord::new(
        ProjectId::generate(),
        TaskId::generate(),
        ubiq_proto::mission::Phase::default(),
        Utc::now(),
    );
    probe.coordinator = Some(ubiq_proto::work::AgentId::generate());
    probe.pending_phase = Some(ubiq_proto::mission::PendingPhase {
        phase: ubiq_proto::mission::Phase::default(),
        by: ubiq_proto::mission::Actor::Host,
        summary: String::new(),
        at: Utc::now(),
    });
    probe.default_kind = Some(String::new());
    match toml::Value::try_from(&probe) {
        Ok(toml::Value::Table(table)) => table.keys().cloned().collect(),
        _ => std::collections::BTreeSet::new(),
    }
}
