//! The catalogue as the host runs it: what each message in the project family does.
//!
//! Nothing here draws, and nothing here decides how a project looks — the colour arrives from the
//! interface, because the palette is the interface's. What this owns is the record, the folder it
//! points at, and whether either can be trusted.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use chrono::Utc;
use ubiq_proto::ids::ProjectId;
use ubiq_proto::projects::{ProjectRecord, ProjectSnapshot, Scope};

use crate::gc;
use crate::health::probe;
use crate::reply::Reply;
use crate::store::{PreferenceStore, ProjectStore, StoreError};

/// The directory inside a project's own that belongs to the interface.
///
/// The host reserves the name and creates it, and never reads or writes inside it — see
/// [`ubiq_proto::projects::ProjectSnapshot::workarea`].
pub const WORKAREA: &str = "ui";

/// How long a preference sits before it is written.
///
/// A panel drag fires continuously, so the writes are coalesced per scope. Long enough that a drag
/// is one write, short enough that quitting straight after a change keeps it.
pub const DEBOUNCE: Duration = Duration::from_millis(400);

/// The catalogue, the view state, and what is running in each project.
pub struct Projects {
    root: PathBuf,
    catalogue: Box<dyn ProjectStore>,
    preferences: Box<dyn PreferenceStore>,
    /// The live catalogue, in the order ids sort, which is the order projects were added.
    records: Vec<ProjectRecord>,
    /// How many panes are running in each project. Only this half can know it.
    open_panes: HashMap<ProjectId, usize>,
    /// Preferences waiting to be written, and when the oldest became due.
    pending: HashMap<Scope, String>,
    due: Option<Instant>,
    /// Whether the user has already been told the catalogue is not durable.
    warned: bool,
}

impl Projects {
    /// Open the catalogue. Answers itself and whatever should be said about how that went.
    pub fn open(
        root: PathBuf,
        catalogue: Box<dyn ProjectStore>,
        preferences: Box<dyn PreferenceStore>,
    ) -> (Self, Vec<Reply>) {
        let mut this = Self {
            root,
            catalogue,
            preferences,
            records: Vec::new(),
            open_panes: HashMap::new(),
            pending: HashMap::new(),
            due: None,
            warned: false,
        };

        let mut replies = Vec::new();
        match this.catalogue.load() {
            Ok(records) => {
                this.records = records;
                // Only ever after a load that worked. Collecting against the empty catalogue a
                // *corrupt* file produces would delete every project's view state.
                let keep: HashSet<ProjectId> = this.records.iter().map(|r| r.id).collect();
                gc::collect(&this.root, &keep);
            }
            Err(error) => {
                this.warned = true;
                replies.push(Reply::Everyone(error_for(None, &error)));
            }
        }
        (this, replies)
    }

    pub fn records(&self) -> &[ProjectRecord] {
        &self.records
    }

    /// Every project, probed.
    pub fn list(&self) -> Vec<ProjectSnapshot> {
        self.records.iter().map(|r| self.snapshot(r)).collect()
    }

    fn snapshot(&self, record: &ProjectRecord) -> ProjectSnapshot {
        ProjectSnapshot {
            health: probe(Path::new(&record.path)),
            open_panes: self.open_panes.get(&record.id).copied().unwrap_or(0),
            workarea: self.reserve_workarea(record.id),
            record: record.clone(),
        }
    }

    /// Where this project's interface keeps its own files.
    ///
    /// One directory per project, beside the `tasks.toml` and `view.toml` that project already
    /// owns, so Forget and the orphan collector cover it without knowing it is there.
    pub fn workarea(&self, id: ProjectId) -> PathBuf {
        self.root
            .join("projects")
            .join(id.to_string())
            .join(WORKAREA)
    }

    /// Reserve it, and answer the path the interface is told.
    ///
    /// Made here rather than by whoever writes the first file, because the interface is told the
    /// path and is not told whether it exists — and this is the last moment the host has any
    /// business with the directory at all. **Nothing after this reads inside it.**
    ///
    /// A directory that will not be made is still named: what is kept there is disposable by
    /// design, so an interface that cannot cache is an interface that redraws, not one that fails.
    fn reserve_workarea(&self, id: ProjectId) -> String {
        let path = self.workarea(id);
        if let Err(error) = std::fs::create_dir_all(&path) {
            tracing::warn!(
                "could not reserve the interface's workarea at {}: {error}",
                path.display()
            );
        }
        path.to_string_lossy().into_owned()
    }

