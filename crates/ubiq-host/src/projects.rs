//! The catalogue as the host runs it: what each message in the project family does.
//!
//! Nothing here draws, and nothing here decides what a swatch looks like — the palette is the
//! interface's. But every creation path funnels through [`Projects::add`], so when a caller passes
//! no colour, this is the one place that picks the next unused index rather than leaving every such
//! project at swatch zero. It applies the same fewest-used rule as the interface's own
//! `AppState::next_colour`, over [`PROJECT_COLOUR_COUNT`], so the two never disagree.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use chrono::Utc;
use ubiq_proto::ids::ProjectId;
use ubiq_proto::projects::{
    DroneChange, IndexChange, LanePref, MissionTermChange, ProjectRecord, ProjectSnapshot, Scope,
    StorageMode,
};
use ubiq_proto::tools::ToolDef;

#[cfg(feature = "harness")]
use crate::gc;
use crate::health::probe;
use crate::host_path::{request_path, wire_string};
use crate::reply::Reply;
use crate::store::project_dir;
use crate::store::{PreferenceStore, ProjectStore, StoreError};

/// The number of distinct project swatches before they repeat.
///
/// Mirrors `crates/ubiq/src/theme.rs`'s `Palette::project`'s `swatches: [Rgba; 16]` — a fixed-size
/// array in both palettes, not something the host can read (the palette is the interface's), so the
/// count is pinned here instead. If that array's length ever changes, this constant has to change
/// with it or [`Projects::next_colour`] and the interface's `AppState::next_colour` drift apart.
const PROJECT_COLOUR_COUNT: usize = 16;

/// Where the shared workarea is, under the config root.
///
/// The mirror of [`Projects::workarea`] one level up: same contract, no project. The host
/// reserves it and never reads inside — see [`ubiq_proto::messages::Message::HostInfo`]'s
/// `shared_workarea`. It hangs off the config root rather than off a project, so it is not under
/// any project's `local/` and keeps its own name.
pub const SHARED_WORKAREA: &str = "ui";

/// The interface's own directory, belonging to no project.
///
/// A free function rather than a method on [`Projects`], because there is no project to ask: what
/// goes here is a property of the host — a vendor bundle five projects want is one copy — so it
/// hangs off the config root beside `projects/`, not inside it.
pub fn shared_workarea(root: &Path) -> PathBuf {
    root.join(SHARED_WORKAREA)
}

/// Reserve it, and answer the path the interface is told.
///
/// [`Projects::reserve_workarea`]'s posture exactly, for the same reason: the interface is handed
/// a path and not a maybe, and a directory that will not be made is still named, because what is
/// kept there is disposable by design. A failure is logged and never propagated.
pub fn reserve_shared_workarea(root: &Path) -> String {
    let path = shared_workarea(root);
    if let Err(error) = std::fs::create_dir_all(&path) {
        tracing::warn!(
            "could not reserve the interface's shared workarea at {}: {error}",
            path.display()
        );
    }
    path.to_string_lossy().into_owned()
}

/// How long a preference sits before it is written.
///
/// A panel drag fires continuously, so the writes are coalesced per scope. Long enough that a drag
/// is one write, short enough that quitting straight after a change keeps it.
pub const DEBOUNCE: Duration = Duration::from_millis(400);

/// A marker file at a project's root, and the folders its presence implies should be skipped.
///
/// One row per ecosystem, kept small and easy to extend — add a row, not a branch.
const AUTODETECT_MARKERS: &[(&str, &[&str])] = &[
    ("package.json", &["node_modules"]),
    ("Cargo.toml", &["target"]),
    ("pyproject.toml", &[".venv", "__pycache__"]),
    ("requirements.txt", &[".venv", "__pycache__"]),
];

