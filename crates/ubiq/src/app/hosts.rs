//! The multiplexer: one window's connection to every host it is attached to.
//!
//! Always the local, in-process host, plus every remote the connect flow has dialled and not yet
//! lost. The rule set for it, from `AGENTS.md`: the local host is always attached, and a remote
//! host is added *alongside* it rather than swapped in, so a window's terminals never all move
//! because one remote connection dropped.
//!
//! [`Bus`] stands in for the `ubiq_proto::bus::Client` field `AppState` used to hold, and exposes
//! the same four methods a `Client` does — `send`, `sender`, `input`, and (through
//! [`Bus::connections`], read once at construction) `from_host` — under the same names and
//! signatures. That is deliberate: `crates/ubiq/src` calls `bus.send(...)` at 130 sites, and none
//! of them may need to learn that a message might now have somewhere else to go. What changed is
//! what answers a call, not how a call is written.
//!
//! [`HostId`] is UI-local. It is minted here, never carried in a [`Message`], and never crosses
//! the bus in either direction — a host has no way to learn that another host exists, because
//! every connection remains, from a host's point of view, an ordinary single-host session. Only
//! the interface tells its several connections apart, purely to decide which one a message goes
//! to.
//!
//! **Why this file is `hosts.rs` and not `bus.rs`.** `app/mod.rs` binds the name `bus` to
//! `ubiq_proto::bus` (`use ubiq_proto::bus;`) and `wire.rs` reaches `bus::pane_output` through that
//! binding — a module named `bus` here would collide with it in every file that does
//! `use super::*`. This file is about the *set of hosts* a window is multiplexed over, so `hosts`
//! is what it is anyway; `bus.rs` was the name suggested before that collision turned up.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};

use ubiq_proto::bus::{Client, Outbox, PaneInput};
use ubiq_proto::carrier::CAP_TOOLS;
use ubiq_proto::ids::{PaneId, ProjectId, RemoteCallId};
use ubiq_proto::messages::Message;
use ubiq_proto::settings::{RemoteScheme, SavedRemoteHost};
use ubiq_proto::stats::HostStats;
use ubiq_proto::work::AgentId;

/// Which host something is about: the one every window is always attached to, or one of the
/// remote ones attached beside it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum HostRef {
    /// The one host this process starts before the first window and that outlives every one of
    /// them. Never absent, and never dropped by anything short of the process exiting.
    Local,
    /// A host reached over a connection the interface opened itself, named by the id `Bus` minted
    /// for it when the connection was registered.
    Remote(HostId),
}

/// A UI-local id for a remote connection, minted by [`Bus::register_remote`].
///
/// Not one of `ubiq-proto`'s ids: it never serialises, never travels in a [`Message`], and is
/// meaningless to any host — it exists only so this one window can tell its own connections apart.
/// Compare it with `ubiq_proto::bus::ClientId`, which plays the same "not really an id" role on
/// the other side of a `Client`, for the same reason.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct HostId(u64);

/// One connection to a host that is not the local, in-process one.
///
/// Built by `app::remote_connect`'s dial, which hands the `Client` a `Detached` pump drives over
/// the socket — see `ubiq_proto::bus::detached` and `_docs/tech/architecture.md`'s "Remote
/// harnesses" section.
pub struct RemoteConn {
    pub id: HostId,
    pub client: Client,
    /// What a screen listing several remotes, like the "Open remote project…" row, calls this
    /// one — the saved host's own name when this connection was dialled from one (a fresh dial or
    /// a reconnect from the Hosts section), or the bare address otherwise. Set once, at
    /// `app::remote_connect::land_remote_connect`'s landing, and never updated afterwards even if
    /// the saved record behind it is later renamed or forgotten — this is a live connection's own
    /// label, not a view onto `HostSettings::remote_hosts`.
    pub label: String,
    /// The saved entry this connection was dialled from, when it was dialled from one. Links the
    /// live connection back to its keychain token and its scheme/trust settings for reconnects.
    /// Empty for one-off dials typed fresh into the connect modal.
    pub save_id: String,
    /// `host:port` as dialled, for status lines and reconnects.
    pub address: String,
    /// Which protocol this connection speaks. Decided at dial time from the saved entry or the
    /// pasted scheme, and fixed for the life of the connection.
    pub scheme: RemoteScheme,
    /// What the connection is doing now. The remote-hosts panel draws this; the reconnect loop
    /// in `app::remote_connect` moves it.
    pub status: ConnStatus,
    /// What the drone advertised in its hello (`ubiq_proto::carrier::CAP_TOOLS`, …). Empty for a
    /// connection that is not a drone, and until [`Bus::set_remote_caps`] files them.
    pub caps: Vec<String>,
}

/// What a live remote connection is doing. `Attached` is the steady state; anything else is the
/// reconnect loop's business, drawn by the remote-hosts panel rather than hidden.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConnStatus {
    /// Frames flow. Set on landing, and again on every successful reconnect.
    Attached,
    /// The socket dropped and a reconnect is scheduled. `attempt` counts tries since the drop;
    /// the delay before the next one backs off from it.
    Reconnecting { attempt: u32 },
    /// Reconnects are still scheduled but the last try failed with `error`. Kept beside
    /// `Reconnecting` rather than folded into it so the panel can say *why* without re-reading
    /// the settings page's failed set.
    Failed { attempt: u32, error: String },
}

/// What a remote host said about itself: its `HostInfo` greeting plus the latest `Stats` poll.
///
/// Per-host, keyed by the `HostId` the greeting arrived under — the local host's answer still
/// owns the status bar and the launch menus (`G188`), but a remote's facts are no longer dropped
/// on arrival. Every field is best-effort: `None` is "the host did not say", never zero.
#[derive(Clone, Debug, Default)]
pub struct RemoteHostMeta {
    pub hostname: Option<String>,
    pub os: Option<String>,
    pub arch: Option<String>,
    pub triplet: Option<String>,
    pub cpu_count: Option<u64>,
    pub mem_total_bytes: Option<u64>,
    pub sessions_count: Option<usize>,
    pub cpu_load_pct: Option<f32>,
    pub mem_free_bytes: Option<u64>,
    pub disk_free_bytes: Option<u64>,
}

impl RemoteHostMeta {
    /// One line for a manager row: `macOS/aarch64 · office · 2 sessions`, skipping what is missing.
    pub fn summary(&self) -> String {
        let mut parts = Vec::new();
        match (&self.os, &self.arch) {
            (Some(os), Some(arch)) => parts.push(format!("{os}/{arch}")),
            (Some(os), None) => parts.push(os.clone()),
            (None, Some(arch)) => parts.push(arch.clone()),
            (None, None) => {}
        }
        if let Some(hostname) = &self.hostname {
            parts.push(hostname.clone());
        }
        if let Some(sessions) = self.sessions_count {
            parts.push(format!(
                "{} session{}",
                sessions,
                if sessions == 1 { "" } else { "s" }
            ));
        }
        if parts.is_empty() {
            "no host facts yet".to_string()
        } else {
            parts.join(" · ")
        }
    }

    /// The tooltip behind the project picker's OS letter: everything the host has said.
    pub fn tooltip(&self) -> String {
        let mut lines = Vec::new();
        if let Some(hostname) = &self.hostname {
            lines.push(format!("host {hostname}"));
        }
        match (&self.os, &self.arch, &self.triplet) {
            (Some(os), Some(arch), Some(triplet)) => {
                lines.push(format!("{os}/{arch} ({triplet})"));
            }
            (Some(os), Some(arch), None) => lines.push(format!("{os}/{arch}")),
            (Some(os), None, _) => lines.push(os.clone()),
            _ => {}
        }
        if let Some(cpus) = self.cpu_count {
            lines.push(format!("{cpus} CPUs"));
        }
        match (self.mem_free_bytes, self.mem_total_bytes) {
            (Some(free), Some(total)) => lines.push(format!(
                "{} free of {} RAM",
                bytes_human(free),
                bytes_human(total)
            )),
            (None, Some(total)) => lines.push(format!("{} RAM", bytes_human(total))),
            _ => {}
        }
        if let Some(disk) = self.disk_free_bytes {
            lines.push(format!("{} disk free", bytes_human(disk)));
        }
        if lines.is_empty() {
            "a remote host Ubiq has not heard from yet".to_string()
        } else {
            lines.join("\n")
        }
    }
}

fn bytes_human(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} {}", UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// The pane-family variants that name a pane directly.
///
/// `ubiq-proto` has no `Message::pane_id()` the way it has [`Message::project_id`] — the pane
/// family is a handful of variants rather than most of the enum, so one is not worth adding to the
/// contract for this alone. This mirrors the match `project_id()` would need if it, too, had never
/// been written, kept local to the interface because routing is the interface's concern, not the
/// contract's.
fn pane_id_of(message: &Message) -> Option<PaneId> {
    match message {
        Message::TerminalOutput { pane_id, .. }
        | Message::PaneExited { pane_id, .. }
        | Message::PaneError { pane_id, .. }
        | Message::TerminalInput { pane_id, .. }
        | Message::TerminalResize { pane_id, .. }
        | Message::Focus { pane_id }
        | Message::CloseWorkspace { pane_id }
        | Message::StopPane { pane_id }
        | Message::HarnessLoginStarted { pane_id, .. }
        | Message::HarnessLoginLink { pane_id, .. } => Some(*pane_id),
        Message::WorkspaceSpawned { workspace } => Some(workspace.id),
        _ => None,
    }
}

