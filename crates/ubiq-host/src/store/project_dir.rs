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
//! want it?** The tasks, the plans, the missions and this file's own metadata, yes. A view blob, a
//! cache, an index, a cloned knowledge base, a run's state, no — those are per-machine, or Ubiq
//! can derive them again, and [`GITIGNORE`] is what keeps them out of the user's commits.

use std::collections::HashMap;
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

/// The file under the config root that names where a project's data actually went. Absent for a
/// Ubiq-managed project, which is every project that never asked for anything else.
pub const POINTER: &str = "storage.toml";

/// The format of both [`POINTER`] and [`METADATA`]. They move together: a reader that understands
/// one understands the other, and neither is worth its own number.
pub const PROJECT_DIR_VERSION: u32 = 1;

/// What `.ubiq/.gitignore` says.
///
/// Written once, when the folder is made, and never rewritten — a user who edits it has said
/// something, and re-imposing this on the next launch would take it back. The entries are
/// deliberately ahead of the tree: a directory Ubiq does not write yet costs a line here and a
/// commit of somebody's machine state if the line is missing when it arrives.
pub const GITIGNORE: &str = "\
# Ubiq keeps this project's own data here.
#
# Committed, and meant to be: project.toml, tasks.toml, tasks-archive/, kb.toml,
# plans/ and missions/ — the project's settings, tasks and configuration, shared
# with whoever clones it.
#
# Ignored below: per-machine and per-person state, and anything Ubiq derives
# again on its own. Losing it costs work, never data.

# The interface's view state and its workarea — one person's panels, not the project's.
view.toml
ui/

# Derived, and rebuilt by looking at the project again.
index/
cache/
searches/

# Fetched, and re-fetchable.
kb/

# What is running, or ran: live workspaces, finished transcripts, harness state.
runs/
sessions/

# Whatever a failed parse was preserved as, and whatever a write left behind.
*.bak.*
*.tmp
";

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

/// Make a project-managed project's `.ubiq/`, and leave behind everything that makes it one: the
/// ignore file, the metadata, and the pointer under the config root that lets a store find it.
///
/// Fails rather than falling back. A project-managed project whose folder could not be made is not
/// a Ubiq-managed project — it is a project the user did not get, and the caller refuses the add.
pub fn provision(root: &Path, record: &ProjectRecord) -> std::io::Result<PathBuf> {
    let dir = in_project_dir(&record.path);
    std::fs::create_dir_all(&dir)?;

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
}

impl ProjectDirs {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            known: RwLock::new(HashMap::new()),
        }
    }

    /// This project's data directory: `.ubiq/` if a pointer says so, the config root's own
    /// `projects/<ulid>/` otherwise.
    ///
    /// **Only a pointer that was found is remembered.** A project asked about before it was
    /// provisioned answers with the config root, and caching that would outlive the moment it was
    /// true; re-reading a file that is not there is one `stat` and costs nothing worth keeping.
    pub fn dir(&self, id: ProjectId) -> PathBuf {
        if let Some(dir) = self
            .known
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(&id)
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
        assert!(ignore.contains("view.toml"), "view state is per person");
        assert!(ignore.contains("index/"), "an index is derived");
        assert!(
            !ignore.contains("\ntasks.toml"),
            "tasks are the point of committing this folder: {ignore}"
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
