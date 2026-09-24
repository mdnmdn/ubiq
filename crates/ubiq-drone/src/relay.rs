//! The run loop that stands in for the coordinator.
//!
//! It is deliberately *not* a smaller coordinator: it is a different thing with a smaller job.
//! The coordinator owns conversations, accounts, quotas, version control, an index and a
//! notification centre, and it needs every feature of the host crate to do it. A drone owns four
//! things — the pseudo-terminals it opened, a catalogue that lives only in memory, the worker that
//! answers the file family, and the worker that shells out for the search family — and that is the
//! whole of its state.
//!
//! **Every message gets an answer.** What this relay cannot serve is refused with the error
//! variant of its own family, never dropped: a drone is reached over a stream where silence is
//! indistinguishable from a lost connection, and an interface waiting forever on a reply it will
//! never get is the worst failure this feature can have. The catch-all arm at the end of
//! [`Relay::dispatch`] is what makes that true for a message written after this file was.
//!
//! It renders nothing, and terminal bytes stay opaque.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ubiq_host::files::{self, Files};
use ubiq_host::host_meta::HostMeta;
use ubiq_host::host_path::wire_string;
use ubiq_host::{health, host_meta, pty, shells};
use ubiq_proto::bus::{ClientId, FromClient, HostEnd, To};
use ubiq_proto::ids::{PaneId, ProjectId, SearchId};
use ubiq_proto::messages::{Message, WorkspaceInfo};
use ubiq_proto::projects::{ProjectRecord, ProjectSnapshot};

use crate::linger::Live;
use crate::scrollback::Scrollback;
use crate::search::{self, Cancel, Search};

/// One `--root`, and the id the interface already minted for it, if any.
///
/// A root given bare gets a fresh [`ProjectId::generate`], as it always did. A root given with
/// `--root-id` announces itself under that id instead, which is what lets a project the interface
/// launched a drone for stay *one* row in its catalogue rather than a second one alongside it —
/// the drone does not know it is being asked for that; it only obeys the id it was handed.
#[derive(Debug, Clone)]
pub struct Root {
    pub path: PathBuf,
    pub id: Option<ProjectId>,
}

impl Root {
    /// A root with no id, which is every root today's `--root` alone still produces.
    pub fn new(path: PathBuf) -> Self {
        Self { path, id: None }
    }
}

/// The geometry a pane starts at, before the interface has measured its own bounds. The same
/// numbers the coordinator uses, for the same reason: the truth arrives a frame later as a
/// `TerminalResize`.
const INITIAL_COLS: u16 = 80;
const INITIAL_ROWS: u16 = 24;

/// How often a **detached** drone wakes to ask whether its linger has run out. The countdown is
/// started by a client leaving — which is what the heartbeat's three missed pings turn a dead
/// link into — so this only bounds how late the exit is, never whether it happens.
const TICK: Duration = Duration::from_millis(250);

/// How long an **attached** drone waits on the bus before looking around. There is nothing for it
/// to look at: it has no countdown, and the end of its stream arrives as a disconnect.
const IDLE: Duration = Duration::from_secs(3600);

/// How long a drone that has **never** had a client waits before giving up, whatever its linger
/// says. `--linger 0` means "go when the last client leaves", and a drone that was started to be
/// attached to must not exit in the gap before the first one arrives; a drone nobody ever attaches
/// to must not become a resident process either.
const STARTUP_GRACE: Duration = Duration::from_secs(30);

/// What a drone that outlives its link is holding.
struct Holding {
    /// Shared with the accept loop and the state file: `--linger` is re-asserted on every attach,
    /// `panes` is kept true here, and `--stop` sets a flag this checks rather than acting on the
    /// bus's own thread.
    live: Arc<Live>,
    /// When the last client left; `None` while one is attached.
    alone_since: Option<Instant>,
    /// Whether a client has ever been here. Until one has, [`STARTUP_GRACE`] is the rule.
    ever: bool,
}

