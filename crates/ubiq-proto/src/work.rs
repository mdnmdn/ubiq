//! What the work is on the wire: a task as it is written down, and the sessions and agents doing it.
//!
//! The split between the three types here is the durability story, and the naming carries it.
//! [`TaskRecord`] is written down — every field of it survives a restart, and `tasks.toml` holds
//! exactly what crosses the bus. [`WorkSession`] and [`WorkAgent`] are not: they are per-request
//! payloads, like a [`crate::files::DirEntry`], with no store behind them and no relation to a
//! record.
//!
//! There is no snapshot type. [`crate::projects::ProjectSnapshot`] exists because a project's
//! health and its pane count can only be known at the moment they are asked for and must never be
//! believed at the next boot. Nothing on a task is like that: everything the two screens over the
//! work draw is either on the record or derivable from the sessions and agents the same reply
//! carries, so a derived field on the wire would be a second copy of what the interface already
//! holds.
//!
//! The words a state answers to live on the state. `label()`, `note()`, `all()` and `bucket()` are
//! here rather than in the interface because the host needs them too — it seeds the columns, it
//! writes a `Status` down, and it classifies its own agents. A `&'static str` is not drawing, and
//! `crates/ubiq/src/theme.rs` keeps its monopoly on colour untouched: which token a state reads in
//! is still the interface's alone.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::ids::{SessionId, StepId, TaskId, WorkspaceId};

/// One agent, which is one workspace: a single running harness with one terminal.
///
/// An alias rather than a kind of its own, because [`WorkspaceId`] was declared for exactly this
/// and the two are one thing until a workspace outlives its pane. Being the same type means no
/// shadowing is possible, and the name says which scale the reader is at.
pub type AgentId = WorkspaceId;

/// What an agent is doing right now. The badge on its card, and what the filter buckets sort on.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum Activity {
    Thinking,
    Writing,
    Tools,
    NeedsYou,
    Ended,
    Failed,
}

impl Activity {
    pub fn label(self) -> &'static str {
        match self {
            Activity::Thinking => "Thinking",
            Activity::Writing => "Writing",
            Activity::Tools => "Tools",
            Activity::NeedsYou => "Needs you",
            Activity::Ended => "Ended",
            Activity::Failed => "Error",
        }
    }

    /// Which filter pill covers this activity. Three ways of working are one bucket, because the
    /// question the filter answers is "is it moving", not "what is it doing".
    pub fn bucket(self) -> Bucket {
        match self {
            Activity::Thinking | Activity::Writing | Activity::Tools => Bucket::Running,
            Activity::NeedsYou => Bucket::Waiting,
            Activity::Ended => Bucket::Ended,
            Activity::Failed => Bucket::Error,
        }
    }
}

/// The four coarse states the toolbar filters on.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum Bucket {
    Running,
    Waiting,
    Ended,
    Error,
}

impl Bucket {
    pub fn label(self) -> &'static str {
        match self {
            Bucket::Running => "running",
            Bucket::Waiting => "waiting",
            Bucket::Ended => "ended",
            Bucket::Error => "error",
        }
    }

    pub fn all() -> [Bucket; 4] {
        [
            Bucket::Running,
            Bucket::Waiting,
            Bucket::Ended,
            Bucket::Error,
        ]
    }
}

/// How the agents on a task are arranged. The shape is a fact about the task, printed on its
/// container, because it is what tells the user whether the cards inside it run in order.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum Shape {
    /// One agent, asked directly.
    Direct,
    /// A hand-off: each agent starts where the last one stopped.
    Chain,
    /// A coordinator splitting the work across workers that run at once.
    Coordinated,
}

impl Shape {
    pub fn label(self) -> &'static str {
        match self {
            Shape::Direct => "DIRECT",
            Shape::Chain => "CHAIN",
            Shape::Coordinated => "COORDINATED",
        }
    }

    /// The shape in a sentence, for the panel that has room for one. The word alone says how the
    /// agents are arranged only to somebody who already knows.
    pub fn note(self) -> &'static str {
        match self {
            Shape::Direct => "one agent, asked directly",
            Shape::Chain => "each agent starts where the last one stopped",
            Shape::Coordinated => "a lead spawns and coordinates workers",
        }
    }

    pub fn all() -> [Shape; 3] {
        [Shape::Direct, Shape::Chain, Shape::Coordinated]
    }
}

/// Which column of the board a task sits in. The order is the order the board draws them in, and
/// the order work moves along: a task only ever changes column, never what it is.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum Status {
    Backlog,
    Ready,
    InProgress,
    InReview,
    Done,
}

impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Status::Backlog => "backlog",
            Status::Ready => "ready",
            Status::InProgress => "in progress",
            Status::InReview => "in review",
            Status::Done => "done",
        }
    }

    pub fn all() -> [Status; 5] {
        [
            Status::Backlog,
            Status::Ready,
            Status::InProgress,
            Status::InReview,
            Status::Done,
        ]
    }
}

/// How much a task matters. `Normal` is the absence of a claim rather than a middle value, which
/// is why it has no word: a board where every card shouts says nothing.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum Priority {
    Low,
    Normal,
    High,
}

impl Priority {
    pub fn label(self) -> Option<&'static str> {
        match self {
            Priority::Low => Some("low"),
            Priority::Normal => None,
            Priority::High => Some("high"),
        }
    }

    pub fn all() -> [Priority; 3] {
        [Priority::Low, Priority::Normal, Priority::High]
    }
}

