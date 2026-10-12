//! A mission: the phase it is in, how it is run, and what the runner needs to say about it.
//!
//! **A mission *is* its parent task.** There is no `MissionId`, no sidecar record and no journal
//! file: the task carrying [`crate::work::Level::Mission`] holds the phase (as its
//! [`crate::work::Status`], through [`Phase::of`]) and the settings
//! ([`crate::work::TaskRecord::mission`]), and every event the runner has to remember is a
//! [`RunEvent`] comment on the task it concerns. What a window draws is a [`MissionSnapshot`],
//! **derived on every pass and never stored**.
//!
//! The pure functions at the bottom ([`is_human`], [`satisfies`], [`worker_gone`]) are here because
//! both halves must agree on the answer — the host's runner and the window's board chips read the
//! same rule — not because they cross the bus.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::ids::{ProjectId, SpawnId, TaskId};
use crate::plan::PlanRevision;
use crate::work::{AgentId, Status, TaskRecord};

/// Where a mission has got to — the parent task's [`Status`], read as a lifecycle.
///
/// Not stored: [`Phase::of`] reads it off the parent's status and [`Phase::status`] is the inverse,
/// so every board filter stays right with nothing to keep in step.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Phase {
    /// The planner and the user are settling what the mission is and writing the plan.
    #[default]
    Planning,
    /// The plan is approved and is being cut into tasks.
    Breakdown,
    /// Tasks are worked.
    Running,
    /// Running, held by the user.
    Paused,
    /// Every task is finished and the mission awaits its final review.
    Review,
    /// Everything is done.
    Done,
    /// Terminal, from any phase.
    Abandoned,
}

impl Phase {
    /// The phase a parent task's status reads as.
    pub fn of(status: Status) -> Phase {
        match status {
            Status::Backlog => Phase::Planning,
            Status::Ready => Phase::Breakdown,
            Status::InProgress => Phase::Running,
            Status::Blocked => Phase::Paused,
            Status::InReview => Phase::Review,
            Status::Done => Phase::Done,
            Status::Abandoned => Phase::Abandoned,
        }
    }

    /// The parent task's status for this phase — [`Phase::of`]'s inverse.
    pub fn status(self) -> Status {
        match self {
            Phase::Planning => Status::Backlog,
            Phase::Breakdown => Status::Ready,
            Phase::Running => Status::InProgress,
            Phase::Paused => Status::Blocked,
            Phase::Review => Status::InReview,
            Phase::Done => Status::Done,
            Phase::Abandoned => Status::Abandoned,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Phase::Planning => "planning",
            Phase::Breakdown => "breakdown",
            Phase::Running => "running",
            Phase::Paused => "paused",
            Phase::Review => "review",
            Phase::Done => "done",
            Phase::Abandoned => "abandoned",
        }
    }

    pub fn all() -> [Phase; 7] {
        [
            Phase::Planning,
            Phase::Breakdown,
            Phase::Running,
            Phase::Paused,
            Phase::Review,
            Phase::Done,
            Phase::Abandoned,
        ]
    }

    /// Where this phase sits on the five-step stepper (planning, breakdown, running, review,
    /// done). Paused sits on the running step; Abandoned is off the stepper.
    pub fn step(self) -> Option<usize> {
        match self {
            Phase::Planning => Some(0),
            Phase::Breakdown => Some(1),
            Phase::Running | Phase::Paused => Some(2),
            Phase::Review => Some(3),
            Phase::Done => Some(4),
            Phase::Abandoned => None,
        }
    }
}

/// Who starts the work while a mission is Running.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Driver {
    /// The runner launches ready tasks, up to [`MissionSettings::parallel`].
    #[default]
    Auto,
    /// The runner launches nothing; the ready tasks are listed and the user starts them.
    Manual,
}

fn default_parallel() -> usize {
    1
}

/// A mission's settings, held on the parent task ([`crate::work::TaskRecord::mission`]).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MissionSettings {
    #[serde(default)]
    pub driver: Driver,
    /// The agent definition (by id) that plans. `None` is the project's default planner.
    #[serde(default)]
    pub planner: Option<String>,
    /// The agent definition (by id) that works tasks. `None` is the project's default worker.
    #[serde(default)]
    pub worker: Option<String>,
    /// How many workers may run at once. Never below one.
    #[serde(default = "default_parallel")]
    pub parallel: usize,
    /// Whether a task waits for the user's confirmation in review by default.
    #[serde(default)]
    pub approve: bool,
    /// The planner's conversation — the handle a resume goes through.
    #[serde(default)]
    pub planner_agent: Option<AgentId>,
}

