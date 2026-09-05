//! The transport contract: the only vocabulary the UI and the coordinator share.
//!
//! One tagged enum, serialisable by construction, so the same values that travel an in-process
//! channel today can be framed onto a socket tomorrow. Nothing here holds a handle, a descriptor
//! or a borrowed byte — see `_docs/tech/transport-contract.md`, which owns the message set.
//!
//! Terminal bytes are opaque on both sides: neither half parses them, and a chunk is whatever one
//! read returned.

use serde::{Deserialize, Serialize};

use crate::conversation::{ConvUpdate, StopReason};
use crate::files::{DiffBase, DirListing, FileContents, FileDiff, FileError, FileVersion, PathOp};
use crate::git::{self, GitCommit, GitEntry, GitRef, GitRollup, RepoOverview};
use crate::ids::{PaneId, ProjectId, SearchId, SessionId, StepId, TaskId};
use crate::projects::{ProjectSnapshot, Scope};
use crate::search::{self, Batch, Query, Source};
use crate::settings::SettingsLayer;
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
    /// The harness ended. The UI closes the pane.
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

    /// Which shells this machine actually has. Answered with [`Message::ShellList`].
    ///
    /// The interface may not look for itself — a program on disk is exactly the kind of local fact
    /// it is not allowed to read — so the new-pane menu asks. It asks again each time it opens
    /// rather than caching the answer, so a shell installed after the window did is offered.
    ListShells,
    /// The shells the host found, in the order the menu offers them.
    ShellList {
        shells: Vec<ShellInfo>,
    },

    /// Which agent harnesses can be started here. Answered with [`Message::AgentTypes`].
    ///
    /// Asked for the same reason as [`Message::ListShells`]: whether a harness is installed is a
    /// local fact, and the interface reads none. The answer is the harness library's list, not a
    /// table Ubiq keeps, so a harness the library learns about is offered without a change here.
    ListAgentTypes,
    /// The agent types the host can start, in the order the menu offers them.
    AgentTypes {
        agent_types: Vec<AgentTypeInfo>,
    },

    // ── Account family: the identities a harness runs as ─────────────
    /// Which accounts exist, and which harnesses each can actually log in. Answered with
    /// [`Message::Accounts`].
    ListAccounts,
    /// The accounts the host holds. References only — an id and which harnesses it covers.
    /// No credential, no path, and nothing that could be pasted into a terminal.
    Accounts {
        accounts: Vec<AccountInfo>,
    },
    /// Start an interactive login for `account` into `agent_type`, in a pane of its own.
    ///
    /// A login is the harness's own flow, unmodified — Ubiq spawns what the library says to
    /// spawn and watches for the credential it says will appear. Answered with
    /// [`Message::HarnessLoginStarted`], or [`Message::HarnessLoginFailed`] when there was
    /// nothing to start. An account id that names no account yet is created by logging in,
    /// which is the only way an account comes into being.
    BeginHarnessLogin {
        agent_type: String,
        account: String,
        /// Run a plain shell under this login's policy instead of the harness itself, and
        /// capture nothing. A diagnostic: it is how a human checks what the login sandbox
        /// actually permits, which no test can answer, and it must never record an account.
        probe: bool,
    },
    /// The login is running in this pane. The pane carries bytes and takes keystrokes like
    /// any other, and it belongs to no project — closing it abandons the login.
    HarnessLoginStarted {
        pane_id: PaneId,
        agent_type: String,
        account: String,
        cols: u16,
        rows: u16,
    },
    /// The login pane has ended and its credential was captured. The account now exists and
    /// a run can name it; [`Message::Accounts`] follows with the new list.
    HarnessLoginCaptured {
        agent_type: String,
        account: String,
    },
    /// The login captured nothing, and why: it was abandoned, the harness exited without
    /// writing a credential, or it could not be started at all. Not an error in Ubiq — the
    /// ordinary outcome of a flow the user closed.
    HarnessLoginFailed {
        agent_type: String,
        account: String,
        error: String,
    },
    /// A URL the running login printed. The host scans the login pane's own output for
    /// one and forwards it so the interface can offer it as a button; the bytes still
    /// reach the pane unchanged, so the user reads the harness's real output either way.
    ///
    /// Sent zero or more times between [`Message::HarnessLoginStarted`] and the login's
    /// end, and never for an ordinary pane. A URL already sent for this login is not
    /// sent again.
    HarnessLoginLink {
        pane_id: PaneId,
        url: String,
    },
    /// Whether `account` has a stored, usable credential for `agent_type`. Answered with
    /// [`Message::HarnessLoginStatus`], always — a credential that is absent is an answer,
    /// not an error.
    CheckHarnessLogin {
        agent_type: String,
        account: String,
    },
    /// The answer to [`Message::CheckHarnessLogin`].
    HarnessLoginStatus {
        agent_type: String,
        account: String,
        status: LoginStatus,
    },
    /// Rename an account — the identity, and so every harness logged in there.
    /// An account is a home, and this renames the home: the harnesses inside it
    /// keep their logins and answer to the new name afterwards. Answered with
    /// [`Message::Accounts`], or [`Message::AccountError`] when the new name is
    /// taken, empty, or not a name a directory can carry.
    RenameAccount {
        account: String,
        new_account: String,
    },
    /// Delete an account and every harness login inside it. The credential is
    /// gone from disk afterwards, which is why the word in the interface is
    /// "Delete" and not "Forget" — unlike a project, there is nothing left
    /// behind to come back to. Answered with [`Message::Accounts`], or
    /// [`Message::AccountError`].
    DeleteAccount {
        account: String,
    },
    /// Sign one harness out of an account, leaving the identity and its other
    /// harnesses alone: only the credential files that harness itself named are
    /// removed. The account survives with a shorter `logged_in`, and an empty
    /// one is an account that is still a name but no longer a login. Answered
    /// with [`Message::Accounts`], or [`Message::AccountError`].
    DeleteHarnessLogin {
        agent_type: String,
        account: String,
    },
    /// A stored credential could not be checked, renamed or deleted. A human-readable
    /// sentence, never a credential and never a path.
    AccountError {
        error: String,
    },

    // ── Project family: UI → host ───────────────────────────────────
    /// Every project in the catalogue, probed. Answered with [`Message::ProjectList`].
    ListProjects,
    /// Take a folder into the catalogue. A path that does not exist is refused; no folder is ever
    /// created. A folder already in the catalogue resolves to the project that is there. A
    /// temporary folder is never written to the catalogue and is forgotten once it closes.
    AddProject {
        path: String,
        name: Option<String>,
        colour: Option<usize>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        custom_colour: Option<u32>,
        #[serde(default)]
        temporary: bool,
    },
    /// Drop the record and the project's own directory in Ubiq's config. Nothing inside the
    /// project's folder is touched — which is why the word in the interface is "Forget".
    ForgetProject {
        project_id: ProjectId,
    },
    /// Rename or recolour. Display only: it touches no filesystem and cannot fail. The two colour
    /// fields travel together: `custom_colour` is applied only when `colour` is `Some`, and a
    /// `None` `custom_colour` alongside a `Some` `colour` clears the custom colour back to the
    /// swatch. A name-only update (`colour: None`) leaves both untouched.
    UpdateProject {
        project_id: ProjectId,
        name: Option<String>,
        colour: Option<usize>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        custom_colour: Option<u32>,
        /// The project's own search excludes. Absent leaves them as they are; `Some` replaces the
        /// whole list.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        search_excludes: Option<Vec<String>>,
        /// Whether this project may be indexed locally. Absent leaves it as it is.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        no_local_index: Option<bool>,
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
    /// Read back the settings stored for a layer. Answered with [`Message::Settings`].
    GetSettings {
        layer: SettingsLayer,
    },
    /// Store a settings blob for a layer. The Ui layer is opaque and answers nothing. The Host
    /// layer is parsed; a blob the host cannot read answers [`Message::SettingsError`].
    SetSettings {
        layer: SettingsLayer,
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
    /// The blob stored for a settings layer. Absent means never set, which is not an empty blob.
    Settings {
        layer: SettingsLayer,
        value: Option<String>,
    },
    /// The host could not read or would not store a settings blob.
    SettingsError {
        layer: SettingsLayer,
        error: String,
    },

    // ── The command-line shortcut ───────────────────────────────────
    /// Ask after, write or delete the small `ubiq` script that puts the application on the
    /// shell's `PATH`. The host owns every path in this exchange and chooses the directory
    /// itself; the interface only says which of the three things to do. Answered with
    /// [`Message::CliShortcutState`].
    CliShortcut {
        action: CliShortcutAction,
    },

    // ── The command-line shortcut: host → UI ────────────────────────
    /// Where the shortcut is, where one would go, and what the candidate directories look like.
    /// Sent in answer to every [`Message::CliShortcut`], whichever action it carried, so one
    /// path in the interface draws the section.
    CliShortcutState {
        /// Where a shortcut this application wrote was found, if one was.
        installed: Option<String>,
        /// It was found, but it launches a different build than the one running.
        stale: bool,
        /// Where writing one would put it. Absent when no directory can be used at all.
        target: Option<String>,
        /// Every directory considered, in the order they were considered.
        candidates: Vec<CliDir>,
        /// Why the last write or delete did not happen. A sentence, never a stack trace.
        error: Option<String>,
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
    /// Compare a file against a version-control base. The host computes the hunks; the interface
    /// draws rows and holds no diff library.
    DiffProjectFile {
        project_id: ProjectId,
        rel_path: String,
        base: DiffBase,
    },
    /// Create, move, copy or remove one path.
    ///
    /// One message for the five, because the interface's need is the same every time — a path, a
    /// destination for the two that have one, and what to do — and five variants would be five
    /// coordinator arms answering the same worker.
    ///
    /// `to` is the destination and is present for [`PathOp::Move`] and [`PathOp::Copy`] only. It is
    /// *refused* where it does not belong rather than ignored, because a field the host silently
    /// drops is a wiring mistake the interface cannot see.
    ///
    /// **Every op refuses a destination that already exists.** The alternative reading is a forced
    /// overwrite, which is the same thing an absent `expected` on [`Message::WriteProjectFile`]
    /// refuses, and for the same reason: the contract does not hand one out for free.
    EditProjectPath {
        project_id: ProjectId,
        rel_path: String,
        to: Option<String>,
        op: PathOp,
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
    /// The file's change against the base it was asked for, hunk by hunk.
    ProjectFileDiffed {
        project_id: ProjectId,
        rel_path: String,
        diff: FileDiff,
    },
    /// The edit was made.
    ///
    /// The request is echoed whole. An answer arrives after the click that asked for it, and the
    /// interface has to know which gesture finished before it can finish it: a created file is
    /// opened, a moved one retargets its tab, a removed one closes it.
    ///
    /// A failed edit is [`Message::ProjectFileError`], on the same reasoning that gives every other
    /// per-path failure that variant — the interface can only mark the row the user is looking at if
    /// the message says which one.
    ProjectPathEdited {
        project_id: ProjectId,
        rel_path: String,
        to: Option<String>,
        op: PathOp,
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
    /// Something changed on disk in a project, said without being asked.
    ///
    /// Coarse by design: relative paths, never content — the same rule a search hit obeys. A
    /// reader that wants the new bytes asks the normal way, with `ProjectTree`, `ReadProjectFile`
    /// or the git family. Coalesced per path over the watcher's debounce window and bounded like a
    /// search batch; `truncated` means the burst was larger than the window could carry and the
    /// whole subtree should be re-listed rather than the named paths patched.
    ProjectFilesChanged {
        project_id: ProjectId,
        changed: Vec<String>,
        truncated: bool,
        /// Something in the repository's plumbing moved — `HEAD`, `MERGE_HEAD`, the index or a
        /// ref. Paths under `.git` never appear in `changed`, so this bool is the only way a
        /// window learns to ask for the git overview again.
        repository: bool,
    },

    // ── Git family: UI → host ───────────────────────────────────────
    // Every variant names a project, because the interface holds no repository identity of its
    // own — a repository is a fact about a project, discovered by the host. Nothing in this
    // family is broadcast: a project is open in exactly one window, so the window that asked is
    // the only one drawing it. Not a repository is [`Message::GitOverview`] with `overview`
    // absent, not an error.
    /// What the status bar reads. Cheap: refs and a handful of files in the git directory.
    ProjectGit {
        project_id: ProjectId,
    },
    /// Re-read the repository. `full` also walks the working tree for the explorer's badges.
    RefreshProjectGit {
        project_id: ProjectId,
        full: bool,
    },
    /// A page of history. **Cursor-paged, not offset-paged**: a page is a bounded walk from a
    /// starting commit and the next cursor is the commit after the last returned. An offset would
    /// re-walk from HEAD every page and be wrong the moment the tree moved underneath.
    ///
    /// Answered with [`Message::GitLogPage`], or [`Message::GitError`].
    ProjectGitLog {
        project_id: ProjectId,
        /// Where the walk starts. Absent is HEAD.
        cursor: Option<String>,
        /// How many commits, clamped to [`git::MAX_LOG_PAGE`].
        count: u32,
        /// The history of one path, project-relative. Absent is the whole repository.
        rel_path: Option<String>,
        first_parent: bool,
    },
    /// Branches, remote-tracking branches, tags and stashes. `with_tracking` adds ahead and behind
    /// per branch, which is one merge-base walk each — the branch picker asks without it.
    ///
    /// Answered with [`Message::GitRefs`], or [`Message::GitError`].
    ProjectGitRefs {
        project_id: ProjectId,
        with_tracking: bool,
    },

    // ── Git family: host → UI ───────────────────────────────────────
    /// `overview` absent means the project is not in a repository. That is an ordinary answer.
    GitOverview {
        project_id: ProjectId,
        overview: Option<RepoOverview>,
    },
    /// Paths that have something to say, plus a rollup for every ancestor directory of those
    /// paths. A row not in the map is clean. `generation` is how a stale walk is discarded.
    GitWorkingTree {
        project_id: ProjectId,
        generation: u64,
        entries: Vec<GitEntry>,
        rollups: Vec<GitRollup>,
        truncated: bool,
    },
    /// A repository that exists and could not be read.
    GitError {
        project_id: ProjectId,
        error: git::GitError,
    },
    /// One page of history. `cursor` echoes the request's own, so a reply that lands after a
    /// later request already advanced the cursor is told apart from a fresh one instead of
    /// guessed at from whether the interface already holds a cursor. `next_cursor` absent is the
    /// end.
    GitLogPage {
        project_id: ProjectId,
        /// The cursor [`Message::ProjectGitLog`] was asked with. `None` was a request for the
        /// first page.
        cursor: Option<String>,
        commits: Vec<GitCommit>,
        next_cursor: Option<String>,
    },
    /// Every ref the sidebar draws, in one reply — five sections would otherwise be five walks.
    GitRefs {
        project_id: ProjectId,
        refs: Vec<GitRef>,
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

    // ── Conversation family: UI → host ──────────────────────────────
    /// Start a live agent: compose the harness, drive it over structured I/O, and stream what it
    /// says back as [`Message::ConversationUpdate`].
    ///
    /// The sibling of [`Message::SpawnWorkspace`], and the other face of the same thing — a
    /// workspace is either a terminal or a conversation, never both, because a child's stdout is
    /// either a tty or a pipe. This one answers with an agent rather than a pane, which is why the
    /// two are separate messages rather than a flag: nothing about a conversation has a size.
    StartConversation {
        /// Minted by the window rather than the host — the [`SessionId`] precedent — so a
        /// conversation can be drawn, selected and given a loader before the harness that will
        /// answer it exists. The host adopts this id rather than minting its own.
        agent_id: AgentId,
        project_id: ProjectId,
        session_id: SessionId,
        /// Where in the project it runs, absent for its root.
        rel_path: Option<String>,
        /// The library's harness id, from [`AgentTypeInfo`].
        agent_type: String,
        /// Which identity to run as, from [`AccountInfo`]. Absent falls back to whatever the
        /// library resolves — the profile named `default`, or the user's own home.
        ///
        /// Chosen once, at the start, and never after: a turn already taken was taken as
        /// somebody, and a conversation that changed identity halfway would be two
        /// conversations wearing one transcript.
        account: Option<String>,
    },
    /// A turn. Nothing is appended by the sender: the line is drawn when it comes back as a
    /// [`ConvUpdate::UserChunk`], which is what the harness actually received.
    PromptAgent {
        agent_id: AgentId,
        text: String,
    },
    /// Interrupt the turn in flight. Every permission request still waiting is answered as
    /// cancelled.
    CancelTurn {
        agent_id: AgentId,
    },
    /// Answer a [`ConvUpdate::PermissionRequest`], naming one of the options it carried.
    AnswerPermission {
        agent_id: AgentId,
        request_id: String,
        option_id: String,
    },
    /// Change a model, a mode, a thinking level — whatever the harness advertised under that id in
    /// a [`ConvUpdate::ConfigOptions`]. One message for all of them, because upstream has one
    /// mechanism for all of them.
    SetAgentConfig {
        agent_id: AgentId,
        config_id: String,
        value: String,
    },
    /// Stop the agent and clean up after it.
    EndConversation {
        agent_id: AgentId,
    },
    /// Kill the harness without ending the conversation. The transcript stays, the run directory
    /// stays — seeded credentials included — and the same `agent_id` can be started again by
    /// [`Message::ResumeConversation`] or by the next [`Message::PromptAgent`], exactly as a
    /// conversation that has not launched yet is. [`Message::EndConversation`] is still what takes
    /// everything with it.
    UnloadConversation {
        agent_id: AgentId,
    },
    /// Start an unloaded conversation's harness again, under the same `agent_id`, with no prompt.
    /// The launch recipe is the `PendingConversation` the agent has carried since it was created,
    /// so this is the same launch a first [`Message::PromptAgent`] performs — only with no turn to
    /// forward afterwards. A conversation that is already live is left alone.
    ResumeConversation {
        agent_id: AgentId,
    },

    // ── Conversation family: host → UI ──────────────────────────────
    /// The agent exists. Its record joins the project's work, so the sidebar and the graph find it
    /// exactly as they find any other.
    ConversationStarted {
        project_id: ProjectId,
        agent: Box<WorkAgent>,
        /// The session the agent belongs to. It travels with the agent because a window's own
        /// session is not one the work invented, and the sidebar lists agents *under* a session —
        /// so an agent whose session nothing names is an agent nothing draws.
        session: WorkSession,
        /// Whether this harness takes anything after its first turn. A one-shot harness answers
        /// `false`, and a composer that offered to send into it would be offering nothing — so
        /// the capability travels with the agent rather than being discovered by a refusal.
        accepts_input: bool,
    },
    /// One thing the agent said.
    ///
    /// A delta, not a record: a token stream cannot re-send a whole conversation, and `seq` is
    /// what an interface checks to know it has missed nothing. Boxed for the reason
    /// [`Message::AgentChanged`] is — an enum is as wide as its widest variant, and the terminal
    /// chunks on the hot path share it.
    ConversationUpdate {
        agent_id: AgentId,
        /// Per agent, monotonic, starting at one. Order is promised per agent and not across
        /// them, on the same terms as a pane's output.
        seq: u64,
        update: Box<ConvUpdate>,
    },
    /// The harness is gone. The transcript stays; the agent stops accepting turns.
    ConversationEnded {
        agent_id: AgentId,
        stop_reason: StopReason,
    },
    /// The harness is gone and the conversation is not. It is back to the state a conversation has
    /// before its first turn: the pickers return, and the next prompt — or a resume — starts a new
    /// process.
    ConversationUnloaded {
        agent_id: AgentId,
    },
    /// The conversation could not be started, or its stream failed. A sentence, for the reason
    /// [`Message::WorkError`] carries one.
    ConversationError {
        agent_id: AgentId,
        error: String,
    },

    // ── Search family: UI → host ────────────────────────────────────
    /// Start a content search across a project. One live search per project; a new one supersedes
    /// the old, which is interrupted mid-file. The interface mints `search_id` and discards every
    /// reply naming a search it is not holding.
    SearchProject {
        project_id: ProjectId,
        search_id: SearchId,
        query: Query,
        scope: search::Scope,
        /// What the search looks at, beside the query. Empty is the whole project.
        filter: search::Filter,
    },
    /// Stop a running search. The flag is checked between files and between matched lines.
    CancelSearch {
        project_id: ProjectId,
        search_id: SearchId,
    },

    // ── Search family: host → UI ────────────────────────────────────
    /// A bounded batch of results. Flushed on whichever comes first: 64 files, 512 hits, or a
    /// short interval. Batches arrive in the file walk's order, not sorted.
    SearchMatches {
        project_id: ProjectId,
        search_id: SearchId,
        batch: Batch,
    },
    /// How many files the walker has seen, so an empty result can be trusted — a search that
    /// finds nothing and a search that has not started look identical without it.
    SearchProgress {
        project_id: ProjectId,
        search_id: SearchId,
        files_seen: usize,
    },
    /// The walk is done. `searched` names what was actually looked at — in v1, `File` only.
    /// `truncated` is true when any ceiling was hit.
    SearchFinished {
        project_id: ProjectId,
        search_id: SearchId,
        searched: Vec<Source>,
        truncated: bool,
    },
    /// The search itself failed — the root is gone, the query is bad, the walk refused.
    SearchError {
        project_id: ProjectId,
        search_id: SearchId,
        error: search::SearchError,
    },
}

impl Message {
    /// The project this message names, for whichever variant carries one. `None` for a pane-only,
    /// account-only or catalogue-wide message — [`Message::ProjectError`]'s own `project_id` is
    /// already an `Option`, for the catalogue-wide case, and is returned as it is.
    pub fn project_id(&self) -> Option<ProjectId> {
        match self {
            Message::SpawnWorkspace { project_id, .. }
            | Message::ForgetProject { project_id, .. }
            | Message::UpdateProject { project_id, .. }
            | Message::LocateProject { project_id, .. }
            | Message::OpenedProject { project_id, .. }
            | Message::RefreshProject { project_id, .. }
            | Message::ProjectForgotten { project_id, .. }
            | Message::ProjectTree { project_id, .. }
            | Message::ReadProjectFile { project_id, .. }
            | Message::WriteProjectFile { project_id, .. }
            | Message::DiffProjectFile { project_id, .. }
            | Message::EditProjectPath { project_id, .. }
            | Message::ProjectTreeListing { project_id, .. }
            | Message::ProjectFileContents { project_id, .. }
            | Message::ProjectFileWritten { project_id, .. }
            | Message::ProjectFileDiffed { project_id, .. }
            | Message::ProjectPathEdited { project_id, .. }
            | Message::ProjectFileError { project_id, .. }
            | Message::ProjectFilesChanged { project_id, .. }
            | Message::ProjectGit { project_id, .. }
            | Message::RefreshProjectGit { project_id, .. }
            | Message::ProjectGitLog { project_id, .. }
            | Message::ProjectGitRefs { project_id, .. }
            | Message::GitOverview { project_id, .. }
            | Message::GitWorkingTree { project_id, .. }
            | Message::GitError { project_id, .. }
            | Message::GitLogPage { project_id, .. }
            | Message::GitRefs { project_id, .. }
            | Message::ListWork { project_id, .. }
            | Message::CreateTask { project_id, .. }
            | Message::UpdateTask { project_id, .. }
            | Message::MoveTask { project_id, .. }
            | Message::AssignTask { project_id, .. }
            | Message::DeleteTask { project_id, .. }
            | Message::AddStep { project_id, .. }
            | Message::RenameStep { project_id, .. }
            | Message::RemoveStep { project_id, .. }
            | Message::MoveStep { project_id, .. }
            | Message::ToggleStep { project_id, .. }
            | Message::AssignAgent { project_id, .. }
            | Message::SendToAgent { project_id, .. }
            | Message::WorkList { project_id, .. }
            | Message::TaskCreated { project_id, .. }
            | Message::TaskChanged { project_id, .. }
            | Message::TaskDeleted { project_id, .. }
            | Message::AgentChanged { project_id, .. }
            | Message::WorkError { project_id, .. }
            | Message::StartConversation { project_id, .. }
            | Message::ConversationStarted { project_id, .. }
            | Message::SearchProject { project_id, .. }
            | Message::CancelSearch { project_id, .. }
            | Message::SearchMatches { project_id, .. }
            | Message::SearchProgress { project_id, .. }
            | Message::SearchFinished { project_id, .. }
            | Message::SearchError { project_id, .. } => Some(*project_id),
            Message::ProjectError { project_id, .. } => *project_id,
            _ => None,
        }
    }
}

/// One shell the host found on this machine, as the new-pane menu offers it.
///
/// `program` is what a spawn asks for, and it is an absolute path wherever the probe found one, so
/// a pane does not depend on the `PATH` Ubiq itself was launched with. The interface reads nothing
/// out of it: it shows `label` and hands `program` straight back on
/// [`Message::SpawnWorkspace`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShellInfo {
    /// What the row says — the shell's own name, `zsh` or `pwsh`.
    pub label: String,
    pub program: String,
    /// Whether this is the one a bare click on the new-pane control already starts.
    pub is_default: bool,
}

/// One agent type a workspace can be, as the UI is told about it.
///
/// Every field comes from the embedded harness library — `id` is the library's harness id, and it
/// is what a spawn asks for on [`Message::SpawnWorkspace`]. The interface shows `label` and hands
/// `id` back; it never names a binary, a config path or a launch flag, because how a harness is
/// started is not a fact it holds.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentTypeInfo {
    /// The library's harness id, e.g. `claude-code`.
    pub id: String,
    /// What the row says — the harness's display name.
    pub label: String,
    /// Whether the harness's own binary was found, so a row that cannot start says so before it is
    /// picked rather than failing as a spawn the user has to interpret.
    pub available: bool,
}

/// One account, as the UI is told about it.
///
/// An account is an identity a harness runs as, and what crosses the bus is only ever a
/// *reference* to one: its id, and which harnesses it can log in. The credential itself
/// never appears here, and neither does the path it lives at — the domain rule is that
/// accounts carry credential references, never credential material, and this type is where
/// that rule is enforced or lost.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountInfo {
    /// What the user named this identity, e.g. `work`.
    pub id: String,
    /// The harness ids this account has a captured login for. Derived rather than recorded:
    /// an account is a home, and a harness is logged in there when the files it names are
    /// present — so one account can serve several harnesses, and an empty list means the
    /// account is a reference to an environment variable rather than a captured session.
    pub logged_in: Vec<String>,
}

/// The three things the interface can ask about the `ubiq` command.
///
/// `Install` names no directory: which one is a fact about the machine's `PATH`, and the host is
/// the half that may look at it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CliShortcutAction {
    /// Look, change nothing.
    Query,
    /// Write the shortcut, replacing one this application wrote before.
    Install,
    /// Delete it. A file this application did not write is left alone.
    Remove,
}

/// One directory the shortcut could live in, as the host found it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CliDir {
    pub path: String,
    /// The directory is there. Absent directories are still offered — the first candidate is
    /// created on install.
    pub exists: bool,
    /// The shell would find a command here.
    pub on_path: bool,
    /// This is the one the shortcut is in, or would go into.
    pub chosen: bool,
}

/// What a stored credential says about its own validity, as the host computed it from
/// the credential's embedded expiry field. This is a claim the credential makes about
/// itself, not a round trip to the provider — a token the provider revoked early still
/// reads as `Valid` here.
///
/// No credential material, and no path: the expiry is a timestamp, which is why it is
/// the one thing about a credential that may cross this bus.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LoginStatus {
    /// An expiry was found and has not passed.
    Valid { expires_at_ms: i64 },
    /// An expiry was found and has passed. A re-authentication is what fixes it.
    Expired { expires_at_ms: i64 },
    /// A credential is stored but names no expiry, so nothing here can say whether it
    /// still works. An API-key credential looks like this and is usually fine.
    Unknown,
    /// Nothing is stored for this account and harness.
    Missing,
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