/// Where one step has got to. The checkbox reads `Done`; every other variant is a reason it is not
/// ticked, and each is a state its owner can actually be in.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum StepState {
    Idle,
    Working,
    NeedsYou,
    Failed,
    Done,
}

impl StepState {
    pub fn label(self) -> &'static str {
        match self {
            StepState::Idle => "idle",
            StepState::Working => "working",
            StepState::NeedsYou => "needs you",
            StepState::Failed => "error",
            StepState::Done => "done",
        }
    }

    /// Which bucket a step reads in, so a step takes the same four colours as everything else on
    /// the screen rather than a palette of its own.
    pub fn bucket(self) -> Bucket {
        match self {
            StepState::Idle | StepState::Done => Bucket::Ended,
            StepState::Working => Bucket::Running,
            StepState::NeedsYou => Bucket::Waiting,
            StepState::Failed => Bucket::Error,
        }
    }
}

/// One step of a task, and which agent has it.
///
/// A step carries an id rather than being addressed by its place in the list. Two clicks in one
/// frame — a remove and a tick — would otherwise arrive as two indices into two different lists,
/// and the second would land on the wrong step.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Step {
    pub id: StepId,
    pub title: String,
    pub state: StepState,
    /// The agent holding it. Durable, so an owner naming no live agent draws as unowned rather
    /// than being written out of the record — the same posture as a project whose folder moved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<AgentId>,
}

impl Step {
    /// A sub-task nobody has picked up: named, unowned and not started, which is everything known
    /// at the moment it is named.
    pub fn new(title: String) -> Self {
        Self {
            id: StepId::generate(),
            title,
            state: StepState::Idle,
            owner: None,
        }
    }

    pub fn done(&self) -> bool {
        self.state == StepState::Done
    }

    /// Tick and untick. Unticking lands on `Idle` rather than on whatever the step was before:
    /// nothing here can know what its owner would go back to doing.
    pub fn toggle(&mut self) {
        self.state = if self.done() {
            StepState::Idle
        } else {
            StepState::Done
        };
    }
}

/// A task as it is written down. Everything here survives a restart.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskRecord {
    pub id: TaskId,
    /// The session doing the work. `None` is a task nobody has started — the board says so, and
    /// the graph has nothing to draw for it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<SessionId>,
    pub status: Status,
    pub priority: Priority,
    pub shape: Shape,
    pub title: String,
    /// Markdown, which the host stores and never parses — the same discipline that keeps terminal
    /// bytes uninterpreted. Which of it is a heading is the interface's decision.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(default, rename = "step", skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<Step>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl TaskRecord {
    /// A task nobody has started, as the board asks for one: in the backlog, unprioritised, direct
    /// and with no steps, because that is everything known at the moment it is named.
    pub fn new(title: String, session: Option<SessionId>, now: DateTime<Utc>) -> Self {
        Self {
            id: TaskId::generate(),
            session,
            status: Status::Backlog,
            priority: Priority::Normal,
            shape: Shape::Direct,
            title,
            description: String::new(),
            steps: Vec::new(),
            created_at: now,
            updated_at: now,
        }
    }

    pub fn done(&self) -> usize {
        self.steps.iter().filter(|s| s.done()).count()
    }

    /// A task nobody can finish without the user: a step failed under whoever had it.
    pub fn blocked(&self) -> bool {
        self.steps.iter().any(|s| s.state == StepState::Failed)
    }

    pub fn step(&self, id: StepId) -> Option<&Step> {
        self.steps.iter().find(|s| s.id == id)
    }

    pub fn step_mut(&mut self, id: StepId) -> Option<&mut Step> {
        self.steps.iter_mut().find(|s| s.id == id)
    }
}

/// A session as the two screens over the work draw it: a named piece of work and the branch it is
/// on.
///
/// The session family's [`crate::messages::Message`] set names the same concept from the process
/// side, where a session is a home folder and a set of panes. The two merge when that family is
/// built; until then this is the half the graph and the board need, and it is not written down.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkSession {
    pub id: SessionId,
    pub name: String,
    pub branch: String,
    /// Whether the session works in a worktree of its own rather than in the project's folder.
    pub worktree: bool,
}

/// Who said a line in an agent's thread.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum Speaker {
    You,
    Agent,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Turn {
    pub from: Speaker,
    pub text: String,
}

/// One agent, as the graph draws it.
///
/// Not [`crate::messages::WorkspaceInfo`], which says what the host *started* — a command, a
/// geometry, whether the process is alive. This says what the agent is *doing*, and only one of
/// the two is a mock.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorkAgent {
    pub id: AgentId,
    pub session: SessionId,
    /// The task whose container the card sits in. `None` is an agent nobody has given work to.
    pub task: Option<TaskId>,
    /// Who spawned it. The connector is drawn from the parent's card to this one.
    pub parent: Option<AgentId>,
    pub name: String,
    pub role: String,
    pub activity: Activity,
    /// The one line the card says about what it is doing.
    pub note: String,
    pub branch: String,
    pub tokens: f32,
    pub harness: String,
    /// The identity this agent runs as, empty when it resolved none and fell back to the
    /// user's own home. An account id — never a credential, per the account family's rule.
    ///
    /// Reported rather than requested: it is what the run actually resolved to, so a
    /// conversation cannot claim an account it is not using. Fixed for the agent's life,
    /// because a turn already taken was taken as somebody.
    pub account: String,
    pub model: String,
    pub context_pct: u8,
    /// What has been said to and by this agent. Nothing answers it, which is what the thread says
    /// in as many words: a fabricated reply is the one thing a screen with no live agent must not
    /// draw.
    pub thread: Vec<Turn>,
}