impl Default for MissionSettings {
    fn default() -> Self {
        Self {
            driver: Driver::default(),
            planner: None,
            worker: None,
            parallel: default_parallel(),
            approve: false,
            planner_agent: None,
        }
    }
}

impl MissionSettings {
    /// What a mission parent with no settings reads as: [`Driver::Manual`], so a mission written
    /// before the runner existed never launches anything on its own.
    pub fn legacy() -> Self {
        Self {
            driver: Driver::Manual,
            ..Self::default()
        }
    }

    /// [`Self::parallel`], clamped to at least one.
    pub fn parallelism(&self) -> usize {
        self.parallel.max(1)
    }
}

/// One of a mission's settings, with the value to set it to — [`crate::messages::TaskField`]'s
/// shape: one variant on the wire rather than a message per setting, and `None` is the cleared
/// state.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MissionField {
    Driver(Driver),
    Planner(Option<String>),
    Worker(Option<String>),
    Parallel(usize),
    Approve(bool),
}

/// What the runner launches an agent as.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LaunchRole {
    /// The mission's planner, persistent, resumable.
    Planner,
    /// An agent working one task.
    Worker,
    /// An agent reviewing a task that has `review_of`.
    Reviewer,
    /// The final review, on the mission parent.
    MissionReviewer,
}

/// One thing the runner recorded, as a comment on the task it concerns
/// ([`crate::work::Comment::event`]). The comment's text carries the prose — feedback, findings or
/// a reason — where the variant says so.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum RunEvent {
    /// The mission moved to a phase.
    Phase { to: Phase },
    /// The planner asked for a phase move; the user decides. Never moves the phase.
    PhaseRequested { to: Phase, summary: String },
    /// An agent was launched on the task.
    Spawned {
        agent: AgentId,
        role: LaunchRole,
        retry: bool,
    },
    /// An agent finished its turn on the task.
    Finished { agent: Option<AgentId> },
    /// The agent working the task is no longer live.
    Gone { agent: AgentId },
    /// The user confirmed the task.
    Confirmed,
    /// The user rejected the task; the text is the feedback.
    Rejected,
    /// A person finished a task assigned to them.
    HumanDone,
    /// A person failed a task assigned to them; the text is the feedback.
    HumanFailed,
    /// A reviewer's verdict; the text is the findings.
    ReviewVerdict { pass: bool },
    /// The user added guidance to the mission.
    Guidance { sent_to_running: bool },
    /// A task changed column.
    Moved { from: Status, to: Status },
    /// An agent was stopped.
    Stopped { agent: AgentId },
    /// A launch failed; the text is the reason.
    LaunchFailed,
}

/// A launch the host asks a window to perform, because only a window can start a conversation.
///
/// Everything in it is decided host-side except which definition runs when `definition` is `None`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LaunchRequest {
    /// Minted host-side; the window echoes it in [`crate::messages::Message::MissionLaunched`].
    pub id: SpawnId,
    pub role: LaunchRole,
    /// What to assign the new agent to — the mission parent for the planner and the mission
    /// reviewer, the task itself otherwise.
    pub task: TaskId,
    /// The definition to start from. `None` leaves the window to pick the project's default for
    /// the role.
    #[serde(default)]
    pub definition: Option<String>,
    /// The MCP servers to attach, chosen by the host.
    #[serde(default)]
    pub mcps: Vec<String>,
    /// The opening prompt, composed by the host.
    #[serde(default)]
    pub prompt: String,
    /// The conversation to revive instead of starting a fresh one. Planner only.
    #[serde(default)]
    pub resume: Option<AgentId>,
    /// Whether the conversation stays loaded between turns. True for the planner.
    #[serde(default)]
    pub persistent: bool,
}

/// What became of a [`LaunchRequest`], as the window reports it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LaunchOutcome {
    /// The window launched it. The id is the window's own mint, as every [`AgentId`] is.
    Launched(AgentId),
    /// It could not be launched; the sentence is the reason.
    Failed(String),
}

