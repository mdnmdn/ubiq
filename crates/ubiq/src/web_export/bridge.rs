//! The typed bridge: the JSON frames a web panel exchanges with the interface.
//!
//! Two enums per tenant, internally tagged, JSON on the wire. The plumbing underneath
//! (`open_session`, `send`, `receive`) is generic over `serde_json::Value`, so a second tenant
//! declares its own pair here and nothing else changes.
//!
//! **Frames are documents, not commands.** Nothing in `FromWeb` may ask the interface to read a
//! file, list a directory or run anything — the web side receives content and returns content.
//! That is what keeps the bridge from becoming an RPC surface with the interface's authority
//! behind it.
//!
//! **Unknown variants are logged and dropped, never an error.** A frame from a newer chrome page
//! must not be able to break the panel, so both enums carry a catch-all and `decode_from_web`
//! answers `None` rather than a `Result`.

use serde::{Deserialize, Serialize};

/// The demo tenant's id, as it appears in `/_web/<app>/<token>/`.
pub const DEMO_APP: &str = "demo";

/// The Excalidraw tenant's id. It is also the `app` the host is asked for a bundle under
/// (`Message::EnsureWebBundle`), and the `<app>` segment of both the route and the cache path —
/// one name, so a session's origin and its vendor bytes cannot drift apart.
pub const EXCALIDRAW_APP: &str = "excalidraw";

/// Interface -> web. Serialised by [`to_value`] and queued with `web_export::send`.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToWeb {
    /// The document to mount, and the window's palette as the component's own theme.
    Open { document: String, palette: String },
    /// The document changed underneath the panel; remount it.
    Reload { document: String },
    /// The window's palette changed.
    Palette { palette: String },
    /// The user pressed save; the chrome answers with `Changed`.
    SaveRequested,
}

/// Web -> interface. Decoded with [`decode_from_web`].
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FromWeb {
    /// The chrome mounted and is ready for an `Open`.
    Ready,
    /// The document's current text. This is the only way content comes back.
    Changed { document: String },
    /// The component reports unsaved edits; the panel marks its tab dirty.
    Dirty,
    /// The user asked to save from inside the panel — `⌘S` never reaches GPUI while the webview
    /// has the keyboard, and the chrome's own Save button is beside the drawing rather than in
    /// the window's chrome. The document travels with it so the write is of what is on screen,
    /// not of whatever the 600 ms debounce last sent.
    Save { document: String },
    /// The chrome failed. The panel shows it; the buffer is untouched.
    Error { message: String },
    /// A variant this build does not know. Never constructed by a caller — `decode_from_web`
    /// turns it into a dropped frame.
    #[serde(other)]
    Unknown,
}

/// Serialise an outbound frame for `web_export::send`.
pub fn to_value(frame: &ToWeb) -> serde_json::Value {
    serde_json::to_value(frame).unwrap_or(serde_json::Value::Null)
}

/// Decode one inbound frame. A frame that does not parse, or that names a variant this build
/// does not know, is logged and dropped — the panel keeps working.
pub fn decode_from_web(frame: &serde_json::Value) -> Option<FromWeb> {
    match serde_json::from_value::<FromWeb>(frame.clone()) {
        Ok(FromWeb::Unknown) => {
            tracing::debug!("web bridge: dropping unknown frame variant: {frame}");
            None
        }
        Ok(known) => Some(known),
        Err(err) => {
            tracing::debug!("web bridge: dropping unparseable frame ({err}): {frame}");
            None
        }
    }
}
