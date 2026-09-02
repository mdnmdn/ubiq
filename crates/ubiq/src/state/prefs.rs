//! What the interface remembers between runs, and the schema it owns.
//!
//! The host stores this as an opaque string it never parses, so **the interface owns the schema
//! and the interface versions it**. A blob that fails to parse, or that carries a schema this
//! build does not know, is discarded and the window opens on defaults — the host could not
//! validate it, and there is nothing here worth a migration.

use serde::{Serialize, de::DeserializeOwned};

use crate::state::RailMode;
use crate::theme::ThemeId;

/// The shape this build writes and understands. Bump it and older blobs are discarded.
///
/// It moved to `2` when the files a project remembers became **tab keys** rather than paths, and
/// the dock's saved arrangement gained one panel per open file. Neither is a field a default could
/// rescue: a path read as a key opens the wrong tab, and an arrangement written before file panels
/// existed has a centre panel where the files belong. `LAYOUT_VERSION` follows this number, so the
/// blob and the arrangement inside it are discarded together rather than one half at a time.
pub const SCHEMA: u32 = 2;

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
    /// The arrangement a mode opens on when it has never been arranged. The side panels are IDE
    /// furniture, so every other mode starts with the window's centre alone; the bottom region is
    /// open in the IDE and available-but-closed everywhere else, the way the titlebar's switch
    /// claims.
    pub fn default_for(mode: RailMode) -> Self {
        if mode.is_ide() {
            Self {
                show_left: true,
                show_bottom: true,
                show_right: true,
                layout: None,
            }
        } else {
            Self {
                show_left: false,
                show_bottom: false,
                show_right: false,
                layout: None,
            }
        }
    }
}

/// What belongs to the whole interface rather than to any one project.
#[derive(Clone, Debug, PartialEq, Serialize, serde::Deserialize)]
pub struct InterfacePrefs {
    pub schema: u32,
    pub theme: ThemeId,
}

impl Default for InterfacePrefs {
    fn default() -> Self {
        Self {
            schema: SCHEMA,
            theme: ThemeId::Dark,
        }
    }
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
    /// Which of `open_files` was in front. A key rather than an index, because a file that fails
    /// to open must not shift what "active" meant.
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
    /// The point size this project's text is drawn at — editors, terminal panes and the explorer
    /// tree together — so a zoom survives a restart. `None` is the interface's default.
    #[serde(default)]
    pub ui_font_size: Option<f32>,
    /// Whether every file editor in this project soft-wraps long lines. `None` is the editor's own
    /// default.
    #[serde(default)]
    pub editor_wrap: Option<bool>,
}

impl Default for ViewPrefs {
    fn default() -> Self {
        Self {
            schema: SCHEMA,
            rail_mode: RailMode::Ide,
            modes: std::collections::HashMap::new(),
            open_files: Vec::new(),
            active_file: None,
            expanded: Vec::new(),
            selected: None,
            file_filter: String::new(),
            ui_font_size: None,
            editor_wrap: None,
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
