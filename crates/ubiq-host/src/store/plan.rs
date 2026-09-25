//! An annotated document on disk: its markdown body, and the sidecar beside it.
//!
//! **Two placements, one shape.** A *plan* lives under the config root, at the path
//! [`FilePlanStore`] computes. A *file document* is the project's own markdown, annotated in place,
//! with its sidecar at `<file>.md.annotation.json` beside it ([`sidecar_beside`]) — inside the
//! user's repository, which is [`Placement`]'s reason for existing: a write there creates no
//! directory and carries the file's mode over. Everything below the placement — the sidecar's
//! shape, the version probe, the preserve-aside rule — is the same for both, which is the point:
//! the block matcher, the orphaning and the provenance have one format to read.
//!
//! `<config root>/projects/<ProjectId>/plans/<TaskId>.md`, beside `tasks.toml` and `kb.toml` in a
//! directory that already exists — `store/file.rs`'s own reasoning for keeping tasks out of the
//! catalogue applies twice over here: a plan is long, is rewritten in full on every save, and must
//! never be inside a file the board rewrites whole on every drag. Keeping plans out of the user's
//! repository is `D30`'s rule about workspace state.
//!
//! **A store with exactly one implementation is a concrete type instead** — `store/harness.rs`'s
//! own line — so this is a plain struct beside [`super::file`] rather than a trait with a memory
//! twin: nothing here needs a corrupt-load or failed-write fixture with no disk under it, unlike
//! the catalogue or a project's tasks.
//!
//! The body is plain markdown, not TOML or JSON, so there is no format to fail parsing — the only
//! way a plan file is unreadable is invalid UTF-8, and that is treated exactly as
//! [`super::file::FileTaskStore`] treats a `tasks.toml` that will not parse: moved aside with
//! [`preserve_aside`], never overwritten, never truncated.
//!
//! **The annotations are a sidecar, not a header.** `<TaskId>.annotations.json` sits beside the
//! `.md`, which stays plain markdown with no ids and no front matter in it: that is what makes the
//! export a copy rather than a render, lets an agent rewrite a plan with ordinary file tools, and
//! leaves a plan opened in any other editor undamaged (`_docs/wip/planning-system.md`, decision
//! 4). The price is that the block index — which [`ubiq_proto::ids::BlockId`] is which passage —
//! has to be written down somewhere, and the sidecar is that somewhere: it carries the blocks the
//! last save assigned as well as the annotations hanging off them, because the next save's
//! matching needs the previous version to match *against*.
//!
//! **The edit-provenance layer is in that same sidecar**, not a third file, for the reason the
//! paragraph above gives for the block index: it is the same fact. A save re-matches the blocks
//! and re-stamps the lines against the *same* previous body, so the index and the provenance are
//! two readings of one comparison — in separate files they could end up a save apart, which is
//! exactly the failure one file exists to make impossible. It costs three additive fields, each
//! `#[serde(default)]`, so [`ANNOTATIONS_VERSION`] stays where it is: a sidecar written before
//! this layer existed loads with revision `0` and no stamps, which is a legible state and not a
//! migration. See [`crate::plan::provenance`] for what those fields mean.
//!
//! JSON rather than TOML, unlike every other store here, because the card asks for JSON and
//! because the shape is a list of records with optional fields and nested threads — TOML's
//! array-of-tables form for that is noticeably worse to read by hand, which is the only reason the
//! other files are TOML.

use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use ubiq_proto::ids::{ProjectId, TaskId};
use ubiq_proto::plan::{Annotation, PlanRevision, SaveOrigin};

use super::StoreError;
use crate::atomic::{preserve_aside, write_atomic, write_atomic_with};
use crate::plan::blocks::IndexedBlock;
use crate::plan::provenance::{ProvenanceRun, RevisionEntry};

/// The annotation format this Ubiq writes and understands.
pub const ANNOTATIONS_VERSION: u32 = 1;

