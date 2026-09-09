//! The transport contract: the only vocabulary the UI and the coordinator share.
//!
//! One tagged enum, serialisable by construction, so the same values that travel an in-process
//! channel today can be framed onto a socket tomorrow. Nothing here holds a handle, a descriptor
//! or a borrowed byte — see `_docs/tech/transport-contract.md`, which owns the message set.
//!
//! Terminal bytes are opaque on both sides: neither half parses them, and a chunk is whatever one
//! read returned.

use serde::{Deserialize, Serialize};

use crate::assist::{
    AiModelList, AiProviderDraft, AiProviderInfo, AssistLimits, AssistReason, SuggestSubject,
};
use crate::connectors::{AuthKind, CertInfo, ConnectError, ConnectStage, Connection, ProviderId};
use crate::conversation::{ConfigChoice, ConvUpdate, StopReason};
use crate::files::{
    DiffBase, DirListing, FileContents, FileDiff, FileError, FileVersion, HostDirEntry,
    HostPathError, PathOp,
};
use crate::git::{self, GitCommit, GitEntry, GitRef, GitRollup, RepoOverview};
use crate::ids::{
    AiProviderId, CloneId, ConnectId, ConnectionId, NotificationId, OauthAppId, PaneId, ProjectId,
    RepoQueryId, SearchId, SessionId, StepId, SuggestId, TaskId,
};
use crate::notifications::{
    Level, MuteFor, MuteScope, Notification, NotificationRequest, Notifications,
};
use crate::projects::{IndexChange, ProjectSnapshot, Scope};
use crate::repos::{CloneError, CloneRequest, CloneStage, RemoteRepo, RepoSource};
use crate::search::{self, Batch, Query, Source};
use crate::settings::SettingsLayer;
use crate::stats::HostStats;
use crate::work::{AgentId, Priority, Shape, Status, TaskRecord, WorkAgent, WorkSession};

