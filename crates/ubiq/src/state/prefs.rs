//! What the interface remembers between runs, and the schema it owns.
//!
//! The host stores this as an opaque string it never parses, so **the interface owns the schema
//! and the interface versions it**. A blob that fails to parse, or that carries a schema this
//! build does not know, is discarded and the window opens on defaults — the host could not
//! validate it, and there is nothing here worth a migration.

use serde::{Serialize, de::DeserializeOwned};

use crate::state::RailMode;
use crate::theme::{AccentId, Density, ThemeId};

/// The shape this build writes and understands. Bump it and older blobs are discarded.
///
/// It moved to `2` when the files a project remembers became **tab keys** rather than paths, and
/// the dock's saved arrangement gained one panel per open file. Neither is a field a default could
/// rescue: a path read as a key opens the wrong tab, and an arrangement written before file panels
/// existed has a centre panel where the files belong. `LAYOUT_VERSION` follows this number, so the
/// blob and the arrangement inside it are discarded together rather than one half at a time.
///
/// It moved to `3` when the graph screen became `RailMode::Orchestration` and `Agents` became the
/// column screen. `rail_mode: "Agents"` is still a name this build reads, and it now names a
/// different screen — the one case a default cannot rescue, because nothing is missing: the value
/// changed meaning. An older blob would open the wrong mode with the wrong arrangement under it.
pub const SCHEMA: u32 = 3;

/// One rail mode's arrangement of one project's window: which edge regions were on screen, and the
/// dock blob that restores it.
///
/// The blob carries the whole arrangement — the tree, the axes, the sizes, which tab of each group
/// was displayed, and whether each region was open. The region flags are written beside it for a
/// `settle` that has the flags and cannot read the blob; the blob is what a restore uses.
#[derive(Clone, Debug, PartialEq, Serialize, serde::Deserialize)]
pub struct ModeLayout {
    pub show_left: bool,
    pub show_bottom: bool,
    pub show_right: bool,
    #[serde(default)]
    pub layout: Option<serde_json::Value>,
}

impl ModeLayout {
    /// The arrangement a mode opens on when it has never been arranged: the centre alone, in every
    /// mode. No region is furniture — a window that opens onto a tree, a chat and a pane region
    /// nobody asked for is three switches the user has to undo before the first frame is legible.
    /// Each region comes back the moment it is asked for, and is remembered from then on.
    pub fn default_for(_mode: RailMode) -> Self {
        Self {
            show_left: false,
            show_bottom: false,
            show_right: false,
            layout: None,
        }
    }
}

/// The last thing a chat tab was started on: the harness, the identity it ran as, and the two
/// answers the New agent form cannot recover from anywhere else.
///
/// Interface scope rather than a project's, because which harnesses this machine has and which
/// account is signed into them is a fact about the machine — a second project on the same laptop
/// should open offering what the first one used, not start from nothing again.
///
/// The model and the reasoning level are deliberately **not** here: the host already remembers
/// those per harness and identity, and hands them back with the catalogue. What it does not know
/// about is the permission mode a start asked for and the subagent ceiling it was given, so those
/// two are written here and read back as the form's preselection.
#[derive(Clone, Debug, Default, PartialEq, Serialize, serde::Deserialize)]
pub struct LastStart {
    pub agent_type: String,
    #[serde(default)]
    pub account: Option<String>,
    #[serde(default)]
    pub profile: Option<String>,
    /// What the last start was allowed to do without asking. `None` is "the harness's own", which
    /// is a real answer as well as what a blob written before this field carried.
    #[serde(default)]
    pub mode: Option<String>,
    /// The subagent ceiling the last start asked for. `None` says nothing about it at all.
    #[serde(default)]
    pub max_subagents: Option<u8>,
}