/// The project-naming messages that read or write the catalogue *record* rather than the folder —
/// routed to the row's owner, never to a drone that only serves it. These are the five a drone
/// refuses with `ProjectError`, which is why a project open on a drone used to raise the
/// picker's banner each time one of them went there.
fn is_record_message(message: &Message) -> bool {
    matches!(
        message,
        Message::ForgetProject { .. }
            | Message::UpdateProject { .. }
            | Message::LocateProject { .. }
            | Message::OpenedProject { .. }
            | Message::RefreshProject { .. }
    )
}

/// The messages that start or revive a conversation — routed to the row's owner too, because a
/// drone has no harness library: an agent in a project a drone serves is a harness on the row
/// owner's machine, bound to that drone (`D226`).
fn is_conversation_start(message: &Message) -> bool {
    matches!(
        message,
        Message::StartConversation { .. } | Message::ReviveConversation { .. }
    )
}

/// What goes to the row's owner rather than to whoever serves the folder.
fn is_owner_message(message: &Message) -> bool {
    is_record_message(message) || is_conversation_start(message) || is_work_message(message)
}

/// The work, board and mission families a window sends — routed to the row's owner, because the
/// agents they assign and the stores they write are the owner's, never a drone's (which answers
/// them `NOT_HERE` or with empty lists). A board's envelope is whatever it carries.
fn is_work_message(message: &Message) -> bool {
    match message {
        Message::OnBoard { message, .. } => is_work_message(message),
        _ => matches!(
            message,
            Message::ListWork { .. }
                | Message::CreateTask { .. }
                | Message::UpdateTask { .. }
                | Message::SetTaskField { .. }
                | Message::MoveTask { .. }
                | Message::AssignTask { .. }
                | Message::DeleteTask { .. }
                | Message::ArchiveTasks { .. }
                | Message::AddStep { .. }
                | Message::RenameStep { .. }
                | Message::RemoveStep { .. }
                | Message::MoveStep { .. }
                | Message::ToggleStep { .. }
                | Message::AddComment { .. }
                | Message::AssignAgent { .. }
                | Message::SendToAgent { .. }
                | Message::DocAgentQueue { .. }
                | Message::ListBoards { .. }
                | Message::CreateBoard { .. }
                | Message::SetBoardEnabled { .. }
                | Message::DeleteBoard { .. }
                | Message::ListMissions { .. }
                | Message::CreateMission { .. }
                | Message::SetMissionField { .. }
                | Message::MissionControl { .. }
                | Message::MissionTask { .. }
                | Message::AddGuidance { .. }
                | Message::LoadHistory { .. }
        ),
    }
}

/// The conversation a window → host message is about, for the conversation-family messages that
/// name an agent and no project — [`pane_id_of`]'s shape, for the same reason.
fn conversation_follow_up(message: &Message) -> Option<AgentId> {
    match message {
        Message::PromptAgent { agent_id, .. }
        | Message::CancelTurn { agent_id, .. }
        | Message::AnswerPermission { agent_id, .. }
        | Message::AnswerAsk { agent_id, .. }
        | Message::AbortConversation { agent_id, .. }
        | Message::EndConversation { agent_id, .. }
        | Message::UnloadConversation { agent_id, .. }
        | Message::ResumeConversation { agent_id, .. }
        | Message::RenameConversation { agent_id, .. }
        | Message::SetAgentConfig { agent_id, .. }
        | Message::SetConversationAcceptAll { agent_id, .. }
        | Message::SetConversationDebugDump { agent_id, .. }
        | Message::SetConversationPersistent { agent_id, .. } => Some(*agent_id),
        _ => None,
    }
}

/// A window's connection to every host it is attached to.
///
/// Replaces the `ubiq_proto::bus::Client` field `AppState` used to hold. `local` takes over
/// exactly what that field did — nothing about the local connection's lifecycle, or what dropping
/// it tells the host, changes.
pub struct Bus {
    /// Always present, never dropped while the window lives. Dropping this — which happens only
    /// when `Bus`, and so `AppState`, is dropped — is how the host learns the window has gone,
    /// exactly as when the field held a bare `Client`.
    local: Client,
    /// Other hosts this window is attached to, beside the local one. Empty until a dial lands.
    remotes: Vec<RemoteConn>,
    /// Which host a message naming neither a pane nor a project should reach — the destination of
    /// "new terminal", "new project" and anything else the user has not pointed at a specific
    /// remote. Moved by the Hosts section's dropdown, and reset to `Local` by
    /// [`Bus::drop_remote`] when the host it named goes away.
    active: HostRef,
    /// Every pane this window has been told about, and which host reported it.
    ///
    /// A `RefCell`: `Bus::send` takes `&self` (unchanged from `Client::send`, so the 130 call
    /// sites need not become `&mut`), but resolving a destination must still be able to look a
    /// pane up. Registering a pane is comparatively rare — once per spawn, once per remote
    /// listing — so the interior mutability costs nothing worth measuring.
    panes: RefCell<HashMap<PaneId, HostRef>>,
    /// Every project this window has been told about, and which host reported it. Same shape and
    /// the same reason as `panes`.
    projects: RefCell<HashMap<ProjectId, HostRef>>,
    /// Every conversation this window started or was told started, and which host runs it — so a
    /// turn, a cancel or an answer, none of which names a project, reaches the host that has the
    /// harness rather than whichever the Hosts dropdown points at. Same shape as `panes`.
    agents: RefCell<HashMap<AgentId, HostRef>>,
    /// Every project id the *local* catalogue has ever named, whatever host serves it now.
    ///
    /// The one thing in this bus that is not a host's wholesale property — see
    /// [`Bus::row_owner`], which is the rule this set exists for.
    owned_locally: RefCell<HashSet<ProjectId>>,
    /// Projects whose catalogue row says they run on a drone (`runs_on`). Such a project's folder
    /// is not on this machine, so while no drone serves it a folder or pane request must not fall
    /// back to the local host, which would read the same path on the wrong machine.
    pinned_remote: RefCell<HashSet<ProjectId>>,
    /// What each attached remote said about itself — its `HostInfo` greeting plus the latest
    /// `Stats` poll. Same shape and same reason as `panes`: written from `receive`, read from
    /// `render`, without taking `&mut`.
    remote_meta: RefCell<HashMap<HostId, RemoteHostMeta>>,
    /// Mints the next [`HostId`]. Process-local to this `Bus`, which is fine — a `HostId` is only
    /// ever compared against other ids this same `Bus` minted.
    next_host_id: AtomicU64,
    /// Remote-tool calls relayed to a drone and not yet answered: where the result goes back to
    /// (`D226`). The first result removes the entry, so a second or a late one is dropped.
    tool_calls: RefCell<HashMap<RemoteCallId, PendingToolCall>>,
}

/// One relayed [`Message::RemoteToolCall`] awaiting its result.
struct PendingToolCall {
    /// The host the call came from, and so where its result returns.
    origin: HostRef,
    /// The drone link it was forwarded to.
    link: HostId,
    project_id: ProjectId,
}

impl Bus {
    /// Wrap the local host's client. Used once, where `AppState` used to store the bare `Client`
    /// it got from `BusHub::connect`.
    pub fn new(local: Client) -> Self {
        Self {
            local,
            remotes: Vec::new(),
            active: HostRef::Local,
            panes: RefCell::new(HashMap::new()),
            projects: RefCell::new(HashMap::new()),
            agents: RefCell::new(HashMap::new()),
            owned_locally: RefCell::new(HashSet::new()),
            pinned_remote: RefCell::new(HashSet::new()),
            remote_meta: RefCell::new(HashMap::new()),
            next_host_id: AtomicU64::new(0),
            tool_calls: RefCell::new(HashMap::new()),
        }
    }

    /// The client behind a [`HostRef`], or nothing at all for a remote this `Bus` no longer holds.
    ///
    /// Deliberately not "fall back to the local host": a dropped remote's pane ids were minted by
    /// that remote and mean nothing here, so sending its keystrokes, its resizes or the close that
    /// kills it to the local host is not a harmless near-miss — it is the local coordinator being
    /// asked about panes it never created. A lost destination is a dropped message.
    fn client_for(&self, host: HostRef) -> Option<&Client> {
        match host {
            HostRef::Local => Some(&self.local),
            HostRef::Remote(id) => self
                .remotes
                .iter()
                .find(|remote| remote.id == id)
                .map(|remote| &remote.client),
        }
    }

