//! The work: the tasks a project has written down, and the sessions and agents doing them.
//!
//! One half of this is durable and one half is not, which is the whole shape of the module. A
//! [`ubiq_proto::work::TaskRecord`] is the user's data, kept in the project's `tasks.toml`;
//! sessions and agents are answered per request with nothing behind them. [`mock`] is where the
//! second half comes from today, and where a new project's first tasks are seeded from.

pub mod mock;

use std::collections::{HashMap, HashSet};

use chrono::Utc;
use ubiq_proto::ids::{ProjectId, SessionId, StepId, TaskId};
use ubiq_proto::messages::Message;
use ubiq_proto::work::{
    AgentId, Priority, Shape, Speaker, Status, Step, TaskRecord, Turn, WorkAgent, WorkSession,
};

use crate::reply::Reply;
use crate::store::{StoreError, TaskStore};

/// One project's invented half. No store, no trait and no file — which is what "sessions and
/// agents are still mocks" means in code rather than in prose.
struct Mock {
    sessions: Vec<WorkSession>,
    agents: Vec<WorkAgent>,
}

/// A project's tasks, and the sessions and agents working on them.
pub struct Work {
    tasks: Box<dyn TaskStore>,
    /// One project's tasks, once a window has asked for them. Memory is authoritative for the
    /// session; the store is where it is made durable.
    loaded: HashMap<ProjectId, Vec<TaskRecord>>,
    /// The mocks, minted per project on the first ask.
    mocks: HashMap<ProjectId, Mock>,
    /// The agents that are actually running, per project. These are not
    /// invented and not written down: a live agent exists for as long as its
    /// harness does, and a restart starts none of them.
    live: HashMap<ProjectId, Vec<WorkAgent>>,
    /// Which projects have already been told their tasks are not durable, so the user hears it
    /// once per project rather than on every edit.
    warned: HashSet<ProjectId>,
    /// Projects whose file this Ubiq must not write, because the load found a version it does not
    /// understand. Overwriting it would replace a format that holds more than this one can with a
    /// format that cannot — which is exactly what refusing to parse it was meant to avoid.
    sealed: HashSet<ProjectId>,
}

impl Work {
    /// Open the work.
    ///
    /// Unlike [`crate::projects::Projects::open`] this answers nothing: no file is read until a
    /// window asks for a project's work, so there is nothing that can have gone wrong before a
    /// window existed. A project's tasks are the opposite of the catalogue — nobody needs them
    /// until a window points at that project, and reading every one at boot would cost a file open
    /// per catalogue row for work nobody looked at.
    pub fn open(tasks: Box<dyn TaskStore>) -> Self {
        Self {
            tasks,
            loaded: HashMap::new(),
            mocks: HashMap::new(),
            live: HashMap::new(),
            warned: HashSet::new(),
            sealed: HashSet::new(),
        }
    }

    /// Forget everything about a project the catalogue no longer holds.
    ///
    /// Its directory, and the `tasks.toml` in it, went with
    /// [`crate::projects::Projects::forget`]; this is the memory that was left.
    pub fn forget(&mut self, project: ProjectId) {
        self.loaded.remove(&project);
        self.mocks.remove(&project);
        self.warned.remove(&project);
        self.sealed.remove(&project);
    }

    // ── loading, seeding and keeping ─────────────────────────────────

