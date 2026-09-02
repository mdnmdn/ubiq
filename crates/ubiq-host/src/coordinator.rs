//! The coordinator: it starts harnesses, supervises them, and answers the bus.
//!
//! It runs on a thread of its own and shares nothing with the window but the channel pair in
//! [`ubiq_proto::bus`]. It renders nothing and has no opinion about layout or colour — everything here
//! is a pane ID, a pseudo-terminal and a process.

use std::collections::HashMap;
use std::path::PathBuf;
use std::thread;
use std::time::Instant;

use ubiq_proto::bus::{ClientId, FromClient, HostEnd, To};
use ubiq_proto::files::FileError;
use ubiq_proto::ids::{PaneId, ProjectId, SessionId};
use ubiq_proto::messages::{Message, WorkspaceInfo};
use ubiq_proto::projects::ProjectHealth;

use crate::config::ConfigRoot;
use crate::files::{self, Files};
use crate::git::{self, Git};
use crate::health;
use crate::projects::Projects;
use crate::pty::{self, Pty};
use crate::reply::Reply;
use crate::settings::Settings;
use crate::work::Work;

/// The geometry a pane starts at, before the emulator has measured its own bounds and said what it
/// really is. The harness is told the truth a frame later, by [`Message::TerminalResize`].
const INITIAL_COLS: u16 = 80;
const INITIAL_ROWS: u16 = 24;

/// Start the coordinator on its own thread. One per process, started before the first window: the
/// catalogue it will own is process-wide, and two of them would disagree about what exists.
///
/// It ends when the hub and every client have gone.
pub fn start(
    host: HostEnd,
    root: ConfigRoot,
    projects: Projects,
    work: Work,
    settings: Settings,
    pending: Vec<Reply>,
) {
    thread::Builder::new()
        .name("ubiq-coordinator".to_string())
        .spawn(move || Coordinator::new(host, root, projects, work, settings, pending).run())
        .expect("the coordinator thread");
}

struct Coordinator {
    host: HostEnd,
    /// Where everything is written down, and whether that is the usual place. Each window is told
    /// as it attaches, because the interface cannot look.
    root: ConfigRoot,
    /// The catalogue, the view state, and what is running in each project.
    projects: Projects,
    /// Each project's tasks, and the sessions and agents the two screens over the work draw.
    work: Work,
    /// Application settings: the Ui layer opaque, the Host layer parsed.
    settings: Settings,
    /// The thread that reads and writes a project's files. Nothing in the file family touches disk
    /// on this thread: a cold `read_dir` here would stall every pane's keystrokes behind it.
    files: Files,
    /// The thread that reads a project's repository. A status walk is seconds on a large tree, and
    /// seconds here would stall every pane behind it.
    git: Git,
    /// Anything the catalogue wanted said before a window existed to hear it — a corrupt store,
    /// most usefully. Delivered to the first window that attaches.
    pending: Vec<Reply>,
    /// Which project each pane belongs to, so a pane opening or ending changes a count the picker
    /// draws.
    pane_projects: HashMap<PaneId, ProjectId>,
    panes: HashMap<PaneId, Pty>,
    /// Which window owns which pane, recorded when the pane is spawned. This is the whole routing
    /// table: everything a pane emits goes to its owner, and nobody else may drive it.
    owners: HashMap<PaneId, ClientId>,
    /// Which pane each window says has focus. Exactly one per window — two attached windows have
    /// two focused panes, and neither is more focused than the other.
    focused: HashMap<ClientId, PaneId>,
}

impl Coordinator {
    fn new(
        host: HostEnd,
        root: ConfigRoot,
        projects: Projects,
        work: Work,
        settings: Settings,
        pending: Vec<Reply>,
    ) -> Self {
        Self {
            host,
            root,
            projects,
            work,
            settings,
            files: Files::start(),
            git: Git::start(),
            pending,
            pane_projects: HashMap::new(),
            panes: HashMap::new(),
            owners: HashMap::new(),
            focused: HashMap::new(),
        }
    }

