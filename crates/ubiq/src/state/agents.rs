//! The agents screen's state: the orchestration graph, what is selected in it, and the tasks
//! hanging off that selection.
//!
//! Nothing here draws and nothing here names a colour — an activity says what it *is*, and
//! `ui::agents` decides which token that reads in. Nothing here says where anything sits either:
//! a definition and its position are separate, and [`super::layout`] owns the second half.
//!
//! This is a fixture screen. Sessions, agents and tasks are invented in [`super::sample`] because
//! the orchestration graph has no transport family yet; it goes the same way the chat does when it
//! gets one.

use std::time::{Duration, Instant};

pub use super::layout::{CARD_HEIGHT, CARD_WIDTH, GROUP_LABEL, GROUP_PAD, Layout};

/// A card in the graph. One card is one workspace — a single running agent — which is why it
/// carries a harness, a model and a context percentage rather than a process of any kind.
pub type AgentId = u32;

/// A named piece of work grouping the agents serving it.
pub type SessionId = u32;

pub type TaskId = u32;

/// What an agent is doing right now. The badge on its card, and what the filter buckets sort on.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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
}

/// Which column of the board a task sits in. The order is the order the board draws them in, and
/// the order work moves along: a task only ever changes column, never what it is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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
}

/// Where one step has got to. The checkbox reads `Done`; every other variant is a reason it is not
/// ticked, and each is a state its owner can actually be in.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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
#[derive(Clone, Debug)]
pub struct Step {
    pub title: String,
    pub state: StepState,
    pub owner: Option<AgentId>,
}

impl Step {
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

#[derive(Clone, Debug)]
pub struct Task {
    pub id: TaskId,
    /// The session doing the work. `None` is a task nobody has started — the board says so, and
    /// the graph has nothing to draw for it.
    pub session: Option<SessionId>,
    pub status: Status,
    pub priority: Priority,
    pub shape: Shape,
    pub title: String,
    pub steps: Vec<Step>,
}

impl Task {
    pub fn done(&self) -> usize {
        self.steps.iter().filter(|s| s.done()).count()
    }

    /// A task nobody can finish without the user: a step failed under whoever had it.
    pub fn blocked(&self) -> bool {
        self.steps.iter().any(|s| s.state == StepState::Failed)
    }

    /// How far along, as the meter draws it.
    ///
    /// A task with no steps answers zero, which is not the same claim as "none of them done" — it
    /// has nothing to be a fraction of. Callers that would be saying the second thing check
    /// `steps` first and draw no meter at all, which is what the board and the panel both do.
    pub fn fraction(&self) -> f32 {
        if self.steps.is_empty() {
            return 0.0;
        }
        self.done() as f32 / self.steps.len() as f32
    }
}

#[derive(Clone, Debug)]
pub struct Session {
    pub id: SessionId,
    pub name: String,
    pub branch: String,
    /// Whether the session works in a worktree of its own rather than in the project's folder.
    pub worktree: bool,
}

/// Who said a line in an agent's thread.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Speaker {
    You,
    Agent,
}

#[derive(Clone, Debug)]
pub struct Turn {
    pub from: Speaker,
    pub text: String,
}

#[derive(Clone, Debug)]
pub struct Agent {
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
    pub model: String,
    pub context_pct: u8,
    /// What has been said to and by this agent. Seeded from the fixture; what the composer sends
    /// is appended to it, and nothing answers, because there is nothing behind it to answer.
    pub thread: Vec<Turn>,
}

impl Agent {
    /// The token count as the card prints it.
    pub fn tokens_label(&self) -> String {
        format!("{:.1}K", self.tokens / 1000.0)
    }
}

/// What the inspector and the tasks strip are about. A session and an agent are both selectable,
/// and the two answer the same questions at different scales.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Selection {
    Session(SessionId),
    Agent(AgentId),
}

/// Which half of the inspector is showing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum InspectorTab {
    Chat,
    Tasks,
}

