//! The transport contract: the only vocabulary the UI and the coordinator share.
//!
//! One tagged enum, serialisable by construction, so the same values that travel an in-process
//! channel today can be framed onto a socket tomorrow. Nothing here holds a handle, a descriptor
//! or a borrowed byte — see `_docs/tech/transport-contract.md`, which owns the message set.
//!
//! Terminal bytes are opaque on both sides: neither half parses them, and a chunk is whatever one
//! read returned.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Everything either half may say. The variant name travels in `type`, the body in `payload`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload")]
pub enum Message {
    // ── Pane family: coordinator → UI ───────────────────────────────
    /// Raw pseudo-terminal output, chunked as it was read.
    TerminalOutput { pane_id: Uuid, bytes: Vec<u8> },
    /// The harness ended. The pane stays, showing its last screen.
    PaneExited { pane_id: Uuid, code: i32 },
    /// Something went wrong for one pane, and only that pane.
    PaneError { pane_id: Uuid, error: String },

    // ── Pane family: UI → coordinator ───────────────────────────────
    /// Raw keystrokes from the focused pane. Effects come back as [`Message::TerminalOutput`].
    TerminalInput { pane_id: Uuid, bytes: Vec<u8> },
    /// New geometry in cells. The coordinator sets the pseudo-terminal size and the kernel
    /// signals the harness.
    TerminalResize { pane_id: Uuid, cols: u16, rows: u16 },
    /// Exactly one pane holds focus.
    Focus { pane_id: Uuid },

    // ── Session family ──────────────────────────────────────────────
    /// Start a workspace, and with it the pane that shows it. `agent_type` and `folder` fall back
    /// to the session's defaults when absent.
    SpawnWorkspace {
        session_id: Uuid,
        agent_type: Option<String>,
        args: Vec<String>,
        folder: Option<String>,
    },
    /// The answer to [`Message::SpawnWorkspace`], carrying the pane the UI now draws.
    WorkspaceSpawned { workspace: WorkspaceInfo },
    /// The user closed the pane: the harness is killed and reaped.
    CloseWorkspace { pane_id: Uuid },
}

/// One running workspace, as the UI is told about it. It carries no process, no writer and no
/// pseudo-terminal — a pane is an ID plus a byte stream.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorkspaceInfo {
    /// The workspace's ID, which is also its pane's.
    pub id: Uuid,
    pub session_id: Uuid,
    /// The resolved agent type — what the coordinator actually started.
    pub agent_type: String,
    pub folder: Option<String>,
    pub cols: u16,
    pub rows: u16,
    pub running: bool,
}
