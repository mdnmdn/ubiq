//! The agents screen's state: the orchestration graph, what is selected in it, and the tasks
//! hanging off that selection.
//!
//! Nothing here draws and nothing here names a colour — an activity says what it *is*, and
//! `ui::agents` decides which token that reads in. Positions are graph coordinates in points at
//! 100% zoom, so the zoom control scales them at draw time and a drag writes back the same numbers
//! whatever the zoom was.
//!
//! This is a fixture screen. Sessions, agents and tasks are invented in [`super::sample`] because
//! the orchestration graph has no transport family yet; it goes the same way the chat does when it
//! gets one.

use std::time::{Duration, Instant};

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
}

/// One step of a task, and which agent has it.
#[derive(Clone, Debug)]
pub struct Step {
    pub title: String,
    pub done: bool,
    pub owner: Option<AgentId>,
}

#[derive(Clone, Debug)]
pub struct Task {
    pub id: TaskId,
    pub session: SessionId,
    pub shape: Shape,
    pub title: String,
    pub steps: Vec<Step>,
}

impl Task {
    pub fn done(&self) -> usize {
        self.steps.iter().filter(|s| s.done).count()
    }
}

#[derive(Clone, Debug)]
pub struct Session {
    pub id: SessionId,
    pub name: String,
    pub branch: String,
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
    /// Where the card sits on the canvas, in points at 100% zoom.
    pub at: (f32, f32),
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

/// A card being carried. The offset is where inside the card it was picked up, so it does not jump
/// under the cursor on the first move.
#[derive(Clone, Copy, Debug)]
pub struct Carry {
    pub agent: AgentId,
    pub grab: (f32, f32),
    /// The card's position when the drag started, so a drop outside the canvas can put it back.
    pub from: (f32, f32),
    /// The task container the pointer is over, which is what a drop would move the card into.
    pub over: Option<TaskId>,
}

pub struct AgentsState {
    pub sessions: Vec<Session>,
    pub agents: Vec<Agent>,
    pub tasks: Vec<Task>,

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
        Self {
            sessions,
            agents,
            tasks,
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
                self.tasks
                    .iter()
                    .filter(|t| Some(t.session) == session)
                    .collect()
            }
        }
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

    /// Pick a card up. The grab point is where inside the card the pointer went down.
    pub fn start_carry(&mut self, agent: AgentId, grab: (f32, f32)) {
        let Some(from) = self.agent(agent).map(|a| a.at) else {
            return;
        };
        self.carry = Some(Carry {
            agent,
            grab,
            from,
            over: None,
        });
    }

    /// Move the carried card, and lay a grain down where it passed.
    ///
    /// `at` is in graph coordinates; `pointer` is where the pointer is in the window, which is the
    /// frame the sand is painted in.
    pub fn carry_to(&mut self, at: (f32, f32), pointer: (f32, f32), now: Instant) {
        let Some(carry) = self.carry else { return };
        if let Some(agent) = self.agent_mut(carry.agent) {
            agent.at = at;
        }
        // Which container the pointer is over decides what a drop means, and is what the canvas
        // lights up while the card is in the air.
        let over = self.task_at(carry.agent, at);
        if let Some(carry) = self.carry.as_mut() {
            carry.over = over;
        }
        self.drop_grain(pointer, now);
    }

    /// Put the card down. Answers the task it landed in, if it changed.
    pub fn end_carry(&mut self) -> Option<TaskId> {
        let carry = self.carry.take()?;
        let over = carry.over;
        if let Some(task) = over
            && let Some(agent) = self.agent_mut(carry.agent)
            && agent.task != Some(task)
        {
            agent.task = Some(task);
            // A card that moved to another task no longer answers to whoever spawned it there.
            agent.parent = None;
            return Some(task);
        }
        None
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
            x0 = x0.min(agent.at.0);
            y0 = y0.min(agent.at.1);
            x1 = x1.max(agent.at.0 + CARD_WIDTH);
            y1 = y1.max(agent.at.1 + CARD_HEIGHT);
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

/// The card's size at 100% zoom. The graph's arithmetic — containers, connectors, hit testing —
/// all works from these, so a card that changes size changes them in one place.
pub const CARD_WIDTH: f32 = 264.0;
pub const CARD_HEIGHT: f32 = 116.0;

/// What a task's container leaves round its cards, and the room its label takes above them.
pub const GROUP_PAD: f32 = 22.0;
pub const GROUP_LABEL: f32 = 26.0;