    fn find(&self, id: ProjectId) -> Option<&ProjectRecord> {
        self.records.iter().find(|r| r.id == id)
    }

    /// One record, for the coordinator.
    ///
    /// Starting a harness and reading a file both need a project's folder and nothing else the
    /// catalogue holds, so this is the whole surface either of them takes — a lookup in memory,
    /// with no syscall on the run loop.
    pub fn record(&self, id: ProjectId) -> Option<&ProjectRecord> {
        self.find(id)
    }

    /// Write a record down, and say so only the first time durability is lost.
    fn keep(&mut self, record: ProjectRecord) -> Option<Reply> {
        let temporary = record.temporary;
        match self.records.iter_mut().find(|r| r.id == record.id) {
            Some(existing) => *existing = record.clone(),
            None => {
                self.records.push(record.clone());
                self.records.sort_by_key(|r| r.id);
            }
        }
        // A temporary project lives in `records` and nowhere else: every file, git and work job
        // resolves through `record()`, which is memory, so skipping the write costs it nothing
        // and is the whole of its impermanence.
        if temporary {
            return None;
        }
        match self.catalogue.upsert(&record) {
            Ok(()) => None,
            Err(error) => self.warn_once(&error),
        }
    }

    fn warn_once(&mut self, error: &StoreError) -> Option<Reply> {
        if self.warned {
            tracing::debug!("the catalogue is still not durable: {error}");
            return None;
        }
        self.warned = true;
        Some(Reply::Everyone(error_for(None, error)))
    }

    // ── the message family ──────────────────────────────────────────

    pub fn list_projects(&self) -> Reply {
        Reply::Asker(ubiq_proto::messages::Message::ProjectList {
            projects: self.list(),
        })
    }

    /// Take a folder into the catalogue.
    ///
    /// Adding is not creating: a path that is not there is refused rather than made. A folder
    /// already in the catalogue resolves to the project that is there, so the picker points at it
    /// and no duplicate appears.
    pub fn add(
        &mut self,
        path: &str,
        name: Option<String>,
        colour: Option<usize>,
        custom_colour: Option<u32>,
        temporary: bool,
    ) -> Vec<Reply> {
        let canonical = match std::fs::canonicalize(path) {
            Ok(canonical) => canonical,
            Err(error) => {
                return vec![Reply::Asker(message_error(
                    None,
                    format!("{path}: {error}"),
                ))];
            }
        };

        if !canonical.is_dir() {
            return vec![Reply::Asker(message_error(
                None,
                format!("{} is not a folder", canonical.display()),
            ))];
        }

        let as_text = canonical.to_string_lossy().into_owned();
        if let Some(existing) = self.records.iter().find(|r| r.path == as_text) {
            // Without this, a folder dropped and then also added through the picker looks
            // persisted to the caller while still carrying the flag, and would be silently
            // forgotten when closed. Anything else about the existing record — dropped onto
            // twice, or added for real twice — is answered exactly as before.
            if existing.temporary && !temporary {
                let id = existing.id;
                return self.promote(id, name, colour, custom_colour);
            }
            // The path is a uniqueness key, not an identity: this is the project that is there.
            return vec![Reply::Asker(ubiq_proto::messages::Message::ProjectAdded {
                project: self.snapshot(existing),
            })];
        }

        let record = ProjectRecord {
            id: ProjectId::generate(),
            // A dropped folder is named by its folder and coloured gray by the interface, so an
            // incoming name or colour is ignored.
            name: if temporary {
                leaf(&canonical)
            } else {
                name.unwrap_or_else(|| leaf(&canonical))
            },
            path: as_text,
            colour: if temporary { 0 } else { colour.unwrap_or(0) },
            custom_colour: if temporary { None } else { custom_colour },
            temporary,
            created_at: Utc::now(),
            last_opened_at: None,
        };

        let snapshot = self.snapshot(&record);
        let mut replies = vec![Reply::Everyone(
            ubiq_proto::messages::Message::ProjectAdded { project: snapshot },
        )];
        replies.extend(self.keep(record));
        replies
    }