    /// Put a project's tasks in memory, seeding them the first time. Answers whatever should be
    /// said about how that went, which is nothing in the ordinary case.
    ///
    /// **The seeding rule lives here and nowhere else.** A project's tasks are seeded from the
    /// fixture exactly once: the first ask for a project with no `tasks.toml` mints the fixture,
    /// writes it, and answers it, and from then on the file is the truth — including when it holds
    /// no tasks at all. A user who deletes every task gets an empty board at the next boot, because
    /// an absent file and an empty list are different things. It is the distinction
    /// [`Message::Preferences`] already draws between a blob never set and an empty one.
    fn ensure(&mut self, project: ProjectId) -> Vec<Reply> {
        if self.loaded.contains_key(&project) {
            return Vec::new();
        }

        match self.tasks.load(project) {
            Ok(Some(list)) => {
                self.loaded.insert(project, list);
                Vec::new()
            }
            Ok(None) => {
                self.loaded.insert(project, mock::tasks());
                // Written now rather than at the first edit, so what the user sees on their first
                // look is already theirs: renamable, movable and deletable, and still there after
                // a restart.
                self.keep(project).into_iter().collect()
            }
            Err(error) => {
                // Never seed over a file that could not be read. Seeding here is how you overwrite
                // the thing preserving it was meant to save — the same reasoning that stops
                // `gc::collect` running after a load that failed.
                if matches!(error, StoreError::UnknownVersion { .. }) {
                    self.sealed.insert(project);
                }
                // A corrupt file was moved aside, which leaves no file at all — and no file means a
                // new project, so the next boot would put the fixture on top of what the user just
                // lost. Writing the empty list down now is what stops that: they start empty, and
                // their tasks are still in the file `preserve_aside` kept.
                //
                // Only then. Every other failure leaves `tasks.toml` exactly as it was — a
                // permissions blip, an EIO, a mount that has not come up — and writing an empty
                // list over an intact file would turn one unlucky read into the loss of the whole
                // board. A project whose tasks could not be read is answered empty for the session
                // and left alone on disk.
                let preserved = matches!(
                    error,
                    StoreError::Parse {
                        preserved_as: Some(_),
                        ..
                    }
                );
                self.loaded.insert(project, Vec::new());
                let mut replies: Vec<Reply> = self.warn_once(project, &error).into_iter().collect();
                if preserved {
                    replies.extend(self.keep(project));
                }
                replies
            }
        }
    }

    /// Make what is in memory durable, and say so only the first time it cannot be.
    fn keep(&mut self, project: ProjectId) -> Option<Reply> {
        if self.sealed.contains(&project) {
            return self.warn_once_message(
                project,
                "this project's tasks were written by a newer Ubiq, so they are not being changed on disk",
            );
        }
        let list = self.loaded.get(&project)?;
        match self.tasks.save(project, list) {
            Ok(()) => None,
            Err(error) => self.warn_once(project, &error),
        }
    }

    fn warn_once(&mut self, project: ProjectId, error: &StoreError) -> Option<Reply> {
        self.warn_once_message(project, error.to_string())
    }

    fn warn_once_message(
        &mut self,
        project: ProjectId,
        message: impl Into<String>,
    ) -> Option<Reply> {
        let message = message.into();
        if !self.warned.insert(project) {
            tracing::debug!("{project}'s tasks are still not durable: {message}");
            return None;
        }
        Some(Reply::Asker(work_error(project, None, message)))
    }

    /// Have a project's work in hand: its tasks read or seeded, and its mock minted.
    ///
    /// Every public method starts here, because the order matters and is not obvious. `mock` links
    /// its agents against the task list, so a mock minted before the tasks were read would be
    /// minted with nothing to point at and stay that way for the session — the graph would draw
    /// eleven cards and not one outline.
    fn prepare(&mut self, project: ProjectId) -> Vec<Reply> {
        let replies = self.ensure(project);
        self.mock(project);
        replies
    }

    /// The project's mock, minted if this is the first ask for it.
    ///
    /// Minting is also where each agent is given the task it serves, because that is the only
    /// moment both lists are in hand: the fixture cannot name a task id, since the ids belong to
    /// whatever `tasks.toml` holds. It happens **once**, not on every ask — an agent the user has
    /// since dragged into another outline must keep where they put it.
    fn mock(&mut self, project: ProjectId) -> &Mock {
        if !self.mocks.contains_key(&project) {
            let sessions = mock::sessions();
            let mut agents = mock::agents();
            link(&mut agents, self.loaded.get(&project).map_or(&[], |l| l));
            self.mocks.insert(project, Mock { sessions, agents });
        }
        &self.mocks[&project]
    }

    /// Take every agent off a task that has gone, and say which ones moved.
    ///
    /// A card pointing at a deleted task would be drawn in no container and counted in one, so the
    /// repair is the interface's to hear about rather than something it works out.
    fn unlink(&mut self, project: ProjectId, task: TaskId) -> Vec<Reply> {
        let Some(mock) = self.mocks.get_mut(&project) else {
            return Vec::new();
        };
        mock.agents
            .iter_mut()
            .filter(|a| a.task == Some(task))
            .map(|agent| {
                agent.task = None;
                Reply::Asker(Message::AgentChanged {
                    project_id: project,
                    agent: Box::new(agent.clone()),
                })
            })
            .collect()
    }