/// Everything either half may say. The variant name travels in `type`, the body in `payload`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload")]
pub enum Message {
    // ── Pane family: coordinator → UI ───────────────────────────────
    /// Raw pseudo-terminal output, chunked as it was read.
    TerminalOutput {
        pane_id: PaneId,
        // A bare `Vec<u8>` encodes as one msgpack int per byte under rmp-serde, which would make
        // the terminal hot path slower than JSON; `serde_bytes` encodes it as a single `bin`
        // blob instead. Does not change the JSON form: serde_json renders a byte-marked slice
        // the same way it renders a plain one, a number array, so the bus tape and JSON tests
        // are unaffected.
        #[serde(with = "serde_bytes")]
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
        // See the comment on `TerminalOutput::bytes`: same reasoning, same fix.
        #[serde(with = "serde_bytes")]
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
    /// Machine facts ride along so a remote-hosts panel can name OS, triplet, resources and
    /// hostname without a second round trip; all are best-effort (`None` = unavailable).
    HostInfo {
        config_root: String,
        is_default: bool,
        #[serde(default)]
        hostname: Option<String>,
        #[serde(default)]
        os: Option<String>,
        #[serde(default)]
        arch: Option<String>,
        #[serde(default)]
        triplet: Option<String>,
        #[serde(default)]
        cpu_count: Option<u64>,
        #[serde(default)]
        mem_total_bytes: Option<u64>,
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

    /// What this Ubiq is doing right now, and what its agents have spent. Answered with
    /// [`Message::Stats`].
    ///
    /// Asked on a timer while the Control screen is open, and at no other time. It is the one
    /// thing the interface polls for, because it is the one thing with no event behind it:
    /// memory and uptime change when nothing happens, so there is nothing for the host to push.
    ListStats,
    /// One reading of the host, for the window that asked. Never broadcast — two windows watching
    /// the screen take their own samples.
    Stats {
        stats: HostStats,
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
    /// Try `command` as the way to start `agent_type` on this machine, without saving it.
    /// Answered with [`Message::AgentCommandChecked`].
    ///
    /// The one question the interface asks about a binary, and it asks it because the user is
    /// typing the answer: a command line that resolves to nothing is a spawn failure a pane
    /// would report minutes later. The host runs it, once, and says what happened.
    CheckAgentCommand {
        agent_type: String,
        command: String,
    },
    /// What that command did: whether it ran, and one line saying what was found or what failed.
    AgentCommandChecked {
        agent_type: String,
        ok: bool,
        detail: String,
    },
    /// Which models — and which reasoning levels for each — `agent_type` will answer for, asked
    /// before anything is started. Answered with [`Message::HarnessCatalogue`].
    ///
    /// The probe shells out and the answer is cached on the harness binary's own version string,
    /// so this is cheap after the first ask and slow exactly once.
    ListHarnessCatalogue {
        agent_type: String,
        /// Already the cache key's identity leg, though the probe is per harness today.
        account: Option<String>,
    },
    /// The catalogue that answers it, plus what this harness was last actually launched with —
    /// a preselection, never a promise: a model gone since simply preselects nothing.
    HarnessCatalogue {
        agent_type: String,
        account: Option<String>,
        models: Vec<CatalogueModel>,
        /// Empty means no model flag was ever passed — the same convention the host's own
        /// `chosen_model` uses.
        last_model: String,
        last_thinking: String,
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

    // ── Profile family: the saved setups a conversation starts from ──
    /// Which profiles exist. Answered with [`Message::Profiles`].
    ListProfiles,
    /// The profiles the host holds. A profile names an account, a model and a mode — every
    /// field a reference, the same rule as [`Message::Accounts`].
    Profiles {
        profiles: Vec<ProfileInfo>,
    },
    /// Write a profile, creating it when its id names none. Answered with
    /// [`Message::Profiles`], or [`Message::AccountError`] when the id is empty or not a
    /// name a directory can carry — profiles are stored beside accounts and fail the same
    /// way, which is why they share the error rather than minting a second one.
    ///
    /// There is deliberately no delete: a profile is a saved setup, and a stale one costs a
    /// row in a list.
    SaveProfile {
        profile: ProfileInfo,
    },

    // ── Connector family: the identities an external *service* runs as ──
    /// Which connections exist. Answered with [`Message::Connections`].
    ///
    /// The list also rides [`Message::Settings`], since the records live in the host settings
    /// blob; this exists for the refresh-after-a-flow case rather than as the primary path.
    ListConnections,
    /// The connections the host holds. References only — no token, and nothing that could be
    /// pasted into a terminal.
    Connections {
        connections: Vec<ConnectionInfo>,
        /// The providers this build ships a registered application for.
        ///
        /// Rides here rather than in a message of its own because it is read at the same moment
        /// the list is — the connect flow offers a "Default" only where the build can honour one,
        /// and whether it can is a compile-time fact of the *host* that the interface must not
        /// guess. An empty list means every browser flow needs a registration.
        #[serde(default)]
        bundled: Vec<ProviderId>,
    },
    /// Start authenticating against `provider`, in a flow of its own.
    ///
    /// `connect_id` is minted by the interface, so a stage arriving after the user has closed the
    /// modal is discarded by id. `instance` is required for a self-hosted provider and absent for
    /// a public cloud; `client_id` names an application registered on that instance, which is the
    /// normal case for a browser flow anywhere but a provider's own cloud.
    ///
    /// This mints a flow, not a connection: a connection exists only once a token is stored, so an
    /// abandoned flow leaves nothing behind. There is deliberately no `AddConnection`.
    BeginConnect {
        connect_id: ConnectId,
        provider: ProviderId,
        instance: Option<String>,
        label: String,
        auth: AuthKind,
        client_id: Option<String>,
        /// The registration to authenticate as, when the user picked one. Written onto the
        /// connection, so a later probe or refresh uses the same application.
        #[serde(default)]
        oauth_app: Option<OauthAppId>,
    },
    /// How far the flow named by `connect_id` has got. Any number of these, then exactly one of
    /// [`Message::ConnectCaptured`] or [`Message::ConnectFailed`].
    ConnectPending {
        connect_id: ConnectId,
        stage: ConnectStage,
    },
    /// The flow succeeded: the token is stored and the connection exists. [`Message::Connections`]
    /// follows with the new list.
    ConnectCaptured {
        connect_id: ConnectId,
        connection: ConnectionInfo,
    },
    /// The flow ended with nothing stored.
    ConnectFailed {
        connect_id: ConnectId,
        error: ConnectError,
    },
    /// Abandon a flow. Drops the loopback listener or the poll; no record, no token.
    CancelConnect {
        connect_id: ConnectId,
    },
    /// Answer a [`ConnectStage::NeedSecret`] — a pasted access token.
    ///
    /// One of the two variants that carry material, and it carries it in a [`Secret`], which is the
    /// rule rather than the exception: material crosses only in a `Secret`, and a `Secret` never
    /// prints itself. Every alternative is worse for a desktop application — a temporary file is a
    /// plaintext file, and an environment-variable reference asks the user to restart.
    SubmitConnectSecret {
        connect_id: ConnectId,
        secret: Secret,
    },
    /// Rename a connection. The label is the user's; the id is what everything else references, so
    /// nothing else changes. Answered with [`Message::Connections`] or [`Message::ConnectorError`].
    RenameConnection {
        connection: ConnectionId,
        label: String,
    },
    /// Delete a connection and its token, together.
    ///
    /// It does not touch the pinned certificate, which belongs to the instance and may be why
    /// another connection still works — [`Message::ForgetCertificate`] is that operation. There is
    /// no "sign out but keep the name": unlike a harness account, which is a home directory that
    /// survives its login, a connection with no token is nothing.
    DeleteConnection {
        connection: ConnectionId,
    },
    /// Ask what a connection's token says about itself.
    ///
    /// `probe: false` reads the stored expiry and calls nobody — the path that runs on every
    /// render. `probe: true` is the "Check" button: a live handshake, run as a flow of its own
    /// because a network call must never happen on the thread that carries keystrokes, and so the
    /// one place an existing connection's certificate can be confirmed or replaced.
    CheckConnection {
        connection: ConnectionId,
        probe: bool,
    },
    /// The answer to [`Message::CheckConnection`]. `Valid` means "not expired", not "will work":
    /// a revoked token still reads `Valid` until something uses it.
    ConnectionStatus {
        connection: ConnectionId,
        status: LoginStatus,
    },
    /// A flow stopped on a certificate that did not validate, and is waiting.
    ///
    /// The host never pins on its own, never pins a fingerprint the interface did not send back,
    /// and never continues an unconfirmed handshake to see if it works.
    ConfirmCertificate {
        connect_id: ConnectId,
        /// The server vouched for, and what the pin is keyed by.
        origin: String,
        cert: CertInfo,
    },
    /// Vouch for the certificate [`Message::ConfirmCertificate`] offered, and resume.
    ///
    /// `sha256` must be the fingerprint the host offered; anything else is a
    /// [`Message::ConnectorError`] and the flow stays stopped. That is what makes the confirmation
    /// meaningful rather than a formality the interface can click through on the user's behalf,
    /// and why the payload is a fingerprint rather than a boolean.
    ///
    /// The pin is keyed by `origin` and so is instance-wide: two connections to one server find
    /// the same row, and a pin outlives the flow that created it.
    TrustCertificate {
        connect_id: ConnectId,
        origin: String,
        sha256: String,
    },
    /// Drop a pinned certificate. After it, the next request to that origin validates normally.
    /// Answered with [`Message::Settings`] or [`Message::ConnectorError`].
    ForgetCertificate {
        origin: String,
    },
    /// Create or rewrite one named OAuth application registration.
    ///
    /// `id: None` creates and the host mints one; `Some` rewrites that registration in place. One
    /// variant for both because it is one form either way, and because a registration the user
    /// renamed is the registration their connections already reference.
    ///
    /// The registrations live in the host-owned part of the settings blob, so this is the only way
    /// to write one: what the interface sends for that part of a `SetSettings` is discarded. A
    /// blank name or a blank client id is a [`Message::ConnectorError`]; success is
    /// [`Message::Settings`], since the blob every window draws from has changed.
    SaveOauthApp {
        id: Option<OauthAppId>,
        provider: ProviderId,
        name: String,
        /// The instance it is registered on, as [`crate::connectors::origin`] normalises it.
        /// `None` is the provider's own cloud.
        origin: Option<String>,
        client_id: String,
    },
    /// Delete a registration, and the client secret filed under it.
    ///
    /// Connections made under it keep working: they hold their own client id, and the id they name
    /// simply matches nothing any more.
    DeleteOauthApp {
        id: OauthAppId,
    },
    /// Store the client *secret* of one registration.
    ///
    /// The second variant that carries material, under the same rule as
    /// [`Message::SubmitConnectSecret`]. The client *id* is public and rides the settings blob like
    /// any other setting, which is why only the secret has a variant of its own.
    SetAppSecret {
        app: OauthAppId,
        secret: Secret,
    },
    /// Forget a stored client secret. Its own variant rather than an empty [`Secret`], because
    /// clearing a credential should not look like setting one.
    ClearAppSecret {
        app: OauthAppId,
    },
    /// Something in the connector family went wrong, outside any one flow.
    ConnectorError {
        error: String,
    },

    // ── Repository family: UI → host ────────────────────────────────
    /// List the repositories a connection can see. `query` is the provider's own search text;
    /// `None` asks for what it offers unprompted, which is what an empty field shows. Answered
    /// with [`Message::Repos`] or [`Message::RepoError`].
    ListRepos {
        query_id: RepoQueryId,
        connection: ConnectionId,
        query: Option<String>,
    },
    /// List one repository's branches, so a clone can offer something other than the default.
    /// Answered with [`Message::RepoBranches`] or [`Message::RepoError`].
    ListRepoBranches {
        query_id: RepoQueryId,
        source: RepoSource,
    },
    /// Clone a repository and take the result into the catalogue.
    ///
    /// There is deliberately no clone-success message: a finished clone is a registered project,
    /// so [`Message::ProjectAdded`] is the success signal and the interface has one place to learn
    /// a project exists rather than two.
    CloneRepo {
        request: CloneRequest,
    },
    /// Stop a clone in flight. A partial destination is the host's to clean up; the interface
    /// hears nothing further about this id.
    CancelClone {
        clone_id: CloneId,
    },

    // ── Repository family: host → UI ────────────────────────────────
    /// The answer to [`Message::ListRepos`].
    Repos {
        query_id: RepoQueryId,
        repos: Vec<RemoteRepo>,
        /// The listing hit its page ceiling. It matters because it is what stops the interface
        /// saying "no such repository": an empty local filter over a truncated list is not proof.
        truncated: bool,
    },
    /// The answer to [`Message::ListRepoBranches`]. `default` is the branch a clone takes when the
    /// user picks none.
    RepoBranches {
        query_id: RepoQueryId,
        branches: Vec<String>,
        default: Option<String>,
    },
    /// A listing failed. It carries [`CloneError`] rather than a kind of its own, because the
    /// reasons are the same ones and the sentence the interface writes is the same sentence.
    RepoError {
        query_id: RepoQueryId,
        error: CloneError,
    },
    /// How far a clone has got. One per change of stage.
    ClonePending {
        clone_id: CloneId,
        stage: CloneStage,
    },
    /// The clone ended without a project. Nothing was registered.
    CloneFailed {
        clone_id: CloneId,
        error: CloneError,
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
        /// What to do with the project's indexing override. Absent leaves it as it is; see
        /// [`IndexChange`] for why this is not an `Option<Option<_>>`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        index: Option<IndexChange>,
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

    // ── The host browse family: UI → host ───────────────────────────
    /// List one absolute directory on the host's own filesystem, with no project in scope yet.
    ///
    /// `path` absent asks for a sensible starting place instead of a listing of one the interface
    /// named — the host's own choice, typically the user's home directory, on the rule that the
    /// interface never composes a path it cannot itself resolve (`D32`, the same one that keeps
    /// `AddProject.path` coming from the platform's own dialog). Unlike the file family, `path` is
    /// absolute and is resolved against nothing: there is no project root yet for it to be
    /// relative to. Answered with [`Message::HostDirListing`] or [`Message::HostDirError`].
    BrowseHostDir {
        path: Option<String>,
    },

    // ── The host browse family: host → UI ───────────────────────────
    /// One absolute directory, listed. Sent in answer to [`Message::BrowseHostDir`].
    HostDirListing {
        /// The path that was listed, canonicalised — so the interface shows where it actually
        /// landed, not the string it asked for (or asked for nothing and got the default).
        /// Without the verbatim `\\?\` prefix on Windows, which does not display and does not
        /// survive the interface joining it with a separator.
        path: String,
        /// `path`'s parent, canonicalised the same way. Absent only at the filesystem root, so a
        /// picker knows when to stop offering to walk up.
        parent: Option<String>,
        entries: Vec<HostDirEntry>,
        /// Whether the entry ceiling cut the listing short, on [`crate::files::DirListing`]'s own
        /// rule.
        truncated: bool,
    },
    /// The path could not be listed: it does not exist, is not a directory, or the host could not
    /// read it.
    HostDirError {
        /// Echoes the request's own `path` — `None` when the failure was in finding a default
        /// starting place rather than in listing a named one.
        path: Option<String>,
        error: HostPathError,
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
        // See the comment on `Message::TerminalOutput::bytes`: same reasoning, same fix.
        #[serde(with = "serde_bytes")]
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
        /// Which saved setup to start from, from [`ProfileInfo`]. Absent is a bare start with
        /// no profile at all. `account` above still wins where both name one — the profile is
        /// the default, the pick is the user saying otherwise.
        profile: Option<String>,
        /// Which model to launch on, from [`Message::HarnessCatalogue`]. `None` or empty leaves
        /// the harness on its own default. Outranks the profile's, the way a pick always does.
        model: Option<String>,
        /// Which reasoning-effort level, same convention as `model`.
        thinking: Option<String>,
        /// Which permission mode, from [`AgentTypeInfo::modes`], same convention.
        mode: Option<String>,
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
    /// Unload, without asking the harness first.
    ///
    /// [`Message::UnloadConversation`] asks the harness to shut down and waits for it to; a
    /// harness that does not act on the ask is what makes that wait long. This kills its process
    /// outright and reaps it, so the answer does not depend on the harness cooperating. Nothing
    /// else differs: the transcript, the run directory and the `WorkAgent` all stay, the same
    /// `agent_id` resumes exactly as after an unload, and the reply is the same
    /// [`Message::ConversationUnloaded`] — so an interface has nothing new to handle. A turn in
    /// flight is not stopped, it is lost; [`Message::CancelTurn`] is what interrupts one and
    /// leaves the harness running.
    AbortConversation {
        agent_id: AgentId,
    },
    /// Start an unloaded conversation's harness again, under the same `agent_id`, with no prompt.
    /// The launch recipe is the `PendingConversation` the agent has carried since it was created,
    /// so this is the same launch a first [`Message::PromptAgent`] performs — only with no turn to
    /// forward afterwards. A conversation that is already live is left alone.
    ResumeConversation {
        agent_id: AgentId,
    },
    /// Mark a conversation as one that outlives the window: its record and its run directory are
    /// kept, and it comes back after a restart rather than being collected.
    ///
    /// Not folded into [`Message::SetAgentConfig`] because the two say different kinds of thing.
    /// That message carries the harness's own config options — model, thinking, mode — under ids
    /// the harness itself advertised, and the host forwards them without knowing what they mean.
    /// This is Ubiq's own property of the conversation: the harness has never heard of it, has no
    /// option id for it, and nothing about it reaches the child process.
    SetConversationPersistent {
        agent_id: AgentId,
        persistent: bool,
    },
    /// Bring a conversation back from its run directory — either the one it left behind, or a copy
    /// of somebody else's.
    ///
    /// **The run directory *is* the conversation.** It holds the harness's own session store, so
    /// resuming a session id inside a given directory is the whole of what "continue this
    /// conversation" means. That is why one message covers both paths, and why the fork needs no
    /// harness-specific support at all — copying a directory is something the filesystem does.
    ///
    /// - `source == agent_id` — **re-attach**. The harness resumes its own session in its own kept
    ///   run directory, and the agent carries on where the last restart left it.
    /// - `source != agent_id` — **fork**. The source's run directory is copied, and a new agent is
    ///   launched in the copy with the same harness session id, so the two share every turn up to
    ///   this point and diverge from the next one on. The source is untouched: it is not stopped,
    ///   not unloaded and not modified.
    ReviveConversation {
        /// The conversation whose run directory is the starting point. Equal to `agent_id` for a
        /// re-attach, different for a fork.
        source: AgentId,
        /// The agent the revived conversation runs as. Minted by the window, the way
        /// [`Message::StartConversation`] mints one.
        agent_id: AgentId,
        project_id: ProjectId,
        session_id: SessionId,
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
        /// The harness's own line that produced this update — the ACP frame or the JSONL line —
        /// where the bridge behind it keeps one. It is what the debug viewer draws beside the
        /// message; nothing in the transcript reads it.
        #[serde(default)]
        raw: Option<String>,
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
    /// Ubiq has read the opening exchange and named the conversation.
    ///
    /// **Not a transcript delta, which is why it is not a [`ConvUpdate`].** It carries no `seq`
    /// and takes no place in the sequence an interface checks for gaps: the naming is Ubiq's own
    /// reading of the conversation rather than something the harness said, and the pump that owns
    /// that sequence is not what produced it. `ConvUpdate::Title` remains the harness naming
    /// itself, and the two write the same field — whichever spoke last is the name.
    ///
    /// Sent at most once per conversation, and only where a provider is configured to write one.
    /// A naming that fails is not reported: nothing was renamed behind anybody's back and the
    /// mechanical name is still there, so there is no state for an interface to unwind.
    ConversationNamed {
        agent_id: AgentId,
        /// What the agent tab says from here on.
        title: String,
        /// The five-word reading of what the conversation is about, drawn as the title's tooltip.
        /// `None` where the model answered a title and nothing after it.
        summary: Option<String>,
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

    // ── Assist family: UI → host ────────────────────────────────────
    /// Ask whether assistance can run here at all, answered with [`Message::Assist`]. A window
    /// asks as it opens a control that would use it, because whether a model is there is a fact
    /// about the host's machine and can change while a window is open.
    GetAssist,
    /// Ask for one suggestion. `subject` names what is being written and carries no prompt: the
    /// prompt is the host's, which is what keeps it out of the interface and out of the bus tape.
    /// The interface mints `suggest_id` and discards every reply naming one it is not holding.
    Suggest {
        suggest_id: SuggestId,
        subject: SuggestSubject,
    },
    /// Give up on a suggestion. Best effort: the model may already be running, and a
    /// [`Message::Suggestion`] for a cancelled id can still arrive — which is what the id is for.
    CancelSuggest {
        suggest_id: SuggestId,
    },
    /// List the configured API providers, answered with [`Message::AiProviders`]. Asked as a
    /// settings page opens, because the host is the half that knows which of them has a key.
    GetAiProviders,
    /// Write a new provider. The host mints the id, files `key` in the OS secret store under it,
    /// and answers with the whole list — a record whose key could not be stored is not written at
    /// all, so a row on screen always has a key behind it.
    ///
    /// The draft's models may be blank, and normally are: a model picker needs an id to name and a
    /// key to call with, so the host lists this provider's models as soon as it has written both
    /// and the user chooses from the answer. A provider with a key and no model reports itself
    /// unavailable with a detail saying so.
    AddAiProvider {
        draft: AiProviderDraft,
        key: Secret,
    },
    /// Change a provider that exists. `key: None` leaves the stored key alone, which is what an
    /// edit that only renames or re-models must do: the interface is never sent a key, so it
    /// cannot send one back.
    UpdateAiProvider {
        provider_id: AiProviderId,
        draft: AiProviderDraft,
        key: Option<Secret>,
    },
    /// Remove a provider and its key together. If the assist setting named it, the setting falls
    /// back to [`crate::assist::AssistProvider::Off`] — a setting pointing at nothing would report
    /// itself unavailable forever.
    ForgetAiProvider {
        provider_id: AiProviderId,
    },
    /// The models a provider says it has, answered with [`Message::AiModels`].
    ///
    /// `refresh: false` is served from the host's cache when there is one and calls the provider
    /// only when there is not, which is what makes a model picker openable without a network. The
    /// interface sets `refresh: true` only for a control the user pressed.
    ListAiModels {
        provider_id: AiProviderId,
        refresh: bool,
    },

    // ── Assist family: host → UI ────────────────────────────────────
    /// Whether assistance can run, and what is behind it. `reason` is present exactly when
    /// `available` is false, in words that name no vendor the user did not configure; `detail` is
    /// an optional sentence to sit under it, and `limits` is present when a backend is there.
    Assist {
        available: bool,
        reason: Option<AssistReason>,
        detail: Option<String>,
        limits: Option<AssistLimits>,
    },
    /// One suggestion, as prose. It is advisory and nothing has been renamed, written or
    /// committed: the interface puts the text where the user can edit or discard it.
    Suggestion {
        suggest_id: SuggestId,
        text: String,
    },
    /// Part of a suggestion, as it arrives.
    ///
    /// Zero or more of these precede the [`Message::Suggestion`] that ends the same id, and every
    /// chunk concatenated is the answer that message carries — modulo the surrounding whitespace
    /// `Suggestion` trims, since a suggestion is put where a user edits it. So an interface that
    /// ignores chunks entirely still renders the right thing, and one that draws them shows a
    /// first token instead of a spinner. A backend that cannot stream sends exactly one.
    SuggestChunk {
        suggest_id: SuggestId,
        text: String,
    },
    /// The suggestion could not be made. The subject keeps its mechanical name — nothing was
    /// changed on the way to failing — so this is a line to show beside the control, not a repair.
    /// Chunks already delivered for this id are not a partial answer: the interface discards them.
    SuggestError {
        suggest_id: SuggestId,
        error: String,
    },
    /// Every configured API provider, with whether each has a key. Which one is *selected* is not
    /// here: that is `HostSettings.assist`, which every window already mirrors, so a row asks it
    /// rather than being told twice.
    ///
    /// Broadcast rather than answered to the asker alone, because a provider list is a property of
    /// the host and a second window's settings page must not show a row that no longer exists.
    AiProviders {
        providers: Vec<AiProviderInfo>,
    },
    /// A provider's model list, and when it was obtained. `refreshed` is true when the provider was
    /// actually called for this answer, false when it came from the cache.
    AiModels {
        list: AiModelList,
        refreshed: bool,
    },
    /// A provider list could not be obtained, or a write to the secret store failed. One line to
    /// show on the settings page: nothing was changed on the way to failing.
    AiProviderError {
        provider_id: Option<AiProviderId>,
        error: String,
    },

    // ── Notification family: UI → host ──────────────────────────────
    /// File a notification. Any subsystem on either side may raise one; the host is what decides
    /// whether a standing mute rule silences it, mints its id, and tells every window.
    RaiseNotification {
        request: NotificationRequest,
    },
    /// Send the bell's whole state. A window asks once, when it attaches.
    ListNotifications,
    /// Tick one notification off, or every one of them when `id` is `None`.
    ReadNotifications {
        id: Option<NotificationId>,
    },
    /// Drop one notification from the history, or every one of them when `id` is `None`.
    DismissNotifications {
        id: Option<NotificationId>,
    },
    /// Stand up a mute rule. A scope holds at most one rule, so this replaces any rule already on
    /// that scope rather than stacking a second.
    MuteNotifications {
        scope: MuteScope,
        max_level: Level,
        duration: MuteFor,
    },
    /// Lift the rule on one scope. Lifting a scope that has no rule is not an error.
    UnmuteNotifications {
        scope: MuteScope,
    },

    // ── Notification family: host → UI ──────────────────────────────
    /// One notification, as it was filed. Broadcast, because the bell is in every window and a
    /// badge that disagrees between two of them is the bug this avoids by construction.
    ///
    /// `notification.muted` is the host's verdict after the rules: a window flashes its bell for a
    /// notification that is not muted, and only counts one that is.
    NotificationRaised {
        notification: Box<Notification>,
    },
    /// The bell's whole state — the history, newest first, and the rules in force. Broadcast for
    /// the same reason, and sent whole rather than as a diff: the list is capped and it changes on
    /// a click, so a patch protocol would buy nothing and could leave two windows disagreeing.
    NotificationsState {
        state: Box<Notifications>,
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
            | Message::ReviveConversation { project_id, .. }
            | Message::ConversationStarted { project_id, .. }
            | Message::SearchProject { project_id, .. }
            | Message::CancelSearch { project_id, .. }
            | Message::SearchMatches { project_id, .. }
            | Message::SearchProgress { project_id, .. }
            | Message::SearchFinished { project_id, .. }
            | Message::SearchError { project_id, .. } => Some(*project_id),
            // The project is inside the subject rather than beside it, so this arm stands alone.
            Message::Suggest {
                subject: SuggestSubject::CommitMessage { project_id },
                ..
            } => Some(*project_id),
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
/// `id` back; it never names a config path or a launch flag, because how a harness is started is
/// not a fact it holds.
///
/// `command` is the one exception, and it is here to be *shown*, not composed: the user may
/// override what this machine runs for a harness ([`crate::settings::HostSettings::agent_commands`]),
/// and a field asking for that override with no idea what it is replacing is a field nobody can
/// fill in.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentTypeInfo {
    /// The library's harness id, e.g. `claude-code`.
    pub id: String,
    /// What the row says — the harness's display name.
    pub label: String,
    /// What the library would run for it, e.g. `claude` — the placeholder a custom command is
    /// typed over, and never something the interface composes a launch from.
    pub command: String,
    /// Whether it can be started here: the harness's own binary was found, or a custom command is
    /// configured for it. A row that cannot start says so before it is picked rather than failing
    /// as a spawn the user has to interpret.
    pub available: bool,
    /// Whether this harness can hold a *conversation* — the library has a structured bridge for
    /// it, so its output arrives as `ConvUpdate`s rather than as terminal bytes. False for a
    /// harness that can only be driven through a pane, which is a real thing it can still do:
    /// this field is what keeps such a harness out of every chat-start menu without taking it out
    /// of the new-pane one.
    ///
    /// It says nothing about how many turns one process takes. A one-shot harness — prompt in
    /// argv, one answer, exit — is `chat: true`, because the host continues it by launching again
    /// with the session id the last run reported.
    pub chat: bool,
    /// The permission modes this harness advertises, as the library reports them. Empty when
    /// the harness has no such axis — a mode is not a universal concept, it is whatever this
    /// particular harness named.
    pub modes: Vec<ConfigChoice>,
    /// Which of [`Self::modes`] means "ask nothing" — the harness's own word for it, from the
    /// library's `Harness::unattended_mode`. `None` where the harness has no such mode, or
    /// already asks nothing. It is what a start form defaults its mode picker to, so the
    /// interface never has to guess which id means "all permissions".
    pub unattended_mode: Option<String>,
    /// Whether this harness keeps its own session store inside the run directory Ubiq provisions
    /// for it — that is, whether its `config_anchor` declares a lever that relocates it there.
    ///
    /// **It is what makes a conversation keepable.** Ubiq resumes a conversation across a restart
    /// by keeping that directory, and forks one by copying it (`D97`), so a harness that writes its
    /// sessions somewhere else can be offered neither: keeping would preserve nothing and copying
    /// would leave two agents sharing one store. Grok is the one that answers `false` — it declares
    /// no lever and writes under the user's real home whatever Ubiq relocates.
    ///
    /// The interface reads it to draw those two controls disabled, with the reason, rather than
    /// offering an action the host will refuse. The host refuses it regardless; this is so the
    /// refusal is not the first the user hears of it.
    pub keeps_sessions: bool,
}

/// One model a harness will answer for, with the reasoning-effort levels it accepts folded in.
///
/// The interface's own copy of the host's `store::harness::CachedModel`: the two probes
/// (`discover_models` + `discover_thinking`) are joined once, host-side, so a picker reads one
/// record rather than two lists it has to correlate.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogueModel {
    /// The harness's own model id, e.g. `sonnet`, `gpt-5-codex`.
    pub id: String,
    pub description: Option<String>,
    /// Whether the harness names this one its default.
    pub default: bool,
    /// The reasoning-effort levels this model accepts, in the harness's own vocabulary. Empty
    /// is the normal case — most models have no such knob.
    pub levels: Vec<ConfigChoice>,
    /// Which level the harness starts at, where it says.
    pub default_level: Option<String>,
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

/// One profile, as the UI is told about it.
///
/// A profile is a saved setup: which harness, as whom, with which model and which permission
/// mode. Like [`AccountInfo`] every field is a reference — an id the library resolves — and
/// nothing here is credential material or a path. `None` on a field means the profile does not
/// mention that axis, and a lower layer decides.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileInfo {
    /// What the user named this setup, e.g. `review`.
    pub id: String,
    /// The library's harness id this profile is for, from [`AgentTypeInfo`].
    pub agent_type: String,
    /// Which identity it runs as, from [`AccountInfo`].
    pub account: Option<String>,
    /// The model id it picks, from the harness's own model list.
    pub model: Option<String>,
    /// The permission mode it picks, from [`AgentTypeInfo::modes`].
    pub mode: Option<String>,
    /// The reasoning-effort level it picks, in the harness's own vocabulary.
    pub thinking: Option<String>,
    /// The most subagents this setup asks for at once. Never a launch flag — no harness has
    /// one: the interface writes it into the conversation's opening prompt as a directive, and
    /// this is only where the number is remembered.
    pub max_subagents: Option<u8>,
    /// An opening prompt to send as the conversation's first turn. It is a turn like any other,
    /// which is why it is text here rather than anything the run is composed from.
    pub prompt: Option<String>,
}

/// Secret material as it crosses the bus.
///
/// Two variants carry material — [`Message::SubmitConnectSecret`] and [`Message::SetAppSecret`] —
/// and the rule they encode is not "these two are blessed" but **material crosses only in a
/// `Secret`, and a `Secret` is never printed**. The type is the enforcement rather than a
/// convention: `Debug` writes `Secret(***)`, and there is deliberately no `Display`, no `Deref`
/// and no `AsRef<str>`, so `tracing::info!(token = %secret)` does not compile and `?secret`
/// redacts itself. [`Self::expose`] is the only way out, which makes every place material leaves
/// this type one grep.
///
/// Serialisation is transparent: the host needs the value, and the log sink never reads the wire.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// The material itself. Named so that reading it is a deliberate act.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(***)")
    }
}

/// One connection as the interface is told about it: the stored record, plus what the host read
/// out of the token without calling anyone.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectionInfo {
    pub connection: Connection,
    pub status: LoginStatus,
    /// Whether this connection's instance has a pinned certificate. Derived from the trusted list
    /// rather than stored, so a row can say so where the user is already looking.
    #[serde(default)]
    pub pinned: bool,
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
