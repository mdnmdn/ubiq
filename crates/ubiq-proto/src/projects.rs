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
use crate::tools::ToolDef;

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
    /// How much of this project Ubiq keeps an index of, or `None` to follow the application-wide
    /// default in [`crate::settings::HostSettings::index_level`].
    ///
    /// An override rather than a value, because "follow the default" and "happens to equal the
    /// default today" are different answers: changing the application setting must move every
    /// project that never said otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index: Option<IndexLevel>,
    /// The repositories *inside* this project the user has taken on, project-relative and
    /// forward-slashed, as [`crate::git::GitNested::rel_path`] spells them.
    ///
    /// A managed repository is walked: its paths join the project's one status map, so it colours
    /// the explorer and its changes show on the Git screen. One that is not on this list is found
    /// and named — the project settings list every repository there is — and nothing more is read
    /// from it. The project's **own** repository is never a member and is always managed: it is
    /// the repository the project *is*, and an entry for it would be a setting with one value.
    ///
    /// Replaced whole through [`Message::UpdateProject`], the way `search_excludes` above is.
    ///
    /// [`Message::UpdateProject`]: crate::messages::Message::UpdateProject
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub managed_repos: Vec<String>,
    /// Runnable tools defined for this project, on top of the machine-wide set in
    /// [`crate::settings::HostSettings::tools`]. Replaced whole through
    /// [`Message::UpdateProject`], the way `search_excludes` above is.
    ///
    /// [`Message::UpdateProject`]: crate::messages::Message::UpdateProject
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<ToolDef>,
}

/// How much of a project Ubiq keeps an index of.
///
/// **Cumulative, not alternative**: `Full` is `Light` plus symbols. The full-text half carries
/// content search at both levels and for every kind of file, including the ones a grammar exists
/// for — a symbol index answers questions about *names* and never about content, so it can never
/// stand in for the other half.
///
/// The watcher runs at every level, `None` included: what a level decides is what is *kept*, not
/// what is noticed.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IndexLevel {
    /// Nothing is kept. Content search walks the project on every query, which is what it did
    /// before any index existed.
    None,
    /// A full-text index, so a content search reads only the files that could match.
    #[default]
    Light,
    /// The full-text index, plus a table of the definitions each file declares.
    Full,
}

impl IndexLevel {
    /// Whether this level keeps a full-text index.
    pub fn keeps_text(self) -> bool {
        matches!(self, Self::Light | Self::Full)
    }

    /// Whether this level keeps a symbol table.
    pub fn keeps_symbols(self) -> bool {
        matches!(self, Self::Full)
    }
}

/// What an update does to a project's indexing override.
///
/// Three states have to cross the wire — leave it alone, clear it, set it — and
/// `Option<Option<IndexLevel>>` cannot carry them: serde reads an absent field and an explicit
/// `null` into the same outer `None`, which would make "clear the override" indistinguishable
/// from "say nothing about it". So the outer `Option` means *was anything said*, and this says
/// what.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum IndexChange {
    /// Drop the override and follow the application-wide default.
    Inherit,
    /// Pin this project to a level of its own.
    Set(IndexLevel),
}

impl IndexChange {
    /// The override this change leaves behind.
    pub fn resolve(self) -> Option<IndexLevel> {
        match self {
            Self::Inherit => None,
            Self::Set(level) => Some(level),
        }
    }
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
    /// Whether forgetting this project also deletes its folder — a throwaway clone, in a tree Ubiq
    /// owns.
    ///
    /// Told rather than derived, for the same reason `workarea` is. The test is the host's: the
    /// record is `temporary` *and* its path resolves inside the ephemeral root. The interface
    /// cannot repeat it, because the root is a setting whose unset value is a default only the
    /// host knows, and a path prefix is not a thing the interface reasons about. It matters
    /// because this is the one close the user cannot undo, so it is the one close that asks first.
    #[serde(default)]
    pub ephemeral: bool,
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
