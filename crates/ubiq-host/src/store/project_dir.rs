//! Where one project's own data is written, and the `.ubiq/` folder that is the exception to
//! `D30`.
//!
//! Every project has a **data directory**. For a Ubiq-managed project — the default, and what
//! `D30` describes — it is `<config root>/projects/<project ulid>/` and nothing is written inside
//! the project's folder at all. For a **project-managed** project (`D173`,
//! [`ubiq_proto::projects::StorageMode`]) it is `.ubiq/` inside the project's own directory, so
//! the data is committed with the project and travels with a clone.
//!
//! **How a store finds it without being told.** A project-managed project still gets its
//! `<config root>/projects/<ulid>/` directory, and [`POINTER`] inside it names where the data
//! actually went. That is the whole mechanism: a store keeps a [`ProjectDirs`] over the config
//! root and asks it, rather than being handed a catalogue it has no business reading. It also
//! keeps Forget and the orphan collector working unchanged — the directory they sweep is still
//! there, and it holds one small file instead of the data.
//!
//! What the folder holds is split by one question: **would another person cloning this project
//! want it?** The tasks, the plans, the missions, the wiki, this file's own metadata, and an
//! edition's own sidecar or sync binding (`studio.toml`, `tasksrc.toml`), yes. A view blob, a
//! cache, an index, a cloned knowledge base, a run's state, no — those are per-machine, or Ubiq
//! can derive them again.
//!
//! **That question is the layout** ([`ProjectData`], `D173`, `G356`). The shared half sits at the
//! top of the data directory, tasks and their archive together under [`TASKS`]; the whole derived
//! half sits under [`LOCAL`], which is why [`GITIGNORE`] names one directory instead of nine and
//! why a future entry's side is a lookup rather than a judgement. A tree written in the flat shape
//! that came before is moved into it by [`migrate`], once, on open.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::RwLock;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ubiq_proto::ids::ProjectId;
use ubiq_proto::projects::{ProjectRecord, StorageMode};

use crate::atomic::write_atomic;

/// The folder a project-managed project keeps its data in, inside the project's own directory.
pub const IN_PROJECT_DIR: &str = ".ubiq";

/// The in-project copy of what the catalogue holds about this project.
pub const METADATA: &str = "project.toml";

/// The tasks and their archive, together.
pub const TASKS: &str = "tasks";

/// Everything one machine derives: the view blob, the interface's workarea, the index, the caches,
/// the cloned knowledge bases. One directory, so [`GITIGNORE`] is one line and a future entry has
/// an obvious side.
pub const LOCAL: &str = "local";

/// The file under the config root that names where a project's data actually went. Absent for a
/// Ubiq-managed project, which is every project that never asked for anything else.
pub const POINTER: &str = "storage.toml";

/// The format of both [`POINTER`] and [`METADATA`]. They move together: a reader that understands
/// one understands the other, and neither is worth its own number.
pub const PROJECT_DIR_VERSION: u32 = 1;

/// What `.ubiq/.gitignore` says.
///
/// Written once, when the folder is made, and never rewritten — a user who edits it has said
/// something, and re-imposing this on the next launch would take it back. It names `local/`
/// rather than a list, so a directory Ubiq does not write yet is already covered on the side it
/// belongs to instead of costing a line here and a commit of somebody's machine state.
pub const GITIGNORE: &str = "\
# Ubiq keeps this project's own data here.
#
# Committed, and meant to be: everything not listed below — project.toml,
# tasks/, kb.toml, plans/, missions/, wiki/, agent-definitions/, studio.toml and
# tasksrc.toml. The project's settings, tasks and configuration, shared with
# whoever clones it. An edition's own sidecar file (studio.toml) and a sync
# provider's binding (tasksrc.toml) pass the same test as everything else here —
# a teammate's clone needs them to work — and neither ever holds credential
# material, only references, so nothing tracked here is a secret.

# Per-machine and per-person state, and anything Ubiq derives again on its own:
# the view blob, the interface's workarea, the index, the caches, cloned
# knowledge bases, what is running. Losing it costs work, never data.
local/