/// One machine, served.
pub struct Relay {
    files: Files,
    search: Search,
    /// One live search per project, the same rule `Coordinator::active_searches` keeps: a second
    /// `SearchProject` for a project supersedes the first, and `CancelSearch` is only honoured
    /// against the search it names. The flag inside `Cancel` also means "this search is over", so
    /// an entry is reaped the next time a search is asked for rather than on its own timer.
    active_searches: HashMap<ProjectId, (SearchId, Cancel)>,
    meta: Arc<Mutex<HostMeta>>,
    /// The catalogue, in memory and nowhere else.
    ///
    /// A plain `Vec` rather than a `ProjectStore` behind it, because a drone has no *write* half
    /// to the catalogue at all: it is seeded from `--root` and never added to, renamed, re-pointed
    /// or forgotten — those are the user's decisions about their own machine and stay with the
    /// coordinator. A store would be a trait object with one implementation and nothing that ever
    /// calls `upsert`.
    records: Vec<ProjectRecord>,
    /// Where this drone would keep disposable files if it kept any. Reported as the config root
    /// and as the shared workarea, and sampled for free space — it is not a config root in the
    /// sense a local host has one, because there is no configuration to be in it.
    scratch: PathBuf,
    panes: HashMap<PaneId, pty::Pty>,
    owners: HashMap<PaneId, ClientId>,
    focused: HashMap<ClientId, PaneId>,
    /// Who is attached right now. One client in the ordinary case; the set is what says a
    /// detached drone's countdown may start, which is a question about the *last* one to leave.
    here: HashSet<ClientId>,
    /// What each live pane was announced as, so a client attaching to a drone that already has
    /// panes can be told about them. A reattach is a new host attach, not a resumed one: nothing
    /// carries over but the pane's id, its process, and its ring.
    announced: HashMap<PaneId, WorkspaceInfo>,
    /// Every pane's output, on its way to whoever is attached, and kept in a bounded ring for
    /// whoever attaches next.
    screens: Scrollback,
    /// Absent when the stream *is* the session — `--stdio`, where a client leaving is the end of
    /// everything this drone was for.
    holding: Option<Holding>,
}

impl Relay {
    /// Build the relay, with one project per `--root`.
    ///
    /// A root that is not a directory is still taken into the catalogue: the interface shows it
    /// with the health the probe gave it, which is the same thing a local host does for a project
    /// whose folder has gone. Refusing here would leave the user with no row to look at.
    pub fn new(roots: Vec<Root>) -> Self {
        Self::build(roots, None)
    }

    /// The same relay, detached: its panes outlive the link that opened them, and `linger` — which
    /// an attach may re-assert at any time — is what decides how long they outlive the last one.
    pub fn holding(roots: Vec<Root>, live: Arc<Live>) -> Self {
        Self::build(
            roots,
            Some(Holding {
                live,
                // A drone that has never had a client is already alone: a link that never
                // arrives must not leave a process nobody knows to go and kill.
                alone_since: Some(Instant::now()),
                ever: false,
            }),
        )
    }

    fn build(roots: Vec<Root>, holding: Option<Holding>) -> Self {
        let scratch = std::env::temp_dir().join(format!("ubiq-drone-{}", std::process::id()));
        if let Err(error) = std::fs::create_dir_all(&scratch) {
            tracing::warn!("could not reserve {}: {error}", scratch.display());
        }

        // No `--root` is a drone with an empty catalogue, not an error: it can still serve a
        // terminal and browse the machine, which is what the host-browse family is for.
        let records = roots.iter().map(record_for).collect();
        Self {
            files: Files::start(),
            search: Search::start(),
            active_searches: HashMap::new(),
            meta: host_meta::start(scratch.clone()),
            records,
            scratch,
            panes: HashMap::new(),
            owners: HashMap::new(),
            focused: HashMap::new(),
            here: HashSet::new(),
            announced: HashMap::new(),
            // Only a drone that outlives its link has anything to replay.
            screens: Scrollback::start(holding.is_some()),
            holding,
        }
    }

    /// Answer the bus until it closes, or until a detached drone has been alone for too long.
    ///
    /// The bus closes when the carrier's stream has ended and the hub behind it has been dropped,
    /// which for `--stdio` is the whole of the lifecycle. A detached drone's hub outlives every
    /// link, so what ends it instead is the linger: [`Self::shutdown`] runs on the way out of
    /// either, and there is no third path — see `carrier` for why a second one is a mistake.
    pub fn run(mut self, host: HostEnd) {
        let wait = if self.holding.is_some() { TICK } else { IDLE };
        loop {
            match host.recv_timeout(wait) {
                Ok(FromClient::Connected(client)) => self.client_here(&host, client),
                Ok(FromClient::Said { client, message }) => self.dispatch(&host, client, message),
                Ok(FromClient::Gone(client)) => self.client_gone(client),
                Err(flume::RecvTimeoutError::Timeout) => {}
                Err(flume::RecvTimeoutError::Disconnected) => break,
            }
            // Kept true on every tick, whether or not this one changed it, so a state file
            // written between ticks never reports a pane count nobody here asked for.
            if let Some(holding) = self.holding.as_ref() {
                holding.live.set_panes(self.panes.len());
            }
            if self.should_stop() {
                tracing::info!("nobody attached within the linger: killing the panes and going");
                break;
            }
        }
        self.shutdown();
    }