/// One grain of the trail a dragged card leaves behind.
///
/// The trail is not decoration: a card that moves with no evidence of having moved reads as a
/// redraw, and a card that leaves a fading track reads as something the user picked up.
#[derive(Clone, Copy, Debug)]
pub struct Grain {
    /// Window coordinates, because the trail is painted over the canvas rather than in it.
    pub at: (f32, f32),
    pub born: Instant,
    /// A fixed jitter per grain, so the trail scatters rather than drawing a rope.
    pub spread: (f32, f32),
    pub size: f32,
}

/// How long a grain takes to disappear.
pub const GRAIN_LIFE: Duration = Duration::from_millis(650);

/// The most grains kept at once. A cap rather than a decay-only rule, so a long drag on a slow
/// frame cannot grow the vector without bound.
pub const GRAIN_CEILING: usize = 240;

impl Grain {
    /// Zero when the grain has just landed, one when it is gone.
    pub fn age(&self, now: Instant) -> f32 {
        let life = GRAIN_LIFE.as_secs_f32();
        (now.saturating_duration_since(self.born).as_secs_f32() / life).clamp(0.0, 1.0)
    }

    pub fn spent(&self, now: Instant) -> bool {
        self.age(now) >= 1.0
    }
}

/// What the pointer has hold of. A card moves alone; a container moves everything in it, because
/// the cards are positioned against its origin and the origin is the only thing that changes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Held {
    Agent(AgentId),
    Task(TaskId),
}

/// Something being carried. The grab point is where inside it the pointer went down, so it does
/// not jump under the cursor on the first move.
#[derive(Clone, Copy, Debug)]
pub struct Carry {
    pub held: Held,
    pub grab: (f32, f32),
    /// The task container the pointer is over, which is what a drop would move a card into. Always
    /// `None` while a container is the thing being carried: a task is not filed inside a task.
    pub over: Option<TaskId>,
}

pub struct AgentsState {
    pub sessions: Vec<Session>,
    pub agents: Vec<Agent>,
    pub tasks: Vec<Task>,

    /// Where all of the above is drawn. Separate from the definitions above it, and thrown away
    /// and recomputed whole by `relayout`.
    pub layout: Layout,

    /// Which buckets the graph is showing. A card in a hidden bucket is not drawn, and neither are
    /// the connectors into it.
    pub buckets: Vec<Bucket>,
    pub zoom: f32,
    pub selection: Option<Selection>,
    pub tab: InspectorTab,
    pub show_inspector: bool,
    pub tasks_open: bool,

    /// What is typed in the inspector's composer.
    pub draft: String,

    pub carry: Option<Carry>,
    pub sand: Vec<Grain>,
}

/// The zoom range and the step the toolbar's `−` and `+` move in.
pub const ZOOM_MIN: f32 = 0.5;
pub const ZOOM_MAX: f32 = 1.6;
pub const ZOOM_STEP: f32 = 0.1;

impl AgentsState {
    pub fn new(sessions: Vec<Session>, agents: Vec<Agent>, tasks: Vec<Task>) -> Self {
        let selection = agents.first().map(|a| Selection::Agent(a.id));
        let layout = Layout::auto(&agents, &tasks);
        Self {
            sessions,
            agents,
            tasks,
            layout,
            buckets: Bucket::all().to_vec(),
            zoom: 0.8,
            selection,
            tab: InspectorTab::Chat,
            show_inspector: true,
            tasks_open: false,
            draft: String::new(),
            carry: None,
            sand: Vec::new(),
        }
    }

    pub fn agent(&self, id: AgentId) -> Option<&Agent> {
        self.agents.iter().find(|a| a.id == id)
    }

    pub fn agent_mut(&mut self, id: AgentId) -> Option<&mut Agent> {
        self.agents.iter_mut().find(|a| a.id == id)
    }

    pub fn task(&self, id: TaskId) -> Option<&Task> {
        self.tasks.iter().find(|t| t.id == id)
    }