# Whatever a failed parse was preserved as, and whatever a write left behind.
*.bak.*
*.tmp
";

/// What actually follows a project between the two trees when its storage mode changes
/// (`T-204`), named here rather than read off the source directory's listing.
///
/// The *shared half* is everything at the top of a data directory. Only these entries of it are
/// resolved through [`ProjectDirs`] today: the plan, mission, knowledge-base and agent-definition
/// stores still compose `<config root>/projects/<ulid>/…` themselves (`G355`), so moving their
/// directories would take them away from the only code that reads them. `studio.toml` is named
/// for the same reason [`GITIGNORE`] names it — the edition writes it through [`ProjectDirs`],
/// and a list that left it out would move a project and not its sidecar.
///
/// An allow-list is the side that fails safely: an entry nobody named is left exactly where it
/// is, in plain sight, rather than moved out from under whatever still reads it. When `G355`
/// lands and every store resolves through [`ProjectDirs`], this becomes the whole shared half and
/// this constant is the one place that changes.
///
/// Nothing under [`LOCAL`] is here, and nothing under it ever will be: it is what one machine
/// derived and can derive again, and copying an index into a directory a team shares is wrong
/// twice over.
pub const FOLLOWS: &[&str] = &[
    METADATA,
    TASKS,
    crate::tasksrc::store::TASKSRC_FILE,
    "studio.toml",
];

/// Move a project's data between the two trees, and leave the pointer telling the truth
/// (`T-204`, `D173`).
///
/// **The order is the whole design.** Copy every [`FOLLOWS`] entry to the destination, *then*
/// rewrite or remove [`POINTER`], *then* remove what was copied from the source. The pointer is
/// the commit point, because it is the only thing a store reads: a crash before it leaves the
/// project working exactly where it was, with a half-written destination nothing points at; a
/// crash after leaves a stale copy at the source that is garbage, not data. There is no moment in
/// which a store resolves a directory that does not hold the data.
///
/// Refuses rather than merges when the destination already holds one of the entries — a `.ubiq/`
/// cloned from a teammate is somebody else's tasks, and deciding whose win is not a thing a
/// storage-mode change may do quietly.
///
/// Returns the data directory the project now has.
pub fn change_mode(
    root: &Path,
    record: &ProjectRecord,
    to: StorageMode,
) -> std::io::Result<PathBuf> {
    let (source, destination) = match to {
        StorageMode::ProjectManaged => {
            (under_config(root, record.id), in_project_dir(&record.path))
        }
        StorageMode::UbiqManaged => (in_project_dir(&record.path), under_config(root, record.id)),
    };

    if to == StorageMode::ProjectManaged && !Path::new(&record.path).is_dir() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("{} is not there", record.path),
        ));
    }

    // Both ends in the shape this function moves: a `.ubiq/` cloned from somebody who wrote it
    // flat, or a config-root directory nothing has opened yet in this process.
    migrate(&source);
    migrate(&destination);

    for name in FOLLOWS {
        if destination.join(name).exists() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                format!(
                    "{} already holds {name} — move or remove it first",
                    destination.display()
                ),
            ));
        }
    }

    std::fs::create_dir_all(&destination)?;

    // ── copy ────────────────────────────────────────────────────────
    let mut copied: Vec<PathBuf> = Vec::new();
    for name in FOLLOWS {
        let from = source.join(name);
        if !from.exists() {
            continue;
        }
        let to_path = destination.join(name);
        if let Err(error) = copy_tree(&from, &to_path) {
            // Nothing is committed yet, so the failure is undone rather than explained: the
            // pointer still names the source and the project is untouched.
            for made in copied.iter().chain(std::iter::once(&to_path)) {
                let _ = remove_any(made);
            }
            return Err(error);
        }
        copied.push(to_path);
    }

    // ── commit: the pointer ─────────────────────────────────────────
    match to {
        // `provision` writes the ignore file (only if absent), the metadata and the pointer, in
        // that order — the pointer last, which is what makes this the commit.
        StorageMode::ProjectManaged => {
            let mut moved = record.clone();
            moved.storage = StorageMode::ProjectManaged;
            provision(root, &moved)?;
        }
        // No pointer is the Ubiq-managed answer, so removing it is the commit.
        StorageMode::UbiqManaged => match std::fs::remove_file(source_pointer(root, record.id)) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        },
    }

    // ── remove the source ───────────────────────────────────────────
    // Past the commit point: everything below is tidying, and a failure is a log line and a
    // directory somebody can delete, never a failed move.
    for name in FOLLOWS {
        let stale = source.join(name);
        if stale.exists()
            && let Err(error) = remove_any(&stale)
        {
            tracing::warn!(
                "{} was copied to {} and could not be removed: {error}",
                stale.display(),
                destination.display()
            );
        }
    }

    // A `.ubiq/` with nothing left in it but what this machine derived is litter inside the
    // user's own tree. Removed only when that is all it holds: anything else in there is
    // somebody's, and `D173`'s rule that Ubiq does not delete from a user's folder stands for it.
    if to == StorageMode::UbiqManaged {
        let _ = std::fs::remove_file(source.join(".gitignore"));
        if holds_nothing_but_local(&source)
            && let Err(error) = std::fs::remove_dir_all(&source)
        {
            tracing::warn!("could not remove the empty {}: {error}", source.display());
        }
    }

    Ok(destination)
}