    /// Whether this drone should end now: `--stop` was asked for, or it has waited alone for as
    /// long as it was told to.
    ///
    /// A stop is checked first and unconditionally: it is a direct order over the same socket a
    /// linger is re-asserted on, and honouring it a tick late for the sake of the countdown would
    /// make `--stop` a request rather than the immediate exit its name promises.
    fn should_stop(&self) -> bool {
        let Some(holding) = self.holding.as_ref() else {
            return false;
        };
        if holding.live.stopping() {
            return true;
        }
        let Some(since) = holding.alone_since else {
            return false;
        };
        if !holding.ever {
            return since.elapsed() >= STARTUP_GRACE;
        }
        holding.live.linger().expired(since.elapsed())
    }

    /// Kill and reap every pane this drone still holds.
    ///
    /// **This is the path that matters most in the whole binary.** A drone runs on somebody
    /// else's machine, usually with no terminal anybody is watching, and a pseudo-terminal it
    /// leaves behind is a process nobody knows to go and kill. Dropping the [`pty::Pty`] closes
    /// the master so the kernel hangs up the slave; the kill is the guarantee for anything that
    /// ignores the hang-up.
    fn shutdown(&mut self) {
        for (pane_id, mut pane) in self.panes.drain() {
            tracing::info!("the session is over: killing pane {pane_id}");
            pane.kill();
        }
        self.owners.clear();
        self.focused.clear();
        self.announced.clear();
    }

    /// A client attached. It is told what this machine is, on the same terms a local host tells a
    /// window — the interface cannot read the far machine's disk, so this is the only way it
    /// learns anything about it.
    ///
    /// A detached drone then says what it is already holding: every live pane is re-announced with
    /// `WorkspaceSpawned` and its ring replayed as `TerminalOutput`. **This is a new host attach,
    /// not a resumed one** — `Bus::drop_remote` took the old one's panes with it when the link
    /// dropped, and nothing here pretends otherwise. What makes it the same pane is the id, which
    /// the drone minted and never reissues; what makes it the same screen is the ring, without
    /// which the user would get a live terminal with nothing drawn in it until the next output.
    fn client_here(&mut self, host: &HostEnd, client: ClientId) {
        tracing::debug!("{client} attached");
        let meta = self.meta.lock().ok().map(|guard| guard.clone());
        host.send(
            To::Client(client),
            Message::HostInfo {
                config_root: wire_string(&self.scratch),
                // Never: a drone's scratch directory is not anybody's default config root, and
                // saying otherwise would have the interface treat it as one.
                is_default: false,
                hostname: meta.as_ref().and_then(|meta| meta.hostname.clone()),
                os: meta.as_ref().map(|meta| meta.os.clone()),
                arch: meta.as_ref().map(|meta| meta.arch.clone()),
                triplet: meta.as_ref().map(|meta| meta.triplet.clone()),
                cpu_count: meta.as_ref().map(|meta| meta.cpu_count),
                mem_total_bytes: meta.as_ref().and_then(|meta| meta.mem_total_bytes),
                shared_workarea: Some(wire_string(&self.scratch)),
            },
        );

        self.here.insert(client);
        // Every pane's output goes to whoever is attached now, whether or not this drone keeps a
        // ring: the reader's sink is the scrollback in both builds, so there is one path.
        let mailbox = host.mailbox(To::Client(client));
        self.screens.attach(mailbox.clone());

        if let Some(holding) = self.holding.as_mut() {
            holding.alone_since = None;
            holding.ever = true;
        } else {
            return;
        }

        // A pane whose process ended while nobody was attached is not re-announced: there was no
        // client to be told, so this is where it is retired.
        self.retire_ended();

        let mut waiting: Vec<PaneId> = self.panes.keys().copied().collect();
        waiting.sort();
        for pane_id in waiting {
            self.owners.insert(pane_id, client);
            if let Some(workspace) = self.announced.get(&pane_id) {
                mailbox.send(Message::WorkspaceSpawned {
                    workspace: workspace.clone(),
                });
            }
            let screen = self.screens.replay(pane_id);
            if !screen.is_empty() {
                tracing::info!(
                    "pane {pane_id}: replaying {} bytes to {client}",
                    screen.len()
                );
                mailbox.send(Message::TerminalOutput {
                    pane_id,
                    bytes: screen,
                });
            }
        }
    }

    /// Drop the panes whose process ended while this drone was holding them.
    fn retire_ended(&mut self) {
        for pane_id in self.screens.ended() {
            self.panes.remove(&pane_id);
            self.owners.remove(&pane_id);
            self.announced.remove(&pane_id);
        }
    }