    fn run(mut self) {
        loop {
            // A queued preference has to be written even if nobody says anything else, so the wait
            // is bounded whenever something is pending.
            let event = match self.projects.next_due(Instant::now()) {
                Some(wait) => match self.host.recv_timeout(wait) {
                    Ok(event) => Some(event),
                    Err(flume::RecvTimeoutError::Timeout) => None,
                    Err(flume::RecvTimeoutError::Disconnected) => break,
                },
                None => match self.host.recv() {
                    Ok(event) => Some(event),
                    Err(_) => break,
                },
            };

            match event {
                Some(FromClient::Connected(client)) => self.client_here(client),
                Some(FromClient::Said { client, message }) => self.dispatch(client, message),
                Some(FromClient::Gone(client)) => self.client_gone(client),
                None => {}
            }
            self.projects.flush_due(Instant::now());
        }
        // Nothing is left to say it to, but what the user last did still belongs on disk.
        self.projects.flush();
    }

    /// A window attached. It is told what the host is, and hears anything the catalogue wanted to
    /// say before there was anybody to say it to.
    fn client_here(&mut self, client: ClientId) {
        tracing::debug!("{client} attached");
        self.host.send(
            To::Client(client),
            Message::HostInfo {
                config_root: self.root.path.to_string_lossy().into_owned(),
                is_default: self.root.is_default(),
            },
        );
        for reply in std::mem::take(&mut self.pending) {
            self.host.send(To::Client(client), reply.into_message());
        }
    }

    /// Say what the catalogue answered, to whoever it is for.
    fn answer(&self, client: ClientId, replies: Vec<Reply>) {
        for reply in replies {
            let to = if reply.is_broadcast() {
                To::Everyone
            } else {
                To::Client(client)
            };
            self.host.send(to, reply.into_message());
        }
    }

    /// A window has gone. Everything it owned goes with it.
    ///
    /// This used to happen by itself: a closed window dropped its whole bus, the coordinator thread
    /// ended, and the pseudo-terminals went with it. One host outlives every window, so nothing
    /// drops now and the reaping has to be deliberate — without this, every closed window leaves a
    /// live harness behind.
    fn client_gone(&mut self, client: ClientId) {
        self.focused.remove(&client);
        let owned: Vec<PaneId> = self
            .owners
            .iter()
            .filter(|(_, owner)| **owner == client)
            .map(|(pane_id, _)| *pane_id)
            .collect();

        // Taking the pseudo-terminal out of the map is most of the work: dropping it closes the
        // master, the kernel hangs up the slave, and the harness goes. The kill is the guarantee
        // for anything that ignores the hang-up.
        for pane_id in owned {
            self.owners.remove(&pane_id);
            if let Some(mut pane) = self.panes.remove(&pane_id) {
                tracing::info!("{client} has gone; killing the harness in pane {pane_id}");
                pane.kill();
            }
            self.pane_gone(client, pane_id);
        }
    }

    /// A pane has ended, however it ended: the project it belonged to has one fewer.
    fn pane_gone(&mut self, client: ClientId, pane_id: PaneId) {
        if let Some(project_id) = self.pane_projects.remove(&pane_id) {
            let replies = self.projects.pane_closed(project_id);
            self.answer(client, replies);
        }
    }

    /// Whether this window is the one that owns the pane. A message about somebody else's pane is
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