/// Where the pointer file for this project is — always under the config root, whichever tree the
/// data is in. That is the point of it.
fn source_pointer(root: &Path, id: ProjectId) -> PathBuf {
    under_config(root, id).join(POINTER)
}

/// Whether every entry left in a data directory is one this machine derived, so removing the
/// directory costs nothing anybody would miss.
fn holds_nothing_but_local(dir: &Path) -> bool {
    match std::fs::read_dir(dir) {
        Ok(entries) => entries
            .filter_map(Result::ok)
            .all(|entry| entry.file_name() == LOCAL),
        // Unreadable is not empty.
        Err(_) => false,
    }
}

/// A file or a whole directory, copied. The destination's parent is made; a symlink is followed,
/// which is what `fs::copy` does and what a data directory's contents mean.
fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    if from.is_dir() {
        std::fs::create_dir_all(to)?;
        for entry in std::fs::read_dir(from)? {
            let entry = entry?;
            copy_tree(&entry.path(), &to.join(entry.file_name()))?;
        }
        return Ok(());
    }
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::copy(from, to).map(|_| ())
}

/// Remove a path whichever kind it is.
fn remove_any(path: &Path) -> std::io::Result<()> {
    if path.is_dir() {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    }
}

/// The pointer file: where this project's data went, and why.
#[derive(Debug, Serialize, Deserialize)]
struct Pointer {
    version: u32,
    mode: StorageMode,
    /// Absolute, because the two trees are unrelated and a relative path between them would break
    /// the moment either moved.
    dir: String,
}

/// The in-project metadata — the project's name and the rest of what identifies it, written where
/// the project itself can carry it.
///
/// The catalogue keeps the name too, as the fast lookup every window draws from without opening a
/// folder that may be on a slow disk or not mounted at all. This copy is the authority when the
/// two disagree: it travelled with the project, and the catalogue entry is this machine's.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Metadata {
    pub version: u32,
    pub id: ProjectId,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub colour: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_colour: Option<u32>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub initials: String,
}

impl Metadata {
    /// What a record says about itself, in the shape the project's own folder keeps it.
    pub fn of(record: &ProjectRecord) -> Self {
        Self {
            version: PROJECT_DIR_VERSION,
            id: record.id,
            name: record.name.clone(),
            created_at: record.created_at,
            colour: record.colour,
            custom_colour: record.custom_colour,
            initials: record.initials.clone(),
        }
    }
}

/// Where a project's data directory would be if it is project-managed.
pub fn in_project_dir(project_path: &str) -> PathBuf {
    Path::new(project_path).join(IN_PROJECT_DIR)
}

