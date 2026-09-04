//! What a project is on the wire: the durable record, the derived snapshot, and the small types
//! the project family carries.
//!
//! The split between the two is the point. **The record is what the store holds**; the snapshot is
//! the record plus what can only be known at the moment it is asked. Keeping them apart is what
//! stops a stale health flag or a pane count from being written down and believed at the next
//! boot.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::ids::ProjectId;

/// A project as it is written down. Everything here survives a restart.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectRecord {
    /// Stable across rename, recolour and a move on disk.
    pub id: ProjectId,
    /// Display name. Defaults to the folder's leaf; a rename never touches the filesystem.
    pub name: String,
    /// The canonical absolute path, as the **host** resolved it.
    pub path: String,
    /// Index into the theme's project swatches. The interface chooses it; the host only keeps it.
    pub colour: usize,
    /// A colour picked outside the swatches, packed as `0x00RRGGBB`. When set it wins over
    /// `colour`, which stays as the swatch the project would fall back to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_colour: Option<u32>,
    /// A folder opened by a drop rather than added to the catalogue. It is never written down, so
    /// it is gone at the next launch; naming it in project settings is what keeps it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub temporary: bool,
    /// When the project entered the catalogue.
    pub created_at: DateTime<Utc>,
    /// Stamped by the host when a window opens it. Absent until first opened.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_opened_at: Option<DateTime<Utc>>,
    /// Paths and globs this project's searches and its filename index skip, on top of the
    /// application-wide set in [`crate::settings::HostSettings`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub search_excludes: Vec<String>,
    /// Whether this project may be indexed locally. Off is the user saying "walk it, do not keep
    /// it" — the watcher still runs.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub no_local_index: bool,
}

/// What the host found when it last looked at the folder.
///
/// A record is never removed because its folder went away — an unplugged drive, a network mount
/// that has not come up and a worktree mid-rebase are all temporary, and a catalogue that forgets
/// on the user's behalf is one the user stops trusting.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", content = "reason")]
pub enum ProjectHealth {
    /// The folder is there and is a directory.
    Ok,
    /// Nothing is at the path.
    Missing,
    /// Something is there, but it is a file, or a symlink that leads nowhere.
    NotADirectory,
    /// It exists and could not be read, with the reason the operating system gave.
    Unreadable(String),
}

impl ProjectHealth {
    /// Whether the project can be worked in. Everything else is a state the picker marks.
    pub fn is_ok(&self) -> bool {
        matches!(self, ProjectHealth::Ok)
    }
}

/// A project as the interface is told about it: the record, plus what was true when it was asked.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectSnapshot {
    #[serde(flatten)]
    pub record: ProjectRecord,
    pub health: ProjectHealth,
    /// How many panes the host has running in this project. Only the half that owns the panes can
    /// know this, which is why it is not a field on the record.
    pub open_panes: usize,
    /// The directory this project's interface may keep its own files in — caches, and anything
    /// else that is the interface's business and not the project's.
    ///
    /// The host reserves the name and creates it; **it never reads inside.** What is in there is
    /// the interface's alone, and it is disposable: deleting it loses a cache and nothing else.
    /// Nothing the user would miss goes here — that is what the view blob and the preference blob
    /// are for, and those still cross the bus.
    ///
    /// It is **not the project's folder**: nothing the interface writes here lands in the user's
    /// repository, which is the whole reason it sits under Ubiq's own config root.
    ///
    /// An absolute path, told rather than composed. The interface never builds it out of
    /// `config_root` itself — using what it was handed is what makes a host on another machine a
    /// change of value rather than a change of code.
    pub workarea: String,
}

impl ProjectSnapshot {
    pub fn id(&self) -> ProjectId {
        self.record.id
    }
}

/// What a stored preference belongs to.
///
/// The palette and the window bounds belong to the interface; the expanded folders and open tabs
/// belong to a project.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Serialize, Deserialize)]
#[serde(tag = "scope", content = "project")]
pub enum Scope {
    Interface,
    Project(ProjectId),
}
