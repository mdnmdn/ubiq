//! What the window knows about an open web panel.
//!
//! A web panel edits one open file through a component somebody else owns, in a browser window
//! beside this one. **`Edit` is not a view of the file** — it is not a fourth position on the
//! Source/Preview/Split toggle, because with the external browser as the container there is no
//! fourth thing to draw in the panel. It is an action on the viewer's header and a session
//! recorded here, and the panel underneath keeps whichever of the three layouts it was in.
//!
//! Nothing here renders and nothing here names a path except the one the host told the window the
//! vendor bundle landed at — a value on a message, never composed.

use std::collections::HashMap;
use std::path::PathBuf;

use ubiq_proto::ids::ProjectId;

/// Where the vendor bundle for one web app stands, as far as this window knows.
///
/// The only state in which editing is offered is [`BundleState::Ready`]. Every other one either
/// waits or says why, and none of them touches the native read path: a first run with no network
/// loses editing and nothing else.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum BundleState {
    /// Never asked for. The first `Edit` press asks.
    #[default]
    Unknown,
    /// The host is fetching. `total` is zero only between the ask and the host's first report,
    /// which carries the count before anything is downloaded; `file` is the cache path of the
    /// last file that landed, empty until one has.
    Fetching { done: u32, total: u32, file: String },
    /// On disk at `path`, which the host named rather than the window composing it.
    Ready { path: PathBuf },
    /// Not there and not fetchable — no network on a first run, a hash that did not match, a
    /// failed download. A sentence to show beside a disabled control.
    Unavailable { reason: String },
}

impl BundleState {
    /// The directory the vendor route serves from, once there is one.
    pub fn path(&self) -> Option<&PathBuf> {
        match self {
            BundleState::Ready { path } => Some(path),
            _ => None,
        }
    }

    /// Why editing is unavailable right now, as a sentence to show, or `None` when it is not.
    ///
    /// [`BundleState::Unknown`] answers `None`: nothing has been asked for yet, so the control is
    /// live and pressing it is what asks.
    pub fn unavailable(&self) -> Option<String> {
        match self {
            BundleState::Ready { .. } | BundleState::Unknown => None,
            BundleState::Fetching { done, total, .. } if *total > 0 => Some(format!(
                "Downloading Excalidraw\u{2026} {done}/{total} files"
            )),
            BundleState::Fetching { .. } => Some("Downloading Excalidraw\u{2026}".to_string()),
            BundleState::Unavailable { reason } => Some(reason.clone()),
        }
    }

    /// The file the fetch last landed, as a line to show under the bar — the last path segment,
    /// which is what tells one of 554 downloads from another; the directory above it is the same
    /// CDN prefix on every row.
    ///
    /// `None` whenever there is nothing being fetched, and before the first file has landed.
    pub fn fetching_file(&self) -> Option<&str> {
        match self {
            BundleState::Fetching { file, .. } if !file.is_empty() => {
                Some(file.rsplit('/').next().unwrap_or(file))
            }
            _ => None,
        }
    }
}

/// One open editing session: a browser window over one tab's buffer.
#[derive(Debug, Clone)]
pub struct WebPanelSession {
    /// The tab whose buffer this session writes into. The session exists only as long as it does.
    pub key: String,
    /// The project that tab belongs to, so a window pointed elsewhere still finds the buffer.
    pub project: ProjectId,
    /// The bridge credential. Every frame repeats it; one that does not is dropped.
    pub token: String,
    /// The URL the browser was opened at, kept so the same session can be reopened.
    pub url: String,
    /// Whether the chrome has said `Ready`. The document is not sent before it has.
    pub ready: bool,
    /// The palette last sent, so a theme change is noticed exactly once.
    pub palette: String,
    /// What the chrome last said it could not do. Shown on the header; the buffer is untouched.
    pub error: Option<String>,
}

/// Every web panel this window has open, and the one bundle they all need.
#[derive(Default)]
pub struct WebPanels {
    pub bundle: BundleState,
    /// Live sessions, by tab key. At most one per tab.
    pub sessions: HashMap<String, WebPanelSession>,
    /// Tabs that pressed `Edit` while the bundle was still being fetched, opened when it lands.
    pub awaiting: Vec<String>,
    /// Documents the web side sent, waiting for a frame that has a `Window` to write them with —
    /// `EditorState::set_value` needs one and a bridge frame does not come with one.
    pub incoming: HashMap<String, String>,
    /// Tabs whose chrome asked for a save, run once the text it sent has reached the buffer.
    /// A save before that write would put yesterday's drawing on disk.
    pub saving: Vec<String>,
    /// Whether the drain loop is running. One per window, however many sessions there are.
    pub polling: bool,
}