/// Where a project's data directory is under the config root — the Ubiq-managed answer, and the
/// directory that exists either way.
pub fn under_config(root: &Path, id: ProjectId) -> PathBuf {
    root.join("projects").join(id.to_string())
}

/// One project's data directory, and the one place that knows what is where inside it.
///
/// Every per-project path in the host composes through this — no store joins a literal onto a
/// project's directory, because the answer to *shared or derived?* has to be given once. Cheap to
/// make and cheap to drop: it is a `PathBuf` and a set of names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectData {
    dir: PathBuf,
}

impl ProjectData {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    /// The Ubiq-managed answer, composed without asking the disk anything. Callers that must
    /// honour `D173` go through [`ProjectDirs::data`] instead; this one is for the paths that are
    /// under the config root whichever mode the project is in.
    pub fn under_config(root: &Path, id: ProjectId) -> Self {
        Self::new(under_config(root, id))
    }

    /// A project-managed project's `.ubiq/`, from the project's own path.
    pub fn in_project(project_path: &Path) -> Self {
        Self::new(project_path.join(IN_PROJECT_DIR))
    }

    /// The directory itself — what Forget removes and the orphan collector sweeps.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    // ── Shared: what another person cloning this project would want ──────────────────────────

    /// The in-project copy of the record.
    pub fn metadata(&self) -> PathBuf {
        self.dir.join(METADATA)
    }

    /// The live task list.
    pub fn tasks(&self) -> PathBuf {
        self.dir.join(TASKS).join("tasks.toml")
    }

    /// The archive pages, beside the live list rather than inside it.
    pub fn tasks_archive(&self) -> PathBuf {
        self.dir.join(TASKS).join("archive")
    }

    /// The knowledge base's roots — the user's list, not the clones it names.
    pub fn kb_sources(&self) -> PathBuf {
        self.dir.join("kb.toml")
    }

    /// One markdown plan per task that carries a level, and its sidecar.
    pub fn plans(&self) -> PathBuf {
        self.dir.join("plans")
    }

    /// One directory per mission.
    pub fn missions(&self) -> PathBuf {
        self.dir.join("missions")
    }

    /// The internal wiki — pages the user wrote, which nothing can derive again.
    pub fn wiki(&self) -> PathBuf {
        self.dir.join("wiki")
    }

    /// This project's own agent definitions (`D174`).
    #[cfg(feature = "harness")]
    pub fn definitions(&self) -> PathBuf {
        self.dir.join(crate::agent::DEFINITIONS_DIR)
    }

    /// The sync provider's binding, if this project has one.
    pub fn tasksrc(&self) -> PathBuf {
        self.dir.join(crate::tasksrc::store::TASKSRC_FILE)
    }

    // ── Derived: one machine's, and rebuilt by asking again ──────────────────────────────────

    /// The whole derived half, in one directory.
    pub fn local(&self) -> PathBuf {
        self.dir.join(LOCAL)
    }

    /// The interface's view blob, opaque to the host.
    pub fn view(&self) -> PathBuf {
        self.local().join("view.toml")
    }

    /// The interface's workarea — the host makes it and never looks inside.
    pub fn workarea(&self) -> PathBuf {
        self.local().join("ui")
    }

    /// The host's index — the mirror of [`Self::workarea`]; the interface is never told it exists.
    pub fn index(&self) -> PathBuf {
        self.local().join("index")
    }

    /// Cloned knowledge-base repositories, one directory per root. Re-fetchable.
    pub fn kb_clones(&self) -> PathBuf {
        self.local().join("kb")
    }
}

/// What [`migrate`] moves, and where: the flat shape that came before, to the grouped one.
///
/// Both halves are listed rather than only the derived one, because the flat shape is what every
/// existing config root and every existing `.ubiq/` holds and a partial move is the one outcome
/// worth ruling out.
const MOVES: &[(&str, &str)] = &[
    ("tasks.toml", "tasks/tasks.toml"),
    ("tasks-archive", "tasks/archive"),
    ("view.toml", "local/view.toml"),
    ("ui", "local/ui"),
    ("index", "local/index"),
    ("cache", "local/cache"),
    ("searches", "local/searches"),
    ("kb", "local/kb"),
];

