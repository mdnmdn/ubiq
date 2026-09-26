//! A project's task-source binding and its link table, as one TOML file.
//!
//! `<project data dir>/tasksrc.toml`, found through [`ProjectDirs`] exactly as `tasks.toml` and
//! `kb.toml` are, and already named in `project_dir::GITIGNORE` as a file that travels with the
//! project — it holds references and no material, so a teammate's clone gets a working binding and
//! no credential (`R1`).
//!
//! Two conventions meet here, and both matter:
//!
//! - **[`crate::kb::store`]'s versioned file**: `version` at the top, one atomic write, a missing
//!   file read as an empty default rather than as an error. A project with no binding is the
//!   ordinary case, not a failure.
//! - **`D179`'s unknown-field retention**: a key this build does not know is copied forward on
//!   save instead of being dropped. R1 files the whole layer here *because* `tasks.toml` has no
//!   such bag; a sidecar that repeated the mistake would have answered nothing. A Studio build, or
//!   a later slice, can add a field beside a binding and a plain base can load, edit and save the
//!   file without erasing it.
//!
//! The file is **binding-keyed from its first row** even though the setup surface allows one
//! binding per project (`R4`): the cost of a second board is then a setup-surface change rather
//! than a file migration.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use ubiq_proto::ids::{BindingId, ProjectId, TaskId};

use super::{Binding, TaskLink};
use crate::atomic::write_atomic;
use crate::store::project_dir::ProjectDirs;

/// The format this Ubiq writes and understands.
pub const TASKSRC_VERSION: u32 = 1;

/// The file's name under the project's data directory.
pub const TASKSRC_FILE: &str = "tasksrc.toml";

/// Every TOML key a [`Binding`] this build emits can carry. `D179`'s `known_fields`: anything else
/// found on a stored binding belongs to somebody else and is copied forward. Kept in step with the
/// struct by hand, the same obligation `store/file.rs` and `store/mission.rs` carry.
const BINDING_FIELDS: &[&str] = &[
    "id",
    "provider",
    "connection",
    "container",
    "lanes",
    "kinds",
    "priorities",
    "filter",
    "direction",
    "authority",
    "poll",
    "auto_import",
];

/// The same, for a link row. Keyed by `task` rather than `id`, because a link row *is* the pair
/// and has no identity apart from it.
const LINK_FIELDS: &[&str] = &[
    "binding",
    "task",
    "item",
    "container",
    "revision",
    "hash",
    // The task's own side of the per-field hash, and what differs right now (`D188`). Both are
    // this build's fields, so both belong here or the retention would resurrect a cleared one.
    "local",
    "drift",
    "synced_at",
    "state",
];

/// The whole file: the binding, and the link table beneath it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TasksrcFile {
    pub version: u32,
    /// One today. A list, so the second is a setup-surface change (`R4`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bindings: Vec<Binding>,
    /// One row per linked task. Pruned on load against nothing here — the worker prunes rows whose
    /// task no longer exists, because only it can see the task list (`R1`'s stated cost).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<TaskLink>,
}

impl TasksrcFile {
    /// An empty file at this build's version — what a project with no binding reads as.
    pub fn empty() -> Self {
        Self {
            version: TASKSRC_VERSION,
            bindings: Vec::new(),
            links: Vec::new(),
        }
    }

    /// The binding a link row belongs to, or the only one there is.
    pub fn binding(&self, id: BindingId) -> Option<&Binding> {
        self.bindings.iter().find(|held| held.id == id)
    }

    /// The project's binding, while there is one per project. `None` is a project nobody has
    /// bound, which is most of them.
    pub fn only_binding(&self) -> Option<&Binding> {
        self.bindings.first()
    }

    pub fn link(&self, task: TaskId) -> Option<&TaskLink> {
        self.links.iter().find(|row| row.task == task)
    }