    /// Turn a temporary project into a durable one: the single path that clears the flag.
    ///
    /// Naming a temporary project in the settings dialog is what keeps it, and both the settings
    /// path (`update`) and re-adding a dropped folder through the picker (`add`) end up here.
    fn promote(
        &mut self,
        id: ProjectId,
        name: Option<String>,
        colour: Option<usize>,
        custom_colour: Option<u32>,
    ) -> Vec<Reply> {
        let Some(record) = self.find(id) else {
            return vec![Reply::Asker(message_error(Some(id), "no such project"))];
        };
        let mut record = record.clone();
        record.temporary = false;
        if let Some(name) = name.filter(|n| !n.trim().is_empty()) {
            record.name = name.trim().to_string();
        }
        if let Some(colour) = colour {
            record.colour = colour;
            record.custom_colour = custom_colour;
        }

        let snapshot = self.snapshot(&record);
        let mut replies = vec![Reply::Everyone(
            ubiq_proto::messages::Message::ProjectChanged { project: snapshot },
        )];
        replies.extend(self.keep(record));
        replies
    }

    /// Drop the record, then the project's own directory in Ubiq's config.
    ///
    /// The order matters: the catalogue is authoritative, so it goes first, and a directory left
    /// behind by a crash between the two is collected at the next load.
    pub fn forget(&mut self, id: ProjectId) -> Vec<Reply> {
        if self.find(id).is_none() {
            return vec![Reply::Asker(message_error(Some(id), "no such project"))];
        }
        self.records.retain(|r| r.id != id);
        self.open_panes.remove(&id);
        let _ = self.preferences.clear(&Scope::Project(id));

        let mut replies = Vec::new();
        if let Err(error) = self.catalogue.remove(id)
            && let Some(reply) = self.warn_once(&error)
        {
            replies.push(reply);
        }

        let dir = self.root.join("projects").join(id.to_string());
        if dir.exists()
            && let Err(error) = std::fs::remove_dir_all(&dir)
        {
            tracing::warn!("could not remove {}: {error}", dir.display());
        }

        replies.push(Reply::Everyone(
            ubiq_proto::messages::Message::ProjectForgotten { project_id: id },
        ));
        replies
    }

    /// Rename or recolour. Display only: it touches no filesystem and cannot fail.
    pub fn update(
        &mut self,
        id: ProjectId,
        name: Option<String>,
        colour: Option<usize>,
        custom_colour: Option<u32>,
    ) -> Vec<Reply> {
        let Some(record) = self.find(id) else {
            return vec![Reply::Asker(message_error(Some(id), "no such project"))];
        };
        // Naming a temporary project in the settings dialog is what keeps it, and this is where
        // that happens — there is deliberately no separate promote message.
        if record.temporary {
            return self.promote(id, name, colour, custom_colour);
        }
        let mut record = record.clone();
        if let Some(name) = name.filter(|n| !n.trim().is_empty()) {
            record.name = name.trim().to_string();
        }
        if let Some(colour) = colour {
            record.colour = colour;
            record.custom_colour = custom_colour;
        }

        let snapshot = self.snapshot(&record);
        let mut replies = vec![Reply::Everyone(
            ubiq_proto::messages::Message::ProjectChanged { project: snapshot },
        )];
        replies.extend(self.keep(record));
        replies
    }

    /// Re-point a record at a folder that moved, keeping everything else.
    ///
    /// Unlike a rename this changes truth, which is why it is its own message: it canonicalises,
    /// it re-probes, and it can be refused.
    pub fn locate(&mut self, id: ProjectId, path: &str) -> Vec<Reply> {
        let Some(record) = self.find(id) else {
            return vec![Reply::Asker(message_error(Some(id), "no such project"))];
        };
        let mut record = record.clone();

        let canonical = match std::fs::canonicalize(path) {
            Ok(canonical) if canonical.is_dir() => canonical,
            Ok(canonical) => {
                return vec![Reply::Asker(message_error(
                    Some(id),
                    format!("{} is not a folder", canonical.display()),
                ))];
            }
            Err(error) => {
                return vec![Reply::Asker(message_error(
                    Some(id),
                    format!("{path}: {error}"),
                ))];
            }
        };

        let as_text = canonical.to_string_lossy().into_owned();
        if let Some(other) = self
            .records
            .iter()
            .find(|r| r.path == as_text && r.id != id)
        {
            return vec![Reply::Asker(message_error(
                Some(id),
                format!("that folder is already {}", other.name),
            ))];
        }

        // The id, the colour and the history are the point of Locate: only the path moves.
        record.path = as_text;
        let snapshot = self.snapshot(&record);
        let mut replies = vec![Reply::Everyone(
            ubiq_proto::messages::Message::ProjectChanged { project: snapshot },
        )];
        replies.extend(self.keep(record));
        replies
    }

