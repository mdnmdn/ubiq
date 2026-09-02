//! Application settings the interface owns: the schema, the overlay's nav, and how a blob is read.
//!
//! The host stores the Ui layer as an opaque string it never parses, so **the interface owns this
//! schema and versions it**. A blob that fails to parse, or that carries a schema this build does
//! not know, is discarded and the window opens on defaults.

use serde::{Deserialize, Serialize};

use crate::state::editor::ViewLayout;

/// The shape this build writes and understands. Bump it and older blobs are discarded.
pub const SCHEMA: u32 = 1;

/// The left nav of the application settings overlay, in the order it is drawn.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum SettingsSection {
    #[default]
    FileExplorer,
    Editor,
    Harnesses,
}

impl SettingsSection {
    pub fn all() -> &'static [SettingsSection] {
        &[
            SettingsSection::FileExplorer,
            SettingsSection::Editor,
            SettingsSection::Harnesses,
        ]
    }

    pub fn label(self) -> &'static str {
        match self {
            SettingsSection::FileExplorer => "File explorer",
            SettingsSection::Editor => "Editor",
            SettingsSection::Harnesses => "Harnesses",
        }
    }
}

/// How a newly opened markdown file is drawn. Split is a per-tab choice, not a default.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MarkdownOpen {
    #[default]
    Preview,
    Source,
}

impl MarkdownOpen {
    pub fn all() -> [MarkdownOpen; 2] {
        [MarkdownOpen::Preview, MarkdownOpen::Source]
    }

    pub fn label(self) -> &'static str {
        match self {
            MarkdownOpen::Preview => "Preview",
            MarkdownOpen::Source => "Source",
        }
    }

    pub fn layout(self) -> ViewLayout {
        match self {
            MarkdownOpen::Preview => ViewLayout::Preview,
            MarkdownOpen::Source => ViewLayout::Source,
        }
    }
}

/// What the interface remembers about how it behaves, as opposed to where it was left.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UiSettings {
    pub schema: u32,
    /// Single click and Enter open a temporary preview tab. Off, they open permanently.
    #[serde(default = "default_true")]
    pub explorer_preview: bool,
    /// The layout a new markdown tab opens in.
    #[serde(default)]
    pub markdown_open: MarkdownOpen,
}

fn default_true() -> bool {
    true
}

impl Default for UiSettings {
    fn default() -> Self {
        Self {
            schema: SCHEMA,
            explorer_preview: true,
            markdown_open: MarkdownOpen::Preview,
        }
    }
}

/// The settings overlay, and the values it is showing.
#[derive(Clone, Debug)]
pub struct SettingsState {
    pub open: bool,
    pub nav: SettingsSection,
    pub ui: UiSettings,
}

impl Default for SettingsState {
    fn default() -> Self {
        Self {
            open: false,
            nav: SettingsSection::FileExplorer,
            ui: UiSettings::default(),
        }
    }
}

/// Read a blob back, or nothing at all.
///
/// The schema is probed before anything else is trusted, so a blob from a newer build is discarded
/// whole rather than half-applied.
pub fn decode(blob: &str) -> Option<UiSettings> {
    #[derive(Deserialize)]
    struct JustTheSchema {
        schema: u32,
    }

    match serde_json::from_str::<JustTheSchema>(blob) {
        Ok(JustTheSchema { schema }) if schema == SCHEMA => {}
        Ok(JustTheSchema { schema }) => {
            tracing::debug!("discarding ui settings written for schema {schema}");
            return None;
        }
        Err(error) => {
            tracing::debug!("discarding unreadable ui settings: {error}");
            return None;
        }
    }

    serde_json::from_str(blob)
        .inspect_err(|error| tracing::debug!("discarding ui settings: {error}"))
        .ok()
}

/// Write one out. Infallible in practice; an unserialisable value becomes an empty blob, which
/// decodes to nothing and opens on defaults.
pub fn encode(value: &UiSettings) -> String {
    serde_json::to_string(value).unwrap_or_default()
}
