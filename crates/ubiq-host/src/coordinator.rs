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
use ubiq_proto::conversation::StopReason;
use ubiq_proto::files::FileError;
use ubiq_proto::ids::{PaneId, ProjectId, SessionId};
use ubiq_proto::messages::{Message, WorkspaceInfo};
use ubiq_proto::projects::ProjectHealth;
use ubiq_proto::work::{Activity, AgentId, WorkAgent, WorkSession};

use crate::agent::Agents;
use crate::config::ConfigRoot;
use crate::conversation::Conversation;
use crate::files::{self, Files};
use crate::git::{self, Git};
use crate::health;
use crate::projects::Projects;
use crate::pty::{self, Pty};
use crate::reply::Reply;
use crate::settings::Settings;
use crate::shells;
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
    /// Which agent types can run here, and the composer that turns one into a launch.
    agents: Agents,
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
    /// The agents being talked to, keyed by the id that multiplexes them onto the bus. One entry
    /// is one harness on one pump thread.
    conversations: HashMap<AgentId, Conversation>,
    /// Which project and window each conversation belongs to — the same routing table the panes
    /// have, for the same two reasons: a reply goes to the window that asked, and a project's
    /// count changes when one ends.
    conversation_owners: HashMap<AgentId, (ClientId, ProjectId)>,
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
        // A run directory outlives its pane only when Ubiq did not get to close it, and no pane
        // from a previous process is still running, so the sweep happens once here.
        let agents = Agents::new(root.path.clone(), settings.host().isolate_agents);
        agents.sweep();

        Self {
            host,
            root,
            projects,
            work,
            settings,
            agents,
            files: Files::start(),
            git: Git::start(),
            pending,
            pane_projects: HashMap::new(),
            panes: HashMap::new(),
            owners: HashMap::new(),
            focused: HashMap::new(),
            conversations: HashMap::new(),
            conversation_owners: HashMap::new(),
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

        let talking: Vec<AgentId> = self
            .conversation_owners
            .iter()
            .filter(|(_, (owner, _))| *owner == client)
            .map(|(agent_id, _)| *agent_id)
            .collect();
        for agent_id in talking {
            tracing::info!("{client} has gone; stopping agent {agent_id}");
            self.end_conversation(agent_id, StopReason::Cancelled);
        }
    }

    /// A pane has ended, however it ended: the project it belonged to has one fewer.
    fn pane_gone(&mut self, client: ClientId, pane_id: PaneId) {
        // The pane owned its run's configuration directory — credentials seeded into it included
        // — so that goes when the pane does. A harness that exited by itself reaches here too:
        // the interface closes the tab on `PaneExited`, and closing a tab is a `CloseWorkspace`.
        self.agents.retire(pane_id);
        if let Some(project_id) = self.pane_projects.remove(&pane_id) {
            let replies = self.projects.pane_closed(project_id);
            self.answer(client, replies);
        }
    }

    /// Tell the window that asked that its pane never started.
    ///
    /// A `PaneError` and no `WorkspaceSpawned`: the interface was never told the pane exists, so
    /// there is nothing on screen to close, and the error belongs where the user was looking.
    fn refuse_pane(&self, client: ClientId, pane_id: PaneId, error: String) {
        self.host
            .send(To::Client(client), Message::PaneError { pane_id, error });
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

            // What can be started here, asked by the new-pane menu as it opens. Probed on every
            // ask: a shell installed since the window opened is offered without a restart.
            Message::ListShells => {
                self.host.send(
                    To::Client(client),
                    Message::ShellList {
                        shells: shells::available(),
                    },
                );
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
            Message::ListAgentTypes => {
                let agent_types = self.agents.types();
                self.host
                    .send(To::Client(client), Message::AgentTypes { agent_types });
            }

            Message::GetSettings { layer } => {
                let reply = self.settings.get(layer);
                self.answer(client, vec![reply]);
            }
            Message::SetSettings { layer, value } => {
                let replies = self.settings.set(layer, value);
                // Whether an agent is confined is acted on at the next spawn, so the setting is
                // re-read here rather than kept in a copy that could go stale.
                self.agents.set_isolate(self.settings.host().isolate_agents);
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

            Message::StartConversation {
                project_id,
                session_id,
                rel_path,
                agent_type,
            } => {
                self.start_conversation(client, project_id, session_id, rel_path, agent_type);
            }
            Message::PromptAgent { agent_id, text } => {
                self.drive(client, agent_id, |conversation| conversation.prompt(text));
            }
            Message::CancelTurn { agent_id } => {
                self.drive(client, agent_id, |conversation| conversation.cancel());
            }
            Message::AnswerPermission {
                agent_id,
                request_id,
                option_id,
            } => {
                self.drive(client, agent_id, |conversation| {
                    conversation.answer_permission(request_id, option_id)
                });
            }
            Message::SetAgentConfig {
                agent_id,
                config_id,
                value,
            } => {
                self.drive(client, agent_id, |conversation| {
                    conversation.set_config(config_id, value)
                });
            }
            Message::EndConversation { agent_id } => {
                self.end_conversation(agent_id, StopReason::Cancelled);
            }

            // Response-direction variants are never received here. Dropping one silently would
            // hide a wiring mistake, so it is named.
            other => {
                tracing::warn!("the coordinator was sent a message only it may send: {other:?}")
            }
        }
    }

    /// Start a live agent: compose the harness, drive it over structured I/O, and pump what it
    /// says onto the bus.
    ///
    /// The conversation sibling of [`Self::spawn_workspace`], and the shape is deliberately the
    /// same — settle the folder, mint the id, compose, then wire the reader — because the two are
    /// one workspace wearing different faces. What differs is that this one has no pseudo-terminal
    /// and so no geometry, and that its id is an agent's.
    fn start_conversation(
        &mut self,
        client: ClientId,
        project_id: ProjectId,
        session_id: SessionId,
        rel_path: Option<String>,
        agent_type: String,
    ) {
        let agent_id = AgentId::generate();

        let Some(cwd) = self.resolve_cwd(client, project_id, rel_path.as_deref()) else {
            return;
        };

        if !self.agents.is_agent_type(&agent_type) {
            // A shell is a pane, not a conversation: there is nothing on the other end of a pipe
            // to have a conversation with.
            self.refuse_conversation(
                client,
                agent_id,
                format!("'{agent_type}' is not an agent type"),
            );
            return;
        }

        let (composed, bridge) = match self.agents.converse(agent_id, &agent_type, &cwd) {
            Ok(started) => started,
            Err(error) => {
                tracing::error!("agent {agent_id}: starting {agent_type} failed: {error:#}");
                self.agents.retire_agent(agent_id);
                self.refuse_conversation(client, agent_id, format!("{error:#}"));
                return;
            }
        };
        tracing::info!(
            agent = %agent_id,
            harness = %agent_type,
            dir = %composed.dir.display(),
            "conversation started"
        );

        let label = self
            .agents
            .types()
            .into_iter()
            .find(|offered| offered.id == agent_type)
            .map(|offered| offered.label)
            .unwrap_or_else(|| agent_type.clone());

        let agent = WorkAgent {
            id: agent_id,
            session: session_id,
            task: None,
            parent: None,
            name: label.clone(),
            role: "agent".to_string(),
            activity: Activity::Thinking,
            note: String::new(),
            branch: String::new(),
            tokens: 0.0,
            harness: label,
            // Empty until the harness says which model answered — it is the only thing that
            // knows, and guessing would put a wrong name under a real conversation.
            model: String::new(),
            context_pct: 0,
            thread: Vec::new(),
        };
        // The window's own session, named after the project it is open on: the work's sessions are
        // invented and this one is not, so nothing else in the list would account for this agent.
        let session = WorkSession {
            id: session_id,
            name: self
                .projects
                .record(project_id)
                .map(|record| record.name.clone())
                .unwrap_or_else(|| "session".to_string()),
            branch: String::new(),
            worktree: false,
        };
        self.work
            .add_live_agent(project_id, agent.clone(), session.clone());
        self.conversation_owners
            .insert(agent_id, (client, project_id));

        // The pump starts before the window is told, so the capability the window needs — whether
        // this harness takes a second turn — is known by the time the message carrying it goes.
        let mailbox = self.host.mailbox(To::Client(client));
        let conversation = Conversation::start(agent_id, bridge, mailbox.clone());
        mailbox.send(Message::ConversationStarted {
            project_id,
            agent: Box::new(agent),
            session,
            accepts_input: conversation.accepts_input(),
        });
        self.conversations.insert(agent_id, conversation);
    }

    /// Hand one conversation-family message to the agent it names.
    ///
    /// A window may only drive an agent it started, on the same terms as a pane; and a harness
    /// that takes no input after launch refuses here rather than swallowing the turn, so a
    /// composer that should not have offered to send finds out.
    fn drive(
        &mut self,
        client: ClientId,
        agent_id: AgentId,
        act: impl FnOnce(&Conversation) -> anyhow::Result<()>,
    ) {
        if !self.drives(client, agent_id) {
            return;
        }
        let Some(conversation) = self.conversations.get(&agent_id) else {
            return;
        };
        if let Err(error) = act(conversation) {
            tracing::warn!("agent {agent_id}: {error:#}");
            self.host.send(
                To::Client(client),
                Message::ConversationError {
                    agent_id,
                    error: format!("{error:#}"),
                },
            );
        }
    }

    /// Whether this window is the one that started the agent.
    fn drives(&self, client: ClientId, agent_id: AgentId) -> bool {
        match self.conversation_owners.get(&agent_id) {
            Some((owner, _)) if *owner == client => true,
            Some(_) => {
                tracing::warn!(
                    "{client} sent a message about agent {agent_id}, which it does not own"
                );
                false
            }
            // The agent has already gone; its last messages are in flight behind it.
            None => false,
        }
    }

    /// Stop an agent and take everything it owned with it.
    ///
    /// The pump answers its own `ConversationEnded` on the way out, so nothing is said here: two
    /// endings for one agent would leave a window unsure which it was.
    fn end_conversation(&mut self, agent_id: AgentId, reason: StopReason) {
        if let Some(conversation) = self.conversations.remove(&agent_id) {
            tracing::info!("agent {agent_id} ending: {reason:?}");
            conversation.stop();
        }
        if let Some((_, project_id)) = self.conversation_owners.remove(&agent_id) {
            self.work.remove_live_agent(project_id, agent_id);
        }
        // The agent owned its run's configuration directory — credentials seeded into it included
        // — so that goes when the agent does.
        self.agents.retire_agent(agent_id);
    }

    /// Tell the window that asked that its agent never started.
    fn refuse_conversation(&self, client: ClientId, agent_id: AgentId, error: String) {
        self.host.send(
            To::Client(client),
            Message::ConversationError { agent_id, error },
        );
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
        let agent_type = agent_type.unwrap_or_else(shells::default_program);

        // An agent type the library knows is composed — its skills, its throwaway configuration
        // and the policy it runs under all come from there. Anything else is a program name,
        // which is what a shell is.
        let composed = if self.agents.is_agent_type(&agent_type) {
            match self
                .agents
                .compose(pane_id, &agent_type, &cwd, args.clone())
            {
                Ok(composed) => Some(composed),
                Err(error) => {
                    tracing::error!("pane {pane_id}: composing {agent_type} failed: {error:#}");
                    self.refuse_pane(client, pane_id, format!("{error:#}"));
                    return;
                }
            }
        } else {
            None
        };

        let program = match &composed {
            Some(composed) => match composed.exec() {
                Ok(launch) => pty::Program {
                    program: launch.program,
                    args: launch.args,
                    env: launch.env,
                    env_remove: launch.env_remove,
                    env_clear: launch.env_clear,
                },
                Err(error) => {
                    tracing::error!("pane {pane_id}: confining {agent_type} failed: {error:#}");
                    self.agents.retire(pane_id);
                    self.refuse_pane(client, pane_id, format!("{error:#}"));
                    return;
                }
            },
            None => pty::Program::plain(&agent_type, args),
        };

        let spawned = pty::spawn(&program, Some(cwd.as_path()), INITIAL_COLS, INITIAL_ROWS);
        let (pane, child) = match spawned {
            Ok(spawned) => spawned,
            Err(error) => {
                self.agents.retire(pane_id);
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
        let confined = composed
            .as_ref()
            .is_some_and(|composed| composed.is_confined());
        tracing::info!(
            "pane {pane_id}: started {agent_type}{} in session {session_id} for {client}",
            if confined { " (confined)" } else { "" }
        );

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
