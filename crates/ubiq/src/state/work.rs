//! One project's work, as this window last heard the host describe it.
//!
//! A projection and nothing else. It is replaced wholesale by `WorkList` and one record at a time
//! by `TaskChanged` and `AgentChanged`, and every apply replaces on id — so a record arriving twice
//! changes nothing. That is the property the whole family rests on: a projection that appends on a
//! re-send is the classic duplicate-card bug.
//!
//! **Nothing here is invented.** The records are [`ubiq_proto::work`]'s own, carried across the bus
//! rather than rebuilt beside it, so all this module adds is the questions the two screens ask of
//! them — who is holding a task, what the worst thing happening in it is, how many agents are in
//! each bucket. Those live on this side because the host has no use for them: each is a fact about
//! what is drawn rather than about what is stored.
//!
//! **Where any of it sits is not here.** Position belongs to [`super::layout`], and what is
//! selected in it belongs to [`super::agents`]. This holds the work; the graph and the board hold
//! their own view of it, and a window holding three projects holds three of each.

use ubiq_proto::ids::{SessionId, StepId, TaskId};
use ubiq_proto::work::{
    Activity, AgentId, Bucket, Shape, Step, TaskRecord, WorkAgent, WorkSession,
};

/// The sessions, agents and tasks of one project, and whether the host has answered yet.
pub struct WorkProjection {
    pub sessions: Vec<WorkSession>,
    pub agents: Vec<WorkAgent>,
    pub tasks: Vec<TaskRecord>,
    /// Whether the host has answered yet. A project whose work has never arrived draws as empty
    /// rather than as a project with no work.
    pub loaded: bool,
}

impl WorkProjection {
    /// A project that has just been taken, before its `ListWork` has been answered.
    pub fn empty() -> Self {
        Self {
            sessions: Vec::new(),
            agents: Vec::new(),
            tasks: Vec::new(),
            loaded: false,
        }
    }

    // ── the projection ──────────────────────────────────────────────

    /// Replace the whole of it, as `WorkList` says it is. All three lists at once, because a board
    /// drawing a card that names a session it has not heard of is what two round trips would buy.
    pub fn replace_all(
        &mut self,
        sessions: Vec<WorkSession>,
        agents: Vec<WorkAgent>,
        tasks: Vec<TaskRecord>,
    ) {
        self.sessions = sessions;
        self.agents = agents;
        self.tasks = tasks;
        self.loaded = true;
    }

    /// Apply one task, whether it is new or a change to one already held. Answers whether it was
    /// new, which is what tells the arrangement there is something to find a place for.
    ///
    /// Replacing on id is what makes this idempotent: the same record twice is the same projection.
    pub fn apply_task(&mut self, task: TaskRecord) -> bool {
        match self.tasks.iter_mut().find(|t| t.id == task.id) {
            Some(held) => {
                *held = task;
                false
            }
            None => {
                self.tasks.push(task);
                true
            }
        }
    }

    /// The host has dropped a task. Answers whether it was there, so a delete that names nothing
    /// costs no redraw.
    pub fn forget_task(&mut self, id: TaskId) -> bool {
        let Some(at) = self.tasks.iter().position(|t| t.id == id) else {
            return false;
        };
        self.tasks.remove(at);
        true
    }

    /// The same for an agent. Answers whether it was new.
    pub fn apply_agent(&mut self, agent: WorkAgent) -> bool {
        match self.agents.iter_mut().find(|a| a.id == agent.id) {
            Some(held) => {
                *held = agent;
                false
            }
            None => {
                self.agents.push(agent);
                true
            }
        }
    }

    // ── what it holds ───────────────────────────────────────────────

    pub fn agent(&self, id: AgentId) -> Option<&WorkAgent> {
        self.agents.iter().find(|a| a.id == id)
    }

    /// The same record, to be written into. The conversation family refreshes what it derives —
    /// the activity, the ring, the token count — straight onto the record the two screens over the
    /// work already read, rather than teaching each of them a second source.
    pub fn agent_mut(&mut self, id: AgentId) -> Option<&mut WorkAgent> {
        self.agents.iter_mut().find(|a| a.id == id)
    }

    pub fn task(&self, id: TaskId) -> Option<&TaskRecord> {
        self.tasks.iter().find(|t| t.id == id)
    }

    pub fn session(&self, id: SessionId) -> Option<&WorkSession> {
        self.sessions.iter().find(|s| s.id == id)
    }

    /// One step of one task, addressed the way every message addresses it: by two ids rather than
    /// by a place in a list.
    pub fn step(&self, task: TaskId, step: StepId) -> Option<&Step> {
        self.task(task)?.step(step)
    }

    /// The agents serving one task.
    pub fn members(&self, task: TaskId) -> impl Iterator<Item = &WorkAgent> {
        self.agents.iter().filter(move |a| a.task == Some(task))
    }

    /// Who the task speaks through: a coordinated task answers through its coordinator — the
    /// member the others were spawned by — and any other shape answers through whoever is holding
    /// it now, which is the first member that has not finished.
    pub fn now(&self, task: &TaskRecord) -> Option<&WorkAgent> {
        if task.shape == Shape::Coordinated {
            let lead = self.members(task.id).find(|a| {
                self.members(task.id)
                    .any(|other| other.parent == Some(a.id))
            });
            if lead.is_some() {
                return lead;
            }
        }
        self.members(task.id)
            .find(|a| a.activity != Activity::Ended)
            .or_else(|| self.members(task.id).next())
    }

    /// The state a task's card carries: the worst thing happening anywhere in it. A card is read at
    /// a glance and from across a column, so it reports what the user would want to be told first.
    pub fn pulse(&self, task: &TaskRecord) -> Bucket {
        let mut worst = Bucket::Ended;
        if task.blocked() {
            return Bucket::Error;
        }
        for bucket in self
            .members(task.id)
            .map(|a| a.activity.bucket())
            .chain(task.steps.iter().map(|s| s.state.bucket()))
        {
            match bucket {
                Bucket::Error => return Bucket::Error,
                Bucket::Waiting => worst = Bucket::Waiting,
                Bucket::Running if worst != Bucket::Waiting => worst = Bucket::Running,
                _ => {}
            }
        }
        worst
    }

    /// How many agents the status line counts, by bucket.
    pub fn count(&self, bucket: Bucket) -> usize {
        self.agents
            .iter()
            .filter(|a| a.activity.bucket() == bucket)
            .count()
    }
}

// ── what a record reads as ──────────────────────────────────────────
//
// Two values the two screens print, computed at draw time rather than carried on the wire, for the
// reason `super::when` renders how long ago something was instead of storing it: a value the
// interface can work out is a value that cannot go stale, and neither of these is a fact the host
// needs.

/// How far along a task is, as the meter draws it.
///
/// A task with no steps answers zero, which is not the same claim as "none of them done" — it has
/// nothing to be a fraction of. Callers that would be saying the second thing check `steps` first
/// and draw no meter at all, which is what the board and the panel both do.
pub fn fraction(task: &TaskRecord) -> f32 {
    if task.steps.is_empty() {
        return 0.0;
    }
    task.done() as f32 / task.steps.len() as f32
}

/// The token count as the card prints it.
pub fn tokens_label(agent: &WorkAgent) -> String {
    format!("{:.1}K", agent.tokens / 1000.0)
}