    /// File a link row, replacing whatever was there for that task. One row per task: a task
    /// linked to two items in two bindings is not a thing the board can draw, and R4 is why.
    pub fn put_link(&mut self, row: TaskLink) {
        self.links.retain(|held| held.task != row.task);
        self.links.push(row);
    }
}

/// `<project data dir>/tasksrc.toml` — on the shared side, beside `tasks/`, wherever the project's
/// data went.
pub fn path(dirs: &ProjectDirs, project: ProjectId) -> PathBuf {
    dirs.data(project).tasksrc()
}

/// A project's binding and link table.
///
/// A missing file is a project nobody bound, which is an empty default rather than an error —
/// [`crate::kb::store::load`]'s own convention.
///
/// **A file from a newer Ubiq is left alone and read as empty.** Believing half of it would be
/// worse than not reading it, and returning empty means the next [`save`] is refused rather than
/// overwriting what this build cannot hold — see [`save`].
pub fn load(path: &Path) -> TasksrcFile {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return TasksrcFile::empty();
    };
    parse(&raw).unwrap_or_else(|| {
        tracing::warn!("ignoring an unreadable task source at {}", path.display());
        TasksrcFile::empty()
    })
}

/// The parse, split out so it can be exercised without a disk: what a file means is a pure
/// function of its bytes.
pub fn parse(raw: &str) -> Option<TasksrcFile> {
    let file: TasksrcFile = toml::from_str(raw).ok()?;
    if file.version > TASKSRC_VERSION {
        tracing::warn!(
            "a task source is version {}, and this Ubiq understands {TASKSRC_VERSION}",
            file.version
        );
        return None;
    }
    Some(file)
}

/// Write the whole file, atomically, keeping every key this build does not know.
///
/// The retention is the reason this is not two lines. The fresh document is serialised, the file
/// that is there is re-read, and any key on a matching row — matched by `id` for a binding and by
/// `task` for a link — that is **not** one of this build's own is copied onto the fresh row. A key
/// that *is* one of ours is never copied forward even when the fresh row omits it: an omitted
/// `Option` is a value this save cleared, not one it forgot, and resurrecting it would undo the
/// edit (`D179`, `store/mission.rs`'s own reasoning). Unknown keys at the top level are kept the
/// same way.
pub fn save(path: &Path, file: &TasksrcFile) -> std::io::Result<()> {
    let existing = std::fs::read_to_string(path).unwrap_or_default();
    let fresh = TasksrcFile {
        version: TASKSRC_VERSION,
        bindings: file.bindings.clone(),
        links: file.links.clone(),
    };
    let toml::Value::Table(mut document) = toml::Value::try_from(&fresh)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?
    else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "a task source serialises as a table",
        ));
    };

    if let Ok(old) = existing.parse::<toml::Table>() {
        merge_rows(&mut document, &old, "bindings", "id", BINDING_FIELDS);
        merge_rows(&mut document, &old, "links", "task", LINK_FIELDS);
        for (key, value) in &old {
            if matches!(key.as_str(), "version" | "bindings" | "links") {
                continue;
            }
            document.entry(key.clone()).or_insert_with(|| value.clone());
        }
    }

    let body = toml::to_string_pretty(&document)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    write_atomic(path, body.as_bytes())
}

/// Copy every unknown key of an old row onto the fresh row with the same key value.
///
/// `store/file.rs::merge_unknown_fields` does this for the catalogue keyed by `id`; a link row has
/// no id of its own, so the key column is a parameter here rather than assumed.
fn merge_rows(
    document: &mut toml::Table,
    old: &toml::Table,
    array_key: &str,
    key_field: &str,
    known_fields: &[&str],
) {
    let Some(toml::Value::Array(old_rows)) = old.get(array_key) else {
        return;
    };
    let Some(toml::Value::Array(fresh_rows)) = document.get_mut(array_key) else {
        return;
    };
    for row in fresh_rows {
        let toml::Value::Table(table) = row else {
            continue;
        };
        let Some(key) = table.get(key_field).cloned() else {
            continue;
        };
        let found = old_rows.iter().find_map(|value| match value {
            toml::Value::Table(old) if old.get(key_field) == Some(&key) => Some(old),
            _ => None,
        });
        let Some(found) = found else { continue };
        for (name, value) in found {
            if known_fields.contains(&name.as_str()) {
                continue;
            }
            table.entry(name.clone()).or_insert_with(|| value.clone());
        }
    }
}