    /// Work out which host a message with no explicit destination should reach.
    ///
    /// Pane first, because a pane is a running harness on one specific host — sending its input,
    /// its resize, or the close that kills it to any other host does nothing to the process that
    /// is actually running it. Project second, because a project has no single pane but is still
    /// hosted on one machine — a git refresh or a file read must land there and nowhere else, even
    /// while no pane of that project happens to be open. Active last, because a message naming
    /// neither — asking the catalogue for its list, starting a brand-new pane — has nothing to be
    /// resolved *from*; it can only mean whichever host the user is currently pointed at.
    fn resolve(&self, message: &Message) -> HostRef {
        if let Some(pane_id) = pane_id_of(message)
            && let Some(host) = self.panes.borrow().get(&pane_id)
        {
            return *host;
        }
        // A conversation's follow-ups name no project: they go where the conversation runs.
        if let Some(agent_id) = conversation_follow_up(message)
            && let Some(host) = self.agents.borrow().get(&agent_id)
        {
            return *host;
        }
        if let Some(project_id) = message.project_id() {
            // The record's own half goes to whoever owns the *row* (`D132`): a drone serving a
            // locally-owned project holds no catalogue, and refuses all five.
            // A conversation runs where the row is too: a drone has no harness library, so an
            // agent in a project it serves is a local harness bound to it (`D226`, `R7`).
            // And so does the work: the agents it assigns live on the row owner's coordinator, and
            // a drone keeps no tasks, boards or missions.
            if is_owner_message(message) {
                return self.row_owner(project_id);
            }
            if let Some(host) = self.projects.borrow().get(&project_id) {
                return *host;
            }
        }
        self.active
    }

    /// Say something to whichever host owns it. Exactly `Client::send`'s signature, so every
    /// existing `bus.send(...)` call site compiles unchanged; what differs is that the message may
    /// now be resolved to one of several hosts instead of the one there used to be.
    pub fn send(&self, message: Message) {
        let host = self.resolve(&message);
        if host == HostRef::Local && self.unserved_pinned(&message) {
            // The bus has no way to answer its own window, so this is a drop. The tree is asked
            // again when the drone attaches (`ask_root_tree`).
            tracing::debug!(
                "dropped a request for a project whose drone is not attached: {message:?}"
            );
            return;
        }
        if let Message::StartConversation { agent_id, .. }
        | Message::ReviveConversation { agent_id, .. } = &message
        {
            self.note_agent(*agent_id, host);
        }
        self.send_to(host, message);
    }

    /// Say something to a specific host, bypassing resolution.
    ///
    /// For the case `send` cannot cover: browsing a remote's filesystem, or asking it for its
    /// repositories, before any project on it has been opened — there is no pane and no project
    /// yet to resolve from, and the host in question is not necessarily `active`.
    pub fn send_to(&self, host: HostRef, message: Message) {
        match self.client_for(host) {
            Some(client) => client.send(message),
            None => tracing::debug!("dropped a message for a host that is gone: {message:?}"),
        }
    }

    /// A sender for callbacks that need to reach a host with no window in hand — today, only the
    /// terminal emulator's own resize measurement. Resolves against `active`, exactly as `send`
    /// would for a message naming neither a pane nor a project, because an `Outbox` is minted with
    /// no message to resolve from.
    ///
    /// An `Outbox` has to exist, so this is the one place a gone host resolves to the local
    /// client anyway — which never actually happens, because [`Bus::drop_remote`] puts `active`
    /// back on `Local` as it removes the connection.
    pub fn sender(&self) -> Outbox {
        self.client_for(self.active).unwrap_or(&self.local).sender()
    }

    /// The write half for one pane's keystrokes. Resolves against the pane's own recorded host —
    /// falling back to `active` for a pane this `Bus` was never told about, which today never
    /// happens, since every pane is recorded when its workspace is drawn.
    ///
    /// A `PaneInput` has to exist, so a host that has gone since falls back to the local client
    /// with a word in the log rather than nothing. Nothing types into one: losing a host closes
    /// every pane it was running, and a closed pane's emulator is dropped with it.
    pub fn input(&self, pane_id: PaneId) -> PaneInput {
        let host = self
            .panes
            .borrow()
            .get(&pane_id)
            .copied()
            .unwrap_or(self.active);
        let client = self.client_for(host).unwrap_or_else(|| {
            tracing::warn!("keystrokes for pane {pane_id}, whose host is gone");
            &self.local
        });
        client.input(pane_id)
    }

    /// Every connection's inbound stream, tagged with the [`HostRef`] it came from — what the
    /// router in `boot.rs` spawns one task per, so an arrival is tagged before `AppState::receive`
    /// ever sees it rather than re-derived from the message afterwards.
    ///
    /// A snapshot, not a live view: today it is read once at construction, when `remotes` is
    /// always empty, so it names exactly the one task boot.rs already spawned. A later phase that
    /// adds a remote after boot will need to spawn its router task itself, from
    /// [`Bus::register_remote`]'s return — this method stays the right shape for that, since it is
    /// already "the connections a router should be draining", just not yet asked for again after
    /// the first.
    pub fn connections(&self) -> Vec<(HostRef, flume::Receiver<Message>)> {
        let mut out = vec![(HostRef::Local, self.local.from_host().clone())];
        out.extend(self.remotes.iter().map(|remote| {
            (
                HostRef::Remote(remote.id),
                remote.client.from_host().clone(),
            )
        }));
        out
    }

    /// Record which host a pane belongs to. Called as panes arrive — today, always with
    /// `HostRef::Local` — so `send` and `input` can route to it later without needing the message
    /// that first announced it.
    pub fn note_pane(&self, pane_id: PaneId, host: HostRef) {
        self.panes.borrow_mut().insert(pane_id, host);
    }

    /// Record which host runs a conversation — as it is started or revived from here, and as any
    /// host reports one started (`ConversationStarted`), so its follow-ups route there.
    pub fn note_agent(&self, agent_id: AgentId, host: HostRef) {
        self.agents.borrow_mut().insert(agent_id, host);
    }

    /// Forget a conversation, once it is deleted — `forget_pane`'s reason.
    pub fn forget_agent(&self, agent_id: AgentId) {
        self.agents.borrow_mut().remove(&agent_id);
    }

    /// Forget a pane, once it has closed. Not required for correctness — an unresolved pane falls
    /// back to `active`, and a closed pane's id is never sent again — but kept so the map does not
    /// grow for the life of the window.
    pub fn forget_pane(&self, pane_id: PaneId) {
        self.panes.borrow_mut().remove(&pane_id);
    }

    /// Which host serves a project — what [`Bus::resolve`] would route a message naming it to.
    ///
    /// For the one case a *destination* is needed with no message to resolve from: browsing the
    /// filesystem a project's knowledge base reads its folders on. `BrowseHostDir` names no
    /// project, so `send` would resolve it to `active`, which is not necessarily where the project
    /// being edited lives.
    pub fn host_of_project(&self, project_id: ProjectId) -> HostRef {
        self.projects
            .borrow()
            .get(&project_id)
            .copied()
            .unwrap_or(self.active)
    }

    /// Record which host a project belongs to, on the same terms as [`Bus::note_pane`].
    ///
    /// A local listing does not take a row back off a drone that is serving it: re-listing says
    /// the row is still the local catalogue's, which [`Bus::row_owner`] already knew, not that
    /// the folder came home. Only [`Bus::drop_remote`] brings it home.
    pub fn note_project(&self, project_id: ProjectId, host: HostRef) {
        if host == HostRef::Local {
            self.owned_locally.borrow_mut().insert(project_id);
            if matches!(
                self.projects.borrow().get(&project_id),
                Some(HostRef::Remote(_))
            ) {
                return;
            }
        }
        self.projects.borrow_mut().insert(project_id, host);
    }

    /// Whose *row* a project is, as against which host is currently serving it.
    ///
    /// Everywhere else here a host owns what it reports wholesale, and this is the one exception.
    /// `D116` is where it comes from: a project configured with `runs_on` is a row the local
    /// catalogue holds and a drone *fills in*. The drone is launched with `--root-id`, so it
    /// announces that folder under the id the local catalogue already names — what it contributes
    /// is its liveness, not the row. Serving moves to the drone, so panes and file reads reach the
    /// machine the folder is actually on; ownership does not move at all, which is what keeps the
    /// row on screen when the drone is down. Without the split a user loses a configured project
    /// every time their laptop sleeps.
    ///
    /// For a duplicate [`ProjectId`], then, the local host owns the row. Every place that asks
    /// "whose rows are these" asks here rather than reading `projects` directly.
    pub fn row_owner(&self, project_id: ProjectId) -> HostRef {
        if self.owned_locally.borrow().contains(&project_id) {
            return HostRef::Local;
        }
        self.project_host(project_id)
    }

    /// The locally-owned rows a remote is serving right now — a `runs_on` project's, and nothing
    /// else. Read *before* [`Bus::drop_remote`], which is what brings them home; the caller is
    /// what marks them unreadable.
    pub fn local_rows_served_by(&self, id: HostId) -> Vec<ProjectId> {
        let host = HostRef::Remote(id);
        let owned = self.owned_locally.borrow();
        self.projects
            .borrow()
            .iter()
            .filter(|(project_id, owner)| **owner == host && owned.contains(*project_id))
            .map(|(project_id, _)| *project_id)
            .collect()
    }