    pub fn session(&self, id: SessionId) -> Option<&Session> {
        self.sessions.iter().find(|s| s.id == id)
    }

    /// Where a card is drawn, on the canvas at 100% zoom.
    pub fn at(&self, agent: &Agent) -> (f32, f32) {
        self.layout.at(agent)
    }

    pub fn at_id(&self, id: AgentId) -> Option<(f32, f32)> {
        self.agent(id).map(|agent| self.layout.at(agent))
    }

    /// Put a card at a point on the canvas, whatever frame it hangs off.
    pub fn place(&mut self, id: AgentId, at: (f32, f32)) {
        let origin = self
            .agent(id)
            .and_then(|agent| agent.task)
            .map(|task| self.layout.task_origin(task))
            .unwrap_or((0.0, 0.0));
        self.layout
            .place_agent(id, (at.0 - origin.0, at.1 - origin.1));
    }

    /// Throw the arrangement away and compute it again from the definitions.
    pub fn relayout(&mut self) {
        self.layout = Layout::auto(&self.agents, &self.tasks);
    }

    /// The selected agent, when an agent is what is selected.
    pub fn selected_agent(&self) -> Option<&Agent> {
        match self.selection {
            Some(Selection::Agent(id)) => self.agent(id),
            _ => None,
        }
    }

    /// Which session the screen is about: the selected one, the selected agent's, or the first.
    pub fn active_session(&self) -> Option<SessionId> {
        match self.selection {
            Some(Selection::Session(id)) => Some(id),
            Some(Selection::Agent(id)) => self.agent(id).map(|a| a.session),
            None => self.sessions.first().map(|s| s.id),
        }
    }

    pub fn showing(&self, bucket: Bucket) -> bool {
        self.buckets.contains(&bucket)
    }

    /// Whether a card is drawn at all, given the filters and which session the screen is on.
    pub fn visible(&self, agent: &Agent) -> bool {
        self.showing(agent.activity.bucket()) && Some(agent.session) == self.active_session()
    }

    /// The tasks the strip lists: every task in the session, or the ones the selected agent has a
    /// step in.
    pub fn listed_tasks(&self) -> Vec<&Task> {
        match self.selection {
            Some(Selection::Agent(id)) => self
                .tasks
                .iter()
                .filter(|t| {
                    t.steps.iter().any(|s| s.owner == Some(id))
                        || self.agent(id).and_then(|a| a.task) == Some(t.id)
                })
                .collect(),
            _ => {
                let session = self.active_session();
                // A task nobody has started belongs to no session, and the graph is a screen about
                // sessions: it is the board that has somewhere to draw it.
                self.tasks
                    .iter()
                    .filter(|t| t.session.is_some() && t.session == session)
                    .collect()
            }
        }
    }

    /// The agents serving one task.
    pub fn members(&self, task: TaskId) -> impl Iterator<Item = &Agent> {
        self.agents.iter().filter(move |a| a.task == Some(task))
    }

