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
    /// When the project entered the catalogue.
    pub created_at: DateTime<Utc>,
    /// Stamped by the host when a window opens it. Absent until first opened.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_opened_at: Option<DateTime<Utc>>,
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