    /// A client has gone. Everything it owned goes with it — the same rule the coordinator lives
    /// by, and for the same reason: nothing drops on its own now that the process outlives the
    /// connection.
    ///
    /// **Unless this drone is detached**, which is the whole of phase 6: what a dropped link ends
    /// is the link. The panes stay, their output keeps landing in their rings, and the linger
    /// countdown starts. `D22` is untouched — closing a *pane* still kills its harness.
    fn client_gone(&mut self, client: ClientId) {
        self.here.remove(&client);
        self.focused.remove(&client);
        if self.holding.is_some() {
            if self.here.is_empty() {
                self.screens.detach();
                if let Some(holding) = self.holding.as_mut() {
                    holding.alone_since = Some(Instant::now());
                }
                let linger = self.holding.as_ref().map(|holding| holding.live.linger());
                tracing::info!(
                    "the last client detached, holding {} panes with linger {}",
                    self.panes.len(),
                    linger.map(|linger| linger.to_string()).unwrap_or_default()
                );
            }
            return;
        }

        let owned: Vec<PaneId> = self
            .owners
            .iter()
            .filter(|(_, owner)| **owner == client)
            .map(|(pane_id, _)| *pane_id)
            .collect();
        for pane_id in owned {
            self.owners.remove(&pane_id);
            self.announced.remove(&pane_id);
            self.screens.forget(pane_id);
            if let Some(mut pane) = self.panes.remove(&pane_id) {
                tracing::info!("{client} has gone: killing pane {pane_id}");
                pane.kill();
            }
        }
    }

