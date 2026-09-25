//! What a project is on the wire: the durable record, the derived snapshot, and the small types
//! the project family carries.
//!
//! The split between the two is the point. **The record is what the store holds**; the snapshot is
//! the record plus what can only be known at the moment it is asked. Keeping them apart is what
//! stops a stale health flag or a pane count from being written down and believed at the next
//! boot.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::ids::{ProjectId, SshProfileId};
use crate::settings::DronePreset;
use crate::tools::ToolDef;
use crate::work::Status;

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
    /// This project's own word for [`crate::work::Level::Mission`], or `None` to follow the
    /// application-wide default in [`crate::settings::HostSettings::mission_term`].
    ///
    /// An override rather than a value, the same reason [`Self::index`] is: "follow the default"
    /// and "happens to equal the default today" are different answers, so changing the
    /// application setting must move every project that never said otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mission_term: Option<String>,
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
    /// How this project's task board draws each lane — which ones it draws at all, and which shut
    /// themselves when they hold nothing. Replaced whole through [`Message::UpdateProject`], the
    /// way `search_excludes` above is.
    ///
    /// Sparse: a lane the user never said anything about has no entry, and reads back as
    /// [`LanePref::plain`]. Every lane still *exists* — a hidden one takes tasks and counts them,
    /// it is only not drawn, because a status the board stops showing is not a status the work
    /// stopped having.
    ///
    /// [`Message::UpdateProject`]: crate::messages::Message::UpdateProject
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lanes: Vec<LanePref>,
    /// The drone this project's folder lives behind, or `None` for a project that runs where Ubiq
    /// does. See [`DroneOrigin`].
    ///
    /// Purely additive, so the catalogue version does not move: a file written before this field
    /// existed reads back with `runs_on: None`, which is the answer every such record already
    /// meant — a local project — rather than a guess.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runs_on: Option<DroneOrigin>,
    /// Overrides the rail badge's letters in place of the name's own first character. At most
    /// two characters; empty is "no override", which is why this skips serialisation like the
    /// rest of this record's sparse fields rather than needing its own `Option`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub initials: String,
    /// Where this project's own data is written. See [`StorageMode`].
    ///
    /// Purely additive, on `runs_on`'s rule: a catalogue written before this field existed reads
    /// back [`StorageMode::UbiqManaged`], which is what every such record already meant, so the
    /// catalogue version does not move.
    #[serde(default, skip_serializing_if = "StorageMode::is_default")]
    pub storage: StorageMode,
}

/// Where a project's own data — its tasks, its metadata, the configuration that belongs to the
/// project rather than to this machine — is written.
///
/// The default is the rule `D30` states: nothing Ubiq remembers goes inside a project's folder,
/// so everything hangs off the config root under `projects/<project ulid>/`. A **project-managed**
/// project is the documented exception (`D173`): its data lives in a `.ubiq/` folder inside the
/// project's own directory, so it can be committed and travels with a clone.
///
/// Chosen when the project is created. Moving an existing project between the two is a migration
/// nothing here performs.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StorageMode {
    /// Under Ubiq's config root. Nothing is written inside the project's folder.
    #[default]
    UbiqManaged,
    /// In a `.ubiq/` folder inside the project's own directory.
    ProjectManaged,
}

impl StorageMode {
    /// Whether this is the mode a record that says nothing means — the test
    /// `skip_serializing_if` needs, so a catalogue keeps naming only what is unusual.
    pub fn is_default(&self) -> bool {
        matches!(self, Self::UbiqManaged)
    }

    pub fn is_project_managed(self) -> bool {
        matches!(self, Self::ProjectManaged)
    }
}

/// What one project has said about one lane of its task board.
///
/// Two independent facets rather than one three-state mode: a lane can be hidden, can shut itself
/// when empty, or both, and hiding one is not a stronger form of collapsing it. Both default to
/// off, which is the board every project starts with.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LanePref {
    /// Which lane this is about.
    pub status: Status,
    /// The board does not draw this lane. Tasks can still be in it — a filter would be a different
    /// setting — and nothing can be dragged into it while it is hidden.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hidden: bool,
    /// The lane draws shut, as the strip a user gets by shutting one by hand, whenever it holds no
    /// task. It opens again the moment one lands in it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub collapse_when_empty: bool,
}

impl LanePref {
    /// The lane nobody has said anything about: drawn, and open whether or not it holds anything.
    pub fn plain(status: Status) -> Self {
        Self {
            status,
            hidden: false,
            collapse_when_empty: false,
        }
    }

    /// Whether this preference says anything at all, and so whether it is worth writing down.
    pub fn is_plain(&self) -> bool {
        !self.hidden && !self.collapse_when_empty
    }
}

impl ProjectRecord {
    /// What this project says about one lane, which is [`LanePref::plain`] for a lane it has never
    /// been asked about.
    pub fn lane(&self, status: Status) -> LanePref {
        self.lanes
            .iter()
            .copied()
            .find(|pref| pref.status == status)
            .unwrap_or_else(|| LanePref::plain(status))
    }
}

/// Where a project's folder actually is, when it is not on this machine.
///
/// Pre-authorised by name: `_docs/inbox/completed/project-handling-proposal.md` reserved "the field
/// a remote drone would need to say *where* the folder is" while ruling a per-project default
/// harness out, which is `agent-manager`'s and not this.
///
/// The record says *where*, never *how to get in*: [`profile`] is a reference into
/// [`crate::settings::HostSettings::ssh_profiles`], whose secret is the host's and reaches `ssh`
/// through the askpass helper, so a catalogue is still a file a user could read out loud.
///
/// [`profile`]: Self::profile
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DroneOrigin {
    /// The saved SSH profile that reaches the machine.
    pub profile: SshProfileId,
    /// The folder on *that* machine, as the drone is launched with `--root`. It is the far
    /// machine's path, never resolvable here, which is why it is a plain string.
    pub root: String,
    /// The drone's lifetime: whether it detaches, and for how long it waits with no client.
    pub preset: DronePreset,
    /// A linger this project overrides the preset's own with, in seconds, or `None` to take the
    /// preset's. An override rather than a value, for the reason
    /// [`ProjectRecord::index`] is one: a preset whose default moves must move every project that
    /// never said otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub linger_secs: Option<u64>,
}