    /// Change one task, whatever the change is.
    ///
    /// The one place a task mutation lands, so the order — apply, stamp, answer, keep — is written
    /// once rather than in each of the ten methods below. `change` answers whether anything
    /// actually changed, so a drop into the column a card is already in costs no write and no
    /// redraw.
    fn with_task(
        &mut self,
        project: ProjectId,
        task: TaskId,
        change: impl FnOnce(&mut TaskRecord) -> Result<bool, String>,
    ) -> Vec<Reply> {
        let mut replies = self.prepare(project);

        let Some(list) = self.loaded.get_mut(&project) else {
            replies.push(Reply::Asker(work_error(
                project,
                Some(task),
                "no work for that project",
            )));
            return replies;
        };
        let Some(record) = list.iter_mut().find(|t| t.id == task) else {
            replies.push(Reply::Asker(work_error(
                project,
                Some(task),
                "no such task",
            )));
            return replies;
        };

        let changed = match change(record) {
            Ok(changed) => changed,
            Err(refusal) => {
                replies.push(Reply::Asker(work_error(project, Some(task), refusal)));
                return replies;
            }
        };
        if !changed {
            return replies;
        }

        record.updated_at = Utc::now();
        let task = record.clone();
        replies.push(Reply::Asker(Message::TaskChanged {
            project_id: project,
            task,
        }));
        replies.extend(self.keep(project));
        replies
    }

    /// Change one mock agent. No `keep`: an agent is not written down.
    fn with_agent(
        &mut self,
        project: ProjectId,
        agent: AgentId,
        change: impl FnOnce(&mut WorkAgent) -> bool,
    ) -> Vec<Reply> {
        let mut replies = self.prepare(project);
        let Some(mock) = self.mocks.get_mut(&project) else {
            return replies;
        };
        let Some(record) = mock.agents.iter_mut().find(|a| a.id == agent) else {
            replies.push(Reply::Asker(work_error(project, None, "no such agent")));
            return replies;
        };
        if !change(record) {
            return replies;
        }
        let agent = Box::new(record.clone());
        replies.push(Reply::Asker(Message::AgentChanged {
            project_id: project,
            agent,
        }));
        replies
    }

    // ── the message family ──────────────────────────────────────────

    /// One project's work, whole. All three lists in one answer, because the graph draws a card
    /// and the session it names in the same frame.
    pub fn list(&mut self, project: ProjectId) -> Vec<Reply> {
        let mut replies = self.prepare(project);

        let mock = &self.mocks[&project];
        // Live agents first: a real one belongs above the invented ones for as
        // long as both are in the list.
        let mut agents = self.live.get(&project).cloned().unwrap_or_default();
        agents.extend(mock.agents.iter().cloned());
        replies.push(Reply::Asker(Message::WorkList {
            project_id: project,
            sessions: mock.sessions.clone(),
            agents,
            tasks: self.loaded.get(&project).cloned().unwrap_or_default(),
        }));
        replies
    }

    /// Put a running agent in the project's list.
    ///
    /// It joins the same list the sidebar and the graph already read, so
    /// nothing downstream has to learn that some agents are real.
    pub fn add_live_agent(&mut self, project: ProjectId, agent: WorkAgent) {
        let agents = self.live.entry(project).or_default();
        match agents.iter_mut().find(|held| held.id == agent.id) {
            Some(held) => *held = agent,
            None => agents.push(agent),
        }
    }

    /// Take a running agent out of the project's list, once its harness has
    /// gone and its transcript is all that is left.
    pub fn remove_live_agent(&mut self, project: ProjectId, agent: AgentId) {
        if let Some(agents) = self.live.get_mut(&project) {
            agents.retain(|held| held.id != agent);
        }
    }

    pub fn create(
        &mut self,
        project: ProjectId,
        title: String,
        session: Option<SessionId>,
    ) -> Vec<Reply> {
        let title = title.trim().to_string();
        if title.is_empty() {
            return vec![Reply::Asker(work_error(
                project,
                None,
                "a task needs a title",
            ))];
        }

        let mut replies = self.prepare(project);
        if let Some(refusal) = self.no_such_session(project, session) {
            replies.push(Reply::Asker(work_error(project, None, refusal)));
            return replies;
        }
        let task = TaskRecord::new(title, session, Utc::now());
        let Some(list) = self.loaded.get_mut(&project) else {
            return replies;
        };
        list.push(task.clone());

        // Its own variant rather than a change, because the interface cannot know an id it did not
        // mint and the board selects the card it just made.
        replies.push(Reply::Asker(Message::TaskCreated {
            project_id: project,
            task,
        }));
        replies.extend(self.keep(project));
        replies
    }