/// What belongs to the whole interface rather than to any one project.
#[derive(Clone, Debug, PartialEq, Serialize, serde::Deserialize)]
pub struct InterfacePrefs {
    pub schema: u32,
    pub theme: ThemeId,
    /// The accent the palette is dressed in. `None` — and a blob written before this field
    /// existed — is the palette's own seed, so no schema bump: `default` like every field added
    /// after the first release.
    #[serde(default)]
    pub accent: Option<AccentId>,
    /// How tight the grid is drawn. `Regular` — and a blob written before this field existed — is
    /// the size every constant declares, so no schema bump.
    #[serde(default)]
    pub density: Density,
    /// The base point size the chrome is drawn at — titlebar, status bar, rail, tabs, menus,
    /// modals, settings, pickers. `None` — and a blob written before this field existed — is
    /// [`crate::theme::CHROME_FONT_SIZE`], so no schema bump.
    ///
    /// Interface-scoped, not the project's: the chrome is the window's furniture, and growing it
    /// reflows the window rather than one project's reading.
    #[serde(default)]
    pub chrome_font_size: Option<f32>,
    /// The base point size a conversation is drawn at — the transcript, the tool blocks, the
    /// composer, the agents columns. `None` is [`crate::theme::CONVERSATION_FONT_SIZE`].
    ///
    /// Its own axis rather than the content family's: a transcript is read as prose, at a size
    /// that has nothing to do with the size code is read at. The content family is the third and
    /// stays per project — see [`ViewPrefs::content_font_size`].
    #[serde(default)]
    pub conversation_font_size: Option<f32>,
    /// What the last conversation was started on, so the next empty tab opens on it. `default`
    /// like every field added after the first release — see [`ViewPrefs`].
    #[serde(default)]
    pub last_start: Option<LastStart>,
    /// Every key in the blob this build does not know, kept as it was found and written back out.
    ///
    /// Serde drops what a struct does not name, so without this a blob carrying more than this
    /// build understands — one written by a newer build at the same schema, or by an edition
    /// keeping a namespaced key of its own — loses those keys the first time this build writes
    /// the blob back. The doc above promises the forward direction; this is the other one.
    #[serde(flatten, default)]
    pub rest: std::collections::BTreeMap<String, serde_json::Value>,
}

impl Default for InterfacePrefs {
    fn default() -> Self {
        Self {
            schema: SCHEMA,
            theme: ThemeId::DARK,
            accent: None,
            density: Density::Regular,
            chrome_font_size: None,
            conversation_font_size: None,
            last_start: None,
            rest: Default::default(),
        }
    }
}

/// An untitled tab's text, kept by name rather than by path: nothing on disk is what makes it
/// untitled in the first place, so the key `open_files` uses for every other tab names nothing
/// here to reopen.
#[derive(Clone, Debug, PartialEq, Serialize, serde::Deserialize)]
pub struct Scratch {
    pub name: String,
    pub text: String,
}

