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
pub const SCHEMA: u32 = 1;

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

/// What belongs to one project: where its furniture was left, and what it was looking at.
///
/// Every field added after the first release is `#[serde(default)]`, so a blob written by an
/// older build opens with the new fields empty rather than being discarded. That is why the
/// schema has not had to move.
#[derive(Clone, Debug, PartialEq, Serialize, serde::Deserialize)]
pub struct ViewPrefs {
    pub schema: u32,
    pub rail_mode: RailMode,
    pub show_left: bool,
    pub show_bottom: bool,
    pub show_right: bool,
    /// The three panels' widths and the dock's height, as they were last dragged.
    #[serde(default)]
    pub explorer_width: Option<f32>,
    #[serde(default)]
    pub chat_width: Option<f32>,
    #[serde(default)]
    pub dock_height: Option<f32>,
    /// The files open in the centre, in tab order. Project-relative, like every path here.
    #[serde(default)]
    pub open_files: Vec<String>,
    /// Which of `open_files` was in front. A path rather than an index, because a file that fails
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
            explorer_width: None,
            chat_width: None,
            dock_height: None,
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