    /// Change what a task is. Display only, so the only refusal is a task that is not there.
    pub fn update(
        &mut self,
        project: ProjectId,
        task: TaskId,
        title: Option<String>,
        description: Option<String>,
        priority: Option<Priority>,
        shape: Option<Shape>,
    ) -> Vec<Reply> {
        self.with_task(project, task, |record| {
            let mut changed = false;
            // An empty title is a slip rather than an intention, so it is ignored the way a
            // project's rename ignores one.
            if let Some(title) = title.filter(|t| !t.trim().is_empty()) {
                let title = title.trim().to_string();
                changed |= record.title != title;
                record.title = title;
            }
            // A description, unlike a title, may be emptied: clearing one is a thing to mean.
            if let Some(description) = description {
                changed |= record.description != description;
                record.description = description;
            }
            if let Some(priority) = priority {
                changed |= record.priority != priority;
                record.priority = priority;
            }
            if let Some(shape) = shape {
                changed |= record.shape != shape;
                record.shape = shape;
            }
            Ok(changed)
        })
    }

    /// Move a task to another column, and nothing else about it.
    pub fn move_task(&mut self, project: ProjectId, task: TaskId, status: Status) -> Vec<Reply> {
        self.with_task(project, task, |record| {
            let changed = record.status != status;
            record.status = status;
            Ok(changed)
        })
    }

    /// Hand a task to a session, or take it back.
    pub fn assign(
        &mut self,
        project: ProjectId,
        task: TaskId,
        session: Option<SessionId>,
    ) -> Vec<Reply> {
        let mut replies = self.prepare(project);
        if let Some(refusal) = self.no_such_session(project, session) {
            replies.push(Reply::Asker(work_error(project, Some(task), refusal)));
            return replies;
        }
        replies.extend(self.with_task(project, task, |record| {
            let changed = record.session != session;
            record.session = session;
            Ok(changed)
        }));
        replies
    }

    /// Drop a task. Unlike forgetting a project this really deletes: nothing is left to point at.
    pub fn delete(&mut self, project: ProjectId, task: TaskId) -> Vec<Reply> {
        let mut replies = self.prepare(project);
        let Some(list) = self.loaded.get_mut(&project) else {
            replies.push(Reply::Asker(work_error(
                project,
                Some(task),
                "no work for that project",
            )));
            return replies;
        };
        let before = list.len();
        list.retain(|t| t.id != task);
        if list.len() == before {
            replies.push(Reply::Asker(work_error(
                project,
                Some(task),
                "no such task",
            )));
            return replies;
        }

        replies.push(Reply::Asker(Message::TaskDeleted {
            project_id: project,
            task_id: task,
        }));
        replies.extend(self.keep(project));
        replies.extend(self.unlink(project, task));
        replies
    }

    pub fn add_step(&mut self, project: ProjectId, task: TaskId, title: String) -> Vec<Reply> {
        self.with_task(project, task, |record| {
            let title = title.trim().to_string();
            if title.is_empty() {
                return Err("a sub-task needs a title".to_string());
            }
            record.steps.push(Step::new(title));
            Ok(true)
        })
    }

    pub fn rename_step(
        &mut self,
        project: ProjectId,
        task: TaskId,
        step: StepId,
        title: String,
    ) -> Vec<Reply> {
        self.with_task(project, task, |record| {
            let title = title.trim().to_string();
            if title.is_empty() {
                return Err("a sub-task needs a title".to_string());
            }
            let Some(record) = record.step_mut(step) else {
                return Err("no such sub-task".to_string());
            };
            let changed = record.title != title;
            record.title = title;
            Ok(changed)
        })
    }

    pub fn remove_step(&mut self, project: ProjectId, task: TaskId, step: StepId) -> Vec<Reply> {
        self.with_task(project, task, |record| {
            let before = record.steps.len();
            record.steps.retain(|s| s.id != step);
            if record.steps.len() == before {
                return Err("no such sub-task".to_string());
            }
            Ok(true)
        })
    }