/// The default lane map a freshly bound board gets, matched by lane **name**.
///
/// `R5` says match by id, and the stored map is id-keyed from the moment it is written. A
/// *default* has no ids to match against: a board nobody has bound yet is a list of names, and the
/// names everybody uses are the same three. So the seeding is by name, once, at bind time — and
/// from then on the map is ids and a rename at the remote costs nothing.
///
/// The other four Ubiq lanes start unbound deliberately. A board with no "In review" list should
/// not acquire one by guess, and an unmapped column simply never receives a pulled task.
pub fn default_lane_names() -> BTreeMap<&'static str, ubiq_proto::work::Status> {
    use ubiq_proto::work::Status;
    BTreeMap::from([
        ("to do", Status::Backlog),
        ("doing", Status::InProgress),
        ("done", Status::Done),
    ])
}

/// Seed a binding's lane map from the lanes a freshly bound container actually has.
///
/// Case-insensitive and whitespace-trimmed, because "To Do" and "to do" are the same list to
/// everybody but a string comparison. A lane whose name is not one of the three is left unmapped.
pub fn seed_lanes(binding: &mut Binding, lanes: &[ubiq_proto::tasksrc::RemoteLane]) {
    let defaults = default_lane_names();
    for lane in lanes {
        let name = lane.name.trim().to_ascii_lowercase();
        if let Some(status) = defaults.get(name.as_str()) {
            binding.lanes.insert(lane.id.to_string(), *status);
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use ubiq_proto::ids::ConnectionId;
    use ubiq_proto::tasksrc::{
        RemoteContainerId, RemoteItemId, RemoteLane, RemoteLaneId, Revision,
    };
    use ubiq_proto::work::Status;

    use super::*;
    use crate::tasksrc::{Direction, LinkState};

    fn file() -> TasksrcFile {
        let binding = Binding::new(
            "trello",
            ConnectionId::generate(),
            RemoteContainerId::new("board1"),
        );
        let link = TaskLink {
            binding: binding.id,
            task: TaskId::generate(),
            item: RemoteItemId::new("card1"),
            container: RemoteContainerId::new("board1"),
            revision: Revision::new("2026-09-01T00:00:00.000Z"),
            hash: BTreeMap::from([("title".to_string(), "abc".to_string())]),
            local: BTreeMap::from([("title".to_string(), "abc".to_string())]),
            drift: Vec::new(),
            synced_at: Utc::now(),
            state: LinkState::Linked,
        };
        TasksrcFile {
            version: TASKSRC_VERSION,
            bindings: vec![binding],
            links: vec![link],
        }
    }

    #[test]
    fn a_project_nobody_bound_reads_as_empty_rather_than_as_an_error() {
        let dir = tempfile::tempdir().expect("a directory");
        let loaded = load(&dir.path().join(TASKSRC_FILE));
        assert_eq!(loaded, TasksrcFile::empty());
        assert!(loaded.only_binding().is_none());
    }

    #[test]
    fn a_binding_and_its_links_round_trip() {
        let dir = tempfile::tempdir().expect("a directory");
        let path = dir.path().join(TASKSRC_FILE);
        let mut original = file();
        original.bindings[0].direction = Direction::TwoWay;
        original.bindings[0]
            .lanes
            .insert("list-a".into(), Status::Done);
        save(&path, &original).expect("it writes");

        let loaded = load(&path);
        assert_eq!(loaded, original);
        assert_eq!(loaded.only_binding().expect("a binding").provider, "trello");
        assert!(loaded.link(original.links[0].task).is_some());
    }

    /// `D179`. The whole reason the layer is filed in a sidecar is that `tasks.toml` drops what it
    /// does not know; a sidecar that did the same would have answered nothing.
    #[test]
    fn a_key_this_build_does_not_know_survives_a_load_edit_and_save() {
        let dir = tempfile::tempdir().expect("a directory");
        let path = dir.path().join(TASKSRC_FILE);
        let original = file();
        save(&path, &original).expect("it writes");

        // A later build — or a Studio one — writes a field beside the binding, beside a link row,
        // and at the top level.
        let raw = std::fs::read_to_string(&path).expect("it reads back");
        let raw = raw
            .replace("[[bindings]]", "[[bindings]]\nstudio_squad = \"green\"")
            .replace("[[links]]", "[[links]]\nstudio_seen_at = \"yesterday\"");
        std::fs::write(&path, format!("studio_epoch = 7\n{raw}")).expect("it writes");

        // This build loads it, changes something of its own, and writes it back.
        let mut loaded = load(&path);
        loaded.bindings[0].poll = 900;
        save(&path, &loaded).expect("it writes again");

        let after = std::fs::read_to_string(&path).expect("it reads back");
        assert!(after.contains("studio_squad"), "{after}");
        assert!(after.contains("studio_seen_at"), "{after}");
        assert!(after.contains("studio_epoch"), "{after}");
        assert!(after.contains("poll = 900"), "{after}");
    }

    /// A field this build *does* know is never copied forward, because an omitted value is one
    /// this save cleared rather than one it forgot.
    #[test]
    fn clearing_a_known_field_is_not_undone_by_the_retention() {
        let dir = tempfile::tempdir().expect("a directory");
        let path = dir.path().join(TASKSRC_FILE);
        let mut original = file();
        original.bindings[0]
            .lanes
            .insert("list-a".into(), Status::Done);
        save(&path, &original).expect("it writes");

        let mut loaded = load(&path);
        loaded.bindings[0].lanes.clear();
        save(&path, &loaded).expect("it writes again");

        assert!(load(&path).bindings[0].lanes.is_empty());
    }

    #[test]
    fn a_file_from_a_newer_ubiq_is_read_as_empty_and_left_alone() {
        let raw = format!("version = {}\n", TASKSRC_VERSION + 1);
        assert!(parse(&raw).is_none());
    }

    #[test]
    fn the_default_trello_map_is_seeded_by_name_and_stored_by_id() {
        let mut binding = Binding::new(
            "trello",
            ConnectionId::generate(),
            RemoteContainerId::new("board1"),
        );
        let lanes = vec![
            RemoteLane {
                id: RemoteLaneId::new("l1"),
                name: "To Do".into(),
                position: Some(1),
            },
            RemoteLane {
                id: RemoteLaneId::new("l2"),
                name: "doing".into(),
                position: Some(2),
            },
            RemoteLane {
                id: RemoteLaneId::new("l3"),
                name: "Done".into(),
                position: Some(3),
            },
            RemoteLane {
                id: RemoteLaneId::new("l4"),
                name: "Icebox".into(),
                position: Some(4),
            },
        ];
        seed_lanes(&mut binding, &lanes);

        assert_eq!(
            binding.status_of(&RemoteLaneId::new("l1")),
            Some(Status::Backlog)
        );
        assert_eq!(
            binding.status_of(&RemoteLaneId::new("l2")),
            Some(Status::InProgress)
        );
        assert_eq!(
            binding.status_of(&RemoteLaneId::new("l3")),
            Some(Status::Done)
        );
        // A list nobody named stays unbound, and the other four Ubiq lanes with it.
        assert_eq!(binding.status_of(&RemoteLaneId::new("l4")), None);
        assert_eq!(binding.lanes.len(), 3);
    }
}