    /// A window pointed at a project: this is where `last_opened_at` is stamped.
    pub fn opened(&mut self, id: ProjectId) -> Vec<Reply> {
        let Some(record) = self.find(id) else {
            return vec![Reply::Asker(message_error(Some(id), "no such project"))];
        };
        let mut record = record.clone();
        record.last_opened_at = Some(Utc::now());

        let snapshot = self.snapshot(&record);
        let mut replies = vec![Reply::Everyone(
            ubiq_proto::messages::Message::ProjectChanged { project: snapshot },
        )];
        replies.extend(self.keep(record));
        replies
    }

    /// Look at the folder again, and say what is there now.
    pub fn refresh(&self, id: ProjectId) -> Vec<Reply> {
        match self.find(id) {
            Some(record) => vec![Reply::Everyone(
                ubiq_proto::messages::Message::ProjectChanged {
                    project: self.snapshot(record),
                },
            )],
            None => vec![Reply::Asker(message_error(Some(id), "no such project"))],
        }
    }

    // ── view state ──────────────────────────────────────────────────

    pub fn get_preferences(&self, scope: Scope) -> Reply {
        // Anything still queued is what the interface last said, so it answers from there first.
        let value = match self.pending.get(&scope) {
            Some(value) => Some(value.clone()),
            None => self.preferences.get(&scope).unwrap_or_default(),
        };
        Reply::Asker(ubiq_proto::messages::Message::Preferences { scope, value })
    }

    /// Queue a preference. Coalesced per scope, so a drag is one write.
    pub fn set_preferences(&mut self, scope: Scope, value: String, now: Instant) {
        if let Scope::Project(id) = &scope
            && self.find(*id).is_none()
        {
            tracing::debug!("dropping view state for a project that is not in the catalogue");
            return;
        }
        self.pending.insert(scope, value);
        self.due.get_or_insert(now + DEBOUNCE);
    }

    /// When the caller should next call [`Projects::flush_due`], if ever.
    pub fn next_due(&self, now: Instant) -> Option<Duration> {
        self.due.map(|due| due.saturating_duration_since(now))
    }

    /// Write anything that has come due. `now` is a parameter so this tests without sleeping.
    pub fn flush_due(&mut self, now: Instant) {
        match self.due {
            Some(due) if due <= now => self.flush(),
            _ => {}
        }
    }

    /// Write everything queued, due or not. Called on the way out.
    pub fn flush(&mut self) {
        for (scope, value) in std::mem::take(&mut self.pending) {
            // A preference that fails to save is a log line, not a `ProjectError`: losing where a
            // splitter sat is not an event.
            if let Err(error) = self.preferences.set(&scope, &value) {
                tracing::warn!("could not store view state for {scope:?}: {error}");
            }
        }
        self.due = None;
    }

    // ── what is running ─────────────────────────────────────────────

    /// A pane opened in a project. Answers the change every window should hear.
    pub fn pane_opened(&mut self, id: ProjectId) -> Vec<Reply> {
        *self.open_panes.entry(id).or_insert(0) += 1;
        self.changed(id)
    }

    /// A pane in a project ended or was closed.
    pub fn pane_closed(&mut self, id: ProjectId) -> Vec<Reply> {
        if let Some(count) = self.open_panes.get_mut(&id) {
            *count = count.saturating_sub(1);
        }
        self.changed(id)
    }

    fn changed(&self, id: ProjectId) -> Vec<Reply> {
        match self.find(id) {
            Some(record) => vec![Reply::Everyone(
                ubiq_proto::messages::Message::ProjectChanged {
                    project: self.snapshot(record),
                },
            )],
            None => Vec::new(),
        }
    }
}

/// The folder's own name, which is what a project is called until it is renamed.
fn leaf(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

fn message_error(
    project_id: Option<ProjectId>,
    error: impl Into<String>,
) -> ubiq_proto::messages::Message {
    ubiq_proto::messages::Message::ProjectError {
        project_id,
        error: error.into(),
    }
}

fn error_for(id: Option<ProjectId>, error: &StoreError) -> ubiq_proto::messages::Message {
    message_error(id, error.to_string())
}