/// Probe a new project's root for well-known markers and answer the excludes it implies.
///
/// Called only when a fresh record's `search_excludes` is still empty — a list the user already
/// curated (or a project promoted from temporary, which keeps whatever it had) is never touched.
/// A folder [`crate::settings`]'s own global default already skips is left out: the coordinator
/// always merges `HostSettings::search_excludes` with the record's own
/// (`coordinator.rs`'s `watch_project`, `settle_index` and the search paths), so seeding a
/// duplicate here would reach the workers either way and just clutters the per-project list the
/// settings UI shows back to the user.
fn detect_search_excludes(root: &Path) -> Vec<String> {
    let already_global: HashSet<String> = ubiq_proto::settings::HostSettings::default()
        .search_excludes
        .into_iter()
        .collect();
    let mut seen = HashSet::new();
    let mut excludes = Vec::new();
    for (marker, folders) in AUTODETECT_MARKERS {
        if !root.join(marker).is_file() {
            continue;
        }
        for folder in *folders {
            if !already_global.contains(*folder) && seen.insert(*folder) {
                excludes.push((*folder).to_string());
            }
        }
    }
    excludes
}

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
    /// The one tree a project's own folder may be deleted from — see
    /// [`crate::settings::ephemeral_root`]. Pointed at the resolved root by the coordinator, which
    /// is the half that can read the settings; until then it is the built-in default.
    ephemeral_root: PathBuf,
    /// Whether the catalogue actually loaded. A sweep against the empty catalogue a *corrupt*
    /// file produces would delete every ephemeral project's folder, which is the thing preserving
    /// the file was meant to avoid.
    loaded: bool,
}