/// Move a data directory written in the flat shape into the grouped one (`G356`).
///
/// **Idempotent, and safe to call on anything.** A directory that does not exist, or that holds
/// nothing with an old name, is untouched and costs one `stat` per entry. Each entry moves only
/// when the old name is there and the new one is not: a rename within one directory, which is
/// atomic and never a copy, so there is no window in which the data is in neither place. A
/// destination that already exists means somebody has been writing in the new shape already, and
/// the old name is **left alone** rather than merged over — an entry whose move is refused or
/// fails is logged loudly and still on disk, and the directory still opens.
pub fn migrate(dir: &Path) -> usize {
    if !dir.is_dir() {
        return 0;
    }

    let mut moved = 0;
    for (from, to) in MOVES {
        let source = dir.join(from);
        if !source.exists() {
            continue;
        }
        let destination = dir.join(to);
        if destination.exists() {
            tracing::warn!(
                "leaving {} alone: {} is already there",
                source.display(),
                destination.display()
            );
            continue;
        }
        if let Some(parent) = destination.parent()
            && let Err(error) = std::fs::create_dir_all(parent)
        {
            tracing::warn!("could not make {}: {error}", parent.display());
            continue;
        }
        match std::fs::rename(&source, &destination) {
            Ok(()) => {
                tracing::info!("moved {} to {}", source.display(), destination.display());
                moved += 1;
            }
            Err(error) => tracing::warn!(
                "could not move {} to {}: {error} — it is still where it was",
                source.display(),
                destination.display()
            ),
        }
    }
    moved
}

/// Migrate whichever of a project's two possible data directories are there.
///
/// Called once per project as the catalogue loads, which is the only moment both answers are known
/// without reading a pointer: the config root's directory always exists, and a project-managed
/// project's `.ubiq/` holds the half that followed it.
///
/// `.ubiq/` is visited whatever the mode says, because a Ubiq-managed project can still have one —
/// a knowledge base the user asked to keep in the project writes there ([`ProjectData::kb_clones`]).
/// [`migrate`] is a no-op on a directory that is not there.
pub fn migrate_project(root: &Path, record: &ProjectRecord) {
    migrate(&under_config(root, record.id));
    migrate(&in_project_dir(&record.path));
}

/// Make a project-managed project's `.ubiq/`, and leave behind everything that makes it one: the
/// ignore file, the metadata, and the pointer under the config root that lets a store find it.
///
/// Fails rather than falling back. A project-managed project whose folder could not be made is not
/// a Ubiq-managed project — it is a project the user did not get, and the caller refuses the add.
pub fn provision(root: &Path, record: &ProjectRecord) -> std::io::Result<PathBuf> {
    let dir = in_project_dir(&record.path);
    std::fs::create_dir_all(&dir)?;

    // The folder may be a clone of one somebody else made before `G356`, in which case it arrives
    // flat and nothing has opened it yet.
    migrate(&dir);

    // Never rewritten: see `GITIGNORE`. The folder may already be there from a clone of a project
    // somebody else made project-managed, and that clone's rules are theirs.
    let ignore = dir.join(".gitignore");
    if !ignore.exists() {
        write_atomic(&ignore, GITIGNORE.as_bytes())?;
    }

    write_metadata(&dir, record)?;

    let pointer = Pointer {
        version: PROJECT_DIR_VERSION,
        mode: StorageMode::ProjectManaged,
        dir: dir.to_string_lossy().into_owned(),
    };
    let body = toml::to_string_pretty(&pointer)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    write_atomic(
        &under_config(root, record.id).join(POINTER),
        body.as_bytes(),
    )?;

    Ok(dir)
}

/// Write the in-project copy of the record. Called on every change to what it holds, so a rename
/// reaches the folder as well as the catalogue.
pub fn write_metadata(dir: &Path, record: &ProjectRecord) -> std::io::Result<()> {
    let body = toml::to_string_pretty(&Metadata::of(record))
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    write_atomic(&dir.join(METADATA), body.as_bytes())
}