/// A plan's sidecar: its block index and its annotations, in one file.
///
/// One file rather than two because they are one fact — an annotation names a block id, and a
/// block id means nothing without the index that says what it points at. Written together, so the
/// two can never be a save apart.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PlanSidecar {
    /// `version` first, [`super::file`]'s own hook for a future migration.
    pub version: u32,
    /// The blocks of the plan as the last save indexed them, in document order. The input to the
    /// next save's matching.
    #[serde(default)]
    pub blocks: Vec<IndexedBlock>,
    #[serde(default)]
    pub annotations: Vec<Annotation>,
    /// How many times the body has been saved. `0` for a plan whose body has never been written,
    /// and for every sidecar written before this field existed — which is why a watermark of `0`
    /// has to mean "everything", and why [`Self::provenance`] being empty is a legible state
    /// rather than a broken one.
    #[serde(default)]
    pub revision: PlanRevision,
    /// Who last wrote each line of the body as it now stands, as runs. See
    /// [`crate::plan::provenance`].
    #[serde(default)]
    pub provenance: Vec<ProvenanceRun>,
    /// What each save changed, counted, oldest first and capped at
    /// [`crate::plan::provenance::HISTORY_LIMIT`].
    #[serde(default)]
    pub history: Vec<RevisionEntry>,
}

impl PlanSidecar {
    pub fn new(blocks: Vec<IndexedBlock>, annotations: Vec<Annotation>) -> Self {
        Self {
            version: ANNOTATIONS_VERSION,
            blocks,
            annotations,
            ..Self::default()
        }
    }

    /// The revision an agent last wrote this plan at, for `plan_changes` defaulting its watermark
    /// to "since I last wrote it". `None` for an agent that has never written this plan — the
    /// caller then has to decide what a watermark means, rather than being handed a zero that
    /// would silently report the whole document.
    pub fn last_written_by(&self, author: &str) -> Option<PlanRevision> {
        self.history
            .iter()
            .rev()
            .find(|entry| {
                entry.origin == SaveOrigin::Agent && entry.author.as_deref() == Some(author)
            })
            .map(|entry| entry.revision)
    }
}

/// The `version` at the top of a sidecar, before anything else about it is believed — the same
/// probe [`super::file`] runs over a TOML file, in JSON: a version above ours is not corruption, and
/// the file is left exactly as it is rather than overwritten with a format that would lose data.
fn version_of(raw: &str) -> Option<u32> {
    #[derive(Deserialize)]
    struct Probe {
        version: u32,
    }
    serde_json::from_str::<Probe>(raw).ok().map(|p| p.version)
}

/// Where a document's two files live, and therefore how they are written.
///
/// **The distinction is the write rule, not the folder.** Under the config root a write may create
/// the directories above it, because Ubiq owns them. Inside a *user's* project it may not create
/// anything and must carry the mode over — `crate::atomic::write_atomic_with`'s own reasoning, and
/// the reason a file document's sidecar cannot simply reuse the plan path.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Placement {
    /// Under `<config root>/projects/…` — a plan and its sidecar.
    ConfigRoot,
    /// Inside the project's own working tree — a file document and its sidecar.
    InsideProject,
}

/// A document's body, from a path. `None` for one nobody has written yet.
///
/// Invalid UTF-8 is the one way a plain-text body fails to read, and it is preserved aside rather
/// than clobbered — [`super::file::FileTaskStore`]'s own rule.
pub fn load_body(path: &Path) -> Result<Option<String>, StoreError> {
    match std::fs::read_to_string(path) {
        Ok(body) => Ok(Some(body)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) if error.kind() == std::io::ErrorKind::InvalidData => {
            let preserved_as = preserve_aside(path, Utc::now()).ok();
            Err(StoreError::Parse {
                path: path.to_path_buf(),
                preserved_as,
                message: error.to_string(),
            })
        }
        Err(source) => Err(StoreError::Io {
            path: path.to_path_buf(),
            source,
        }),
    }
}

/// Replace a document's body, whole and atomically.
pub fn save_body(path: &Path, body: &str, placement: Placement) -> Result<(), StoreError> {
    write_placed(path, body.as_bytes(), placement)
}

/// A document's sidecar, from the sidecar's own path. `None` for a document nobody has annotated
/// or saved yet, which is not an error.
pub fn load_sidecar(path: &Path) -> Result<Option<PlanSidecar>, StoreError> {
    let raw = match std::fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(StoreError::Io {
                path: path.to_path_buf(),
                source,
            });
        }
    };

    if let Some(version) = version_of(&raw)
        && version > ANNOTATIONS_VERSION
    {
        return Err(StoreError::UnknownVersion {
            path: path.to_path_buf(),
            found: version,
            supported: ANNOTATIONS_VERSION,
        });
    }

    match serde_json::from_str::<PlanSidecar>(&raw) {
        Ok(sidecar) => Ok(Some(sidecar)),
        Err(error) => {
            // Preserved, never truncated: a thread the user wrote is worth as much as a task, and
            // an annotation lost to a bad parse cannot be reconstructed from the markdown.
            let preserved_as = preserve_aside(path, Utc::now()).ok();
            Err(StoreError::Parse {
                path: path.to_path_buf(),
                preserved_as,
                message: error.to_string(),
            })
        }
    }
}

