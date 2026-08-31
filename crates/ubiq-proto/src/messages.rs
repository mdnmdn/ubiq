//! The transport contract: the only vocabulary the UI and the coordinator share.
//!
//! One tagged enum, serialisable by construction, so the same values that travel an in-process
//! channel today can be framed onto a socket tomorrow. Nothing here holds a handle, a descriptor
//! or a borrowed byte — see `_docs/tech/transport-contract.md`, which owns the message set.
//!
//! Terminal bytes are opaque on both sides: neither half parses them, and a chunk is whatever one
//! read returned.

use serde::{Deserialize, Serialize};

use crate::files::{DirListing, FileContents, FileError, FileVersion};
use crate::ids::{PaneId, ProjectId, SessionId};
use crate::projects::{ProjectSnapshot, Scope};

/// Everything either half may say. The variant name travels in `type`, the body in `payload`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload")]
pub enum Message {
    // ── Pane family: coordinator → UI ───────────────────────────────
    /// Raw pseudo-terminal output, chunked as it was read.
    TerminalOutput {
        pane_id: PaneId,
        bytes: Vec<u8>,
    },
    /// The harness ended. The pane stays, showing its last screen.
    PaneExited {
        pane_id: PaneId,
        code: i32,
    },
    /// Something went wrong for one pane, and only that pane.
    PaneError {
        pane_id: PaneId,
        error: String,
    },

    // ── Pane family: UI → coordinator ───────────────────────────────
    /// Raw keystrokes from the focused pane. Effects come back as [`Message::TerminalOutput`].
    TerminalInput {
        pane_id: PaneId,
        bytes: Vec<u8>,
    },
    /// New geometry in cells. The coordinator sets the pseudo-terminal size and the kernel
    /// signals the harness.
    TerminalResize {
        pane_id: PaneId,
        cols: u16,
        rows: u16,
    },
    /// Exactly one pane holds focus.
    Focus {
        pane_id: PaneId,
    },

    // ── Session family ──────────────────────────────────────────────
    /// Start a workspace, and with it the pane that shows it.
    ///
    /// The harness starts in the project's folder, or in `rel_path` below it. `agent_type` falls
    /// back to the session's default when absent. A spawn into a project whose folder is missing,
    /// is not a directory or cannot be read is refused with [`Message::ProjectError`] before a
    /// pseudo-terminal exists, rather than becoming a failed spawn.
    SpawnWorkspace {
        session_id: SessionId,
        /// Which project the pane runs in. Not optional: the project's folder is the only thing a
        /// pane's working directory can be, so the interface draws no pane without one.
        project_id: ProjectId,
        /// Where in the project to start. Absent is its root.
        rel_path: Option<String>,
        agent_type: Option<String>,
        args: Vec<String>,
    },
    /// The answer to [`Message::SpawnWorkspace`], carrying the pane the UI now draws.
    WorkspaceSpawned {
        workspace: WorkspaceInfo,
    },
    /// The user closed the pane: the harness is killed and reaped.
    CloseWorkspace {
        pane_id: PaneId,
    },

    /// What the host is, said once to each window as it attaches. The interface cannot read disk,
    /// so this is the only way it learns its own config root is not the usual one.
    HostInfo {
        config_root: String,
        is_default: bool,
    },

    // ── Project family: UI → host ───────────────────────────────────
    /// Every project in the catalogue, probed. Answered with [`Message::ProjectList`].
    ListProjects,
    /// Take a folder into the catalogue. A path that does not exist is refused; no folder is ever
    /// created. A folder already in the catalogue resolves to the project that is there.
    AddProject {
        path: String,
        name: Option<String>,
        colour: Option<usize>,
    },
    /// Drop the record and the project's own directory in Ubiq's config. Nothing inside the
    /// project's folder is touched — which is why the word in the interface is "Forget".
    ForgetProject {
        project_id: ProjectId,
    },
    /// Rename or recolour. Display only: it touches no filesystem and cannot fail.
    UpdateProject {
        project_id: ProjectId,
        name: Option<String>,
        colour: Option<usize>,
    },
    /// Re-point a record at a folder that moved, keeping its id, colour and history. Unlike
    /// [`Message::UpdateProject`] this changes truth, so it can answer [`Message::ProjectError`].
    LocateProject {
        project_id: ProjectId,
        path: String,
    },
    /// A window pointed at a project. The host decides what that means and stamps it.
    OpenedProject {
        project_id: ProjectId,
    },
    /// Probe the folder again — the Locate-and-refresh path for a project marked missing.
    RefreshProject {
        project_id: ProjectId,
    },
    /// Read back what the interface stored for a scope.
    GetPreferences {
        scope: Scope,
    },
    /// Store an opaque blob for a scope. Answers nothing: losing where a splitter sat is not an
    /// event, and the write is debounced.
    SetPreferences {
        scope: Scope,
        value: String,
    },