impl Projects {
    /// Open the catalogue. Answers itself and whatever should be said about how that went.
    pub fn open(
        root: PathBuf,
        catalogue: Box<dyn ProjectStore>,
        preferences: Box<dyn PreferenceStore>,
    ) -> (Self, Vec<Reply>) {
        let mut this = Self {
            ephemeral_root: root.join("ephemeral"),
            loaded: false,
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
                this.loaded = true;
                // Before anything composes a path: every store reads the grouped shape, and a
                // tree written flat holds the same data under the names that came before
                // (`G356`). Idempotent, so the second launch is a handful of `stat`s per project.
                for record in &this.records {
                    project_dir::migrate_project(&this.root, record);
                }
                this.reconcile_in_project_metadata();
                // Only ever after a load that worked. Collecting against the empty catalogue a
                // *corrupt* file produces would delete every project's view state.
                #[cfg(feature = "harness")]
                {
                    let keep: HashSet<ProjectId> = this.records.iter().map(|r| r.id).collect();
                    gc::collect(&this.root, &keep);
                }
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

    /// Take the name from each project-managed project's own folder, where the two disagree.
    ///
    /// The catalogue keeps the name so the picker can draw every project without opening a folder
    /// that may be slow, unmounted or gone — that is the fast lookup, and it is the copy that goes
    /// stale. The in-project copy travelled with the project: somebody renamed it on another
    /// machine, or edited `project.toml` by hand, and this machine's catalogue has not heard.
    /// So the folder wins, and the catalogue is corrected rather than argued with.
    ///
    /// Silent about everything it cannot read: a folder that is not mounted keeps the name the
    /// catalogue has, which is the whole reason the catalogue keeps one.
    fn reconcile_in_project_metadata(&mut self) {
        let corrections: Vec<(ProjectId, String)> = self
            .records
            .iter()
            .filter(|record| record.storage.is_project_managed())
            .filter_map(|record| {
                let dir = project_dir::in_project_dir(&record.path);
                let metadata = project_dir::read_metadata(&dir)?;
                let name = metadata.name.trim();
                (!name.is_empty() && name != record.name).then(|| (record.id, name.to_string()))
            })
            .collect();

        for (id, name) in corrections {
            let Some(record) = self.records.iter_mut().find(|record| record.id == id) else {
                continue;
            };
            tracing::info!(
                "taking {id}'s name from its own folder: {} becomes {name}",
                record.name
            );
            record.name = name;
            let record = record.clone();
            if let Err(error) = self.catalogue.upsert(&record) {
                tracing::warn!("could not correct {id}'s name in the catalogue: {error}");
            }
        }
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
            ephemeral: self.ephemeral(record),
            record: record.clone(),
        }
    }

    /// Whether forgetting this project also takes its folder.
    ///
    /// The one place the question is answered, so that what the interface warns about and what
    /// [`Self::forget`] deletes can never come apart. Both conditions are load-bearing: see the
    /// comment in `forget` for why neither alone is enough.
    fn ephemeral(&self, record: &ProjectRecord) -> bool {
        record.temporary && inside(&self.ephemeral_root, Path::new(&record.path))
    }

    /// Where this project's interface keeps its own files.
    ///
    /// One directory per project, under the `local/` half of that project's data directory with
    /// the view blob and the index, so Forget and the orphan collector cover it without knowing it
    /// is there and a project-managed project never commits it.
    pub fn workarea(&self, id: ProjectId) -> PathBuf {
        project_dir::ProjectData::under_config(&self.root, id).workarea()
    }

    /// Where this project's index lives.
    ///
    /// A sibling of [`Self::workarea`] and the mirror of it: that directory belongs to the
    /// interface and the host never looks inside, this one belongs to the host and the interface
    /// is never told it exists. Both sit under the project's own directory, so Forget and the
    /// orphan collector already remove them without knowing what either holds.
    ///
    /// Nothing here is ever written into the user's project folder.
    pub fn index_dir(&self, id: ProjectId) -> PathBuf {
        project_dir::ProjectData::under_config(&self.root, id).index()
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
        // The project's own copy, for a project-managed project: one write per change to what it
        // holds, so a rename reaches the folder as well as the catalogue. A failure is a log line
        // and not a `ProjectError` — the catalogue below is the durable answer, and the folder is
        // the copy that can be behind.
        if record.storage.is_project_managed() {
            let dir = project_dir::in_project_dir(&record.path);
            if let Err(error) = project_dir::write_metadata(&dir, &record) {
                tracing::warn!("could not write {}: {error}", dir.display());
            }
        }
        match self.catalogue.upsert(&record) {
            Ok(()) => None,
            Err(error) => self.warn_once(&error),
        }
    }

    /// The swatch a new project gets when its caller names none: the one fewest existing projects
    /// use, so the palette spreads before it repeats.
    ///
    /// The host-side twin of the interface's `AppState::next_colour` — same rule, same tie-break
    /// (lowest index wins a tie), so a project coloured here and one coloured there never disagree.
    /// This is the one point every creation path goes through, unlike the interface's helper, which
    /// only the "Add project" dialog calls.
    fn next_colour(&self) -> usize {
        let mut used = vec![0usize; PROJECT_COLOUR_COUNT];
        for record in &self.records {
            used[record.colour % PROJECT_COLOUR_COUNT] += 1;
        }
        used.iter()
            .enumerate()
            .min_by_key(|(index, taken)| (**taken, *index))
            .map(|(index, _)| index)
            .unwrap_or(0)
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
    ///
    /// This is the one place [`StorageMode`] is chosen. A project-managed add makes the project's
    /// `.ubiq/` before the record exists, and a failure to make it refuses the add — see
    /// [`crate::store::project_dir::provision`]. A temporary folder is never written down at all,
    /// so it is always Ubiq-managed whatever it asked for.
    pub fn add(
        &mut self,
        path: &str,
        name: Option<String>,
        colour: Option<usize>,
        custom_colour: Option<u32>,
        temporary: bool,
        storage: StorageMode,
    ) -> Vec<Reply> {
        let canonical = match std::fs::canonicalize(request_path(path)) {
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

        let as_text = wire_string(&canonical);
        if let Some(existing) = self.records.iter().find(|r| r.path == as_text) {
            // Without this, a folder dropped and then also added through the picker looks
            // persisted to the caller while still carrying the flag, and would be silently
            // forgotten when closed. Anything else about the existing record — dropped onto
            // twice, or added for real twice — is answered exactly as before.
            if existing.temporary && !temporary {
                let id = existing.id;
                return self.promote(
                    id,
                    name,
                    colour,
                    custom_colour,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                );
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
            colour: if temporary {
                0
            } else {
                colour.unwrap_or_else(|| self.next_colour())
            },
            custom_colour: if temporary { None } else { custom_colour },
            temporary,
            created_at: Utc::now(),
            last_opened_at: None,
            search_excludes: detect_search_excludes(&canonical),
            index: None,
            mission_term: None,
            managed_repos: Vec::new(),
            tools: Vec::new(),
            lanes: Vec::new(),
            runs_on: None,
            initials: String::new(),
            storage: if temporary {
                StorageMode::UbiqManaged
            } else {
                storage
            },
        };

        // Before the record exists anywhere, so a refusal leaves no half-made project behind.
        // `keep` writes the metadata again below; this is the call that makes the folder, the
        // ignore file and the pointer a store resolves through.
        if record.storage.is_project_managed()
            && let Err(error) = project_dir::provision(&self.root, &record)
        {
            return vec![Reply::Asker(message_error(
                None,
                format!(
                    "could not make {}: {error}",
                    project_dir::in_project_dir(&record.path).display()
                ),
            ))];
        }

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
    // The same mirror as `update` above: one argument per field of `Message::UpdateProject`.
    #[allow(clippy::too_many_arguments)]
    fn promote(
        &mut self,
        id: ProjectId,
        name: Option<String>,
        colour: Option<usize>,
        custom_colour: Option<u32>,
        search_excludes: Option<Vec<String>>,
        index: Option<IndexChange>,
        mission_term: Option<MissionTermChange>,
        tools: Option<Vec<ToolDef>>,
        managed_repos: Option<Vec<String>>,
        lanes: Option<Vec<LanePref>>,
        runs_on: Option<DroneChange>,
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
        if let Some(search_excludes) = search_excludes {
            record.search_excludes = search_excludes;
        }
        if let Some(index) = index {
            record.index = index.resolve();
        }
        if let Some(mission_term) = mission_term {
            record.mission_term = mission_term.resolve();
        }
        if let Some(tools) = tools {
            record.tools = tools;
        }
        if let Some(managed_repos) = managed_repos {
            record.managed_repos = managed_repos;
        }
        if let Some(lanes) = lanes {
            record.lanes = lanes;
        }
        if let Some(runs_on) = runs_on {
            record.runs_on = runs_on.resolve();
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
    ///
    /// A project-managed project's `.ubiq/` is **not** touched. It is inside the user's own
    /// folder, which Forget has never deleted anything from, and it is committed — removing it
    /// would be a change to the user's repository rather than to what Ubiq remembers. Adding the
    /// folder again as project-managed picks the tasks back up, under a new id.
    pub fn forget(&mut self, id: ProjectId) -> Vec<Reply> {
        let Some(folder) = self
            .find(id)
            .map(|record| (record.path.clone(), self.ephemeral(record)))
        else {
            return vec![Reply::Asker(message_error(Some(id), "no such project"))];
        };
        let folder = Some(folder);
        self.records.retain(|r| r.id != id);
        self.open_panes.remove(&id);
        let _ = self.preferences.clear(&Scope::Project(id));

        let mut replies = Vec::new();
        if let Err(error) = self.catalogue.remove(id)
            && let Some(reply) = self.warn_once(&error)
        {
            replies.push(reply);
        }

        let dir = project_dir::under_config(&self.root, id);
        if dir.exists()
            && let Err(error) = std::fs::remove_dir_all(&dir)
        {
            tracing::warn!("could not remove {}: {error}", dir.display());
        }

        // The project's *own* folder, which is a different thing from the workarea above and is
        // usually the user's. Two independent conditions have to hold, because deleting one of
        // these is the mistake nobody can undo. It must sit inside the ephemeral root — a clone
        // Ubiq made, in a tree Ubiq owns — and it must be `temporary`. Neither alone is enough:
        // the flag is already set for a folder dragged in from anywhere on the disk, and the root
        // is a setting, so a user who points it at their home directory would otherwise turn the
        // ordinary Forget action into a delete.
        if let Some((path, ephemeral)) = folder
            && ephemeral
            && let Err(error) = std::fs::remove_dir_all(&path)
        {
            tracing::warn!("could not remove the ephemeral clone at {path}: {error}");
        }

        replies.push(Reply::Everyone(
            ubiq_proto::messages::Message::ProjectForgotten { project_id: id },
        ));
        replies
    }

    /// Rename, recolour, change what a project's searches skip, or change which repositories
    /// inside it the project takes on. Touches no filesystem and cannot fail beyond "no such
    /// project": `search_excludes`, `index`, `managed_repos`, `lanes` and `runs_on` are display state
    /// exactly like the rest — `None` leaves a field as it is, `Some` replaces it. A managed path
    /// naming no repository the walk can find is kept as given; only the observation decides what
    /// it means.
    // One argument per field of `Message::UpdateProject` plus the id: the mirror is the
    // point, and the same shape `Coordinator::start_conversation` keeps for its message.
    #[allow(clippy::too_many_arguments)]
    pub fn update(
        &mut self,
        id: ProjectId,
        name: Option<String>,
        colour: Option<usize>,
        custom_colour: Option<u32>,
        search_excludes: Option<Vec<String>>,
        index: Option<IndexChange>,
        mission_term: Option<MissionTermChange>,
        tools: Option<Vec<ToolDef>>,
        managed_repos: Option<Vec<String>>,
        lanes: Option<Vec<LanePref>>,
        runs_on: Option<DroneChange>,
    ) -> Vec<Reply> {
        let Some(record) = self.find(id) else {
            return vec![Reply::Asker(message_error(Some(id), "no such project"))];
        };
        // Naming a temporary project in the settings dialog is what keeps it, and this is where
        // that happens — there is deliberately no separate promote message.
        if record.temporary {
            return self.promote(
                id,
                name,
                colour,
                custom_colour,
                search_excludes,
                index,
                mission_term,
                tools,
                managed_repos,
                lanes,
                runs_on,
            );
        }
        let mut record = record.clone();
        if let Some(name) = name.filter(|n| !n.trim().is_empty()) {
            record.name = name.trim().to_string();
        }
        if let Some(colour) = colour {
            record.colour = colour;
            record.custom_colour = custom_colour;
        }
        if let Some(search_excludes) = search_excludes {
            record.search_excludes = search_excludes;
        }
        if let Some(index) = index {
            record.index = index.resolve();
        }
        if let Some(mission_term) = mission_term {
            record.mission_term = mission_term.resolve();
        }
        if let Some(tools) = tools {
            record.tools = tools;
        }
        if let Some(managed_repos) = managed_repos {
            record.managed_repos = managed_repos;
        }
        if let Some(lanes) = lanes {
            record.lanes = lanes;
        }
        if let Some(runs_on) = runs_on {
            record.runs_on = runs_on.resolve();
        }

        let snapshot = self.snapshot(&record);
        let mut replies = vec![Reply::Everyone(
            ubiq_proto::messages::Message::ProjectChanged { project: snapshot },
        )];
        replies.extend(self.keep(record));
        replies
    }

    /// Override, or clear, the letters the rail's badge shows for this project. Trimmed and
    /// capped at three characters here rather than trusted from the wire — a field this narrow is
    /// cheaper to enforce once, at the one place it is written, than to re-check at every draw.
    /// Its own message rather than a field on [`Message::UpdateProject`]; see
    /// [`Message::SetProjectInitials`].
    ///
    /// [`Message::UpdateProject`]: ubiq_proto::messages::Message::UpdateProject
    /// [`Message::SetProjectInitials`]: ubiq_proto::messages::Message::SetProjectInitials
    pub fn set_initials(&mut self, id: ProjectId, initials: &str) -> Vec<Reply> {
        let Some(record) = self.find(id) else {
            return vec![Reply::Asker(message_error(Some(id), "no such project"))];
        };
        let mut record = record.clone();
        record.initials = initials.trim().chars().take(3).collect();

        let snapshot = self.snapshot(&record);
        let mut replies = vec![Reply::Everyone(
            ubiq_proto::messages::Message::ProjectChanged { project: snapshot },
        )];
        replies.extend(self.keep(record));
        replies
    }

    /// Move a project's data between the two storage modes (`T-204`, `D173`).
    ///
    /// The mode is chosen at creation and changed only here, because this is a migration rather
    /// than a setting: [`crate::store::project_dir::change_mode`] copies the shared half across
    /// two unrelated trees, rewrites the pointer as its commit point, and removes the source. The
    /// catalogue record's mode is changed **after** that returns, so a failure leaves the record
    /// naming the tree the data is still in.
    ///
    /// **Refused while a pane is running in the project.** A harness in a pane is an agent
    /// writing tasks through the MCP listener, and a store resolving the old directory mid-copy
    /// would write into the tree being taken away. `open_panes` is the count the picker already
    /// draws, kept by [`Self::pane_opened`] and [`Self::pane_closed`]; the coordinator refuses on
    /// its own behalf for a conversation, which has no pane and so no entry here.
    ///
    /// Nothing under `local/` moves: it is what this machine derived — the view blob, the
    /// workarea, the index, the caches — and it is rebuilt at the destination by being asked for
    /// again. Today every one of those resolves under the config root whichever mode the project
    /// is in, so the move costs the user nothing at all.
    pub fn set_storage(&mut self, id: ProjectId, storage: StorageMode) -> Vec<Reply> {
        let Some(record) = self.find(id) else {
            return vec![Reply::Asker(storage_error(id, storage, "no such project"))];
        };
        if record.storage == storage {
            // Already there, and a move that moves nothing touches no disk.
            let dir = project_dir::ProjectData::new(match storage {
                StorageMode::ProjectManaged => project_dir::in_project_dir(&record.path),
                StorageMode::UbiqManaged => project_dir::under_config(&self.root, id),
            });
            return vec![Reply::Asker(
                ubiq_proto::messages::Message::ProjectStorageMoved {
                    project_id: id,
                    storage,
                    dir: wire_string(dir.dir()),
                },
            )];
        }
        // A temporary project is never written down, so it has no mode to change — and promoting
        // it is what the settings dialog's name field is for.
        if record.temporary {
            return vec![Reply::Asker(storage_error(
                id,
                storage,
                "a temporary project keeps no data of its own",
            ))];
        }
        if self.open_panes.get(&id).copied().unwrap_or(0) > 0 {
            return vec![Reply::Asker(storage_error(
                id,
                storage,
                "close what is running in this project first",
            ))];
        }

        let mut record = record.clone();
        let dir = match project_dir::change_mode(&self.root, &record, storage) {
            Ok(dir) => dir,
            Err(error) => {
                return vec![Reply::Asker(storage_error(
                    id,
                    storage,
                    format!("the project's data is where it was: {error}"),
                ))];
            }
        };

        record.storage = storage;
        let snapshot = self.snapshot(&record);
        let mut replies = vec![
            Reply::Everyone(ubiq_proto::messages::Message::ProjectChanged { project: snapshot }),
            Reply::Asker(ubiq_proto::messages::Message::ProjectStorageMoved {
                project_id: id,
                storage,
                dir: wire_string(&dir),
            }),
        ];
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

        let canonical = match std::fs::canonicalize(request_path(path)) {
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

        let as_text = wire_string(&canonical);
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
        // A project-managed project's `.ubiq/` moved with the folder it is inside, but the pointer
        // under the config root still names where the folder was. Re-provisioning rewrites it —
        // and makes the folder again if the move was a copy that left it behind.
        if record.storage.is_project_managed()
            && let Err(error) = project_dir::provision(&self.root, &record)
        {
            return vec![Reply::Asker(message_error(
                Some(id),
                format!(
                    "could not reach {}: {error}",
                    project_dir::in_project_dir(&record.path).display()
                ),
            ))];
        }
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

    /// Point the ephemeral gate at the root the settings actually name.
    ///
    /// Called once by the coordinator, which is the half that can read the host settings —
    /// [`Projects::open`] runs before they are parsed and starts from the built-in default.
    pub fn point_ephemeral_at(&mut self, root: PathBuf) {
        self.ephemeral_root = root;
    }

    /// Remove the folders under the ephemeral root that no record names.
    ///
    /// The same job [`crate::gc`] does for a project's config directory, over the other tree a
    /// clone writes to: an ephemeral project is dropped from the catalogue when its window closes,
    /// and a crash between the two leaves a folder nobody will ever open again.
    ///
    /// **Only ever after a load that succeeded**, on `gc`'s rule: against the empty catalogue a
    /// corrupt file produces, every ephemeral clone still on disk would look like an orphan.
    pub fn sweep_ephemeral(&mut self) {
        if !self.loaded {
            return;
        }
        let keep: HashSet<PathBuf> = self
            .records
            .iter()
            .filter_map(|record| std::fs::canonicalize(&record.path).ok())
            .collect();
        let Ok(entries) = std::fs::read_dir(&self.ephemeral_root) else {
            return;
        };
        for entry in entries.flatten() {
            if !entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false) {
                continue;
            }
            let path = entry.path();
            let canonical = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
            if keep.contains(&canonical) {
                continue;
            }
            match std::fs::remove_dir_all(&path) {
                Ok(()) => tracing::info!("collected {}, which no record names", path.display()),
                Err(error) => tracing::warn!("could not collect {}: {error}", path.display()),
            }
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

    /// How many projects have at least one pane running in them — not how many are in the
    /// catalogue, which is a different and much larger number. A count of zero is kept in the map
    /// rather than removed, so the filter is the answer.
    pub fn open_count(&self) -> usize {
        self.open_panes.values().filter(|n| **n > 0).count()
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

/// A refused or failed storage move. Its own message rather than a `ProjectError` so the panel
/// that asked can put its row back — see `Message::ProjectStorageError`.
fn storage_error(
    project_id: ProjectId,
    storage: StorageMode,
    error: impl Into<String>,
) -> ubiq_proto::messages::Message {
    ubiq_proto::messages::Message::ProjectStorageError {
        project_id,
        storage,
        error: error.into(),
    }
}

fn error_for(id: Option<ProjectId>, error: &StoreError) -> ubiq_proto::messages::Message {
    message_error(id, error.to_string())
}

/// Whether `path` really sits inside `root`.
///
/// Both sides are canonicalised, which is the whole point: a record naming `<root>/../elsewhere`
/// is a string that starts with the root and a folder that is nowhere near it, and a textual test
/// would delete the wrong tree. Anything that cannot be canonicalised — a folder already gone, a
/// root that was never made — answers `false`, so the gate fails closed. The root itself is not
/// inside itself: emptying the whole tree is not what forgetting one project means.
fn inside(root: &Path, path: &Path) -> bool {
    match (std::fs::canonicalize(root), std::fs::canonicalize(path)) {
        (Ok(root), Ok(path)) => path != root && path.starts_with(&root),
        _ => false,
    }
}