    /// Forget a project, once it has been forgotten by the catalogue too.
    pub fn forget_project(&self, project_id: ProjectId) {
        self.projects.borrow_mut().remove(&project_id);
        self.owned_locally.borrow_mut().remove(&project_id);
    }

    /// Record whether a catalogue row pins its project to a drone.
    pub fn note_pinned(&self, project_id: ProjectId, pinned: bool) {
        if pinned {
            self.pinned_remote.borrow_mut().insert(project_id);
        } else {
            self.pinned_remote.borrow_mut().remove(&project_id);
        }
    }

    /// Whether a project's row pins it to a drone, served or not.
    pub fn is_pinned(&self, id: ProjectId) -> bool {
        self.pinned_remote.borrow().contains(&id)
    }

    /// Whether a project's row pins it to a drone and no drone is serving it right now.
    pub fn is_pinned_unserved(&self, id: ProjectId) -> bool {
        self.pinned_remote.borrow().contains(&id)
            && matches!(self.projects.borrow().get(&id), None | Some(HostRef::Local))
    }

    /// Whether `message` is project-scoped for a drone-pinned project that no drone is serving
    /// right now. Everything is, except what goes to the row's owner whoever serves the folder:
    /// the record's own half, the conversations and the work.
    fn unserved_pinned(&self, message: &Message) -> bool {
        !is_owner_message(message)
            && message
                .project_id()
                .is_some_and(|id| self.is_pinned_unserved(id))
    }

    /// Which host reported a project. `Local` for one never recorded — the catalogue a window
    /// lists at boot is the local one, so an unrecorded id belongs to it by construction.
    pub fn project_host(&self, project_id: ProjectId) -> HostRef {
        self.projects
            .borrow()
            .get(&project_id)
            .copied()
            .unwrap_or(HostRef::Local)
    }

    /// Which host a message naming neither a pane nor a project resolves to.
    pub fn active(&self) -> HostRef {
        self.active
    }

    /// Point `active` at a different host, as the Hosts section's dropdown does.
    pub fn set_active(&mut self, host: HostRef) {
        self.active = host;
    }

    /// Add a remote connection and mint the [`HostId`] it is known by from here on.
    ///
    /// Hands back the new connection's inbound stream alongside its id: the caller still has to
    /// spawn a router task for it — see `AppState::route_host` — and [`Bus::connections`] cannot
    /// be asked again for just this one without re-handing out every connection that already has
    /// a router draining it, which would spawn a second one racing the first.
    pub fn register_remote(
        &mut self,
        client: Client,
        label: String,
        save_id: String,
        address: String,
        scheme: RemoteScheme,
    ) -> (HostId, flume::Receiver<Message>) {
        let id = HostId(self.next_host_id.fetch_add(1, Ordering::Relaxed));
        let from_host = client.from_host().clone();
        self.remotes.push(RemoteConn {
            id,
            client,
            label,
            save_id,
            address,
            scheme,
            status: ConnStatus::Attached,
            caps: Vec::new(),
        });
        (id, from_host)
    }

    /// Lose a remote: drop the connection, forget every project recorded under it, and put
    /// `active` back on the local host if it was pointed at this one — otherwise every message
    /// naming neither a pane nor a project would go on resolving to a host that is not there.
    ///
    /// Hands back the panes it was running rather than forgetting them, because closing one is
    /// the caller's job and `AppState::close_pane` still sends a `CloseWorkspace` for it: with the
    /// pane still recorded here that close resolves to this now-absent host and is dropped, where
    /// forgetting it first would resolve it to `active` and ask the local host about a pane id it
    /// never minted. `close_pane` forgets each as it goes.
    ///
    /// **A row this host only *served* stays.** A `runs_on` project is the local catalogue's, and
    /// a drone going away is not the catalogue forgetting anything — see [`Bus::row_owner`]. Its
    /// serving comes home to the local host, which is what makes the surviving row resolve
    /// somewhere rather than to a `HostId` nothing answers for.
    pub fn drop_remote(&mut self, id: HostId) -> Vec<PaneId> {
        let host = HostRef::Remote(id);
        self.remotes.retain(|remote| remote.id != id);
        self.projects.borrow_mut().retain(|project_id, owner| {
            if *owner != host {
                return true;
            }
            if self.owned_locally.borrow().contains(project_id) {
                *owner = HostRef::Local;
                return true;
            }
            false
        });
        self.remote_meta.borrow_mut().remove(&id);
        self.agents.borrow_mut().retain(|_, owner| *owner != host);
        if self.active == host {
            self.active = HostRef::Local;
        }
        self.panes
            .borrow()
            .iter()
            .filter(|(_, owner)| **owner == host)
            .map(|(pane_id, _)| *pane_id)
            .collect()
    }

    /// How many panes this window knows `host` reported.
    pub fn pane_count(&self, host: HostRef) -> usize {
        self.panes
            .borrow()
            .values()
            .filter(|owner| **owner == host)
            .count()
    }

    /// Every project this window knows of that some host other than `host` reported.
    ///
    /// What keeps one host's catalogue answer from erasing another's: a `ProjectList` is the whole
    /// truth about the host that sent it and says nothing at all about any other, so the rows to
    /// keep are named here and handed to
    /// [`WindowRegistry::replace_all_except`](crate::state::windows::WindowRegistry).
    ///
    /// Asked of [`Bus::row_owner`] and not of `projects`, so a drone's list arrives with the
    /// `runs_on` row it also names already in `keep`: the local record is neither evicted by the
    /// drone's version of it nor duplicated beside it.
    pub fn projects_not_on(&self, host: HostRef) -> Vec<ProjectId> {
        self.projects
            .borrow()
            .keys()
            .copied()
            .filter(|project_id| self.row_owner(*project_id) != host)
            .collect()
    }

    /// Every remote this window is attached to beside the local host, with the label it was
    /// dialled under. Empty until the user has actually connected to something.
    pub fn remotes(&self) -> impl Iterator<Item = (HostId, &str)> {
        self.remotes
            .iter()
            .map(|remote| (remote.id, remote.label.as_str()))
    }

    /// The live connection behind a [`HostId`], for the remote-hosts panel.
    pub fn remote(&self, id: HostId) -> Option<&RemoteConn> {
        self.remotes.iter().find(|remote| remote.id == id)
    }

    /// Every live connection, for the remote-hosts panel.
    pub fn remote_conns(&self) -> &[RemoteConn] {
        &self.remotes
    }

    /// File what a drone advertised in its hello against its connection.
    pub fn set_remote_caps(&mut self, id: HostId, caps: Vec<String>) {
        if let Some(remote) = self.remotes.iter_mut().find(|remote| remote.id == id) {
            remote.caps = caps;
        }
    }

    /// The attached link of the saved host `save_id` that serves remote tools, or the sentence
    /// the agent reads as the tool's error.
    pub fn tool_link(&self, save_id: &str) -> Result<HostId, String> {
        let Some(conn) = self
            .remotes
            .iter()
            .find(|remote| !save_id.is_empty() && remote.save_id == save_id)
        else {
            return Err(format!("remote {save_id} is not connected"));
        };
        if conn.status != ConnStatus::Attached {
            return Err(format!("remote {} is not connected", conn.label));
        }
        if !conn.caps.iter().any(|cap| cap == CAP_TOOLS) {
            return Err(format!("remote {} does not support tools", conn.label));
        }
        Ok(conn.id)
    }

    /// The saved id of the remote serving `project_id` right now, for a project whose row is
    /// this machine's — a `runs_on` project a drone serves. `None` for one served here, or by a
    /// connection dialled from no saved entry.
    pub fn serving_save_id(&self, project_id: ProjectId) -> Option<String> {
        if self.row_owner(project_id) != HostRef::Local {
            return None;
        }
        let HostRef::Remote(id) = self.project_host(project_id) else {
            return None;
        };
        self.remotes
            .iter()
            .find(|remote| remote.id == id && !remote.save_id.is_empty())
            .map(|remote| remote.save_id.clone())
    }

    /// Whether the saved host `save_id` is attached and serves remote tools — what the new-agent
    /// modal's "Run against remote" choice offers.
    pub fn serves_tools(&self, save_id: &str) -> bool {
        self.tool_link(save_id).is_ok()
    }

    /// Relay a [`Message::RemoteToolCall`] that arrived from `origin` to the drone link its
    /// `remote` names, unchanged. With no such link the caller gets a `RemoteToolResult` error at
    /// once. Anything else is ignored.
    pub fn relay_tool_call(&self, origin: HostRef, message: Message) {
        let Message::RemoteToolCall {
            call_id,
            project_id,
            remote,
            ..
        } = &message
        else {
            return;
        };
        let (call_id, project_id) = (*call_id, *project_id);
        let link = match remote.as_deref() {
            Some(save_id) => self.tool_link(save_id),
            None => Err("this agent is not bound to a remote".to_string()),
        };
        match link {
            Ok(link) => {
                self.tool_calls.borrow_mut().insert(
                    call_id,
                    PendingToolCall {
                        origin,
                        link,
                        project_id,
                    },
                );
                self.send_to(HostRef::Remote(link), message);
            }
            Err(error) => self.send_to(
                origin,
                Message::RemoteToolResult {
                    call_id,
                    project_id,
                    result: Err(error),
                },
            ),
        }
    }