    /// Reorder one step. `to` is clamped, because a list that shortened under a drag is not an
    /// error the user can do anything about.
    pub fn move_step(
        &mut self,
        project: ProjectId,
        task: TaskId,
        step: StepId,
        to: usize,
    ) -> Vec<Reply> {
        self.with_task(project, task, |record| {
            let Some(from) = record.steps.iter().position(|s| s.id == step) else {
                return Err("no such sub-task".to_string());
            };
            let to = to.min(record.steps.len().saturating_sub(1));
            if from == to {
                return Ok(false);
            }
            let step = record.steps.remove(from);
            record.steps.insert(to, step);
            Ok(true)
        })
    }

    /// Tick or untick a step. Unticking lands on idle, because nothing here can know what its
    /// owner would go back to doing.
    pub fn toggle_step(&mut self, project: ProjectId, task: TaskId, step: StepId) -> Vec<Reply> {
        self.with_task(project, task, |record| {
            let Some(record) = record.step_mut(step) else {
                return Err("no such sub-task".to_string());
            };
            record.toggle();
            Ok(true)
        })
    }

    /// Move an agent's card into another task, or out of every one.
    ///
    /// Where the card *sits* never reaches the host — that is the interface's own fact, and it is
    /// what the interface keeps when a drag ends. Which task the agent *serves* is this.
    pub fn assign_agent(
        &mut self,
        project: ProjectId,
        agent: AgentId,
        task: Option<TaskId>,
    ) -> Vec<Reply> {
        let mut replies = self.prepare(project);
        if let Some(task) = task
            && !self
                .loaded
                .get(&project)
                .is_some_and(|list| list.iter().any(|t| t.id == task))
        {
            replies.push(Reply::Asker(work_error(
                project,
                Some(task),
                "no such task",
            )));
            return replies;
        }
        replies.extend(self.with_agent(project, agent, |record| {
            if record.task == task {
                return false;
            }
            record.task = task;
            // An agent that moved to another task no longer answers to whoever spawned it there.
            record.parent = None;
            true
        }));
        replies
    }

    /// Put a line in an agent's thread.
    ///
    /// Nothing answers it, and that is not an omission: the thread says as much in as many words,
    /// because a fabricated reply is the one thing a screen with no live agent must not draw.
    pub fn send_to_agent(
        &mut self,
        project: ProjectId,
        agent: AgentId,
        text: String,
    ) -> Vec<Reply> {
        let text = text.trim().to_string();
        if text.is_empty() {
            return Vec::new();
        }
        self.with_agent(project, agent, |record| {
            record.thread.push(Turn {
                from: Speaker::You,
                text,
            });
            true
        })
    }

    /// Whether a session id names one of this project's, so a task cannot be handed to a session
    /// that is not there. `None` is a task nobody has started and is always allowed.
    ///
    /// **Call `prepare` first.** This reads the mock rather than minting it, so a project whose
    /// mock has never been asked for would refuse every one of its five sessions.
    fn no_such_session(
        &self,
        project: ProjectId,
        session: Option<SessionId>,
    ) -> Option<&'static str> {
        let session = session?;
        let known = self
            .mocks
            .get(&project)
            .is_some_and(|mock| mock.sessions.iter().any(|s| s.id == session));
        (!known).then_some("no such session")
    }
}

/// Give each mock agent the task its session is working on, and no task at all when its session is
/// not working on one.
///
/// The fixture cannot say which task an agent serves, because a task's id belongs to the project's
/// `tasks.toml` rather than to the fixture. What it can say is which *session* the agent is in, and
/// a session is a piece of work — so an agent serves the task its session has **in flight**. That
/// stays true after the user has renamed, moved, added or deleted a task, which a stored id could
/// not.
///
/// **In flight, and nothing else.** An agent whose session has only finished work is left with no
/// task, which is not a gap: the graph draws an agent nobody gave work to above the containers, and
/// that is exactly where the project manager coordinating everything belongs. Reaching for the
/// session's first task instead puts it inside a container for work nobody is doing.
fn link(agents: &mut [WorkAgent], tasks: &[TaskRecord]) {
    for agent in agents {
        agent.task = tasks
            .iter()
            .find(|t| {
                t.session == Some(agent.session)
                    && matches!(t.status, Status::InProgress | Status::InReview)
            })
            .map(|t| t.id);
    }
}

fn work_error(project: ProjectId, task: Option<TaskId>, error: impl Into<String>) -> Message {
    let error = error.into();
    tracing::warn!("{project}'s work: {error}");
    Message::WorkError {
        project_id: project,
        task_id: task,
        error,
    }
}