    fn dispatch(&mut self, client: ClientId, message: Message) {
        match message {
            Message::SpawnWorkspace {
                session_id,
                project_id,
                rel_path,
                agent_type,
                args,
            } => self.spawn_workspace(client, session_id, project_id, rel_path, agent_type, args),

            Message::TerminalInput { pane_id, bytes } => {
                if !self.owns(client, pane_id) {
                    return;
                }
                if let Some(pane) = self.panes.get_mut(&pane_id)
                    && let Err(error) = pane.write(&bytes)
                {
                    self.host.send(
                        To::Client(client),
                        Message::PaneError {
                            pane_id,
                            error: error.to_string(),
                        },
                    );
                }
            }

            // A resize for a pane that has gone is ignored: the geometry has nowhere to land.
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
                    self.host.send(
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
                    tracing::info!("closing pane {pane_id}, killing its harness");
                    pane.kill();
                }
                if self.focused.get(&client) == Some(&pane_id) {
                    self.focused.remove(&client);
                }
                self.pane_gone(client, pane_id);
            }

            // ── the project family ──────────────────────────────────
            Message::ListProjects => {
                let reply = self.projects.list_projects();
                self.answer(client, vec![reply]);
            }
            Message::AddProject { path, name, colour } => {
                let replies = self.projects.add(&path, name, colour);
                self.answer(client, replies);
            }
            Message::ForgetProject { project_id } => {
                let replies = self.projects.forget(project_id);
                // The catalogue went first and took the project's directory with it, so the
                // tasks on disk are already gone. This is the memory that was left, and this
                // is the only place that knows both services.
                self.work.forget(project_id);
                self.git_forget(client, project_id);
                self.answer(client, replies);
            }
            Message::UpdateProject {
                project_id,
                name,
                colour,
            } => {
                let replies = self.projects.update(project_id, name, colour);
                self.answer(client, replies);
            }
            Message::LocateProject { project_id, path } => {
                let replies = self.projects.locate(project_id, &path);
                self.git_forget(client, project_id);
                self.answer(client, replies);
            }
            Message::OpenedProject { project_id } => {
                let replies = self.projects.opened(project_id);
                self.answer(client, replies);
            }
            Message::RefreshProject { project_id } => {
                let replies = self.projects.refresh(project_id);
                self.answer(client, replies);
            }
            Message::GetPreferences { scope } => {
                let reply = self.projects.get_preferences(scope);
                self.answer(client, vec![reply]);
            }
            Message::SetPreferences { scope, value } => {
                self.projects.set_preferences(scope, value, Instant::now());
            }
            Message::GetSettings { layer } => {
                let reply = self.settings.get(layer);
                self.answer(client, vec![reply]);
            }
            Message::SetSettings { layer, value } => {
                let replies = self.settings.set(layer, value);
                self.answer(client, replies);
            }

            // ── the file family ─────────────────────────────────────
            // Four arms, no syscall: the record is a lookup in memory and the work goes to the
            // worker with the root it resolved against.
            Message::ProjectTree {
                project_id,
                rel_path,
                depth,
            } => {
                let request = files::Request::Tree {
                    rel_path: rel_path.clone(),
                    depth,
                };
                self.file_job(client, project_id, &rel_path, request);
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
                self.file_job(client, project_id, &rel_path, request);
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
                self.file_job(client, project_id, &rel_path, request);
            }
            Message::DiffProjectFile {
                project_id,
                rel_path,
                base,
            } => {
                let request = files::Request::Diff {
                    rel_path: rel_path.clone(),
                    base,
                };
                self.file_job(client, project_id, &rel_path, request);
            }

            // ── the git family ──────────────────────────────────────
            // Two arms, no syscall: the record is a lookup in memory and the work goes to the
            // worker with the root it resolved against. A status walk on this thread would stall
            // every pane behind it.
            Message::ProjectGit { project_id } => {
                self.git_job(client, project_id, git::Request::Overview);
            }
            Message::RefreshProjectGit { project_id, full } => {
                let request = if full {
                    git::Request::Full
                } else {
                    git::Request::Overview
                };
                self.git_job(client, project_id, request);
            }

            // ── the work family ─────────────────────────────────────
            // Thirteen arms and one helper. Every one names a project, and none of them touches a
            // user's folder — a task file lives under Ubiq's own config root, which the catalogue
            // and the view state already write from this thread.
            Message::ListWork { project_id } => {
                self.work_job(client, project_id, |work| work.list(project_id));
            }
            Message::CreateTask {
                project_id,
                title,
                session,
            } => {
                self.work_job(client, project_id, |work| {
                    work.create(project_id, title, session)
                });
            }
            Message::UpdateTask {
                project_id,
                task_id,
                title,
                description,
                priority,
                shape,
            } => {
                self.work_job(client, project_id, |work| {
                    work.update(project_id, task_id, title, description, priority, shape)
                });
            }
            Message::MoveTask {
                project_id,
                task_id,
                status,
            } => {
                self.work_job(client, project_id, |work| {
                    work.move_task(project_id, task_id, status)
                });
            }
            Message::AssignTask {
                project_id,
                task_id,
                session,
            } => {
                self.work_job(client, project_id, |work| {
                    work.assign(project_id, task_id, session)
                });
            }
            Message::DeleteTask {
                project_id,
                task_id,
            } => {
                self.work_job(client, project_id, |work| work.delete(project_id, task_id));
            }
            Message::AddStep {
                project_id,
                task_id,
                title,
            } => {
                self.work_job(client, project_id, |work| {
                    work.add_step(project_id, task_id, title)
                });
            }
            Message::RenameStep {
                project_id,
                task_id,
                step_id,
                title,
            } => {
                self.work_job(client, project_id, |work| {
                    work.rename_step(project_id, task_id, step_id, title)
                });
            }
            Message::RemoveStep {
                project_id,
                task_id,
                step_id,
            } => {
                self.work_job(client, project_id, |work| {
                    work.remove_step(project_id, task_id, step_id)
                });
            }
            Message::MoveStep {
                project_id,
                task_id,
                step_id,
                to,
            } => {
                self.work_job(client, project_id, |work| {
                    work.move_step(project_id, task_id, step_id, to)
                });
            }
            Message::ToggleStep {
                project_id,
                task_id,
                step_id,
            } => {
                self.work_job(client, project_id, |work| {
                    work.toggle_step(project_id, task_id, step_id)
                });
            }
            Message::AssignAgent {
                project_id,
                agent_id,
                task_id,
            } => {
                self.work_job(client, project_id, |work| {
                    work.assign_agent(project_id, agent_id, task_id)
                });
            }
            Message::SendToAgent {
                project_id,
                agent_id,
                text,
            } => {
                self.work_job(client, project_id, |work| {
                    work.send_to_agent(project_id, agent_id, text)
                });
            }

            // Response-direction variants are never received here. Dropping one silently would
            // hide a wiring mistake, so it is named.
            other => {
                tracing::warn!("the coordinator was sent a message only it may send: {other:?}")
            }
        }
    }

    /// Hand one work-family message to the work.
    ///
    /// The only thing this decides is whether the project exists, and it is not a formality: a task
    /// file written for an id the catalogue does not hold would be collected as an orphan at the
    /// next boot, so the write must never happen. The record is a lookup in memory, so this costs
    /// no syscall — the same as `file_job`.
    fn work_job(
        &mut self,
        client: ClientId,
        project_id: ProjectId,
        change: impl FnOnce(&mut Work) -> Vec<Reply>,
    ) {
        if self.projects.record(project_id).is_none() {
            self.host.send(
                To::Client(client),
                Message::WorkError {
                    project_id,
                    task_id: None,
                    error: "no such project".to_string(),
                },
            );
            return;
        }
        let replies = change(&mut self.work);
        self.answer(client, replies);
    }

    /// Hand one file-family request to the worker.
    ///
    /// The only thing this decides is which folder the request is against; a project the catalogue
    /// does not hold is refused here rather than reaching a thread that could not answer it.
    fn file_job(
        &self,
        client: ClientId,
        project_id: ProjectId,
        rel_path: &str,
        request: files::Request,
    ) {
        let Some(record) = self.projects.record(project_id) else {
            self.host.send(
                To::Client(client),
                files::file_error(
                    project_id,
                    rel_path,
                    FileError::Refused("no such project".to_string()),
                ),
            );
            return;
        };

        self.files.submit(files::Job {
            project_id,
            root: PathBuf::from(&record.path),
            request,
            reply_to: self.host.mailbox(To::Client(client)),
        });
    }

    /// Hand one git-family request to the worker.
    ///
    /// The only thing this decides is which folder the request is against; a project the catalogue
    /// does not hold is refused here rather than reaching a thread that could not answer it.
    fn git_job(&self, client: ClientId, project_id: ProjectId, request: git::Request) {
        let Some(record) = self.projects.record(project_id) else {
            self.host.send(
                To::Client(client),
                git::git_error(project_id, ubiq_proto::git::GitError::NotFound),
            );
            return;
        };

        self.git.submit(git::Job {
            project_id,
            root: PathBuf::from(&record.path),
            request,
            reply_to: self.host.mailbox(To::Client(client)),
        });
    }

    fn git_forget(&self, client: ClientId, project_id: ProjectId) {
        self.git.submit(git::Job {
            project_id,
            root: PathBuf::new(),
            request: git::Request::Forget,
            reply_to: self.host.mailbox(To::Client(client)),
        });
    }

    fn spawn_workspace(
        &mut self,
        client: ClientId,
        session_id: SessionId,
        project_id: ProjectId,
        rel_path: Option<String>,
        agent_type: Option<String>,
        args: Vec<String>,
    ) {
        // A pane runs in a project's folder, so everything about that folder is settled before a
        // pseudo-terminal exists. A spawn that fails here leaves nothing on screen to close.
        let cwd = match self.resolve_cwd(client, project_id, rel_path.as_deref()) {
            Some(cwd) => cwd,
            None => return,
        };

        let pane_id = PaneId::generate();
        let agent_type = agent_type.unwrap_or_else(default_agent_type);

        let spawned = pty::spawn(
            &agent_type,
            &args,
            Some(cwd.as_path()),
            INITIAL_COLS,
            INITIAL_ROWS,
        );
        let (pane, child) = match spawned {
            Ok(spawned) => spawned,
            Err(error) => {
                tracing::error!("pane {pane_id}: starting {agent_type} failed: {error:#}");
                self.host.send(
                    To::Client(client),
                    Message::PaneError {
                        pane_id,
                        error: error.to_string(),
                    },
                );
                return;
            }
        };
        tracing::info!("pane {pane_id}: started {agent_type} in session {session_id} for {client}");

        // The owner is recorded before the pane is announced, so nothing can arrive about a pane
        // the routing table has never heard of.
        self.owners.insert(pane_id, client);
        let mailbox = self.host.mailbox(To::Client(client));

        if let Err(error) = pane.forward_output(pane_id, mailbox.clone()) {
            self.owners.remove(&pane_id);
            mailbox.send(Message::PaneError {
                pane_id,
                error: error.to_string(),
            });
            return;
        }
        pty::reap(pane_id, child, mailbox.clone());
        self.panes.insert(pane_id, pane);

        // The picker's terminal count, and the confirmation it puts in front of a close, are only
        // real because of this.
        self.pane_projects.insert(pane_id, project_id);
        let replies = self.projects.pane_opened(project_id);
        self.answer(client, replies);

        mailbox.send(Message::WorkspaceSpawned {
            workspace: WorkspaceInfo {
                id: pane_id,
                session_id,
                agent_type,
                project_id,
                rel_path,
                cols: INITIAL_COLS,
                rows: INITIAL_ROWS,
                running: true,
            },
        });
    }

    /// Where a pane starts, or nothing at all and the window told why.
    ///
    /// The refusal is a `ProjectError` rather than a `PaneError`, because a `PaneError` names a pane
    /// the interface has never been told about and so has nowhere to put it. An unhealthy project
    /// also gets its fresh snapshot broadcast, so every picker marks the row from the probe that
    /// just happened rather than waiting to be asked.
    fn resolve_cwd(
        &mut self,
        client: ClientId,
        project_id: ProjectId,
        rel_path: Option<&str>,
    ) -> Option<PathBuf> {
        let Some(record) = self.projects.record(project_id) else {
            self.refuse_spawn(client, project_id, "no such project".to_string(), false);
            return None;
        };
        let root = PathBuf::from(&record.path);

        let health = health::probe(&root);
        if !health.is_ok() {
            let reason = match &health {
                ProjectHealth::Missing => "its folder is not there".to_string(),
                ProjectHealth::NotADirectory => "its path is not a folder".to_string(),
                ProjectHealth::Unreadable(reason) => reason.clone(),
                ProjectHealth::Ok => unreachable!("the branch above tested for this"),
            };
            self.refuse_spawn(client, project_id, reason, true);
            return None;
        }

        match rel_path {
            None => Some(root),
            Some(rel_path) => match files::path::resolve(&root, rel_path) {
                Ok(cwd) if cwd.is_dir() => Some(cwd),
                Ok(_) => {
                    self.refuse_spawn(
                        client,
                        project_id,
                        format!("{rel_path} is not a folder"),
                        false,
                    );
                    None
                }
                Err(error) => {
                    self.refuse_spawn(client, project_id, format!("{rel_path}: {error}"), false);
                    None
                }
            },
        }
    }

    /// Say why a pane was not started, and re-probe when the folder itself is the reason.
    fn refuse_spawn(
        &mut self,
        client: ClientId,
        project_id: ProjectId,
        error: String,
        reprobe: bool,
    ) {
        tracing::warn!("refusing a workspace in project {project_id}: {error}");
        self.host.send(
            To::Client(client),
            Message::ProjectError {
                project_id: Some(project_id),
                error,
            },
        );
        if reprobe {
            let replies = self.projects.refresh(project_id);
            self.answer(client, replies);
        }
    }
}

/// What a session starts when it is not told what to start: the user's own shell.
fn default_agent_type() -> String {
    std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string())
}