/// Read it back. `None` covers every way it could fail to say anything — absent, unreadable,
/// unparseable, or written by a newer Ubiq — because the catalogue is a complete answer on its
/// own and a project that will not open is a worse outcome than a stale name.
pub fn read_metadata(dir: &Path) -> Option<Metadata> {
    let raw = std::fs::read_to_string(dir.join(METADATA)).ok()?;
    let metadata: Metadata = toml::from_str(&raw)
        .map_err(|error| {
            tracing::warn!("ignoring unreadable {}/{METADATA}: {error}", dir.display());
        })
        .ok()?;
    (metadata.version <= PROJECT_DIR_VERSION).then_some(metadata)
}

/// Where each project's data is, resolved from the config root alone.
///
/// Cheap and shareable: the answer comes off the disk once per project and is kept. A store holds
/// one of these instead of a catalogue, which is the whole reason the pointer file exists — the
/// stores may not read the catalogue, and the catalogue must not know how a store composes a path.
#[derive(Debug)]
pub struct ProjectDirs {
    root: PathBuf,
    known: RwLock<HashMap<ProjectId, PathBuf>>,
    /// Every data directory [`Self::data`] has already migrated in this process. Directories, not
    /// project ids: a project-managed project has two of them.
    migrated: RwLock<HashSet<PathBuf>>,
}