    /// Who the task speaks through: a coordinated task answers through its coordinator — the
    /// member the others were spawned by — and any other shape answers through whoever is holding
    /// it now, which is the first member that has not finished.
    pub fn now(&self, task: &Task) -> Option<&Agent> {
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
    pub fn pulse(&self, task: &Task) -> Bucket {
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

    /// Put a new task in the backlog, and answer its id.
    ///
    /// It starts direct, unprioritised and with no steps, because that is all the board knows when
    /// it is asked for one. Everything else about a task is learnt later.
    pub fn add_task(&mut self, title: String, session: Option<SessionId>) -> TaskId {
        let id = self.tasks.iter().map(|t| t.id).max().unwrap_or(0) + 1;
        self.tasks.push(Task {
            id,
            session,
            status: Status::Backlog,
            priority: Priority::Normal,
            shape: Shape::Direct,
            title,
            steps: Vec::new(),
        });
        id
    }

    /// Move a task to another column. Answers whether anything changed, so a drop that landed
    /// where the card already was costs no redraw.
    pub fn move_task(&mut self, id: TaskId, status: Status) -> bool {
        let Some(task) = self.tasks.iter_mut().find(|t| t.id == id) else {
            return false;
        };
        if task.status == status {
            return false;
        }
        task.status = status;
        true
    }

    /// Tick or untick one step of a task.
    pub fn toggle_step(&mut self, task: TaskId, step: usize) -> bool {
        let Some(task) = self.tasks.iter_mut().find(|t| t.id == task) else {
            return false;
        };
        let Some(step) = task.steps.get_mut(step) else {
            return false;
        };
        step.toggle();
        true
    }

    /// How many agents the status line counts, by bucket.
    pub fn count(&self, bucket: Bucket) -> usize {
        self.agents
            .iter()
            .filter(|a| a.activity.bucket() == bucket)
            .count()
    }

    /// Put what was typed into the selected agent's thread.
    ///
    /// Nothing replies. The composer is real — what is typed lands where it was sent — and the
    /// answer is the one thing a screen with no transport family cannot honestly invent.
    pub fn send(&mut self) -> bool {
        let text = self.draft.trim().to_string();
        if text.is_empty() {
            return false;
        }
        let Some(Selection::Agent(id)) = self.selection else {
            return false;
        };
        let Some(agent) = self.agent_mut(id) else {
            return false;
        };
        agent.thread.push(Turn {
            from: Speaker::You,
            text,
        });
        self.draft.clear();
        true
    }

    // ── Mutators ────────────────────────────────────────────────────
    //
    // None of them notifies: they are called from `AppState`, which is what owns the redraw.

    pub fn toggle_bucket(&mut self, bucket: Bucket) {
        if let Some(ix) = self.buckets.iter().position(|b| *b == bucket) {
            // The last pill cannot be turned off — an empty graph is a filter bug that looks like
            // an empty session.
            if self.buckets.len() > 1 {
                self.buckets.remove(ix);
            }
        } else {
            self.buckets.push(bucket);
        }
    }

    pub fn zoom_by(&mut self, delta: f32) {
        self.zoom = (self.zoom + delta).clamp(ZOOM_MIN, ZOOM_MAX);
    }

    pub fn zoom_pct(&self) -> u32 {
        (self.zoom * 100.0).round() as u32
    }

    /// Pick something up. The grab point is where inside it the pointer went down.
    pub fn start_carry(&mut self, held: Held, grab: (f32, f32)) {
        self.carry = Some(Carry {
            held,
            grab,
            over: None,
        });
    }

    /// Move whatever is being carried, and lay a grain down where it passed.
    ///
    /// `at` is in graph coordinates — the top-left of the card, or of the container's box.
    /// `pointer` is where the pointer is in the window, which is the frame the sand is painted in;
    /// `None` lays no trail, which is what reduced motion asks for.
    pub fn carry_to(&mut self, at: (f32, f32), pointer: Option<(f32, f32)>, now: Instant) {
        let Some(carry) = self.carry else { return };
        match carry.held {
            Held::Agent(id) => {
                self.place(id, at);
                // Which container the pointer is over decides what a drop means, and is what the
                // canvas lights up while the card is in the air.
                let over = self.task_at(id, at);
                if let Some(carry) = self.carry.as_mut() {
                    carry.over = over;
                }
            }
            // A container has no position of its own — its box is the box round its cards — so it
            // is moved by the difference between where the box is and where the pointer wants it,
            // and every card in it comes along because none of them was ever placed absolutely.
            Held::Task(id) => {
                if let Some((x, y, _, _)) = self.bounds_of(id) {
                    let origin = self.layout.task_origin(id);
                    self.layout
                        .place_task(id, (origin.0 + at.0 - x, origin.1 + at.1 - y));
                }
            }
        }
        if let Some(pointer) = pointer {
            self.drop_grain(pointer, now);
        }
    }

    /// Put it down. Answers the task a card landed in, if that changed.
    pub fn end_carry(&mut self) -> Option<TaskId> {
        let carry = self.carry.take()?;
        let Held::Agent(id) = carry.held else {
            return None;
        };
        let task = carry.over?;
        if self.agent(id)?.task == Some(task) {
            return None;
        }
        // Where it was let go of, so re-anchoring it to the new container's origin leaves it under
        // the pointer rather than jumping it to the same offset in a different frame.
        let at = self.at_id(id)?;
        let agent = self.agent_mut(id)?;
        agent.task = Some(task);
        // A card that moved to another task no longer answers to whoever spawned it there.
        agent.parent = None;
        self.place(id, at);
        Some(task)
    }

    /// Which task's container the carried card is over. Containers do not overlap, so the first
    /// hit wins.
    ///
    /// The carried card is left out of every container it is measured against. Without that, a
    /// card is always inside its own task's box — the box is computed from where its cards are, and
    /// it is one of them — so dragging it anywhere would read as dropping it back where it came
    /// from.
    fn task_at(&self, carried: AgentId, at: (f32, f32)) -> Option<TaskId> {
        let centre = (at.0 + CARD_WIDTH / 2.0, at.1 + CARD_HEIGHT / 2.0);
        self.tasks
            .iter()
            .find(|task| {
                self.bounds_excluding(task.id, Some(carried))
                    .is_some_and(|(x, y, w, h)| {
                        centre.0 >= x && centre.0 <= x + w && centre.1 >= y && centre.1 <= y + h
                    })
            })
            .map(|task| task.id)
    }

    /// The container a task is drawn in: the box round its cards, with room for the label.
    pub fn bounds_of(&self, task: TaskId) -> Option<(f32, f32, f32, f32)> {
        self.bounds_excluding(task, None)
    }

    fn bounds_excluding(
        &self,
        task: TaskId,
        skip: Option<AgentId>,
    ) -> Option<(f32, f32, f32, f32)> {
        let mut members = self
            .agents
            .iter()
            .filter(|a| a.task == Some(task) && Some(a.id) != skip && self.visible(a))
            .peekable();
        members.peek()?;

        let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for agent in members {
            let at = self.layout.at(agent);
            x0 = x0.min(at.0);
            y0 = y0.min(at.1);
            x1 = x1.max(at.0 + CARD_WIDTH);
            y1 = y1.max(at.1 + CARD_HEIGHT);
        }
        Some((
            x0 - GROUP_PAD,
            y0 - GROUP_PAD - GROUP_LABEL,
            (x1 - x0) + GROUP_PAD * 2.0,
            (y1 - y0) + GROUP_PAD * 2.0 + GROUP_LABEL,
        ))
    }

    /// Lay one grain down, and sweep the spent ones while we are here.
    fn drop_grain(&mut self, at: (f32, f32), now: Instant) {
        self.sand.retain(|g| !g.spent(now));
        if self.sand.len() >= GRAIN_CEILING {
            return;
        }
        // Deterministic scatter: no random number generator for four floats, and a repeatable
        // trail is easier to look at than a truly random one.
        let seed = self.sand.len() as f32;
        let spread = ((seed * 12.9898).sin() * 43_758.55).fract();
        let lift = ((seed * 78.233).sin() * 26_963.13).fract();
        self.sand.push(Grain {
            at,
            born: now,
            spread: ((spread - 0.5) * 26.0, (lift - 0.5) * 26.0),
            size: 1.6 + spread * 2.6,
        });
    }

    /// Drop the grains that have run out, and answer whether any are left — which is what tells
    /// the window whether it still owes the trail a frame.
    pub fn settle_sand(&mut self, now: Instant) -> bool {
        self.sand.retain(|g| !g.spent(now));
        !self.sand.is_empty()
    }
}
