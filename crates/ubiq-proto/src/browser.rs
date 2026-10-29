//! The internal browser: a project's browsing history, and the commands an agent drives a
//! browser tab with.
//!
//! Two halves, both project-scoped. **History** is the host's: one list per project, most recent
//! first, capped at [`HISTORY_CAP`], persisted under the project's per-machine `local/` directory.
//! **Control** is the window's: the host never holds a web view, so an agent's `ubiq-browser` tool
//! call travels to the window as a [`BrowserCmd`] and comes back as a JSON string the tool hands
//! the model as it is.
//!
//! A tab is named by an opaque string the window hands out (`tab` below). The host never mints,
//! parses or remembers one; `None` means the window's active browser tab for the project.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// How many entries one project's history keeps. The oldest visit falls off past it.
pub const HISTORY_CAP: usize = 1000;

/// How long the host waits for a window to answer one [`BrowserCmd`] before the tool call fails
/// with a timeout the model can read.
pub const BROWSER_TIMEOUT_SECS: u64 = 30;

/// One page in a project's history: the last title it was seen with, how many times it was
/// visited, and when last. One row per URL — a revisit moves the row to the top.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowserHistoryEntry {
    pub url: String,
    pub title: String,
    pub visits: u32,
    pub last_visited: DateTime<Utc>,
}

/// What [`BrowserCmd::GetContent`] returns: the page's rendered text, or its serialised DOM.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserContentFormat {
    #[default]
    Text,
    Html,
}

/// One thing an agent asks a browser tab to do. `tab: None` is the active browser tab of the
/// project in the window that received the command.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BrowserCmd {
    /// Every browser tab of the project in this window.
    ListTabs,
    /// Load `url`: in a new tab when `new_tab` or when the project has no browser tab yet, else in
    /// the active one.
    Open {
        url: String,
        new_tab: bool,
    },
    Navigate {
        tab: Option<String>,
        url: String,
    },
    Back {
        tab: Option<String>,
    },
    Forward {
        tab: Option<String>,
    },
    Reload {
        tab: Option<String>,
    },
    Stop {
        tab: Option<String>,
    },
    Close {
        tab: Option<String>,
    },
    GetContent {
        tab: Option<String>,
        format: BrowserContentFormat,
    },
    /// Run `script` in the page and return its value, JSON-serialised.
    Eval {
        tab: Option<String>,
        script: String,
    },
    /// The tab's url, title and whether it is still loading.
    Info {
        tab: Option<String>,
    },
    /// The visible page as a PNG no side of which exceeds `max_dimension` pixels. The reply's JSON
    /// carries it base64-encoded under `png_base64`, beside its `width` and `height`.
    Screenshot {
        tab: Option<String>,
        max_dimension: u32,
    },
    /// Lay the page out `width` CSS pixels wide, centred in the tab; `None` fills the tab.
    SetViewport {
        tab: Option<String>,
        width: Option<u32>,
    },
}

/// `browser_screenshot`'s default bound on the image's longer side: what a vision model takes
/// without scaling it down itself.
pub const SCREENSHOT_MAX_DIMENSION: u32 = 1568;

/// The narrowest and widest page [`BrowserCmd::SetViewport`] lays out, in CSS pixels.
pub const VIEWPORT_MIN: u32 = 200;
pub const VIEWPORT_MAX: u32 = 4096;

impl BrowserCmd {
    /// The command's name, for a log line.
    pub fn name(&self) -> &'static str {
        match self {
            BrowserCmd::ListTabs => "list_tabs",
            BrowserCmd::Open { .. } => "open",
            BrowserCmd::Navigate { .. } => "navigate",
            BrowserCmd::Back { .. } => "back",
            BrowserCmd::Forward { .. } => "forward",
            BrowserCmd::Reload { .. } => "reload",
            BrowserCmd::Stop { .. } => "stop",
            BrowserCmd::Close { .. } => "close",
            BrowserCmd::GetContent { .. } => "get_content",
            BrowserCmd::Eval { .. } => "eval",
            BrowserCmd::Info { .. } => "info",
            BrowserCmd::Screenshot { .. } => "screenshot",
            BrowserCmd::SetViewport { .. } => "set_viewport",
        }
    }
}