/// Something that needs the user, derived from the tree on every pass.
///
/// *Planner is asking* and *worker permission request* are not here: the window reads them off the
/// agent's own `Activity::NeedsYou` for the agents in [`MissionSnapshot::agents`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum NeedsYou {
    /// The planner asked for the breakdown; `open_threads` annotation threads are still open.
    PlanReady { open_threads: usize },
    /// The planner drafted the tasks and asked to run.
    TasksDrafted,
    /// The planner is not live while the mission is Planning or Breakdown.
    PlannerStopped,
    /// A task is in review and waits for the user's confirmation.
    AwaitingApproval { task: TaskId },
    /// A task is assigned to a person.
    HumanTask { task: TaskId },
    /// The agent working the task is gone.
    WorkerGone { task: TaskId },
    /// Two agent reviews in a row failed against the task.
    ReviewFailedTwice { task: TaskId },
    /// Every task is finished; these are still in review.
    ReadyForReview { left_in_review: Vec<TaskId> },
}

/// A live agent working a task.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunningWorker {
    pub task: TaskId,
    pub agent: AgentId,
    pub role: LaunchRole,
    /// When the `Spawned` comment was written.
    pub since: DateTime<Utc>,
}

/// A mission as a window draws it. **Derived on every runner pass and never stored.**
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MissionSnapshot {
    pub project_id: ProjectId,
    /// The parent task — the mission's identity. No default: an id that was not minted is a nil
    /// id that looks real.
    pub task_id: TaskId,
    #[serde(default)]
    pub phase: Phase,
    #[serde(default)]
    pub settings: MissionSettings,
    /// The names in the docs directory, without the `.md`, `guidance` included.
    #[serde(default)]
    pub documents: Vec<String>,
    /// Open annotation threads on the plan.
    #[serde(default)]
    pub open_threads: usize,
    #[serde(default)]
    pub needs_you: Vec<NeedsYou>,
    #[serde(default)]
    pub running: Vec<RunningWorker>,
    /// Ready tasks, filled only for [`Driver::Manual`].
    #[serde(default)]
    pub next: Vec<TaskId>,
    /// The live planner and the live workers and reviewers.
    #[serde(default)]
    pub agents: Vec<AgentId>,
    /// When the latest breakdown that followed a Running phase began; the added, changed and
    /// abandoned marks are read against it.
    #[serde(default)]
    pub replan_at: Option<DateTime<Utc>>,
}

/// Where a new mission's first plan comes from.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum InitialPlan {
    /// The planner writes it from scratch.
    #[default]
    None,
    /// A file: a project-relative path or `kb:{source}:{path}`.
    File { target: String },
    /// Pasted text, saved as revision 1.
    Paste { body: String },
}

/// Everything the new-mission dialog decides, in one message: the host does the rest.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewMission {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub description: String,
    /// Attachment targets, as [`crate::work::Attachment::target`].
    #[serde(default)]
    pub attachments: Vec<String>,
    /// Existing tasks to adopt as children, in Backlog.
    #[serde(default)]
    pub include: Vec<TaskId>,
    #[serde(default)]
    pub initial_plan: InitialPlan,
    #[serde(default)]
    pub settings: MissionSettings,
}

/// What the user does to the mission as a whole.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MissionAction {
    ApprovePlan,
    StartMission,
    RedoBreakdown { note: String },
    Pause,
    Resume,
    Replan { note: String },
    RunAgentReview,
    RejectMission { feedback: String },
    Complete,
    Abandon,
    ResumePlanner,
}

/// What the user does to one task of a mission.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MissionTaskAction {
    /// Launch now, whatever the driver.
    Start,
    Confirm,
    Reject { feedback: String },
    /// A person finished their task.
    Done,
    /// A person failed their review task; the feedback goes to the task under review.
    Fail { feedback: String },
    Retry,
    /// Add a review task for this one; `human` assigns it to a person.
    AddReview { human: bool },
}

/// The filter key of a [`HistoryEntry`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HistoryKind {
    Phase,
    Plan,
    Agent,
    Review,
    Feedback,
    Guidance,
    Status,
    Comment,
}

/// One line of a mission's history, assembled by the host from comments and the plan's revision
/// log.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub at: DateTime<Utc>,
    pub kind: HistoryKind,
    #[serde(default)]
    pub task: Option<TaskId>,
    #[serde(default)]
    pub agent: Option<AgentId>,
    #[serde(default)]
    pub plan_revision: Option<PlanRevision>,
    /// The one sentence a surface draws.
    #[serde(default)]
    pub text: String,
}

/// Whether a task is assigned to a person: `assigned_to` is non-empty once trimmed, does not parse
/// as an [`AgentId`] and is not one of the agent `definitions` (ids).
pub fn is_human(task: &TaskRecord, definitions: &[String]) -> bool {
    let Some(who) = task.assigned_to.as_deref().map(str::trim) else {
        return false;
    };
    !who.is_empty() && who.parse::<AgentId>().is_err() && !definitions.iter().any(|d| d == who)
}