/// Replace a document's sidecar, whole and atomically.
pub fn save_sidecar(
    path: &Path,
    sidecar: &PlanSidecar,
    placement: Placement,
) -> Result<(), StoreError> {
    let body = serde_json::to_string_pretty(sidecar).map_err(|error| StoreError::Parse {
        path: path.to_path_buf(),
        preserved_as: None,
        message: error.to_string(),
    })?;
    write_placed(path, body.as_bytes(), placement)
}

fn write_placed(path: &Path, bytes: &[u8], placement: Placement) -> Result<(), StoreError> {
    let written = match placement {
        Placement::ConfigRoot => write_atomic(path, bytes),
        // No directory is brought into existence inside a user's project, and the mode the file
        // already had is carried over the rename that replaces it.
        Placement::InsideProject => {
            let mode = std::fs::metadata(path).ok().map(|stat| stat.permissions());
            write_atomic_with(path, bytes, mode)
        }
    };
    written.map_err(|source| StoreError::Io {
        path: path.to_path_buf(),
        source,
    })
}

/// The sidecar beside a document's body, in the project's own tree: the body's whole file name
/// with `.annotation.json` appended, so `notes.md` is annotated by `notes.md.annotation.json`.
///
/// **Appended, never substituted** — `Path::with_extension` would turn `notes.md` into
/// `notes.annotation.json` and collide with whatever `notes.annotation` a repository happens to
/// hold, and the sidecar has to name the file it belongs to exactly.
pub fn sidecar_beside(body: &Path) -> PathBuf {
    let mut name = body.as_os_str().to_os_string();
    name.push(".annotation.json");
    PathBuf::from(name)
}

/// One project's plans, one file per task.
pub struct FilePlanStore {
    root: PathBuf,
}

impl FilePlanStore {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// The config root itself — what a mission document resolves its own path against (M8):
    /// `crate::store::mission::MissionStore` computes the same root's `missions/<TaskId>/docs/`
    /// subtree, and a mission document is neither a plan nor a file document, so it has no other
    /// path of its own to read this from.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Under the project's own directory, the way [`super::file::FileTaskStore::path`] is — so
    /// Forget and the orphan collector already cover it, and a plan never outlives the project it
    /// belongs to.
    pub fn path(&self, project: ProjectId, task: TaskId) -> PathBuf {
        self.root
            .join("projects")
            .join(project.to_string())
            .join("plans")
            .join(format!("{task}.md"))
    }

    /// A task's plan, or `None` for a task that has never had one written — the same distinction
    /// [`super::file::FileTaskStore::load`] draws between never-written and empty, which is not
    /// relevant here (an empty plan and no plan read the same to a caller that wants a string) but
    /// costs nothing to keep, and matters to [`Self::delete`]'s caller, which must not report a
    /// deletion for a file that was never there.
    pub fn load(&self, project: ProjectId, task: TaskId) -> Result<Option<String>, StoreError> {
        load_body(&self.path(project, task))
    }

    /// Replace a task's plan, whole. Atomic, the way every write under the config root is: a crash
    /// mid-save leaves the previous body or the new one, never a mixture.
    pub fn save(&self, project: ProjectId, task: TaskId, body: &str) -> Result<(), StoreError> {
        save_body(&self.path(project, task), body, Placement::ConfigRoot)
    }

    /// The annotations sidecar, beside the body it annotates.
    pub fn annotations_path(&self, project: ProjectId, task: TaskId) -> PathBuf {
        self.path(project, task).with_extension("annotations.json")
    }

    /// A plan's block index and annotations, or `None` for a plan nobody has annotated — which is
    /// every plan until the first annotation or the first save, and is not an error.
    pub fn load_sidecar(
        &self,
        project: ProjectId,
        task: TaskId,
    ) -> Result<Option<PlanSidecar>, StoreError> {
        load_sidecar(&self.annotations_path(project, task))
    }