    fn dispatch(&mut self, host: &HostEnd, client: ClientId, message: Message) {
        match message {
            // ── the pane family ─────────────────────────────────────
            Message::SpawnWorkspace {
                session_id,
                project_id,
                rel_path,
                agent_type,
                args,
                // A drone has no harness library, so there is nothing here it could honour. It is
                // ignored rather than refused: a shell row sends the default and means nothing by
                // it, and refusing that would refuse every ordinary spawn.
                picks: _,
            } => {
                let Some(cwd) = self.resolve_cwd(host, client, project_id, rel_path.as_deref())
                else {
                    return;
                };
                let pane_id = PaneId::generate();
                // Whatever the interface named is a program name here. A drone starts programs;
                // composing an agent is the coordinator's job and stays on the user's machine.
                let program = agent_type.unwrap_or_else(shells::default_program);
                let spawned = pty::spawn(
                    &pty::Program::plain(&program, args),
                    Some(cwd.as_path()),
                    INITIAL_COLS,
                    INITIAL_ROWS,
                );
                let (mut pane, mut child) = match spawned {
                    Ok(spawned) => spawned,
                    Err(error) => {
                        host.send(
                            To::Client(client),
                            Message::PaneError {
                                pane_id,
                                error: format!("{error:#}"),
                            },
                        );
                        return;
                    }
                };

                // Recorded before the pane is announced, so nothing can arrive about a pane the
                // routing table has never heard of.
                self.owners.insert(pane_id, client);
                let mailbox = host.mailbox(To::Client(client));
                // The pane's own output goes to the scrollback, never straight to the client: a
                // sink addressed to a client stops the moment that client goes, which is right
                // for a window that closed and wrong for a link that dropped. See `scrollback`.
                let screen = self.screens.sink();
                // No link scan: that is a harness login's business, and a drone hosts no logins.
                if let Err(error) = pane.forward_output(pane_id, screen.clone(), false) {
                    self.owners.remove(&pane_id);
                    // The reader never started, so nothing else will ever wait on this child.
                    pane.kill();
                    let _ = child.wait();
                    mailbox.send(Message::PaneError {
                        pane_id,
                        error: format!("{error:#}"),
                    });
                    return;
                }
                pty::reap(pane_id, child, screen);
                self.panes.insert(pane_id, pane);
                tracing::info!("pane {pane_id}: started {program} for {client}");

                let workspace = WorkspaceInfo {
                    id: pane_id,
                    session_id,
                    agent_type: program,
                    project_id,
                    rel_path,
                    cols: INITIAL_COLS,
                    rows: INITIAL_ROWS,
                    running: true,
                    wait_on_exit: false,
                    wait_on_error: false,
                    // A drone spawns shells, never a configured tool — `RunTool` is the local
                    // coordinator's alone.
                    tool: None,
                };
                // Kept so a client attaching later can be told about this pane in the same words
                // the client that spawned it heard.
                self.announced.insert(pane_id, workspace.clone());
                mailbox.send(Message::WorkspaceSpawned { workspace });
            }

            Message::TerminalInput { pane_id, bytes } => {
                if !self.owns(client, pane_id) {
                    return;
                }
                if let Some(pane) = self.panes.get_mut(&pane_id)
                    && let Err(error) = pane.write(&bytes)
                {
                    host.send(
                        To::Client(client),
                        Message::PaneError {
                            pane_id,
                            error: error.to_string(),
                        },
                    );
                }
            }

            // A resize is incomplete until the harness knows, so this reaches the pseudo-terminal
            // and the kernel signals the process. For a pane that has gone it is ignored: the
            // geometry has nowhere to land.
            Message::TerminalResize {
                pane_id,
                cols,
                rows,
            } => {
                if !self.owns(client, pane_id) {
                    return;
                }
                // Remembered as well as applied: a client attaching later is told the geometry
                // the pseudo-terminal actually has, not the one it started at.
                if let Some(workspace) = self.announced.get_mut(&pane_id) {
                    workspace.cols = cols;
                    workspace.rows = rows;
                }
                if let Some(pane) = self.panes.get(&pane_id)
                    && let Err(error) = pane.resize(cols, rows)
                {
                    host.send(
                        To::Client(client),
                        Message::PaneError {
                            pane_id,
                            error: error.to_string(),
                        },
                    );
                }
            }

            Message::Focus { pane_id } => {
                if self.owns(client, pane_id) {
                    self.focused.insert(client, pane_id);
                }
            }

            Message::CloseWorkspace { pane_id } => {
                if !self.owns(client, pane_id) {
                    return;
                }
                self.owners.remove(&pane_id);
                self.announced.remove(&pane_id);
                self.screens.forget(pane_id);
                if let Some(mut pane) = self.panes.remove(&pane_id) {
                    tracing::info!("closing pane {pane_id}, killing what is in it");
                    pane.kill();
                }
                if self.focused.get(&client) == Some(&pane_id) {
                    self.focused.remove(&client);
                }
            }

            // ── this machine's shells ───────────────────────────────
            // Probed on every ask, the same as a local host: a shell installed since the window
            // opened is offered without a restart.
            Message::ListShells => {
                host.send(
                    To::Client(client),
                    Message::ShellList {
                        shells: shells::available(),
                    },
                );
            }

            // ── the project family, read-only ───────────────────────
            // A drone's catalogue is what it was launched with. Adding, forgetting, renaming and
            // re-pointing are the user's own decisions about their own machine and belong to the
            // coordinator; they are refused below with the rest.
            Message::ListProjects => {
                host.send(
                    To::Client(client),
                    Message::ProjectList {
                        projects: self.snapshots(),
                    },
                );
            }

            // ── the host browse family ──────────────────────────────
            // No project to look up and no syscall here: straight to the worker.
            Message::BrowseHostDir { path } => {
                self.files.submit(files::Job {
                    kind: files::JobKind::Browse { path },
                    reply_to: host.mailbox(To::Client(client)),
                });
            }

            // ── the file family, minus the diff ─────────────────────
            // A diff needs version control, which a lean build does not link; it is refused below.
            Message::ProjectTree {
                project_id,
                rel_path,
                depth,
            } => {
                let request = files::Request::Tree {
                    rel_path: rel_path.clone(),
                    depth,
                };
                self.file_job(host, client, project_id, &rel_path, request);
            }
            Message::ReadProjectFile {
                project_id,
                rel_path,
                max_bytes,
            } => {
                let request = files::Request::Read {
                    rel_path: rel_path.clone(),
                    max_bytes,
                };
                self.file_job(host, client, project_id, &rel_path, request);
            }
            Message::WriteProjectFile {
                project_id,
                rel_path,
                bytes,
                expected,
                overwrite,
            } => {
                let request = files::Request::Write {
                    rel_path: rel_path.clone(),
                    bytes,
                    expected,
                    overwrite,
                };
                self.file_job(host, client, project_id, &rel_path, request);
            }
            Message::EditProjectPath {
                project_id,
                rel_path,
                to,
                op,
                carry_related,
            } => {
                let request = files::Request::Edit {
                    rel_path: rel_path.clone(),
                    to,
                    op,
                    carry_related,
                };
                self.file_job(host, client, project_id, &rel_path, request);
            }

            // ── the search family ────────────────────────────────────
            // The lean host carries `ubiq_host::search`'s own walk, but a drone shells out
            // instead — see `search`'s own doc for why. `scope` decides whether there is
            // anything here to answer at all.
            Message::SearchProject {
                project_id,
                search_id,
                query,
                scope,
                filter,
            } => {
                self.search_job(host, client, project_id, search_id, scope, query, filter);
            }
            Message::CancelSearch {
                project_id,
                search_id,
            } => {
                if let Some((active_id, cancel)) = self.active_searches.get(&project_id)
                    && *active_id == search_id
                {
                    cancel.cancel();
                    tracing::info!(search = %search_id, project = %project_id, "search cancelled");
                }
            }

            // Everything else. A refusal the asker can act on, or — where the family has no error
            // variant to carry one — a log line, which is the only honest alternative.
            other => match refusal(&other) {
                Some(answer) => host.send(To::Client(client), answer),
                None => {
                    tracing::warn!("a drone has no answer for {other:?}, and none to refuse with")
                }
            },
        }
    }

