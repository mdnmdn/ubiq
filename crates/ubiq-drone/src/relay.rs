//! The run loop that stands in for the coordinator.
//!
//! It is deliberately *not* a smaller coordinator: it is a different thing with a smaller job.
//! The coordinator owns conversations, accounts, quotas, version control, an index and a
//! notification centre, and it needs every feature of the host crate to do it. A drone owns three
//! things — the pseudo-terminals it opened, a catalogue that lives only in memory, and the worker
//! that answers the file family — and that is the whole of its state.
//!
//! **Every message gets an answer.** What this relay cannot serve is refused with the error
//! variant of its own family, never dropped: a drone is reached over a stream where silence is
//! indistinguishable from a lost connection, and an interface waiting forever on a reply it will
//! never get is the worst failure this feature can have. The catch-all arm at the end of
//! [`Relay::dispatch`] is what makes that true for a message written after this file was.
//!
//! It renders nothing, and terminal bytes stay opaque.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use ubiq_host::files::{self, Files};
use ubiq_host::host_meta::HostMeta;
use ubiq_host::host_path::wire_string;
use ubiq_host::{health, host_meta, pty, shells};
use ubiq_proto::bus::{ClientId, FromClient, HostEnd, To};
use ubiq_proto::ids::{PaneId, ProjectId};
use ubiq_proto::messages::{Message, WorkspaceInfo};
use ubiq_proto::projects::{ProjectRecord, ProjectSnapshot};

/// The geometry a pane starts at, before the interface has measured its own bounds. The same
/// numbers the coordinator uses, for the same reason: the truth arrives a frame later as a
/// `TerminalResize`.
const INITIAL_COLS: u16 = 80;
const INITIAL_ROWS: u16 = 24;

/// One machine, served.
pub struct Relay {
    files: Files,
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
}

impl Relay {
    /// Build the relay, with one project per `--root`.
    ///
    /// A root that is not a directory is still taken into the catalogue: the interface shows it
    /// with the health the probe gave it, which is the same thing a local host does for a project
    /// whose folder has gone. Refusing here would leave the user with no row to look at.
    pub fn new(roots: Vec<PathBuf>) -> Self {
        let scratch = std::env::temp_dir().join(format!("ubiq-drone-{}", std::process::id()));
        if let Err(error) = std::fs::create_dir_all(&scratch) {
            tracing::warn!("could not reserve {}: {error}", scratch.display());
        }

        // No `--root` is a drone with an empty catalogue, not an error: it can still serve a
        // terminal and browse the machine, which is what the host-browse family is for.
        let records = roots.iter().map(|root| record_for(root)).collect();
        Self {
            files: Files::start(),
            meta: host_meta::start(scratch.clone()),
            records,
            scratch,
            panes: HashMap::new(),
            owners: HashMap::new(),
            focused: HashMap::new(),
        }
    }

    /// Answer the bus until it closes.
    ///
    /// It closes when the carrier's stream has ended and the hub behind it has been dropped —
    /// which is why [`Self::shutdown`] runs on the way out and not on a signal: see `carrier`.
    pub fn run(mut self, host: HostEnd) {
        while let Ok(event) = host.recv() {
            match event {
                FromClient::Connected(client) => self.client_here(&host, client),
                FromClient::Said { client, message } => self.dispatch(&host, client, message),
                FromClient::Gone(client) => self.client_gone(client),
            }
        }
        self.shutdown();
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
            tracing::info!("the stream ended: killing pane {pane_id}");
            pane.kill();
        }
        self.owners.clear();
        self.focused.clear();
    }

    /// A client attached. It is told what this machine is, on the same terms a local host tells a
    /// window — the interface cannot read the far machine's disk, so this is the only way it
    /// learns anything about it.
    fn client_here(&self, host: &HostEnd, client: ClientId) {
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
    }

    /// A client has gone. Everything it owned goes with it — the same rule the coordinator lives
    /// by, and for the same reason: nothing drops on its own now that the process outlives the
    /// connection.
    fn client_gone(&mut self, client: ClientId) {
        self.focused.remove(&client);
        let owned: Vec<PaneId> = self
            .owners
            .iter()
            .filter(|(_, owner)| **owner == client)
            .map(|(pane_id, _)| *pane_id)
            .collect();
        for pane_id in owned {
            self.owners.remove(&pane_id);
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
                // No link scan: that is a harness login's business, and a drone hosts no logins.
                if let Err(error) = pane.forward_output(pane_id, mailbox.clone(), false) {
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
                pty::reap(pane_id, child, mailbox.clone());
                self.panes.insert(pane_id, pane);
                tracing::info!("pane {pane_id}: started {program} for {client}");

                mailbox.send(Message::WorkspaceSpawned {
                    workspace: WorkspaceInfo {
                        id: pane_id,
                        session_id,
                        agent_type: program,
                        project_id,
                        rel_path,
                        cols: INITIAL_COLS,
                        rows: INITIAL_ROWS,
                        running: true,
                        wait_on_exit: false,
                    },
                });
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
            } => {
                let request = files::Request::Write {
                    rel_path: rel_path.clone(),
                    bytes,
                    expected,
                };
                self.file_job(host, client, project_id, &rel_path, request);
            }
            Message::EditProjectPath {
                project_id,
                rel_path,
                to,
                op,
            } => {
                let request = files::Request::Edit {
                    rel_path: rel_path.clone(),
                    to,
                    op,
                };
                self.file_job(host, client, project_id, &rel_path, request);
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
fn record_for(root: &std::path::Path) -> ProjectRecord {
    let resolved = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let name = resolved
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| wire_string(&resolved));
    ProjectRecord {
        id: ProjectId::generate(),
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
        managed_repos: Vec::new(),
        tools: Vec::new(),
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
        | AssignAgent { project_id, .. }
        | SendToAgent { project_id, .. } => WorkError {
            project_id: *project_id,
            task_id: None,
            error: NOT_HERE.to_string(),
        },

        // ── content search ──
        // The lean host carries the search worker, so this is a scope decision and not a missing
        // capability: a drone answers the file family and nothing built on top of it yet.
        SearchProject {
            project_id,
            search_id,
            ..
        }
        | CancelSearch {
            project_id,
            search_id,
        } => SearchError {
            project_id: *project_id,
            search_id: *search_id,
            error: ubiq_proto::search::SearchError::Walk(NOT_HERE.to_string()),
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

        // Everything left is either something only a host says — an answer arriving at the wrong
        // end of the wire — or a family with no error variant to refuse with. The caller logs it.
        _ => return None,
    })
}
