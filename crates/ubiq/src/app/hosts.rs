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
use ubiq_proto::ids::{PaneId, ProjectId};
use ubiq_proto::messages::Message;
use ubiq_proto::settings::{RemoteScheme, SavedRemoteHost};
use ubiq_proto::stats::HostStats;

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
        | Message::HarnessLoginStarted { pane_id, .. }
        | Message::HarnessLoginLink { pane_id, .. } => Some(*pane_id),
        Message::WorkspaceSpawned { workspace } => Some(workspace.id),
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
    /// What each attached remote said about itself — its `HostInfo` greeting plus the latest
    /// `Stats` poll. Same shape and same reason as `panes`: written from `receive`, read from
    /// `render`, without taking `&mut`.
    remote_meta: RefCell<HashMap<HostId, RemoteHostMeta>>,
    /// Mints the next [`HostId`]. Process-local to this `Bus`, which is fine — a `HostId` is only
    /// ever compared against other ids this same `Bus` minted.
    next_host_id: AtomicU64,
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
            remote_meta: RefCell::new(HashMap::new()),
            next_host_id: AtomicU64::new(0),
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
        if let Some(project_id) = message.project_id()
            && let Some(host) = self.projects.borrow().get(&project_id)
        {
            return *host;
        }
        self.active
    }

    /// Say something to whichever host owns it. Exactly `Client::send`'s signature, so every
    /// existing `bus.send(...)` call site compiles unchanged; what differs is that the message may
    /// now be resolved to one of several hosts instead of the one there used to be.
    pub fn send(&self, message: Message) {
        let host = self.resolve(&message);
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

    /// Forget a pane, once it has closed. Not required for correctness — an unresolved pane falls
    /// back to `active`, and a closed pane's id is never sent again — but kept so the map does not
    /// grow for the life of the window.
    pub fn forget_pane(&self, pane_id: PaneId) {
        self.panes.borrow_mut().remove(&pane_id);
    }

    /// Record which host a project belongs to, on the same terms as [`Bus::note_pane`].
    pub fn note_project(&self, project_id: ProjectId, host: HostRef) {
        self.projects.borrow_mut().insert(project_id, host);
    }

    /// Forget a project, once it has been forgotten by the catalogue too.
    pub fn forget_project(&self, project_id: ProjectId) {
        self.projects.borrow_mut().remove(&project_id);
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
    pub fn drop_remote(&mut self, id: HostId) -> Vec<PaneId> {
        let host = HostRef::Remote(id);
        self.remotes.retain(|remote| remote.id != id);
        self.projects.borrow_mut().retain(|_, owner| *owner != host);
        self.remote_meta.borrow_mut().remove(&id);
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

    /// Every project this window knows of that some host other than `host` reported.
    ///
    /// What keeps one host's catalogue answer from erasing another's: a `ProjectList` is the whole
    /// truth about the host that sent it and says nothing at all about any other, so the rows to
    /// keep are named here and handed to
    /// [`WindowRegistry::replace_all_except`](crate::state::windows::WindowRegistry).
    pub fn projects_not_on(&self, host: HostRef) -> Vec<ProjectId> {
        self.projects
            .borrow()
            .iter()
            .filter(|(_, owner)| **owner != host)
            .map(|(project_id, _)| *project_id)
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

    fn a_saved_host(name: &str, address: &str) -> SavedRemoteHost {
        SavedRemoteHost {
            id: String::new(),
            name: name.to_string(),
            address: address.to_string(),
            scheme: RemoteScheme::Http,
            trust_insecure: false,
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