impl ProjectDirs {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            known: RwLock::new(HashMap::new()),
            migrated: RwLock::new(HashSet::new()),
        }
    }

    /// This project's data directory: `.ubiq/` if a pointer says so, the config root's own
    /// `projects/<ulid>/` otherwise.
    ///
    /// **Only a pointer that was found is remembered.** A project asked about before it was
    /// provisioned answers with the config root, and caching that would outlive the moment it was
    /// true; re-reading a file that is not there is one `stat` and costs nothing worth keeping.
    /// **The memo lasts exactly as long as the pointer it came from.** A storage-mode change
    /// ([`change_mode`]) removes that file, and there are three of these in the process — one
    /// inside the task store, two in the task-source family — that no message could reach to
    /// invalidate. So a remembered answer is re-read once its pointer is gone: one `stat` per
    /// lookup, on a file under the config root rather than on the project's own folder, which is
    /// why an unmounted volume still resolves to the `.ubiq/` it belongs in rather than falling
    /// back to the config root and writing the project's tasks in two places.
    pub fn dir(&self, id: ProjectId) -> PathBuf {
        if let Some(dir) = self
            .known
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(&id)
            .filter(|_| source_pointer(&self.root, id).exists())
        {
            return dir.clone();
        }
        match self.resolve(id) {
            Some(dir) => {
                self.known
                    .write()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(id, dir.clone());
                dir
            }
            None => under_config(&self.root, id),
        }
    }

    /// The same answer, with the layout attached — what a store actually wants.
    ///
    /// Migrates that directory the first time this process asks about it (`G356`), so a store that
    /// reaches a project the catalogue never loaded still finds a tree in the shape it reads. The
    /// memo is per directory rather than per project: the two modes are two directories and a
    /// project-managed one has both.
    pub fn data(&self, id: ProjectId) -> ProjectData {
        let dir = self.dir(id);
        if self
            .migrated
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(dir.clone())
        {
            migrate(&dir);
        }
        ProjectData::new(dir)
    }

    fn resolve(&self, id: ProjectId) -> Option<PathBuf> {
        let under_config = under_config(&self.root, id);
        let raw = std::fs::read_to_string(under_config.join(POINTER)).ok()?;
        match toml::from_str::<Pointer>(&raw) {
            Ok(pointer) if pointer.version <= PROJECT_DIR_VERSION => {
                Some(PathBuf::from(pointer.dir))
            }
            Ok(pointer) => {
                tracing::warn!(
                    "{}/{POINTER} is version {}, and this Ubiq understands {PROJECT_DIR_VERSION}",
                    under_config.display(),
                    pointer.version
                );
                None
            }
            Err(error) => {
                tracing::warn!(
                    "ignoring unreadable {}/{POINTER}: {error}",
                    under_config.display()
                );
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(path: &str) -> ProjectRecord {
        ProjectRecord {
            id: ProjectId::generate(),
            name: "demo".into(),
            path: path.into(),
            colour: 3,
            custom_colour: None,
            temporary: false,
            created_at: Utc::now(),
            last_opened_at: None,
            search_excludes: vec![],
            index: None,
            mission_term: None,
            managed_repos: vec![],
            tools: vec![],
            lanes: vec![],
            runs_on: None,
            initials: String::new(),
            storage: StorageMode::ProjectManaged,
        }
    }

    #[test]
    fn a_project_managed_project_gets_its_folder_its_ignore_and_its_metadata() {
        let config = tempfile::tempdir().expect("a config root");
        let project = tempfile::tempdir().expect("a project folder");
        let record = record(&project.path().to_string_lossy());

        let dir = provision(config.path(), &record).expect("the folder is made");
        assert_eq!(dir, project.path().join(IN_PROJECT_DIR));
        assert!(dir.join(".gitignore").is_file());
        assert_eq!(read_metadata(&dir).expect("metadata").name, "demo");

        let ignore = std::fs::read_to_string(dir.join(".gitignore")).expect("the ignore file");
        let rules: Vec<&str> = ignore
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .collect();
        assert_eq!(
            rules,
            vec!["local/", "*.bak.*", "*.tmp"],
            "the derived half is one entry, not nine: {ignore}"
        );
        assert!(
            !rules.iter().any(|rule| rule.starts_with("tasks")),
            "tasks are the point of committing this folder: {ignore}"
        );
    }

    /// Everything the flat shape held ends up where the grouped one reads it, and saying so twice
    /// changes nothing.
    #[test]
    fn migration_groups_a_flat_directory_and_is_idempotent() {
        let dir = tempfile::tempdir().expect("a data directory");
        let dir = dir.path();
        std::fs::write(dir.join("tasks.toml"), "version = 1").expect("tasks");
        std::fs::create_dir_all(dir.join("tasks-archive")).expect("the archive");
        std::fs::write(dir.join("tasks-archive").join("0001.toml"), "old").expect("a page");
        std::fs::write(dir.join("view.toml"), "blob").expect("view state");
        std::fs::create_dir_all(dir.join("ui")).expect("the workarea");
        std::fs::create_dir_all(dir.join("index")).expect("the index");
        std::fs::write(dir.join("kb.toml"), "version = 1").expect("kb sources");

        assert_eq!(migrate(dir), 5, "tasks, archive, view, ui, index");

        let data = ProjectData::new(dir.to_path_buf());
        assert_eq!(
            std::fs::read_to_string(data.tasks()).expect("the tasks moved"),
            "version = 1"
        );
        assert_eq!(
            std::fs::read_to_string(data.tasks_archive().join("0001.toml")).expect("the page"),
            "old"
        );
        assert_eq!(
            std::fs::read_to_string(data.view()).expect("the view blob moved"),
            "blob"
        );
        assert!(data.workarea().is_dir() && data.index().is_dir());
        assert!(
            data.kb_sources().is_file(),
            "the shared half that did not move is still where it was"
        );
        assert!(!dir.join("tasks.toml").exists() && !dir.join("ui").exists());

        assert_eq!(migrate(dir), 0, "nothing left to move");
        assert_eq!(
            std::fs::read_to_string(data.tasks()).expect("still there"),
            "version = 1"
        );
    }

    /// Half-migrated is a state the next open finishes, and never one that loses the half already
    /// in the new shape.
    #[test]
    fn migration_finishes_a_half_done_move_without_overwriting_what_is_there() {
        let dir = tempfile::tempdir().expect("a data directory");
        let dir = dir.path();
        let data = ProjectData::new(dir.to_path_buf());

        // The tasks made it across last time; the view blob did not.
        std::fs::create_dir_all(data.tasks().parent().expect("tasks/")).expect("tasks/");
        std::fs::write(data.tasks(), "new").expect("the moved file");
        std::fs::write(dir.join("tasks.toml"), "old").expect("a leftover");
        std::fs::write(dir.join("view.toml"), "blob").expect("view state");

        assert_eq!(migrate(dir), 1, "only the view blob had anywhere to go");
        assert_eq!(
            std::fs::read_to_string(data.tasks()).expect("the tasks"),
            "new",
            "what was already migrated wins"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("tasks.toml")).expect("the leftover"),
            "old",
            "and the leftover is left on disk rather than deleted"
        );
        assert_eq!(
            std::fs::read_to_string(data.view()).expect("the view blob"),
            "blob"
        );
    }

    /// The `.ubiq/` of a project cloned from a teammate who wrote it flat is migrated when it is
    /// provisioned, before any store reads it.
    #[test]
    fn provisioning_migrates_a_folder_that_arrived_flat() {
        let config = tempfile::tempdir().expect("a config root");
        let project = tempfile::tempdir().expect("a project folder");
        let record = record(&project.path().to_string_lossy());
        let flat = project.path().join(IN_PROJECT_DIR);
        std::fs::create_dir_all(&flat).expect("a cloned .ubiq/");
        std::fs::write(flat.join("tasks.toml"), "shared").expect("their tasks");

        let dir = provision(config.path(), &record).expect("the folder is made");
        assert_eq!(
            std::fs::read_to_string(ProjectData::new(dir).tasks()).expect("their tasks"),
            "shared"
        );
    }

    /// A store that reaches a project the catalogue never swept still finds the grouped shape.
    #[test]
    fn asking_for_a_projects_data_migrates_it_once() {
        let config = tempfile::tempdir().expect("a config root");
        let id = ProjectId::generate();
        let flat = under_config(config.path(), id);
        std::fs::create_dir_all(&flat).expect("the directory");
        std::fs::write(flat.join("tasks.toml"), "mine").expect("tasks");

        let dirs = ProjectDirs::new(config.path().to_path_buf());
        let data = dirs.data(id);
        assert_eq!(
            std::fs::read_to_string(data.tasks()).expect("the tasks moved"),
            "mine"
        );

        // Asking again does not re-migrate: a file written back at the old name by something else
        // would be left exactly where it was put.
        std::fs::write(flat.join("tasks.toml"), "not ours").expect("an intruder");
        let _ = dirs.data(id);
        assert!(
            flat.join("tasks.toml").is_file(),
            "untouched the second time"
        );
    }

    /// The pointer is the whole of how a store that never sees the catalogue finds the folder.
    #[test]
    fn the_pointer_sends_a_store_into_the_project_and_nowhere_else_without_one() {
        let config = tempfile::tempdir().expect("a config root");
        let project = tempfile::tempdir().expect("a project folder");
        let record = record(&project.path().to_string_lossy());
        provision(config.path(), &record).expect("the folder is made");

        let dirs = ProjectDirs::new(config.path().to_path_buf());
        assert_eq!(dirs.dir(record.id), project.path().join(IN_PROJECT_DIR));

        let other = ProjectId::generate();
        assert_eq!(dirs.dir(other), under_config(config.path(), other));
    }

    /// A user's hand edit is theirs to keep: provisioning again leaves the ignore file alone and
    /// still refreshes the metadata, which is what a rename needs.
    #[test]
    fn provisioning_again_keeps_the_ignore_file_and_updates_the_metadata() {
        let config = tempfile::tempdir().expect("a config root");
        let project = tempfile::tempdir().expect("a project folder");
        let mut record = record(&project.path().to_string_lossy());
        let dir = provision(config.path(), &record).expect("the folder is made");
        std::fs::write(dir.join(".gitignore"), "mine\n").expect("the user edits it");

        record.name = "renamed".into();
        provision(config.path(), &record).expect("again");
        assert_eq!(
            std::fs::read_to_string(dir.join(".gitignore")).expect("the ignore file"),
            "mine\n"
        );
        assert_eq!(read_metadata(&dir).expect("metadata").name, "renamed");
    }
}
