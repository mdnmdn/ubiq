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
    pub rail_mode: RailMode,
    /// Whether each of the dock's three edge regions was on screen. Written from the dock's own
    /// state; the arrangement in `layout` carries the same fact, and is what a restore reads.
    pub show_left: bool,
    pub show_bottom: bool,
    pub show_right: bool,
    /// The window's whole arrangement, as the dock serialises it: the tree, the axes, the sizes,
    /// and which tab of each group was displayed.
    ///
    /// **The host stores it as an opaque value it never parses**, like everything else here, so
    /// the schema stays the interface's own. It carries a version of its own inside, and one
    /// written for another is discarded for the default arrangement rather than half-applied.
    /// Terminal panels are in it and are dropped on load: layout persists, harnesses do not.
    #[serde(default)]
    pub layout: Option<serde_json::Value>,
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
}

impl Default for ViewPrefs {
    fn default() -> Self {
        Self {
            schema: SCHEMA,
            rail_mode: RailMode::Ide,
            show_left: true,
            show_bottom: true,
            show_right: true,
            layout: None,
            open_files: Vec::new(),
            active_file: None,
            expanded: Vec::new(),
            selected: None,
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