/// Whether `prereq` lets `dependent` start: it is `Done` or `Abandoned`, or `InReview` and either
/// needs no approval or is the very task `dependent` reviews.
pub fn satisfies(prereq: &TaskRecord, dependent: &TaskRecord) -> bool {
    match prereq.status {
        Status::Done | Status::Abandoned => true,
        Status::InReview => !prereq.approve || dependent.review_of == Some(prereq.id),
        _ => false,
    }
}

/// Whether a task reads as having lost its agent: it is `InProgress`, has a `worker`, and that
/// worker is not `live`.
pub fn worker_gone(task: &TaskRecord, live: impl Fn(AgentId) -> bool) -> bool {
    task.status == Status::InProgress && task.worker.is_some_and(|agent| !live(agent))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(status: Status) -> TaskRecord {
        let mut t = TaskRecord::new("t".into(), None, Utc::now());
        t.status = status;
        t
    }

    #[test]
    fn phase_and_status_are_inverse() {
        for status in [
            Status::Backlog,
            Status::Ready,
            Status::InProgress,
            Status::Blocked,
            Status::InReview,
            Status::Done,
            Status::Abandoned,
        ] {
            assert_eq!(Phase::of(status).status(), status);
        }
        for phase in Phase::all() {
            assert_eq!(Phase::of(phase.status()), phase);
        }
        assert_eq!(Phase::of(Status::Backlog), Phase::Planning);
        assert_eq!(Phase::of(Status::Blocked), Phase::Paused);
        assert_eq!(Phase::Paused.step(), Some(2));
        assert_eq!(Phase::Abandoned.step(), None);
    }

    #[test]
    fn satisfies_follows_the_rule() {
        let dependent = task(Status::Ready);
        assert!(satisfies(&task(Status::Done), &dependent));
        assert!(satisfies(&task(Status::Abandoned), &dependent));
        assert!(!satisfies(&task(Status::InProgress), &dependent));
        assert!(!satisfies(&task(Status::Ready), &dependent));

        let mut approved = task(Status::InReview);
        assert!(satisfies(&approved, &dependent));
        approved.approve = true;
        assert!(!satisfies(&approved, &dependent));
        let mut review = task(Status::Ready);
        review.review_of = Some(approved.id);
        assert!(satisfies(&approved, &review));
    }

    #[test]
    fn is_human_reads_assigned_to() {
        let defs = vec!["worker".to_string()];
        let mut t = task(Status::Ready);
        assert!(!is_human(&t, &defs));
        t.assigned_to = Some("  ".into());
        assert!(!is_human(&t, &defs));
        t.assigned_to = Some("worker".into());
        assert!(!is_human(&t, &defs));
        t.assigned_to = Some(AgentId::generate().to_string());
        assert!(!is_human(&t, &defs));
        t.assigned_to = Some("human".into());
        assert!(is_human(&t, &defs));
    }

    #[test]
    fn worker_gone_needs_a_dead_worker_in_progress() {
        let agent = AgentId::generate();
        let mut t = task(Status::InProgress);
        assert!(!worker_gone(&t, |_| false));
        t.worker = Some(agent);
        assert!(worker_gone(&t, |_| false));
        assert!(!worker_gone(&t, |a| a == agent));
        t.status = Status::InReview;
        assert!(!worker_gone(&t, |_| false));
    }

    #[test]
    fn settings_defaults_and_legacy() {
        let s: MissionSettings = serde_json::from_str("{}").unwrap();
        assert_eq!(s, MissionSettings::default());
        assert_eq!(s.parallel, 1);
        assert_eq!(s.driver, Driver::Auto);
        assert_eq!(MissionSettings::legacy().driver, Driver::Manual);
    }

    #[test]
    fn run_event_round_trips() {
        let event = RunEvent::Spawned {
            agent: AgentId::generate(),
            role: LaunchRole::Worker,
            retry: true,
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"event\":\"spawned\""));
        assert_eq!(serde_json::from_str::<RunEvent>(&json).unwrap(), event);
        let unit = serde_json::to_string(&RunEvent::Confirmed).unwrap();
        assert_eq!(
            serde_json::from_str::<RunEvent>(&unit).unwrap(),
            RunEvent::Confirmed
        );
    }
}