    /// Every project, probed.
    ///
    /// The workarea a drone names is under its scratch directory and is reserved lazily, the same
    /// contract a local host's is: the host names it and creates it, the interface owns what goes
    /// in it, and deleting it loses a cache and nothing else.
    fn snapshots(&self) -> Vec<ProjectSnapshot> {
        self.records
            .iter()
            .map(|record| ProjectSnapshot {
                health: health::probe(std::path::Path::new(&record.path)),
                open_panes: 0,
                workarea: wire_string(&self.scratch.join(record.id.to_string())),
                // Nothing a drone holds is ephemeral in the sense the interface means: it deletes
                // no folder on the far machine, ever.
                ephemeral: false,
                record: record.clone(),
            })
            .collect()
    }

    /// Hand one file-family request to the worker, with the root it resolves against.
    fn file_job(
        &self,
        host: &HostEnd,
        client: ClientId,
        project_id: ProjectId,
        rel_path: &str,
        request: files::Request,
    ) {
        let Some(record) = self.records.iter().find(|record| record.id == project_id) else {
            host.send(
                To::Client(client),
                files::file_error(
                    project_id,
                    rel_path,
                    ubiq_proto::files::FileError::Refused("no such project".to_string()),
                ),
            );
            return;
        };
        self.files.submit(files::Job {
            kind: files::JobKind::File {
                project_id,
                root: PathBuf::from(&record.path),
                request,
            },
            reply_to: host.mailbox(To::Client(client)),
        });
    }

    /// Hand one `SearchProject` to the search worker, or answer it directly when there is nothing
    /// for the worker to do.
    #[allow(clippy::too_many_arguments)]
    fn search_job(
        &mut self,
        host: &HostEnd,
        client: ClientId,
        project_id: ProjectId,
        search_id: SearchId,
        scope: ubiq_proto::search::Scope,
        query: ubiq_proto::search::Query,
        filter: ubiq_proto::search::Filter,
    ) {
        // Tasks, chats and the knowledge base are the coordinator's own state — a drone holds
        // none of it — so `Scope::Files` is the only scope this machine can ever answer. That is
        // answered empty rather than refused: nothing here failed, there was simply nothing a
        // drone could have looked at.
        if !matches!(scope, ubiq_proto::search::Scope::Files) {
            host.send(
                To::Client(client),
                Message::SearchFinished {
                    project_id,
                    search_id,
                    searched: Vec::new(),
                    truncated: false,
                },
            );
            return;
        }

        // Reap searches that have finished — the flag also means "this search is over", set by
        // the worker itself. This is the one place `active_searches` gains an entry, so it is the
        // one place that needs to drop stale ones.
        self.active_searches
            .retain(|_, (_, cancel)| !cancel.is_set());

        let Some(record) = self.records.iter().find(|record| record.id == project_id) else {
            host.send(
                To::Client(client),
                Message::SearchError {
                    project_id,
                    search_id,
                    error: ubiq_proto::search::SearchError::Root,
                },
            );
            return;
        };

        // A second search for this project supersedes the first, which is interrupted mid-tool.
        if let Some((superseded, cancel)) = self.active_searches.remove(&project_id) {
            cancel.cancel();
            tracing::info!(
                search = %superseded,
                by = %search_id,
                project = %project_id,
                "search superseded"
            );
        }

        let cancel = Cancel::new();
        self.active_searches
            .insert(project_id, (search_id, cancel.clone()));

        self.search.submit(search::Job {
            project_id,
            search_id,
            root: PathBuf::from(&record.path),
            query,
            filter,
            cancel,
            reply_to: host.mailbox(To::Client(client)),
        });
    }

    /// Where a pane starts, or nothing and a refusal already sent.
    ///
    /// Settled before a pseudo-terminal exists, so a spawn that fails leaves nothing on screen to
    /// close — which is why the refusal is a `ProjectError` and not a `PaneError` about a pane the
    /// interface was never told about.
    fn resolve_cwd(
        &self,
        host: &HostEnd,
        client: ClientId,
        project_id: ProjectId,
        rel_path: Option<&str>,
    ) -> Option<PathBuf> {
        let Some(record) = self.records.iter().find(|record| record.id == project_id) else {
            self.refuse_spawn(host, client, project_id, "no such project".to_string());
            return None;
        };
        let root = PathBuf::from(&record.path);
        if !health::probe(&root).is_ok() {
            self.refuse_spawn(
                host,
                client,
                project_id,
                format!("{} cannot be used as a folder", record.path),
            );
            return None;
        }
        match rel_path {
            None => Some(root),
            Some(rel_path) => match files::path::resolve(&root, rel_path) {
                Ok(cwd) if cwd.is_dir() => Some(cwd),
                Ok(_) => {
                    self.refuse_spawn(
                        host,
                        client,
                        project_id,
                        format!("{rel_path} is not a folder"),
                    );
                    None
                }
                Err(error) => {
                    self.refuse_spawn(host, client, project_id, format!("{rel_path}: {error}"));
                    None
                }
            },
        }
    }