/// What an update does to a project's drone origin.
///
/// Three states have to cross the wire — leave it alone, clear it, set it — and
/// `Option<Option<DroneOrigin>>` cannot carry them: serde reads an absent field and an explicit
/// `null` into the same outer `None`, which would make "bring this project home" indistinguishable
/// from "say nothing about it". So the outer `Option` means *was anything said*, and this says
/// what. The same shape as [`IndexChange`], for the same reason.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DroneChange {
    /// Drop the origin: the project runs where Ubiq does.
    Local,
    /// Pin this project to a drone.
    Set(DroneOrigin),
}

impl DroneChange {
    /// The origin this change leaves behind.
    pub fn resolve(self) -> Option<DroneOrigin> {
        match self {
            Self::Local => None,
            Self::Set(origin) => Some(origin),
        }
    }
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

/// What to do with [`ProjectRecord::mission_term`]. The same shape as [`IndexChange`], for the
/// same reason: three states have to cross the wire — leave it alone, clear it, set it — and
/// `Option<Option<String>>` cannot carry them, so the outer `Option` on
/// [`crate::messages::Message::UpdateProject`] means *was anything said*, and this says what.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MissionTermChange {
    /// Drop the override and follow the application-wide default.
    Inherit,
    /// Pin this project to a word of its own.
    Set(String),
}

impl MissionTermChange {
    /// The override this change leaves behind.
    pub fn resolve(self) -> Option<String> {
        match self {
            Self::Inherit => None,
            Self::Set(term) => Some(term),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn origin() -> DroneOrigin {
        DroneOrigin {
            profile: SshProfileId::generate(),
            root: "/srv/work".into(),
            preset: DronePreset::Session,
            linger_secs: Some(60),
        }
    }

    fn record() -> ProjectRecord {
        ProjectRecord {
            id: ProjectId::generate(),
            name: "demo".into(),
            path: "/tmp/demo".into(),
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
            storage: StorageMode::default(),
        }
    }

    /// A lane never configured is drawn and never shuts itself, and says so without an entry.
    #[test]
    fn an_unconfigured_lane_reads_back_plain() {
        let mut record = record();
        record.lanes = vec![LanePref {
            status: Status::Blocked,
            hidden: false,
            collapse_when_empty: true,
        }];
        assert_eq!(record.lane(Status::Ready), LanePref::plain(Status::Ready));
        assert!(record.lane(Status::Blocked).collapse_when_empty);

        let raw = serde_json::to_string(&record).expect("a record serialises");
        let back: ProjectRecord = serde_json::from_str(&raw).expect("and reads back");
        assert_eq!(back.lanes, record.lanes);
    }

    #[test]
    fn an_index_change_resolves_the_two_it_can_mean() {
        assert_eq!(IndexChange::Inherit.resolve(), None);
        assert_eq!(
            IndexChange::Set(IndexLevel::Full).resolve(),
            Some(IndexLevel::Full)
        );
    }

    /// The third state is the absent `Option<IndexChange>` itself, which is what makes this enum
    /// two variants rather than three.
    #[test]
    fn a_drone_change_resolves_the_three_states() {
        let said: Option<DroneChange> = None;
        assert!(said.is_none(), "saying nothing carries no change at all");
        assert_eq!(DroneChange::Local.resolve(), None);
        let origin = origin();
        assert_eq!(
            DroneChange::Set(origin.clone()).resolve(),
            Some(origin.clone())
        );
    }

    #[test]
    fn a_record_round_trips_its_drone_origin() {
        let origin = origin();
        let mut record = record();
        record.runs_on = Some(origin.clone());
        let raw = serde_json::to_string(&record).expect("a record serialises");
        let back: ProjectRecord = serde_json::from_str(&raw).expect("and reads back");
        assert_eq!(back.runs_on, Some(origin));
    }

    /// The same additive rule `runs_on` follows: a record that says nothing about storage writes
    /// nothing, and reads back as the Ubiq-managed project every older catalogue entry already is.
    #[test]
    fn a_storage_mode_round_trips_and_defaults_to_ubiq_managed() {
        let record = record();
        assert_eq!(record.storage, StorageMode::UbiqManaged);
        let raw = serde_json::to_string(&record).expect("a record serialises");
        assert!(
            !raw.contains("storage"),
            "the default is written nowhere: {raw}"
        );

        let mut record = record;
        record.storage = StorageMode::ProjectManaged;
        let raw = serde_json::to_string(&record).expect("a record serialises");
        let back: ProjectRecord = serde_json::from_str(&raw).expect("and reads back");
        assert_eq!(back.storage, StorageMode::ProjectManaged);
    }

    /// A catalogue written before `runs_on` existed: the field is simply absent, and it reads back
    /// as the local project it always was. This is why `CATALOGUE_VERSION` does not move.
    #[test]
    fn a_record_without_a_drone_origin_reads_back_local() {
        let record = record();
        let raw = serde_json::to_string(&record).expect("a record serialises");
        assert!(
            !raw.contains("runs_on"),
            "a local project writes no origin at all: {raw}"
        );
        let back: ProjectRecord = serde_json::from_str(&raw).expect("and reads back");
        assert_eq!(back.runs_on, None);
    }
}
