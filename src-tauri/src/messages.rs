use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub type SessionId = Uuid;
pub type WorkspaceId = Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionInfo {
    pub id: SessionId,
    pub name: String,
    pub home_folder: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkspaceInfo {
    pub id: WorkspaceId,
    pub session_id: SessionId,
    pub agent_type: String,
    pub folder: String,
    pub cols: u16,
    pub rows: u16,
    pub running: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentTypeInfo {
    pub name: String,
    pub command: String,
    pub description: String,
    pub default_args: Vec<String>,
    /// Filesystem root of this harness's configuration, when known.
    #[serde(default)]
    pub config_root: Option<String>,
}

/// The single message type for all UI ↔ Orchestrator communication.
/// Serialized as `{"type":"...","payload":{...}}` via serde's tagged representation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload")]
pub enum BusMessage {
    // ── UI → Orchestrator (Commands) ──────────────────────────────

    ListSessions,

    CreateSession {
        name: String,
        agent_type: String,
        #[serde(default)]
        home_folder: Option<String>,
    },

    ListAgentTypes,

    AttachToSession {
        session_id: SessionId,
    },

    DetachFromSession {
        session_id: SessionId,
    },

    SpawnWorkspace {
        session_id: SessionId,
        agent_type: String,
        #[serde(default)]
        folder: Option<String>,
    },

    TerminalInput {
        workspace_id: WorkspaceId,
        bytes: Vec<u8>,
    },

    TerminalResize {
        workspace_id: WorkspaceId,
        cols: u16,
        rows: u16,
    },

    // ── Orchestrator → UI (Events / Responses) ────────────────────

    SessionList {
        sessions: Vec<SessionInfo>,
    },

    SessionCreated {
        session: SessionInfo,
    },

    SessionAttached {
        session: SessionInfo,
        workspaces: Vec<WorkspaceInfo>,
    },

    AgentTypes {
        types: Vec<AgentTypeInfo>,
    },

    WorkspaceSpawned {
        workspace: WorkspaceInfo,
    },

    TerminalOutput {
        workspace_id: WorkspaceId,
        bytes: Vec<u8>,
    },

    WorkspaceExited {
        workspace_id: WorkspaceId,
        code: i32,
    },

    WorkspaceError {
        workspace_id: WorkspaceId,
        error: String,
    },

    Status {
        message: String,
    },

    Error {
        message: String,
    },
}

impl BusMessage {
    /// Serialize to JSON bytes (wire format).
    pub fn to_wire(&self) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec(self)
    }

    /// Deserialize from JSON bytes.
    pub fn from_wire(data: &[u8]) -> Result<Self, serde_json::Error> {
        serde_json::from_slice(data)
    }

    /// Returns true for high-frequency terminal I/O messages.
    pub fn is_stream(&self) -> bool {
        matches!(
            self,
            BusMessage::TerminalInput { .. } | BusMessage::TerminalOutput { .. }
        )
    }

    /// Return the variant name as a static string (for logging).
    pub fn variant_name(&self) -> &'static str {
        match self {
            BusMessage::ListSessions => "ListSessions",
            BusMessage::CreateSession { .. } => "CreateSession",
            BusMessage::ListAgentTypes => "ListAgentTypes",
            BusMessage::AttachToSession { .. } => "AttachToSession",
            BusMessage::DetachFromSession { .. } => "DetachFromSession",
            BusMessage::SpawnWorkspace { .. } => "SpawnWorkspace",
            BusMessage::TerminalInput { .. } => "TerminalInput",
            BusMessage::TerminalResize { .. } => "TerminalResize",
            BusMessage::SessionList { .. } => "SessionList",
            BusMessage::SessionCreated { .. } => "SessionCreated",
            BusMessage::SessionAttached { .. } => "SessionAttached",
            BusMessage::AgentTypes { .. } => "AgentTypes",
            BusMessage::WorkspaceSpawned { .. } => "WorkspaceSpawned",
            BusMessage::TerminalOutput { .. } => "TerminalOutput",
            BusMessage::WorkspaceExited { .. } => "WorkspaceExited",
            BusMessage::WorkspaceError { .. } => "WorkspaceError",
            BusMessage::Status { .. } => "Status",
            BusMessage::Error { .. } => "Error",
        }
    }
}