    fn refuse_spawn(&self, host: &HostEnd, client: ClientId, project_id: ProjectId, error: String) {
        host.send(
            To::Client(client),
            Message::ProjectError {
                project_id: Some(project_id),
                error,
            },
        );
    }

    /// Whether this client is the one that owns the pane. A message about somebody else's pane is
    /// a wiring mistake, not something to act on.
    fn owns(&self, client: ClientId, pane_id: PaneId) -> bool {
        match self.owners.get(&pane_id) {
            Some(owner) if *owner == client => true,
            Some(_) => {
                tracing::warn!(
                    "{client} sent a message about pane {pane_id}, which it does not own"
                );
                false
            }
            // The pane has already gone; its last messages are in flight behind it.
            None => false,
        }
    }
}

/// One project for one `--root`.
///
/// The path is canonicalised where it can be, because everything downstream resolves against it
/// and a relative root would resolve against whatever directory the drone happened to be started
/// in. A path that cannot be canonicalised is kept as given, so the interface gets a row with a
/// health to show rather than a project that silently is not there.
fn record_for(root: &Root) -> ProjectRecord {
    let resolved = std::fs::canonicalize(&root.path).unwrap_or_else(|_| root.path.clone());
    let name = resolved
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| wire_string(&resolved));
    ProjectRecord {
        // `--root-id` binds this root to an id the interface already minted; a bare `--root`
        // keeps announcing a fresh one, as it always has.
        id: root.id.unwrap_or_else(ProjectId::generate),
        name,
        path: wire_string(&resolved),
        colour: 0,
        custom_colour: None,
        // Not `temporary`: that flag means "never written down", and it is also what lets a local
        // host delete the folder on Forget. Neither is true here — a drone writes nothing down
        // and deletes nothing — so the honest value is the plain one.
        temporary: false,
        created_at: chrono::Utc::now(),
        last_opened_at: None,
        search_excludes: Vec::new(),
        index: None,
        mission_term: None,
        managed_repos: Vec::new(),
        tools: Vec::new(),
        // A drone does not know it is one: `runs_on` is the interface's own record of *where* a
        // project's folder is, and this catalogue is built on the machine the folder is already
        // on.
        lanes: Vec::new(),
        runs_on: None,
        initials: String::new(),
    }
}