    /// Replace a plan's sidecar, whole and atomically — the body's own discipline, for the same
    /// reason: a crash mid-write leaves the previous annotations or the new ones, never a mixture.
    pub fn save_sidecar(
        &self,
        project: ProjectId,
        task: TaskId,
        sidecar: &PlanSidecar,
    ) -> Result<(), StoreError> {
        save_sidecar(
            &self.annotations_path(project, task),
            sidecar,
            Placement::ConfigRoot,
        )
    }

    /// Drop a task's plan — the body **and** its sidecar. Not an error when there was none — the
    /// caller asked for it to be gone, and it already was. The sidecar never outlives the document
    /// it annotates: an annotations file beside no markdown would anchor to blocks that no longer
    /// exist anywhere.
    pub fn delete(&self, project: ProjectId, task: TaskId) -> Result<(), StoreError> {
        remove(self.path(project, task))?;
        remove(self.annotations_path(project, task))
    }
}

fn remove(path: PathBuf) -> Result<(), StoreError> {
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(StoreError::Io { path, source }),
    }
}

/// Export a plan's already-loaded body to an arbitrary destination on disk — the one place this
/// store writes outside its own root, for [`ubiq_proto::messages::Message::ExportPlan`]'s explicit,
/// one-shot copy. Takes the destination rather than resolving one, because resolving and
/// containing a project-relative path is `crate::files::path`'s job and must not be duplicated
/// here.
pub fn export_to(dest: &Path, body: &str) -> Result<(), StoreError> {
    write_atomic(dest, body.as_bytes()).map_err(|source| StoreError::Io {
        path: dest.to_path_buf(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids() -> (ProjectId, TaskId) {
        (ProjectId::generate(), TaskId::generate())
    }

    #[test]
    fn a_plan_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let store = FilePlanStore::new(dir.path().to_path_buf());
        let (project, task) = ids();

        store
            .save(project, task, "# The plan\n\nDo the thing.")
            .unwrap();
        let body = store.load(project, task).unwrap();
        assert_eq!(body.as_deref(), Some("# The plan\n\nDo the thing."));

        store
            .save(project, task, "# Revised\n\nDo the other thing.")
            .unwrap();
        let body = store.load(project, task).unwrap();
        assert_eq!(body.as_deref(), Some("# Revised\n\nDo the other thing."));
    }

    #[test]
    fn an_absent_plan_loads_as_none() {
        let dir = tempfile::tempdir().unwrap();
        let store = FilePlanStore::new(dir.path().to_path_buf());
        let (project, task) = ids();

        assert_eq!(store.load(project, task).unwrap(), None);
    }

    #[test]
    fn a_corrupt_plan_is_preserved_aside_not_clobbered() {
        let dir = tempfile::tempdir().unwrap();
        let store = FilePlanStore::new(dir.path().to_path_buf());
        let (project, task) = ids();

        let path = store.path(project, task);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        // Invalid UTF-8: the one way a plain-text file fails to read as a plan.
        std::fs::write(&path, [0xff, 0xfe, 0xfd]).unwrap();

        let error = store.load(project, task).unwrap_err();
        let StoreError::Parse { preserved_as, .. } = error else {
            panic!("expected a Parse error, got {error:?}");
        };
        let preserved_as = preserved_as.expect("the corrupt file was moved aside");
        assert!(preserved_as.exists(), "the preserved copy exists");
        assert!(!path.exists(), "nothing was left at the original path");
        assert_eq!(
            std::fs::read(&preserved_as).unwrap(),
            vec![0xff, 0xfe, 0xfd],
            "the corrupt bytes were kept, not truncated",
        );
    }

    #[test]
    fn deleting_an_absent_plan_is_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let store = FilePlanStore::new(dir.path().to_path_buf());
        let (project, task) = ids();

        store.delete(project, task).unwrap();
    }

    #[test]
    fn deleting_a_plan_removes_it() {
        let dir = tempfile::tempdir().unwrap();
        let store = FilePlanStore::new(dir.path().to_path_buf());
        let (project, task) = ids();

        store.save(project, task, "body").unwrap();
        assert!(store.path(project, task).exists());
        store.delete(project, task).unwrap();
        assert!(!store.path(project, task).exists());
        assert_eq!(store.load(project, task).unwrap(), None);
    }
}
