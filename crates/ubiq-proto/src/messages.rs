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
use crate::ids::{PaneId, ProjectId, SessionId, StepId, TaskId};
use crate::projects::{ProjectSnapshot, Scope};
use crate::work::{AgentId, Priority, Shape, Status, TaskRecord, WorkAgent, WorkSession};

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

    // ── Work family: UI → host ──────────────────────────────────────
    // Every variant here is addressed by `project_id`, because the work belongs to a project: its
    // tasks are written down under that project's own directory, and its sessions and agents are
    // minted per project. A task id alone would not say which store to write.
    //
    // Nothing in this family is broadcast. A project is open in exactly one window at a time, so
    // the window that asked is the only one drawing that project's work — the file family's rule,
    // for the file family's reason.
    /// Every session, agent and task the host holds for one project. Answered with
    /// [`Message::WorkList`], which carries all three at once: two round trips would let the board
    /// draw a card naming a session it has not heard of.
    ListWork {
        project_id: ProjectId,
    },
    /// Name a task. It lands in the backlog, unprioritised, direct and with no steps, because that
    /// is everything known at the moment it is named.
    CreateTask {
        project_id: ProjectId,
        title: String,
        session: Option<SessionId>,
    },
    /// Change what a task *is*. Display only, like [`Message::UpdateProject`]: it touches nothing
    /// outside the record and cannot be refused for anything but a task that is not there.
    UpdateTask {
        project_id: ProjectId,
        task_id: TaskId,
        title: Option<String>,
        description: Option<String>,
        priority: Option<Priority>,
        shape: Option<Shape>,
    },
    /// Move a task to another column.
    ///
    /// Its own variant rather than a field on [`Message::UpdateTask`], because a column is a stage:
    /// moving a card changes where the work has got to and nothing else about it.
    MoveTask {
        project_id: ProjectId,
        task_id: TaskId,
        status: Status,
    },
    /// Hand a task to a session, or take it back. Absent is a task nobody has started.
    ///
    /// Its own variant because it names another entity, will be fallible the day sessions are real,
    /// and because `Option<Option<SessionId>>` inside an update is a wire type nobody should have
    /// to read.
    AssignTask {
        project_id: ProjectId,
        task_id: TaskId,
        session: Option<SessionId>,
    },
    /// Drop a task. Unlike [`Message::ForgetProject`] this really deletes: there is nothing left
    /// behind for it to point at.
    DeleteTask {
        project_id: ProjectId,
        task_id: TaskId,
    },
    AddStep {
        project_id: ProjectId,
        task_id: TaskId,
        title: String,
    },
    RenameStep {
        project_id: ProjectId,
        task_id: TaskId,
        step_id: StepId,
        title: String,
    },
    RemoveStep {
        project_id: ProjectId,
        task_id: TaskId,
        step_id: StepId,
    },
    /// Reorder one step. `to` is the place it should end up in, clamped by the host.
    MoveStep {
        project_id: ProjectId,
        task_id: TaskId,
        step_id: StepId,
        to: usize,
    },
    /// Tick or untick a step.
    ///
    /// A toggle rather than a target state, because "unticking lands on idle, because nothing here
    /// can know what its owner would go back to doing" is a rule about the work and so is the
    /// host's to keep.
    ToggleStep {
        project_id: ProjectId,
        task_id: TaskId,
        step_id: StepId,
    },
    /// Move an agent's card into another task's outline, or out of every one.
    ///
    /// Where a card *sits* is the interface's own fact and never crosses; which task it *serves* is
    /// the host's, even while the agent is a mock.
    AssignAgent {
        project_id: ProjectId,
        agent_id: AgentId,
        task_id: Option<TaskId>,
    },
    /// Put a line in an agent's thread. Nothing answers it, and the reply is the agent record with
    /// one more turn on it — inventing a response is the one thing a screen with no live agent must
    /// not draw.
    SendToAgent {
        project_id: ProjectId,
        agent_id: AgentId,
        text: String,
    },

    // ── Work family: host → UI ──────────────────────────────────────
    /// One project's work, whole. The graph needs all three lists in the same frame.
    WorkList {
        project_id: ProjectId,
        sessions: Vec<WorkSession>,
        agents: Vec<WorkAgent>,
        tasks: Vec<TaskRecord>,
    },
    /// The task that was just made, carrying the id the interface could not have known.
    TaskCreated {
        project_id: ProjectId,
        task: TaskRecord,
    },
    /// The task as it now is, whole rather than as a diff — [`Message::ProjectChanged`]'s
    /// discipline, which is what makes the interface's projection idempotent by replacing on id.
    TaskChanged {
        project_id: ProjectId,
        task: TaskRecord,
    },
    TaskDeleted {
        project_id: ProjectId,
        task_id: TaskId,
    },
    /// Boxed, and it is the one payload in the set that is: a [`WorkAgent`] is the widest record
    /// here by some way, and an unboxed one makes every message on the bus as wide as it —
    /// including the terminal chunks on the hot path. Nothing about the wire form changes.
    AgentChanged {
        project_id: ProjectId,
        agent: Box<WorkAgent>,
    },
    /// Something went wrong for one task, or for a project's work as a whole when the id is absent.
    ///
    /// The error is a sentence rather than an enum, unlike [`FileError`]: an enum earns its keep
    /// when each arm is a different thing for the interface to do, and every failure here — no such
    /// project, no such task, no such step, a store that will not write — comes down to saying so
    /// once, where the user is looking.
    WorkError {
        project_id: ProjectId,
        task_id: Option<TaskId>,
        error: String,
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