/// What to say about a message a drone will not serve.
///
/// One arm per family, answering with that family's own error so the asker learns where the
/// refusal landed instead of watching a spinner. `None` is a family with no error variant at all;
/// the caller logs those, and they are listed in the module header of the crate's tests.
fn refusal(message: &Message) -> Option<Message> {
    use Message::*;

    const NOT_HERE: &str =
        "this host is a drone: it serves a terminal, files and machine facts, and nothing else";

    Some(match message {
        // ── conversations, and the harness library behind them ──
        StartConversation { agent_id, .. }
        | PromptAgent { agent_id, .. }
        | CancelTurn { agent_id }
        | AnswerPermission { agent_id, .. }
        | SetAgentConfig { agent_id, .. }
        | EndConversation { agent_id }
        | UnloadConversation { agent_id }
        | AbortConversation { agent_id }
        | ResumeConversation { agent_id }
        | SetConversationPersistent { agent_id, .. }
        | SetConversationAcceptAll { agent_id, .. }
        | SetConversationDebugDump { agent_id, .. }
        | ReviveConversation { agent_id, .. } => ConversationError {
            agent_id: *agent_id,
            error: NOT_HERE.to_string(),
        },

        // ── accounts, logins, profiles, MCP servers, agent types ──
        ListAgentTypes
        | CheckAgentCommand { .. }
        | ListHarnessCatalogue { .. }
        | ListAccounts
        | BeginHarnessLogin { .. }
        | CheckHarnessLogin { .. }
        | RenameAccount { .. }
        | DeleteAccount { .. }
        | DeleteHarnessLogin { .. }
        | ListProfiles
        | SaveProfile { .. }
        | ListMcps
        | ListTools { .. }
        | RunTool { .. } => AccountError {
            error: NOT_HERE.to_string(),
        },

        // ── how much of a plan is left ──
        QueryQuota {
            account, harness, ..
        } => QuotaRead {
            account: account.clone(),
            harness: harness.clone(),
            snapshot: None,
            error: Some(NOT_HERE.to_string()),
        },

        // ── the identities Ubiq holds at external services ──
        ListConnections
        | BeginConnect { .. }
        | CancelConnect { .. }
        | SubmitConnectSecret { .. }
        | RenameConnection { .. }
        | DeleteConnection { .. }
        | CheckConnection { .. }
        | TrustCertificate { .. }
        | ForgetCertificate { .. }
        | SaveOauthApp { .. }
        | DeleteOauthApp { .. }
        | SetAppSecret { .. }
        | ClearAppSecret { .. } => ConnectorError {
            error: NOT_HERE.to_string(),
        },

        // ── repositories and clones ──
        ListRepos { query_id, .. } | ListRepoBranches { query_id, .. } => RepoError {
            query_id: *query_id,
            error: ubiq_proto::repos::CloneError::Refused(NOT_HERE.to_string()),
        },
        CancelClone { clone_id } => CloneFailed {
            clone_id: *clone_id,
            error: ubiq_proto::repos::CloneError::Refused(NOT_HERE.to_string()),
        },

        // ── version control ──
        // A lean build links no git library at all, which is why the diff is out of the file
        // family too.
        ProjectGit { project_id }
        | RefreshProjectGit { project_id, .. }
        | ProjectGitLog { project_id, .. }
        | ProjectGitRefs { project_id, .. }
        | WriteProjectGit { project_id, .. }
        | ProjectGitChanged { project_id, .. }
        | DiffProjectFile { project_id, .. } => GitError {
            project_id: *project_id,
            error: ubiq_proto::git::GitError::Failed(NOT_HERE.to_string()),
        },

        // ── the tasks a project has written down ──
        ListWork { project_id }
        | CreateTask { project_id, .. }
        | UpdateTask { project_id, .. }
        | SetTaskField { project_id, .. }
        | MoveTask { project_id, .. }
        | AssignTask { project_id, .. }
        | DeleteTask { project_id, .. }
        | AddStep { project_id, .. }
        | RenameStep { project_id, .. }
        | RemoveStep { project_id, .. }
        | MoveStep { project_id, .. }
        | ToggleStep { project_id, .. }
        | AddComment { project_id, .. }
        | AssignAgent { project_id, .. }
        | SendToAgent { project_id, .. } => WorkError {
            project_id: *project_id,
            task_id: None,
            error: NOT_HERE.to_string(),
        },

        // ── assistance, and the providers behind it ──
        GetAssist => Assist {
            available: false,
            reason: Some(ubiq_proto::assist::AssistReason::UnsupportedPlatform),
            detail: Some(NOT_HERE.to_string()),
            limits: None,
        },
        Suggest { suggest_id, .. } | CancelSuggest { suggest_id } => SuggestError {
            suggest_id: *suggest_id,
            error: NOT_HERE.to_string(),
        },
        GetAiProviders | ListAiModels { .. } | AddAiProvider { .. } => AiProviderError {
            provider_id: None,
            error: NOT_HERE.to_string(),
        },
        UpdateAiProvider { provider_id, .. } | ForgetAiProvider { provider_id } => {
            AiProviderError {
                provider_id: Some(*provider_id),
                error: NOT_HERE.to_string(),
            }
        }

        // ── the vendor bundle a web panel needs ──
        EnsureWebBundle { app, .. } => WebBundleFailed {
            app: app.clone(),
            error: NOT_HERE.to_string(),
        },

        // ── the catalogue's write half, and the blobs beside it ──
        // Refused rather than served: what is in a drone's catalogue is what it was launched
        // with, and a settings or preference blob belongs to the machine the user is at.
        AddProject { .. }
        | ForgetProject { .. }
        | UpdateProject { .. }
        | LocateProject { .. }
        | OpenedProject { .. }
        | RefreshProject { .. }
        | GetPreferences { .. }
        | SetPreferences { .. } => ProjectError {
            project_id: message.project_id(),
            error: NOT_HERE.to_string(),
        },
        GetSettings { layer } | SetSettings { layer, .. } => SettingsError {
            layer: *layer,
            error: NOT_HERE.to_string(),
        },
        // An ssh profile's secret belongs to the machine the user is at, and a drone has no
        // keychain to file one in — it is a guest. Refused on the settings layer it edits.
        SetSshSecret { .. } | ClearSshSecret { .. } => SettingsError {
            layer: ubiq_proto::settings::SettingsLayer::Host,
            error: NOT_HERE.to_string(),
        },

        // Everything left is either something only a host says — an answer arriving at the wrong
        // end of the wire — or a family with no error variant to refuse with. The caller logs it.
        _ => return None,
    })
}