    /// Relay a [`Message::RemoteToolResult`] a drone sent back to the host its call came from,
    /// unchanged. A result for no pending call, or from a link other than the one asked, is dropped.
    pub fn relay_tool_result(&self, from: HostRef, message: Message) {
        let Message::RemoteToolResult { call_id, .. } = &message else {
            return;
        };
        let mut calls = self.tool_calls.borrow_mut();
        let HostRef::Remote(link) = from else {
            return;
        };
        if calls.get(call_id).is_none_or(|call| call.link != link) {
            return;
        }
        if let Some(call) = calls.remove(call_id) {
            drop(calls);
            self.send_to(call.origin, message);
        }
    }

    /// Answer every call still parked on `link` with an error, because the link is going.
    pub fn fail_tool_calls(&self, link: HostId) {
        let dead: Vec<(RemoteCallId, PendingToolCall)> = {
            let mut calls = self.tool_calls.borrow_mut();
            let ids: Vec<RemoteCallId> = calls
                .iter()
                .filter(|(_, call)| call.link == link)
                .map(|(id, _)| *id)
                .collect();
            ids.into_iter()
                .filter_map(|id| calls.remove(&id).map(|call| (id, call)))
                .collect()
        };
        for (call_id, call) in dead {
            self.send_to(
                call.origin,
                Message::RemoteToolResult {
                    call_id,
                    project_id: call.project_id,
                    result: Err("the remote disconnected before it answered".to_string()),
                },
            );
        }
    }

    /// Move a live connection's status. The reconnect loop owns these transitions; the panel
    /// only reads them.
    pub fn set_remote_status(&mut self, id: HostId, status: ConnStatus) {
        if let Some(remote) = self.remotes.iter_mut().find(|remote| remote.id == id) {
            remote.status = status;
        }
    }

    /// File a remote's `HostInfo` greeting under the host that sent it. Called from `receive`
    /// instead of dropping the greeting — what the manager panel and the picker's tooltips read.
    pub fn note_remote_info(&self, host: HostRef, message: &Message) {
        let HostRef::Remote(id) = host else {
            return;
        };
        if let Message::HostInfo {
            hostname,
            os,
            arch,
            triplet,
            cpu_count,
            mem_total_bytes,
            ..
        } = message
        {
            let mut meta = self.remote_meta.borrow_mut();
            let entry = meta.entry(id).or_default();
            entry.hostname = hostname.clone();
            entry.os = os.clone();
            entry.arch = arch.clone();
            entry.triplet = triplet.clone();
            entry.cpu_count = *cpu_count;
            entry.mem_total_bytes = *mem_total_bytes;
        }
    }

    /// File a remote's `Stats` answer under the host that sent it. Only remotes are filed here —
    /// the local reading still owns `state::stats::host`, which the Control screen draws.
    pub fn note_remote_stats(&self, host: HostRef, stats: &HostStats) {
        let HostRef::Remote(id) = host else {
            return;
        };
        let mut meta = self.remote_meta.borrow_mut();
        let entry = meta.entry(id).or_default();
        entry.sessions_count = Some(stats.sessions_count);
        entry.cpu_load_pct = stats.cpu_load_pct;
        entry.mem_free_bytes = stats.mem_free_bytes;
        if entry.mem_total_bytes.is_none() {
            entry.mem_total_bytes = stats.mem_total_bytes;
        }
        entry.disk_free_bytes = stats.disk_free_bytes;
    }

    /// What a remote has said about itself, if it has said anything yet.
    pub fn remote_meta(&self, host: HostRef) -> Option<RemoteHostMeta> {
        let HostRef::Remote(id) = host else {
            return None;
        };
        self.remote_meta.borrow().get(&id).cloned()
    }
}

/// One live remote connection, as the manager panel draws it.
///
/// Owned rather than borrowed: a row is built from `&AppState` and holds nothing of it once
/// drawn, the same reason `AppState::remote_hosts` returns owned strings. Painting a control
/// from `Bus` state is not the same as `Bus` itself crossing into UI code, which stays exactly
/// as forbidden as ever.
#[derive(Clone, Debug)]
pub struct LiveRemote {
    pub id: HostId,
    pub label: String,
    pub save_id: String,
    pub address: String,
    pub scheme: RemoteScheme,
    pub status: ConnStatus,
}

impl crate::app::AppState {
    /// Every live remote connection, for the manager panel.
    pub fn live_remotes(&self) -> Vec<LiveRemote> {
        self.bus
            .remote_conns()
            .iter()
            .map(|conn| LiveRemote {
                id: conn.id,
                label: conn.label.clone(),
                save_id: conn.save_id.clone(),
                address: conn.address.clone(),
                scheme: conn.scheme,
                status: conn.status.clone(),
            })
            .collect()
    }

    /// The attached remotes a new agent can run against — `(saved id, label)`, those that serve
    /// remote tools. Empty hides the new-agent modal's "Run against remote" choice.
    pub fn tool_remotes(&self) -> Vec<(String, String)> {
        self.bus
            .remote_conns()
            .iter()
            .filter(|conn| self.bus.serves_tools(&conn.save_id))
            .map(|conn| (conn.save_id.clone(), conn.label.clone()))
            .collect()
    }

    /// The saved remote a conversation in `project_id` is bound to by where the project is: the
    /// drone serving it, while that drone serves remote tools (`R7`). `None` for a project served
    /// here — a pick on the new-agent form is the only binding one of those gets.
    pub fn drone_binding(&self, project_id: ProjectId) -> Option<String> {
        self.bus
            .serving_save_id(project_id)
            .filter(|save_id| self.bus.serves_tools(save_id))
    }

    /// What a remote has said about itself, for the manager panel and the picker's tooltips.
    /// `None` for the local host and for a remote that has said nothing yet.
    pub fn remote_host_meta(&self, host: HostRef) -> Option<RemoteHostMeta> {
        self.bus.remote_meta(host)
    }

    /// Which host a project was reported by, for the picker's host badges. Unrecorded projects
    /// read as local — the catalogue they came from is the local one by construction.
    pub fn project_host(&self, project: ProjectId) -> HostRef {
        self.bus.project_host(project)
    }

    /// How many panes the live connections to saved remote `save_id` hold — what detaching it
    /// would close. Zero when it is not attached.
    pub fn saved_remote_panes(&self, save_id: &str) -> usize {
        self.bus
            .remote_conns()
            .iter()
            .filter(|conn| conn.save_id == save_id)
            .map(|conn| self.bus.pane_count(HostRef::Remote(conn.id)))
            .sum()
    }
}

/// Whether the settings page's host dropdown can offer a row, and how it draws it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum HostStatus {
    /// A live connection this window can send messages over right now — the local host, always,
    /// or a remote this session has dialled.
    Attached,
    /// A saved host with no live connection. Picking it dials it, exactly as typing its address
    /// into the connect modal would.
    NotAttached,
    /// A saved host the last dial to this address ended in `RemoteConnectStep::Failed`. Still
    /// pickable — picking it tries again — drawn differently only so the list says why nothing
    /// happened last time.
    Failed,
}

/// One row the host dropdown offers, and what picking it means.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum HostEntry {
    /// Always first, always [`HostStatus::Attached`] — see [`HostRef::Local`].
    Local,
    /// A live remote connection, named by the [`HostId`] `set_active` takes.
    Remote { host: HostId, label: String },
    /// A saved host with nothing live behind it yet — see
    /// [`ubiq_proto::settings::HostSettings::remote_hosts`]. Named by its stable id, which is
    /// what the keychain token and the reconnect loop reference; the name and address are the
    /// user's and may have changed since.
    Saved {
        id: String,
        name: String,
        address: String,
    },
}

impl HostEntry {
    /// What the row is called, before [`HostStatus`] is folded in.
    pub fn name(&self) -> &str {
        match self {
            HostEntry::Local => "Local",
            HostEntry::Remote { label, .. } => label,
            HostEntry::Saved { name, .. } => name,
        }
    }
}

/// Build the host dropdown's rows: `Local`, then every remote this window is attached to, then
/// every saved host with no live connection of its own.
///
/// **A saved host already attached is not listed twice.** Fold by [`LiveRemote::save_id`] when
/// both sides have one, and by [`LiveRemote::address`] otherwise — never by [`LiveRemote::label`],
/// which is the saved name on a manager connect. Showing both would let the same host answer to
/// two rows with two different fates for a pick.
///
/// `failed` is the set of addresses a reconnect attempt from this list most recently ended in
/// [`RemoteConnectStep::Failed`](crate::state::remote::RemoteConnectStep::Failed) for — cleared the
/// moment that address attaches, so a stale failure never outlives the connection that fixed it.
pub fn host_menu_rows(
    remotes: &[LiveRemote],
    saved: &[SavedRemoteHost],
    failed: &HashSet<String>,
) -> Vec<(HostEntry, HostStatus)> {
    let mut rows = vec![(HostEntry::Local, HostStatus::Attached)];
    rows.extend(remotes.iter().map(|remote| {
        (
            HostEntry::Remote {
                host: remote.id,
                label: remote.label.clone(),
            },
            HostStatus::Attached,
        )
    }));
    for host in saved {
        if remotes.iter().any(|remote| {
            (!host.id.is_empty() && remote.save_id == host.id) || remote.address == host.address
        }) {
            continue;
        }
        let status = if failed.contains(&host.address) {
            HostStatus::Failed
        } else {
            HostStatus::NotAttached
        };
        rows.push((
            HostEntry::Saved {
                id: host.id.clone(),
                name: host.name.clone(),
                address: host.address.clone(),
            },
            status,
        ));
    }
    rows
}