    // ── Project family: host → UI ───────────────────────────────────
    ProjectList {
        projects: Vec<ProjectSnapshot>,
    },
    ProjectAdded {
        project: ProjectSnapshot,
    },
    ProjectChanged {
        project: ProjectSnapshot,
    },
    ProjectForgotten {
        project_id: ProjectId,
    },
    /// Something went wrong for one project, or for the catalogue as a whole when the id is absent.
    ProjectError {
        project_id: Option<ProjectId>,
        error: String,
    },
    /// The blob stored for a scope. Absent means never set, which is not an empty blob.
    Preferences {
        scope: Scope,
        value: Option<String>,
    },

    // ── File family: UI → host ──────────────────────────────────────
    /// One level of a project's tree. `rel_path` is empty for the root; `depth` is how many levels
    /// below it to list, clamped by the host, and one is what an expand asks for.
    ProjectTree {
        project_id: ProjectId,
        rel_path: String,
        depth: u8,
    },
    /// Read a file. `max_bytes` narrows the host's own ceiling and never widens it.
    ReadProjectFile {
        project_id: ProjectId,
        rel_path: String,
        max_bytes: Option<u64>,
    },
    /// Save a file, whole.
    ///
    /// The interface sends the buffer it holds, never a patch, and names the version it read so a
    /// write cannot land on somebody else's change. `expected` absent means it is creating a file
    /// that must not already exist. No folder is ever created — the mirror of
    /// [`Message::AddProject`] never creating one.
    WriteProjectFile {
        project_id: ProjectId,
        rel_path: String,
        bytes: Vec<u8>,
        expected: Option<FileVersion>,
    },

    // ── File family: host → UI ──────────────────────────────────────
    /// `rel_path` first, then every directory listed below it. Both paths are echoed, because an
    /// answer arrives after the click that asked for it and has to land on the right row.
    ProjectTreeListing {
        project_id: ProjectId,
        rel_path: String,
        listings: Vec<DirListing>,
    },
    ProjectFileContents {
        project_id: ProjectId,
        rel_path: String,
        contents: FileContents,
    },
    /// The file as it now is, so the tab's next save has a version to name and its dirty mark
    /// clears on a fact rather than on optimism.
    ProjectFileWritten {
        project_id: ProjectId,
        rel_path: String,
        version: FileVersion,
    },
    /// Something went wrong for one path in one project.
    ///
    /// Separate from [`Message::ProjectError`] on the reasoning that separates
    /// [`Message::PaneError`] from a catalogue-wide one: the interface can only mark the row or the
    /// tab the user is looking at if the message says which one.
    ProjectFileError {
        project_id: ProjectId,
        rel_path: String,
        error: FileError,
    },
}

/// One running workspace, as the UI is told about it. It carries no process, no writer and no
/// pseudo-terminal — a pane is an ID plus a byte stream.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorkspaceInfo {
    /// The workspace's ID, which is also its pane's.
    pub id: PaneId,
    pub session_id: SessionId,
    /// The resolved agent type — what the coordinator actually started.
    pub agent_type: String,
    /// Which project the pane runs in. It is what routes a spawn that lands after the window has
    /// switched projects, and it is why no absolute path is needed here: the interface already
    /// holds the project's name and path-free identity in its projection.
    pub project_id: ProjectId,
    /// Where in the project it started, absent for its root.
    pub rel_path: Option<String>,
    pub cols: u16,
    pub rows: u16,
    pub running: bool,
}