/// What belongs to one project: how its window was arranged, and what it was looking at.
///
/// Every field added after the first release is `#[serde(default)]`, so a blob written by an
/// older build at the same schema opens with the new fields empty rather than being discarded.
/// That is what keeps the schema still: it moves only when a field a build already writes changes
/// meaning, which a default cannot rescue.
#[derive(Clone, Debug, PartialEq, Serialize, serde::Deserialize)]
pub struct ViewPrefs {
    pub schema: u32,
    /// The rail mode the window was left in. The mode's own arrangement is one `modes` entry below.
    pub rail_mode: RailMode,
    /// The window's arrangement, remembered **per rail mode**. Each mode keeps its own picture of
    /// which regions were on screen and a dock blob for the whole tree, because the IDE's side
    /// panels are not the sink's firewalls and arriving in one mode must not undo the other.
    ///
    /// A mode with no entry has never been arranged; opening it falls to
    /// [`ModeLayout::default_for`]. Entries are written when the window leaves a mode (the blob is
    /// the whole arrangement, read off the dock) and read back into the window when the mode
    /// returns.
    ///
    /// The blob is stored by the host as an opaque value it never parses, like everything else
    /// here, so the schema stays the interface's own. It carries a version of its own inside, and
    /// one written for another is discarded for the default arrangement rather than half-applied.
    /// Terminal panels are in it and are dropped on load: layout persists, harnesses do not.
    #[serde(default)]
    pub modes: std::collections::HashMap<RailMode, ModeLayout>,
    /// The tabs open in the centre, in tab order, as `state/editor.rs`'s tab keys.
    ///
    /// A key rather than a path, because a file and its diff are two tabs on one path and a path
    /// names both. An unprefixed key *is* the path, which is what the file itself opens under.
    #[serde(default)]
    pub open_files: Vec<String>,
    /// Untitled buffers, kept by name and text rather than as `open_files` keys: an untitled tab
    /// names nothing on disk, so a real project file's unsaved edits still close for good — only
    /// what has no path to reread from is worth carrying past a close.
    #[serde(default)]
    pub scratch: Vec<Scratch>,
    /// The open files protected from close, as the same tab keys `open_files` uses. A tab key is
    /// the one tab identity a restart can act on — a pane dies with its process, so only a file's
    /// pin is worth writing down; see `OpenFile::pinned`.
    #[serde(default)]
    pub pinned_files: Vec<String>,
    /// The chat tabs that were open, in tab order, as the agent each was attached to. A tab
    /// attached to nothing is not written down: there is nothing to bring back, and a fresh tab
    /// is what an empty list already produces.
    ///
    /// **The tab's own id is not what is remembered.** A `ChatId` is a process-local counter and
    /// means nothing in the next run; the attachment is what does, now that a conversation can be
    /// marked persistent and the host keeps its run directory across a restart. A restore mints a
    /// fresh id per tab and asks the host to revive the agent named here.
    ///
    /// Stored as text rather than as an `AgentId`, the rule `bookmarks` follows: an id this build
    /// can no longer read costs one tab on restore rather than the whole blob.
    ///
    /// **No schema bump for it** — it is `#[serde(default)]` like every field added after the
    /// first release, and moving the schema would throw away every user's whole layout for the
    /// sake of a convenience.
    #[serde(default)]
    pub chats: Vec<String>,
    /// Which of `open_files` or `scratch` was in front. A key rather than an index, because a
    /// file that fails to open must not shift what "active" meant.
    #[serde(default)]
    pub active_file: Option<String>,
    /// The folders the explorer had open, so a tree comes back as it was left rather than shut.
    #[serde(default)]
    pub expanded: Vec<String>,
    /// The row the explorer had selected, open or not.
    #[serde(default)]
    pub selected: Option<String>,
    /// The text in the explorer's "Go to file…" field, kept per project so a switch back does not
    /// have to be re-typed. Absent means the field was empty.
    #[serde(default)]
    pub file_filter: String,
    /// The base point size the **content** family is drawn at — editors, the viewer, terminal
    /// panes, the explorer tree and search results together — so a zoom survives a restart.
    /// `None` is [`crate::theme::EDITOR_FONT_SIZE`].
    ///
    /// The one text family that is the project's rather than the interface's: a zoom travels with
    /// the project it was chosen for. Written as `ui_font_size` before the three families were
    /// named, which is the alias — a blob already on disk keeps its zoom, so no schema bump.
    #[serde(default, alias = "ui_font_size")]
    pub content_font_size: Option<f32>,
    /// Whether every file editor in this project soft-wraps long lines. `None` is the editor's own
    /// default.
    #[serde(default)]
    pub editor_wrap: Option<bool>,
    /// The rail modes this project hides. Empty is every mode on screen; the last enabled mode
    /// cannot be turned off, so the rail is never empty.
    #[serde(default)]
    pub hidden_modes: Vec<RailMode>,
    /// The places written down in this project. Each destination is stored as its `ubiq://` text,
    /// and one that no longer parses is dropped rather than costing the blob — see
    /// [`crate::state::nav::kept_bookmarks`].
    #[serde(default, deserialize_with = "crate::state::nav::kept_bookmarks")]
    pub bookmarks: Vec<crate::state::nav::Bookmark>,
    /// The places most recently arrived at, newest first, as `ubiq://` text. Only destinations
    /// that survive a restart are in it.
    #[serde(default)]
    pub recents: Vec<String>,
    /// Every key in the blob this build does not know, kept as it was found and written back out.
    ///
    /// Serde drops what a struct does not name, so without this a blob carrying more than this
    /// build understands — one written by a newer build at the same schema, or by an edition
    /// keeping a namespaced key of its own — loses those keys the first time this build writes
    /// the blob back. The doc above promises the forward direction; this is the other one.
    #[serde(flatten, default)]
    pub rest: std::collections::BTreeMap<String, serde_json::Value>,
}

impl Default for ViewPrefs {
    fn default() -> Self {
        Self {
            schema: SCHEMA,
            rail_mode: RailMode::Ide,
            modes: std::collections::HashMap::new(),
            open_files: Vec::new(),
            scratch: Vec::new(),
            pinned_files: Vec::new(),
            chats: Vec::new(),
            active_file: None,
            expanded: Vec::new(),
            selected: None,
            file_filter: String::new(),
            content_font_size: None,
            editor_wrap: None,
            hidden_modes: Vec::new(),
            bookmarks: Vec::new(),
            recents: Vec::new(),
            rest: Default::default(),
        }
    }
}

/// Read a blob back, or nothing at all.
///
/// The schema is probed before anything else is trusted, so a blob from a newer build is discarded
/// whole rather than half-applied.
pub fn decode<T: DeserializeOwned>(blob: &str) -> Option<T> {
    #[derive(serde::Deserialize)]
    struct JustTheSchema {
        schema: u32,
    }

    match serde_json::from_str::<JustTheSchema>(blob) {
        Ok(JustTheSchema { schema }) if schema == SCHEMA => {}
        Ok(JustTheSchema { schema }) => {
            tracing::debug!("discarding view state written for schema {schema}");
            return None;
        }
        Err(error) => {
            tracing::debug!("discarding unreadable view state: {error}");
            return None;
        }
    }

    serde_json::from_str(blob)
        .inspect_err(|error| tracing::debug!("discarding view state: {error}"))
        .ok()
}

/// Write one out. Infallible in practice; an unserialisable value becomes an empty blob, which
/// decodes to nothing and opens on defaults.
pub fn encode<T: Serialize>(value: &T) -> String {
    serde_json::to_string(value).unwrap_or_default()
}