/// Which attached remote "Open remote project…" should offer, now that the Hosts section gives
/// the user a way to say which one that is.
///
/// `active` when it names an attached remote — an explicit choice made from the Hosts section
/// rather than an accident of dial order, and the same field every other unaddressed message
/// already resolves against. Falling back to the first attached remote when `active` is `Local`
/// (the common case, since nothing points it anywhere else until the dropdown is used) keeps this
/// usable the moment a single remote is attached; the ambiguity this replaces only bites once a
/// *second* remote is attached, which is exactly when the dropdown becomes the way to resolve it.
pub fn preferred_remote(active: HostRef, remotes: &[(HostId, String)]) -> Option<(HostId, String)> {
    if let HostRef::Remote(active) = active
        && let Some((id, label)) = remotes.iter().find(|(id, _)| *id == active)
    {
        return Some((*id, label.clone()));
    }
    remotes.first().cloned()
}

/// How one row reads in the dropdown: the name alone for `Local` — it is always attached, so
/// saying so would be noise — and the name plus its state for everything else.
pub fn host_row_label(entry: &HostEntry, status: HostStatus) -> String {
    if matches!(entry, HostEntry::Local) {
        return entry.name().to_string();
    }
    let state = match status {
        HostStatus::Attached => "attached",
        HostStatus::NotAttached => "not attached",
        HostStatus::Failed => "failed",
    };
    format!("{} \u{2014} {state}", entry.name())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ubiq_proto::messages::WorkspaceInfo;

    fn a_project_id() -> ProjectId {
        ProjectId::generate()
    }

    fn test_remote(
        bus: &mut Bus,
        client: Client,
        label: &str,
    ) -> (HostId, flume::Receiver<Message>) {
        bus.register_remote(
            client,
            label.to_string(),
            String::new(),
            label.to_string(),
            RemoteScheme::Http,
        )
    }

    /// With only the local host attached — today's whole world — every route resolves to it,
    /// whatever the message names or fails to name.
    #[test]
    fn only_local_attached_routes_everywhere_to_local() {
        let (client, _detached) = ubiq_proto::bus::detached();
        let bus = Bus::new(client);

        assert_eq!(bus.resolve(&Message::ListProjects), HostRef::Local);

        let project_id = a_project_id();
        bus.note_project(project_id, HostRef::Local);
        assert_eq!(
            bus.resolve(&Message::RefreshProject { project_id }),
            HostRef::Local
        );

        let pane_id = PaneId::generate();
        bus.note_pane(pane_id, HostRef::Local);
        assert_eq!(bus.resolve(&Message::Focus { pane_id }), HostRef::Local);
    }

    /// A message that names a project routes to whatever host that project was recorded under,
    /// not to whatever happens to be active.
    #[test]
    fn project_message_routes_to_the_projects_host() {
        let (local, local_end) = ubiq_proto::bus::detached();
        let (remote_client, remote_end) = ubiq_proto::bus::detached();
        let mut bus = Bus::new(local);
        let (remote, _from_host) = test_remote(&mut bus, remote_client, "test-remote");

        let project_id = a_project_id();
        bus.note_project(project_id, HostRef::Remote(remote));

        bus.send(Message::RefreshProject { project_id });

        assert!(matches!(
            remote_end.said().try_recv(),
            Ok(ubiq_proto::bus::FromClient::Said {
                message: Message::RefreshProject { .. },
                ..
            })
        ));
        assert!(local_end.said().try_recv().is_err());
    }

    /// A message that names a pane routes to that pane's host, even when the message also carries
    /// a project that belongs to a different one — the pane is more specific, and is checked
    /// first.
    #[test]
    fn pane_message_routes_to_the_panes_host() {
        let (local, local_end) = ubiq_proto::bus::detached();
        let (remote_client, remote_end) = ubiq_proto::bus::detached();
        let mut bus = Bus::new(local);
        let (remote, _from_host) = test_remote(&mut bus, remote_client, "test-remote");

        let project_id = a_project_id();
        bus.note_project(project_id, HostRef::Local);

        let workspace = WorkspaceInfo {
            id: PaneId::generate(),
            session_id: ubiq_proto::ids::SessionId::generate(),
            agent_type: "shell".to_string(),
            project_id,
            rel_path: None,
            cols: 80,
            rows: 24,
            running: true,
            wait_on_exit: false,
            wait_on_error: false,
            tool: None,
            handle: None,
        };
        bus.note_pane(workspace.id, HostRef::Remote(remote));

        bus.send(Message::CloseWorkspace {
            pane_id: workspace.id,
        });

        assert!(matches!(
            remote_end.said().try_recv(),
            Ok(ubiq_proto::bus::FromClient::Said {
                message: Message::CloseWorkspace { .. },
                ..
            })
        ));
        assert!(local_end.said().try_recv().is_err());
    }

    /// A message naming neither a pane nor a project goes to whichever host is active.
    #[test]
    fn unowned_message_routes_to_active() {
        let (local, local_end) = ubiq_proto::bus::detached();
        let (remote_client, remote_end) = ubiq_proto::bus::detached();
        let mut bus = Bus::new(local);
        let (remote, _from_host) = test_remote(&mut bus, remote_client, "test-remote");
        bus.set_active(HostRef::Remote(remote));

        bus.send(Message::ListProjects);

        assert!(matches!(
            remote_end.said().try_recv(),
            Ok(ubiq_proto::bus::FromClient::Said {
                message: Message::ListProjects,
                ..
            })
        ));
        assert!(local_end.said().try_recv().is_err());
    }

    /// `send_to` overrides resolution entirely, reaching a host that owns neither the pane nor the
    /// project named in the message, and that is not active either.
    #[test]
    fn send_to_overrides_resolution() {
        let (local, local_end) = ubiq_proto::bus::detached();
        let (remote_client, remote_end) = ubiq_proto::bus::detached();
        let mut bus = Bus::new(local);
        let (remote, _from_host) = test_remote(&mut bus, remote_client, "test-remote");

        let project_id = a_project_id();
        bus.note_project(project_id, HostRef::Local);

        bus.send_to(
            HostRef::Remote(remote),
            Message::RefreshProject { project_id },
        );

        assert!(matches!(
            remote_end.said().try_recv(),
            Ok(ubiq_proto::bus::FromClient::Said {
                message: Message::RefreshProject { .. },
                ..
            })
        ));
        assert!(local_end.said().try_recv().is_err());
    }

    fn tool_call(call_id: RemoteCallId, project_id: ProjectId, remote: &str) -> Message {
        Message::RemoteToolCall {
            call_id,
            project_id,
            remote: Some(remote.to_string()),
            root: None,
            op: ubiq_proto::remote_tools::RemoteOp::Info,
        }
    }

    fn drone(bus: &mut Bus, save_id: &str, caps: &[&str]) -> (HostId, ubiq_proto::bus::Detached) {
        let (client, end) = ubiq_proto::bus::detached();
        let (id, _from_host) = bus.register_remote(
            client,
            save_id.to_string(),
            save_id.to_string(),
            save_id.to_string(),
            RemoteScheme::Http,
        );
        bus.set_remote_caps(id, caps.iter().map(|c| c.to_string()).collect());
        (id, end)
    }

    /// A call goes unchanged to the link whose saved id it names, and the result returns to the
    /// host the call came from — once.
    #[test]
    fn tool_call_relays_to_the_named_link_and_back() {
        let (local, local_end) = ubiq_proto::bus::detached();
        let mut bus = Bus::new(local);
        let (_other, other_end) = drone(&mut bus, "a", &[CAP_TOOLS]);
        let (link, link_end) = drone(&mut bus, "b", &[CAP_TOOLS]);
        let (call_id, project_id) = (RemoteCallId::generate(), a_project_id());
        let call = tool_call(call_id, project_id, "b");

        bus.relay_tool_call(HostRef::Local, call.clone());
        assert!(matches!(
            link_end.said().try_recv(),
            Ok(ubiq_proto::bus::FromClient::Said { message, .. }) if format!("{message:?}") == format!("{call:?}")
        ));
        assert!(other_end.said().try_recv().is_err());

        let result = Message::RemoteToolResult {
            call_id,
            project_id,
            result: Err("x".into()),
        };
        bus.relay_tool_result(HostRef::Remote(link), result.clone());
        assert!(matches!(
            local_end.said().try_recv(),
            Ok(ubiq_proto::bus::FromClient::Said { message, .. }) if format!("{message:?}") == format!("{result:?}")
        ));
        bus.relay_tool_result(HostRef::Remote(link), result);
        assert!(local_end.said().try_recv().is_err());
    }

    /// No attached link, or one without the tools capability, answers the caller at once.
    #[test]
    fn tool_call_without_a_tool_link_errors_back() {
        let (local, local_end) = ubiq_proto::bus::detached();
        let mut bus = Bus::new(local);
        let (_id, link_end) = drone(&mut bus, "b", &[]);
        for remote in ["missing", "b"] {
            bus.relay_tool_call(
                HostRef::Local,
                tool_call(RemoteCallId::generate(), a_project_id(), remote),
            );
            assert!(matches!(
                local_end.said().try_recv(),
                Ok(ubiq_proto::bus::FromClient::Said {
                    message: Message::RemoteToolResult { result: Err(_), .. },
                    ..
                })
            ));
        }
        assert!(link_end.said().try_recv().is_err());
    }

    fn a_saved_host(name: &str, address: &str) -> SavedRemoteHost {
        SavedRemoteHost {
            id: String::new(),
            name: name.to_string(),
            address: address.to_string(),
            scheme: RemoteScheme::Http,
            trust_insecure: false,
            carrier: ubiq_proto::settings::RemoteCarrier::Socket,
            tunnels: Vec::new(),
            tunnel_rate_bps: None,
            tls_pin: None,
            tls_pin_rev: 0,
        }
    }

    fn attached_row(id: HostId, label: &str, save_id: &str, address: &str) -> LiveRemote {
        LiveRemote {
            id,
            label: label.to_string(),
            save_id: save_id.to_string(),
            address: address.to_string(),
            scheme: RemoteScheme::Http,
            status: ConnStatus::Attached,
        }
    }

    /// With nothing attached and nothing saved, the dropdown is `Local` alone.
    #[test]
    fn with_nothing_else_the_list_is_local_alone() {
        let rows = host_menu_rows(&[], &[], &HashSet::new());
        assert_eq!(rows, vec![(HostEntry::Local, HostStatus::Attached)]);
    }

    /// Local first, then every attached remote, then every saved host with no live connection —
    /// in that order, whatever order the inputs themselves came in.
    #[test]
    fn rows_are_ordered_local_then_attached_then_saved() {
        let (local, _local_end) = ubiq_proto::bus::detached();
        let mut bus = Bus::new(local);
        let (remote, _) = test_remote(&mut bus, ubiq_proto::bus::detached().0, "10.0.0.4:7420");

        let saved = vec![a_saved_host("build box", "build.internal:7420")];
        let rows = host_menu_rows(
            &[attached_row(remote, "10.0.0.4:7420", "", "10.0.0.4:7420")],
            &saved,
            &HashSet::new(),
        );

        assert_eq!(
            rows,
            vec![
                (HostEntry::Local, HostStatus::Attached),
                (
                    HostEntry::Remote {
                        host: remote,
                        label: "10.0.0.4:7420".to_string()
                    },
                    HostStatus::Attached
                ),
                (
                    HostEntry::Saved {
                        id: String::new(),
                        name: "build box".to_string(),
                        address: "build.internal:7420".to_string()
                    },
                    HostStatus::NotAttached
                ),
            ]
        );
    }

    /// A saved host already reached over a live connection is folded into that attached row
    /// rather than drawn a second time as "not attached".
    #[test]
    fn a_saved_host_that_is_attached_is_not_listed_twice() {
        let (local, _local_end) = ubiq_proto::bus::detached();
        let mut bus = Bus::new(local);
        let (remote, _) = test_remote(&mut bus, ubiq_proto::bus::detached().0, "10.0.0.4:7420");

        let saved = vec![a_saved_host("office desktop", "10.0.0.4:7420")];
        let rows = host_menu_rows(
            &[attached_row(remote, "10.0.0.4:7420", "", "10.0.0.4:7420")],
            &saved,
            &HashSet::new(),
        );

        assert_eq!(rows.len(), 2);
        assert!(
            !rows
                .iter()
                .any(|(entry, _)| matches!(entry, HostEntry::Saved { .. }))
        );
    }

    /// A live connection labelled with the saved name still folds the saved row — folding
    /// by label==address would have listed it twice.
    #[test]
    fn a_named_attached_host_is_not_listed_twice() {
        let (local, _local_end) = ubiq_proto::bus::detached();
        let mut bus = Bus::new(local);
        let (remote, _) = test_remote(&mut bus, ubiq_proto::bus::detached().0, "office desktop");

        let mut saved = a_saved_host("office desktop", "10.0.0.4:7420");
        saved.id = "01ARZ3NDEKTSV4RRFFQ69G5FAV".to_string();
        let rows = host_menu_rows(
            &[attached_row(
                remote,
                "office desktop",
                "01ARZ3NDEKTSV4RRFFQ69G5FAV",
                "10.0.0.4:7420",
            )],
            &[saved],
            &HashSet::new(),
        );

        assert_eq!(rows.len(), 2);
        assert!(
            !rows
                .iter()
                .any(|(entry, _)| matches!(entry, HostEntry::Saved { .. }))
        );
    }

    /// A saved host whose address is in the failed set is drawn `Failed`, and one that is not
    /// stays `NotAttached` — the only two states a saved-but-unattached row can be in.
    #[test]
    fn a_failed_address_is_marked_failed_others_are_not_attached() {
        let saved = vec![
            a_saved_host("flaky box", "flaky.internal:7420"),
            a_saved_host("build box", "build.internal:7420"),
        ];
        let failed: HashSet<String> = ["flaky.internal:7420".to_string()].into_iter().collect();
        let rows = host_menu_rows(&[], &saved, &failed);

        assert_eq!(
            rows,
            vec![
                (HostEntry::Local, HostStatus::Attached),
                (
                    HostEntry::Saved {
                        id: String::new(),
                        name: "flaky box".to_string(),
                        address: "flaky.internal:7420".to_string()
                    },
                    HostStatus::Failed
                ),
                (
                    HostEntry::Saved {
                        id: String::new(),
                        name: "build box".to_string(),
                        address: "build.internal:7420".to_string()
                    },
                    HostStatus::NotAttached
                ),
            ]
        );
    }

    /// `Local` reads bare; every other row carries its state as a suffix.
    #[test]
    fn labels_carry_state_for_everything_but_local() {
        assert_eq!(
            host_row_label(&HostEntry::Local, HostStatus::Attached),
            "Local"
        );
        assert_eq!(
            host_row_label(
                &HostEntry::Saved {
                    id: String::new(),
                    name: "build box".to_string(),
                    address: "build.internal:7420".to_string()
                },
                HostStatus::Failed
            ),
            "build box \u{2014} failed"
        );
    }

    /// `set_active` changes only what an unaddressed message resolves to. A pane and a project
    /// already recorded under one host stay routed there after `active` moves to a different
    /// host entirely — the Hosts section's dropdown must never look like it just relocated
    /// something that was already open.
    #[test]
    fn set_active_does_not_move_an_existing_pane_or_project() {
        let (local, local_end) = ubiq_proto::bus::detached();
        let (remote_client, remote_end) = ubiq_proto::bus::detached();
        let mut bus = Bus::new(local);
        let (remote, _from_host) = test_remote(&mut bus, remote_client, "test-remote");

        let project_id = a_project_id();
        bus.note_project(project_id, HostRef::Local);
        let pane_id = PaneId::generate();
        bus.note_pane(pane_id, HostRef::Local);

        // Move `active` to the remote — the dropdown's whole effect.
        bus.set_active(HostRef::Remote(remote));
        assert_eq!(bus.active(), HostRef::Remote(remote));

        // The project and the pane, both recorded under Local before `active` moved, still
        // resolve there.
        bus.send(Message::RefreshProject { project_id });
        bus.send(Message::Focus { pane_id });

        let mut local_messages = Vec::new();
        while let Ok(ubiq_proto::bus::FromClient::Said { message, .. }) =
            local_end.said().try_recv()
        {
            local_messages.push(message);
        }
        assert!(matches!(local_messages[0], Message::RefreshProject { .. }));
        assert!(matches!(local_messages[1], Message::Focus { .. }));
        assert!(remote_end.said().try_recv().is_err());
    }

    /// Losing a host takes everything recorded under it with it: its projects are forgotten, its
    /// panes are handed back for the caller to close, and `active` — which pointed at it — is put
    /// back on the local host so an unaddressed message has somewhere real to go.
    #[test]
    fn drop_remote_forgets_the_hosts_projects_and_resets_active() {
        let (local, _local_end) = ubiq_proto::bus::detached();
        let mut bus = Bus::new(local);
        let (remote, _) = test_remote(&mut bus, ubiq_proto::bus::detached().0, "gone");

        let theirs = a_project_id();
        let ours = a_project_id();
        bus.note_project(theirs, HostRef::Remote(remote));
        bus.note_project(ours, HostRef::Local);
        let pane_id = PaneId::generate();
        bus.note_pane(pane_id, HostRef::Remote(remote));
        bus.set_active(HostRef::Remote(remote));

        assert_eq!(bus.drop_remote(remote), vec![pane_id]);

        assert_eq!(bus.active(), HostRef::Local);
        assert_eq!(bus.remotes().count(), 0);
        // The local host's project survives; the remote's is gone.
        assert_eq!(bus.projects_not_on(HostRef::Local), Vec::<ProjectId>::new());
        assert_eq!(bus.projects_not_on(HostRef::Remote(remote)), vec![ours]);
    }

    /// The one row in the bus a host does not own wholesale: a project the local catalogue named
    /// and a drone serves under the same id. The drone's list must not be treated as this row's
    /// truth, so `projects_not_on` names it as something the drone's answer has no opinion about.
    #[test]
    fn a_drone_serving_a_local_row_does_not_come_to_own_it() {
        let (local, _local_end) = ubiq_proto::bus::detached();
        let mut bus = Bus::new(local);
        let (drone, _) = test_remote(&mut bus, ubiq_proto::bus::detached().0, "drone");

        let pinned = a_project_id();
        let theirs = a_project_id();
        bus.note_project(pinned, HostRef::Local);
        // The drone announces the same folder under the id `--root-id` gave it, plus one of its
        // own.
        bus.note_project(pinned, HostRef::Remote(drone));
        bus.note_project(theirs, HostRef::Remote(drone));

        // Serving moved — that is what makes a pane open on the far machine.
        assert_eq!(bus.project_host(pinned), HostRef::Remote(drone));
        // Ownership did not.
        assert_eq!(bus.row_owner(pinned), HostRef::Local);
        // So the drone's own catalogue answer is told to keep the row rather than replace it.
        assert_eq!(bus.projects_not_on(HostRef::Remote(drone)), vec![pinned]);
        assert_eq!(bus.projects_not_on(HostRef::Local), vec![theirs]);

        // And the local host re-listing does not take the folder back off the drone.
        bus.note_project(pinned, HostRef::Local);
        assert_eq!(bus.project_host(pinned), HostRef::Remote(drone));

        // The record's own messages go to the row's owner; the folder's go to the drone.
        assert_eq!(
            bus.resolve(&Message::OpenedProject { project_id: pinned }),
            HostRef::Local
        );
        assert_eq!(
            bus.resolve(&Message::RefreshProject { project_id: pinned }),
            HostRef::Local
        );
        assert_eq!(
            bus.resolve(&Message::ProjectTree {
                project_id: pinned,
                rel_path: String::new(),
                depth: 1,
                prefetch: false,
            }),
            HostRef::Remote(drone)
        );
        // The work is the owner's too: its agents run here, and a drone keeps no tasks.
        assert_eq!(
            bus.resolve(&Message::ListWork { project_id: pinned }),
            HostRef::Local
        );
        assert_eq!(
            bus.resolve(&Message::ListWork { project_id: theirs }),
            HostRef::Remote(drone)
        );
        // A drone's own row is the drone's, record and all.
        assert_eq!(
            bus.resolve(&Message::OpenedProject { project_id: theirs }),
            HostRef::Remote(drone)
        );
    }

    /// An agent in a project a drone serves is a harness on the row owner's machine, bound to
    /// that drone (`R7`): the start goes to the local host, and the drone it binds is the one
    /// serving the row. A drone's own row stays the drone's.
    #[test]
    fn a_conversation_in_a_drone_served_row_starts_locally_bound_to_its_drone() {
        let (local, _local_end) = ubiq_proto::bus::detached();
        let mut bus = Bus::new(local);
        let (drone, _) = bus.register_remote(
            ubiq_proto::bus::detached().0,
            "drone".to_string(),
            "rdp-box".to_string(),
            String::new(),
            RemoteScheme::Http,
        );

        let pinned = a_project_id();
        let theirs = a_project_id();
        bus.note_project(pinned, HostRef::Local);
        bus.note_project(pinned, HostRef::Remote(drone));
        bus.note_project(theirs, HostRef::Remote(drone));

        let revive = |project_id| Message::ReviveConversation {
            source: AgentId::generate(),
            agent_id: AgentId::generate(),
            project_id,
            session_id: ubiq_proto::ids::SessionId::generate(),
            remote: None,
        };
        assert_eq!(bus.resolve(&revive(pinned)), HostRef::Local);
        assert_eq!(bus.resolve(&revive(theirs)), HostRef::Remote(drone));

        assert_eq!(bus.serving_save_id(pinned), Some("rdp-box".to_string()));
        assert_eq!(bus.serving_save_id(theirs), None);
        let local_only = a_project_id();
        bus.note_project(local_only, HostRef::Local);
        assert_eq!(bus.serving_save_id(local_only), None);
    }

    /// A conversation's follow-ups name no project, so they follow the host that runs it — not
    /// the Hosts dropdown. One this window was never told about still falls back to `active`.
    #[test]
    fn a_conversation_follow_up_goes_where_the_conversation_runs() {
        let (local, _local_end) = ubiq_proto::bus::detached();
        let mut bus = Bus::new(local);
        let (remote, _) = test_remote(&mut bus, ubiq_proto::bus::detached().0, "far");
        bus.set_active(HostRef::Remote(remote));

        let ours = AgentId::generate();
        bus.note_agent(ours, HostRef::Local);
        let cancel = |agent_id| Message::CancelTurn { agent_id };
        assert_eq!(bus.resolve(&cancel(ours)), HostRef::Local);
        assert_eq!(
            bus.resolve(&cancel(AgentId::generate())),
            HostRef::Remote(remote)
        );

        // A remote's conversations go with it.
        let theirs = AgentId::generate();
        bus.note_agent(theirs, HostRef::Remote(remote));
        bus.drop_remote(remote);
        assert_eq!(bus.resolve(&cancel(theirs)), HostRef::Local);
    }

    /// A drone going away is not the catalogue forgetting anything: the row it was serving stays,
    /// resolves to the local host again, and is named to the caller so the picker can mark it.
    #[test]
    fn dropping_a_drone_keeps_the_row_it_was_only_serving() {
        let (local, _local_end) = ubiq_proto::bus::detached();
        let mut bus = Bus::new(local);
        let (drone, _) = test_remote(&mut bus, ubiq_proto::bus::detached().0, "drone");

        let pinned = a_project_id();
        let theirs = a_project_id();
        bus.note_project(pinned, HostRef::Local);
        bus.note_project(pinned, HostRef::Remote(drone));
        bus.note_project(theirs, HostRef::Remote(drone));

        assert_eq!(bus.local_rows_served_by(drone), vec![pinned]);
        bus.drop_remote(drone);

        // The pinned row survives and comes home; the drone's own row goes with it.
        assert_eq!(bus.project_host(pinned), HostRef::Local);
        assert_eq!(bus.row_owner(pinned), HostRef::Local);
        assert_eq!(bus.projects_not_on(HostRef::Local), Vec::<ProjectId>::new());
    }

    /// The poison this replaces: after a remote goes, its pane ids mean nothing to the local
    /// host, so a message still addressed to it is dropped rather than delivered there.
    #[test]
    fn a_dropped_remotes_messages_do_not_land_on_the_local_host() {
        let (local, local_end) = ubiq_proto::bus::detached();
        let mut bus = Bus::new(local);
        let (remote, _) = test_remote(&mut bus, ubiq_proto::bus::detached().0, "gone");

        let pane_id = PaneId::generate();
        bus.note_pane(pane_id, HostRef::Remote(remote));
        bus.drop_remote(remote);

        // The pane is still recorded — `close_pane` forgets it — so this resolves to the host
        // that is no longer there, which is exactly the case that must go nowhere.
        bus.send(Message::CloseWorkspace { pane_id });
        bus.send_to(HostRef::Remote(remote), Message::ListProjects);

        assert!(local_end.said().try_recv().is_err());
    }

    /// With nothing attached, there is nothing to prefer.
    #[test]
    fn preferred_remote_is_none_with_nothing_attached() {
        assert_eq!(preferred_remote(HostRef::Local, &[]), None);
    }

    /// `active` naming an attached remote is preferred over the first one by attach order.
    #[test]
    fn preferred_remote_is_active_when_active_is_a_remote() {
        let (local, _local_end) = ubiq_proto::bus::detached();
        let mut bus = Bus::new(local);
        let (first, _) = test_remote(&mut bus, ubiq_proto::bus::detached().0, "first");
        let (second, _) = test_remote(&mut bus, ubiq_proto::bus::detached().0, "second");
        let remotes = vec![(first, "first".to_string()), (second, "second".to_string())];

        assert_eq!(
            preferred_remote(HostRef::Remote(second), &remotes),
            Some((second, "second".to_string()))
        );
    }

    /// `active` is `Local` — the common case — so this falls back to the first attached remote,
    /// exactly the attach-order rule the old stopgap used before there was a choice to make.
    #[test]
    fn preferred_remote_falls_back_to_the_first_attached_when_active_is_local() {
        let (local, _local_end) = ubiq_proto::bus::detached();
        let mut bus = Bus::new(local);
        let (first, _) = test_remote(&mut bus, ubiq_proto::bus::detached().0, "first");
        let remotes = vec![(first, "first".to_string())];

        assert_eq!(
            preferred_remote(HostRef::Local, &remotes),
            Some((first, "first".to_string()))
        );
    }
}
