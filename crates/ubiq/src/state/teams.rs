//! The Teams screen's own view of the work — a clone of [`super::orchestration`]'s under the new
//! rail mode: what is selected in the graph, which session
//! and which states it is showing, how far in it is zoomed, and what the pointer has hold of.
//!
//! **Every filter can be cleared, and cleared means everything.** A graph showing one session and
//! four states is the useful default and not the only view: the session row has an "all" and no
//! bucket lit is no bucket filter, so the whole of a project's work is always one click away. An
//! empty canvas therefore means an empty project rather than a filter nobody can see.
//!
//! **The work itself is not here.** Sessions, agents and tasks arrive from the host and live in
//! [`super::work`]; this is the view over them, which is why every reader takes a
//! [`WorkProjection`] as its first parameter rather than holding one. The split is what keeps both
//! halves testable without a frame, and it is the same shape `BoardState`'s readers have.
//!
//! **This mode draws only the agents the window can talk to.** The projection every reader is
//! handed is [`live_work`]'s, not the host's whole one, so the fixtures the mock work thread seeds
//! never reach the canvas.
//!
//! Nothing here draws and nothing here names a colour — an activity says what it *is*, and
//! `ui::work` decides which token that reads in. Nothing here says where anything sits either:
//! a record and its position are separate, and [`super::layout`] owns the second half.
//!
//! **Position is the interface's own fact, membership is the host's.** A drag moves a card on the
//! canvas, which nothing outside this window has an opinion about; which task that card *serves* is
//! written down, so a drop answers the pair and the caller sends `AssignAgent`.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use ubiq_proto::conversation::ToolStatus;
use ubiq_proto::ids::{ProjectId, SessionId, TaskId};
use ubiq_proto::work::{Activity, AgentId, Bucket, TaskRecord, WorkAgent};

use super::conversation::{Conversation, Run, SubagentTab};
use super::work::WorkProjection;

pub use super::layout::{
    Algo, CARD_HEIGHT, CARD_WIDTH, GROUP_LABEL, GROUP_PAD, Layout, RING_PAD, Rings, SUB_BOX,
    SUB_GAP, SUB_HEIGHT, SUB_WIDTH, fence, sub_slot,
};

/// The work this mode draws: the host's projection, narrowed to the agents this window actually
/// holds a conversation with.
///
/// **The host's projection is wider than what a window can talk to.** It carries the mock work
/// thread's fixtures as well — a name, an activity, a note, and nothing behind any of them — and a
/// canvas drawing one would be a map of agents that do not exist. So every reader in this mode
/// goes through here, the way the agents screen goes through [`super::agents::AgentsView::live_agents`],
/// and a project with nothing running draws as empty rather than as busy with fictions. `TeamsOld`
/// keeps reading the whole projection.
///
/// A session with no live agent left in it goes with them, and so does a task no live agent serves
/// or holds a step in: both are drawn *about* agents, and an outline round nothing is furniture.
pub fn live_work(work: &WorkProjection, live: &[AgentId]) -> WorkProjection {
    let agents: Vec<WorkAgent> = work
        .agents
        .iter()
        .filter(|agent| live.contains(&agent.id))
        .cloned()
        .collect();
    let sessions = work
        .sessions
        .iter()
        .filter(|session| agents.iter().any(|agent| agent.session == session.id))
        .cloned()
        .collect();
    let tasks = work
        .tasks
        .iter()
        .filter(|task| {
            agents.iter().any(|agent| {
                agent.task == Some(task.id)
                    || task.steps.iter().any(|step| step.owner == Some(agent.id))
            })
        })
        .cloned()
        .collect();
    WorkProjection {
        sessions,
        agents,
        tasks,
        loaded: work.loaded,
    }
}

/// What the canvas is about: the project on screen, or every project the window holds.
///
/// **A span beside the filters, not a second mode.** `RailMode::Teams` stays one mode with one
/// rail entry; this is one more thing the canvas is narrowed by, the way the session row and the
/// bucket row are. The window's own fact, like the zoom and the arrangement — nothing outside this
/// window has an opinion about it, and it is not sent anywhere.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum TeamsSpan {
    /// The active project's agents, which is the only reach the screen had before the span.
    #[default]
    Project,
    /// Every project the window holds, merged into one projection by [`window_work`].
    Window,
}

/// Every open project's live work in one projection, and which project each card came from.
///
/// **Nothing collides, so nothing is renamed.** `AgentId`, `SessionId` and `TaskId` are ULIDs
/// minted per record, so two projects' records never share an id: the merged lists need no
/// prefixing and no composite key, and what comes out is a [`WorkProjection`] like any other —
/// which is why `Layout`, every packer and the whole of [`super::layout`] are untouched by the
/// span. The owner map carries the one thing the merge loses, and every *write* the screen makes
/// needs: whose agent this is.
///
/// Order is the caller's — the window's project order, which is picker order and never moves — so
/// a relayout puts the same project's cards in the same region twice running.
pub fn window_work(
    projects: &[(ProjectId, &WorkProjection, &[AgentId])],
) -> (WorkProjection, HashMap<AgentId, ProjectId>) {
    // Loaded until a project says otherwise: the claim is "nothing here is still waiting on the
    // host", and no project is nothing waiting. An empty slice is therefore `loaded: true`, which
    // is what keeps a window with no project from drawing a spinner nobody can end.
    let mut merged = WorkProjection {
        sessions: Vec::new(),
        agents: Vec::new(),
        tasks: Vec::new(),
        loaded: true,
    };
    let mut owner: HashMap<AgentId, ProjectId> = HashMap::new();
    for (project, work, live) in projects {
        let narrowed = live_work(work, live);
        for agent in &narrowed.agents {
            owner.insert(agent.id, *project);
        }
        merged.sessions.extend(narrowed.sessions);
        merged.agents.extend(narrowed.agents);
        merged.tasks.extend(narrowed.tasks);
        merged.loaded &= narrowed.loaded;
    }
    (merged, owner)
}

/// What a card says it is doing, at the grain the canvas can draw a mark for.
///
/// **One state more than [`Activity`] has, and it is one the data already carries.** The record's
/// activity collapses "finished its turn and is waiting for you to type" into `Ended`, because the
/// host classifies a conversation without knowing whether its harness is still there. The window
/// does know — a live [`Conversation`] keeps [`Run`] beside its stop reason — so a card drawn in
/// this mode tells the two apart: [`AgentStatus::Idle`] is a harness still running with nothing to
/// do, [`AgentStatus::Ended`] is one that has gone.
///
/// Nothing beyond that is invented. Every other state is the activity the record or the live
/// conversation already reports, renamed here so one enum covers both readings.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AgentStatus {
    Thinking,
    Writing,
    Tools,
    NeedsYou,
    /// Alive, its last turn finished, waiting on a human to say something next.
    Idle,
    Ended,
    Failed,
}

impl AgentStatus {
    pub fn label(self) -> &'static str {
        match self {
            AgentStatus::Idle => "Idle",
            AgentStatus::Thinking => Activity::Thinking.label(),
            AgentStatus::Writing => Activity::Writing.label(),
            AgentStatus::Tools => Activity::Tools.label(),
            AgentStatus::NeedsYou => Activity::NeedsYou.label(),
            AgentStatus::Ended => Activity::Ended.label(),
            AgentStatus::Failed => Activity::Failed.label(),
        }
    }

    /// Which filter bucket — and so which colour — the state reads in. An idle agent is waiting on
    /// the reader exactly as one asking a permission question is, so the two share a bucket and
    /// are told apart by their glyph.
    pub fn bucket(self) -> Bucket {
        match self {
            AgentStatus::Thinking | AgentStatus::Writing | AgentStatus::Tools => Bucket::Running,
            AgentStatus::NeedsYou | AgentStatus::Idle => Bucket::Waiting,
            AgentStatus::Ended => Bucket::Ended,
            AgentStatus::Failed => Bucket::Error,
        }
    }

    fn from_activity(activity: Activity) -> Self {
        match activity {
            Activity::Thinking => AgentStatus::Thinking,
            Activity::Writing => AgentStatus::Writing,
            Activity::Tools => AgentStatus::Tools,
            Activity::NeedsYou => AgentStatus::NeedsYou,
            Activity::Ended => AgentStatus::Ended,
            Activity::Failed => AgentStatus::Failed,
        }
    }
}

/// What a card is doing, read off the live conversation where the window holds one and off the
/// record where it does not.
///
/// The conversation is the better witness of the two — it is the stream itself, and the record is
/// the host's periodic reading of it — so it wins wherever it exists, with the record as the
/// honest fallback.
pub fn agent_status(agent: &WorkAgent, conversation: Option<&Conversation>) -> AgentStatus {
    let Some(conversation) = conversation else {
        return AgentStatus::from_activity(agent.activity);
    };
    let status = AgentStatus::from_activity(conversation.activity());
    // The one refinement: a turn that has stopped on a harness that has not.
    match (status, conversation.run) {
        (AgentStatus::Ended, Run::Idle) => AgentStatus::Idle,
        _ => status,
    }
}

/// What one delegate in a card's ring is doing.
///
/// **Only what the spawning call says.** A delegate is a stamp on a transcript's lines rather than
/// a record of its own, so the states here are the four a [`ToolStatus`] has plus the permission
/// count the tab already carries — and [`DelegateStatus::Unknown`], which is drawn as no mark at
/// all, because a call that is not in the transcript tells the window nothing to draw.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DelegateStatus {
    Queued,
    Working,
    NeedsYou,
    Done,
    Failed,
    Unknown,
}

impl DelegateStatus {
    pub fn label(self) -> &'static str {
        match self {
            DelegateStatus::Queued => "Queued",
            DelegateStatus::Working => "Working",
            DelegateStatus::NeedsYou => "Needs you",
            DelegateStatus::Done => "Done",
            DelegateStatus::Failed => "Error",
            DelegateStatus::Unknown => "Unknown",
        }
    }

    pub fn bucket(self) -> Bucket {
        match self {
            DelegateStatus::Working => Bucket::Running,
            DelegateStatus::Queued | DelegateStatus::NeedsYou => Bucket::Waiting,
            DelegateStatus::Done | DelegateStatus::Unknown => Bucket::Ended,
            DelegateStatus::Failed => Bucket::Error,
        }
    }
}

/// A delegate's state: the question it is blocked on if it has one, and otherwise what its
/// spawning call last said.
pub fn delegate_status(tab: &SubagentTab) -> DelegateStatus {
    if tab.waiting > 0 {
        return DelegateStatus::NeedsYou;
    }
    match tab.status {
        Some(ToolStatus::Pending) => DelegateStatus::Queued,
        Some(ToolStatus::InProgress) => DelegateStatus::Working,
        Some(ToolStatus::Completed) => DelegateStatus::Done,
        Some(ToolStatus::Failed) => DelegateStatus::Failed,
        None => DelegateStatus::Unknown,
    }
}

/// What the inspector and the tasks strip are about. A session, an agent and one of an agent's
/// delegates are all selectable, and the three answer the same questions at different scales.
///
/// **A delegate is not an agent.** The host reports no `WorkAgent` for one — `parent` is always
/// `None` on the wire — so a subagent is named by the conversation it spoke in plus the id of the
/// `Task` call that spawned it, and every reader that wants "which workspace is this about"
/// answers it through [`TeamsView::agent_in_focus`]. That id is a `String`, which is why this is
/// `Clone` and not `Copy`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum TeamsSelection {
    Session(SessionId),
    Agent(AgentId),
    Subagent { agent: AgentId, subagent: String },
}

/// Which half of the inspector is showing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TeamsInspectorTab {
    Chat,
    Tasks,
}

/// One grain of the trail a dragged card leaves behind.
///
/// The trail is not decoration: a card that moves with no evidence of having moved reads as a
/// redraw, and a card that leaves a fading track reads as something the user picked up.
#[derive(Clone, Copy, Debug)]
pub struct TeamsGrain {
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

impl TeamsGrain {
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
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum TeamsHeld {
    Agent(AgentId),
    Task(TaskId),
    /// One of a card's delegates. Named the way [`TeamsSelection::Subagent`] is — by the id of the
    /// `Task` call that spawned it — which is what makes this `Clone` rather than `Copy`.
    Subagent {
        agent: AgentId,
        subagent: String,
    },
}

/// Something being carried. The grab point is where inside it the pointer went down, so it does
/// not jump under the cursor on the first move.
#[derive(Clone, Debug)]
pub struct TeamsCarry {
    pub held: TeamsHeld,
    pub grab: (f32, f32),
    /// The task container the pointer is over, which is what a drop would move a card into. Always
    /// `None` while a container is the thing being carried: a task is not filed inside a task.
    pub over: Option<TaskId>,
}

pub struct TeamsView {
    /// Where the work is drawn. Thrown away and recomputed whole by `relayout`, and topped up one
    /// arriving card at a time by `Layout::place_new`.
    pub layout: Layout,
    /// Which arrangement `relayout` and `absorb_new` compute. The window's own fact, like zoom —
    /// it is not sent anywhere, and nothing outside this window has an opinion about it.
    pub algo: Algo,

    /// Which session the graph is drawing, or every one of them. Its own field rather than a
    /// reading of `selection`, because which session is *shown* and which is *selected* are two
    /// questions: the inspector and the drawer report on the second, and clearing the first must
    /// not throw the second away.
    pub session: Option<SessionId>,
    /// Which buckets the graph is showing. **Empty is no filter, not nothing** — a card in a hidden
    /// bucket is not drawn, and neither are the connectors into it, so a row with every pill off
    /// would otherwise be an empty screen with no way back.
    pub buckets: Vec<Bucket>,
    /// Whether a card stops drawing the delegates that have finished.
    ///
    /// **A third filter beside the session and the buckets, and the only one about delegates.**
    /// The bucket row hides whole cards; a card's ring goes on growing under it for the life of
    /// the conversation, because every delegate a transcript ever named is still named there. A
    /// long session therefore ends as a wall of finished boxes round three working ones, which is
    /// the clutter this answers.
    ///
    /// **Done is a state the data already carries, not a timer.** A delegate is the spawning
    /// `Task` call, and that call's [`ToolStatus`] reaches `Completed` when the delegate returns —
    /// so [`DelegateStatus::Done`] is a real terminal reading and nothing here has to guess from
    /// how long it has been quiet. `Failed` is left on screen: an error is what a reader came to
    /// find.
    pub hide_done: bool,
    pub zoom: f32,
    pub selection: Option<TeamsSelection>,
    pub tab: TeamsInspectorTab,
    pub show_inspector: bool,
    pub tasks_open: bool,

    /// Which delegates each card is drawing, in the order the transcript named them, as of the
    /// last frame.
    ///
    /// **The ids are copied here rather than read where they are needed.** They come off the
    /// conversation — a delegate is a stamp on a transcript's lines, not a record the host sends —
    /// and the geometry readers below take a [`WorkProjection`] and nothing else. Keeping the list
    /// they need means a card's fence, and the task container round it, can be measured without
    /// this module learning what a transcript is. Refreshed by the window each frame, in
    /// `settle_teams`.
    pub rings: HashMap<AgentId, Vec<String>>,

    pub carry: Option<TeamsCarry>,
    pub sand: Vec<TeamsGrain>,
}

/// The screen as it opens: every session and every state showing, zoomed out far enough to see the
/// work whole, the inspector up on the thread and the tasks drawer shut. Written out rather than
/// derived, because the derived zero of a zoom is a graph nobody can see.
impl Default for TeamsView {
    fn default() -> Self {
        Self {
            layout: Layout::default(),
            algo: Algo::default(),
            session: None,
            buckets: Bucket::all().to_vec(),
            hide_done: false,
            zoom: 0.8,
            // Nothing is selected until there is something to select; the window points the
            // selection at the first agent the moment the work arrives.
            selection: None,
            tab: TeamsInspectorTab::Chat,
            show_inspector: true,
            tasks_open: false,
            rings: HashMap::new(),
            carry: None,
            sand: Vec::new(),
        }
    }
}

/// The zoom range and the step the toolbar's `−` and `+` move in.
pub const ZOOM_MIN: f32 = 0.5;
pub const ZOOM_MAX: f32 = 1.6;
pub const ZOOM_STEP: f32 = 0.1;

impl TeamsView {
    /// Where a card is drawn, on the canvas at 100% zoom. The layout alone answers this, which is
    /// why it is the one reader that needs no projection.
    pub fn at(&self, agent: &WorkAgent) -> (f32, f32) {
        self.layout.at(agent)
    }

    pub fn at_id(&self, work: &WorkProjection, id: AgentId) -> Option<(f32, f32)> {
        work.agent(id).map(|agent| self.layout.at(agent))
    }

    /// Put a card at a point on the canvas, whatever frame it hangs off.
    pub fn place(&mut self, work: &WorkProjection, id: AgentId, at: (f32, f32)) {
        let origin = work
            .agent(id)
            .and_then(|agent| agent.task)
            .map(|task| self.layout.task_origin(task))
            .unwrap_or((0.0, 0.0));
        self.layout
            .place_agent(id, (at.0 - origin.0, at.1 - origin.1));
    }

    /// Throw the arrangement away and compute it again from the records, in the chosen algorithm.
    pub fn relayout(&mut self, work: &WorkProjection) {
        self.layout = Layout::auto(&work.agents, &work.tasks, self.algo, &self.ring_counts());
    }

    /// Choose an arrangement and lay the graph out in it at once — picking one *is* asking for it,
    /// so there is no second control to press.
    pub fn set_algo(&mut self, algo: Algo, work: &WorkProjection) {
        self.algo = algo;
        self.relayout(work);
    }

    /// Give an arriving card a place, and leave every placed card alone — the "topped up" path
    /// `relayout` is not: a card the user has not seen yet gets slotted in by the chosen algorithm,
    /// nothing already on the canvas moves. Forwards to [`Layout::place_new`] so no caller outside
    /// this module has to know which algorithm is current.
    pub fn absorb_new(&mut self, work: &WorkProjection) {
        self.layout
            .place_new(&work.agents, &work.tasks, self.algo, &self.ring_counts());
    }

    /// How many delegates each card wears, which is all the arrangement needs of them.
    fn ring_counts(&self) -> Rings {
        self.rings
            .iter()
            .map(|(agent, subs)| (*agent, subs.len()))
            .collect()
    }

    /// Which workspace the screen is about — the selected card, or the card whose delegate is
    /// selected. **The one answer**, so the composer, the inspector and the link a card copies
    /// cannot disagree about whose conversation is on screen.
    pub fn agent_in_focus(&self) -> Option<AgentId> {
        match &self.selection {
            Some(TeamsSelection::Agent(id)) => Some(*id),
            Some(TeamsSelection::Subagent { agent, .. }) => Some(*agent),
            _ => None,
        }
    }

    /// Which delegate of that workspace is selected, where one is.
    pub fn subagent_in_focus(&self) -> Option<&str> {
        match &self.selection {
            Some(TeamsSelection::Subagent { subagent, .. }) => Some(subagent.as_str()),
            _ => None,
        }
    }

    /// The selected agent, when an agent — or one of its delegates — is what is selected.
    pub fn selected_agent<'a>(&self, work: &'a WorkProjection) -> Option<&'a WorkAgent> {
        work.agent(self.agent_in_focus()?)
    }

    /// Which delegates a card is drawing, in transcript order. Empty is no ring at all.
    pub fn ring(&self, agent: AgentId) -> &[String] {
        self.rings.get(&agent).map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn ring_count(&self, agent: AgentId) -> usize {
        self.ring(agent).len()
    }

    /// The slot the `ix`-th delegate starts in, in the shape the chosen arrangement asks for.
    ///
    /// **The ring is the arrangement's, not a constant.** A grid ring and a one-per-row ring put
    /// the same delegate in different places, and the packers reserved room for whichever one this
    /// answers — so reading `sub_slot` directly here would draw a shape nothing left room for.
    fn slot(&self, agent: AgentId, ix: usize) -> (f32, f32) {
        let slots = (self.algo.ring())(self.ring_count(agent));
        slots.get(ix).copied().unwrap_or_else(|| sub_slot(ix))
    }

    /// Where one delegate is drawn, on the canvas: the offset a drag wrote for it, or the slot it
    /// starts in. `at` is where its parent card is.
    pub fn sub_at(&self, agent: AgentId, at: (f32, f32), sub: &str, ix: usize) -> (f32, f32) {
        let offset = self
            .layout
            .sub_offset(agent, sub)
            .unwrap_or_else(|| self.slot(agent, ix));
        (at.0 + offset.0, at.1 + offset.1)
    }

    /// What one delegate box measures. [`SUB_BOX`] under every arrangement: a block is drawn at
    /// the size the interface draws it, and no ring shape scales one.
    pub fn sub_box(&self) -> (f32, f32) {
        SUB_BOX
    }

    /// Every delegate of one card, in transcript order, where each is drawn.
    pub fn subs_at(&self, agent: AgentId, at: (f32, f32)) -> Vec<(f32, f32)> {
        self.ring(agent)
            .iter()
            .enumerate()
            .map(|(ix, sub)| self.sub_at(agent, at, sub, ix))
            .collect()
    }

    /// The fence round a card and its delegates, `(x, y, w, h)` — `None` for a card with none.
    ///
    /// Derived from where the delegates actually are, so dragging one resizes the fence and
    /// nothing has to hold a rectangle in step with them.
    pub fn fence_of(&self, agent: AgentId, at: (f32, f32)) -> Option<(f32, f32, f32, f32)> {
        fence(at, &self.subs_at(agent, at))
    }

    /// What a card takes on the canvas, its fence included: `(x0, y0, x1, y1)` at 100% zoom.
    ///
    /// The one measurement every container's outline is taken from, so a fence is never drawn
    /// poking out of the box round it.
    pub fn card_bounds(&self, agent: AgentId, at: (f32, f32)) -> (f32, f32, f32, f32) {
        match self.fence_of(agent, at) {
            Some((x, y, w, h)) => (x, y, x + w, y + h),
            None => (at.0, at.1, at.0 + CARD_WIDTH, at.1 + CARD_HEIGHT),
        }
    }

    /// Which session the screen is *about*: the one selected, or the one the selected agent runs
    /// in, falling back to the first so the inspector and the drawer always have something to
    /// report. What the canvas *draws* is `session`, which is a separate question.
    pub fn active_session(&self, work: &WorkProjection) -> Option<SessionId> {
        match &self.selection {
            Some(TeamsSelection::Session(id)) => Some(*id),
            // A delegate runs in its parent's session, because it runs *inside* its parent.
            Some(_) => work.agent(self.agent_in_focus()?).map(|a| a.session),
            None => work.sessions.first().map(|s| s.id),
        }
    }

    /// Whether one bucket is drawn. **No pill lit is no filter**: the row means "narrow it to
    /// these", and narrowing to nothing is what an untouched row already does.
    pub fn showing(&self, bucket: Bucket) -> bool {
        self.buckets.is_empty() || self.buckets.contains(&bucket)
    }

    /// The delegates one card draws, out of everything its transcript named.
    ///
    /// **The one place the delegate filter is applied**, because the ring is read twice — once by
    /// the window, to write [`Self::rings`] and so tell the arrangement how much room a card
    /// needs, and once by the canvas, to draw the boxes. Two readings that disagree are a card
    /// packed for a ring it does not draw.
    pub fn drawn_delegates(&self, tabs: Vec<SubagentTab>) -> Vec<SubagentTab> {
        if !self.hide_done {
            return tabs;
        }
        tabs.into_iter()
            .filter(|tab| delegate_status(tab) != DelegateStatus::Done)
            .collect()
    }

    /// Whether a card is drawn at all, given the two filters. `session` absent is every session.
    pub fn visible(&self, agent: &WorkAgent) -> bool {
        self.showing(agent.activity.bucket()) && self.session.is_none_or(|id| agent.session == id)
    }

    /// The tasks the strip lists: every task in the session, or the ones the selected agent has a
    /// step in.
    pub fn listed_tasks<'a>(&self, work: &'a WorkProjection) -> Vec<&'a TaskRecord> {
        match self.agent_in_focus() {
            Some(id) => work
                .tasks
                .iter()
                .filter(|t| {
                    t.steps.iter().any(|s| s.owner == Some(id))
                        || work.agent(id).and_then(|a| a.task) == Some(t.id)
                })
                .collect(),
            None => {
                let session = self.active_session(work);
                // A task nobody has started belongs to no session, and the graph is a screen about
                // sessions: it is the board that has somewhere to draw it.
                work.tasks
                    .iter()
                    .filter(|t| t.session.is_some() && t.session == session)
                    .collect()
            }
        }
    }

    // ── Mutators ────────────────────────────────────────────────────
    //
    // None of them notifies: they are called from `AppState`, which is what owns the redraw.

    /// Turn one bucket's pill on or off. Any of them may be the last: with none lit the row is not
    /// filtering, which is the way back from having turned them all off.
    pub fn toggle_bucket(&mut self, bucket: Bucket) {
        if let Some(ix) = self.buckets.iter().position(|b| *b == bucket) {
            self.buckets.remove(ix);
        } else {
            self.buckets.push(bucket);
        }
    }

    /// Stop drawing the delegates that have finished, or draw them again.
    pub fn toggle_hide_done(&mut self) {
        self.hide_done = !self.hide_done;
    }

    /// Show one session, or every one. It leaves the selection alone: "show me all of it" is not
    /// "stop looking at this".
    pub fn show_session(&mut self, session: Option<SessionId>) {
        self.session = session;
    }

    /// Put every filter back, which is the toolbar's one control for "show everything".
    pub fn clear_filters(&mut self) {
        self.session = None;
        self.buckets = Bucket::all().to_vec();
        self.hide_done = false;
    }

    /// Whether anything is being hidden, so the control that clears the filters can say whether it
    /// has anything to do.
    pub fn filtered(&self) -> bool {
        self.session.is_some() || self.buckets.len() < Bucket::all().len() || self.hide_done
    }

    pub fn zoom_by(&mut self, delta: f32) {
        self.zoom = (self.zoom + delta).clamp(ZOOM_MIN, ZOOM_MAX);
    }

    pub fn zoom_pct(&self) -> u32 {
        (self.zoom * 100.0).round() as u32
    }

    /// Pick something up. The grab point is where inside it the pointer went down.
    /// Put a delegate at a point on the canvas, as an offset against the card that spawned it —
    /// so the card it belongs to carries it when *that* is dragged.
    pub fn place_sub(
        &mut self,
        work: &WorkProjection,
        agent: AgentId,
        sub: String,
        at: (f32, f32),
    ) {
        let Some(card) = self.at_id(work, agent) else {
            return;
        };
        self.layout
            .place_sub(agent, sub, (at.0 - card.0, at.1 - card.1));
    }

    pub fn start_carry(&mut self, held: TeamsHeld, grab: (f32, f32)) {
        self.carry = Some(TeamsCarry {
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
    ///
    /// `eligible` is which containers this card may be filed into, `None` being all of them. The
    /// caller decides: under a canvas spanning several projects the projection's tasks are every
    /// project's, and a card filed into a foreign container would name a task its own host has
    /// never heard of — but *whose* a task is is a question this module deliberately cannot ask,
    /// so it arrives already answered. Narrowing it here rather than at the drop is what keeps the
    /// canvas honest: a container that never lights up is a hand-over that never looks offered,
    /// and the card is never re-anchored to a frame it cannot join.
    pub fn carry_to(
        &mut self,
        work: &WorkProjection,
        at: (f32, f32),
        pointer: Option<(f32, f32)>,
        now: Instant,
        eligible: Option<&HashSet<TaskId>>,
    ) {
        let Some(carry) = self.carry.clone() else {
            return;
        };
        match carry.held {
            TeamsHeld::Agent(id) => {
                self.place(work, id, at);
                // Which container the pointer is over decides what a drop means, and is what the
                // canvas lights up while the card is in the air.
                let over = self
                    .task_at(work, id, at)
                    .filter(|task| eligible.is_none_or(|open| open.contains(task)));
                if let Some(carry) = self.carry.as_mut() {
                    carry.over = over;
                }
            }
            // A container has no position of its own — its box is the box round its cards — so it
            // is moved by the difference between where the box is and where the pointer wants it,
            // and every card in it comes along because none of them was ever placed absolutely.
            TeamsHeld::Task(id) => {
                if let Some((x, y, _, _)) = self.bounds_of(work, id) {
                    let origin = self.layout.task_origin(id);
                    self.layout
                        .place_task(id, (origin.0 + at.0 - x, origin.1 + at.1 - y));
                }
            }
            // A delegate moves inside its parent's frame and nowhere else: it is not a card the
            // host knows about, so no drop of one is ever a hand-over and nothing lights up under
            // it.
            TeamsHeld::Subagent { agent, subagent } => {
                self.place_sub(work, agent, subagent, at);
            }
        }
        if let Some(pointer) = pointer {
            self.drop_grain(pointer, now);
        }
    }

    /// Put it down, and answer the card and the container it landed in — for the caller to send as
    /// an `AssignAgent`.
    ///
    /// **Position is the interface's own fact, membership is the host's.** The offset is written
    /// here, because where a card sits on this canvas is nothing anybody outside the window has an
    /// opinion about; which task the card *serves* is written down, so this touches none of it and
    /// the answer is a request rather than a result. The offset is taken against the container the
    /// card landed in rather than the one it is still recorded in, so the card is where it was let
    /// go of the moment the host confirms — the one frame in between draws it against its old
    /// origin, which is the cost of not writing the answer down before it is given.
    ///
    /// `None` is a card put down on open ground, a card put back where it came from, or a container
    /// that was carried — none of them a hand-over.
    pub fn end_carry(&mut self, work: &WorkProjection) -> Option<(AgentId, TaskId)> {
        let carry = self.carry.take()?;
        let TeamsHeld::Agent(id) = carry.held else {
            return None;
        };
        let task = carry.over?;
        if work.agent(id)?.task == Some(task) {
            return None;
        }
        // Where it was let go of, so re-anchoring it to the new container's origin leaves it under
        // the pointer rather than jumping it to the same offset in a different frame.
        let at = self.at_id(work, id)?;
        let origin = self.layout.task_origin(task);
        self.layout
            .place_agent(id, (at.0 - origin.0, at.1 - origin.1));
        Some((id, task))
    }

    /// Which task's container the carried card is over. Containers do not overlap, so the first
    /// hit wins.
    ///
    /// The carried card is left out of every container it is measured against. Without that, a
    /// card is always inside its own task's box — the box is computed from where its cards are, and
    /// it is one of them — so dragging it anywhere would read as dropping it back where it came
    /// from.
    fn task_at(&self, work: &WorkProjection, carried: AgentId, at: (f32, f32)) -> Option<TaskId> {
        let centre = (at.0 + CARD_WIDTH / 2.0, at.1 + CARD_HEIGHT / 2.0);
        work.tasks
            .iter()
            .find(|task| {
                self.bounds_excluding(work, task.id, Some(carried))
                    .is_some_and(|(x, y, w, h)| {
                        centre.0 >= x && centre.0 <= x + w && centre.1 >= y && centre.1 <= y + h
                    })
            })
            .map(|task| task.id)
    }

    /// The container a task is drawn in: the box round its cards, with room for the label.
    pub fn bounds_of(&self, work: &WorkProjection, task: TaskId) -> Option<(f32, f32, f32, f32)> {
        self.bounds_excluding(work, task, None)
    }

    fn bounds_excluding(
        &self,
        work: &WorkProjection,
        task: TaskId,
        skip: Option<AgentId>,
    ) -> Option<(f32, f32, f32, f32)> {
        let mut members = work
            .agents
            .iter()
            .filter(|a| a.task == Some(task) && Some(a.id) != skip && self.visible(a))
            .peekable();
        members.peek()?;

        let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for agent in members {
            // The card's ring counts as the card: a container is the box round what its members
            // draw, and a ring is part of what one of them draws.
            let (ax0, ay0, ax1, ay1) = self.card_bounds(agent.id, self.layout.at(agent));
            x0 = x0.min(ax0);
            y0 = y0.min(ay0);
            x1 = x1.max(ax1);
            y1 = y1.max(ay1);
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
        self.sand.push(TeamsGrain {
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

#[cfg(test)]
mod tests {
    use super::*;
    use ubiq_proto::work::Activity;

    fn a_delegate(id: &str, status: Option<ToolStatus>) -> SubagentTab {
        SubagentTab {
            id: id.to_string(),
            name: id.to_string(),
            status,
            kind: None,
            model: None,
            thinking: None,
            waiting: 0,
            activity: None,
        }
    }

    fn an_agent(session: SessionId, parent: Option<AgentId>) -> WorkAgent {
        WorkAgent {
            id: AgentId::generate(),
            session,
            task: None,
            parent,
            name: "worker".to_string(),
            summary: None,
            role: "Implementer".to_string(),
            activity: Activity::Writing,
            note: String::new(),
            branch: "main".to_string(),
            tokens: 0.0,
            harness: "Claude Code".to_string(),
            account: "work".to_string(),
            model: String::new(),
            context_pct: 0,
            persistent: false,
            accept_all: false,
            debug_dump: None,
            run_dir: None,
            config_dir: None,
            thread: Vec::new(),
        }
    }

    /// The filter drops the finished delegates and nothing else. A failed one stays: an error is
    /// what a reader came to the canvas to find, and `Unknown` is a delegate whose spawning call
    /// the transcript does not hold — not a claim that it is over.
    #[test]
    fn hide_done_drops_only_the_delegates_that_finished() {
        let tabs = vec![
            a_delegate("done", Some(ToolStatus::Completed)),
            a_delegate("working", Some(ToolStatus::InProgress)),
            a_delegate("failed", Some(ToolStatus::Failed)),
            a_delegate("unknown", None),
        ];

        let mut view = TeamsView::default();
        assert_eq!(
            view.drawn_delegates(tabs.clone()).len(),
            4,
            "the filter off is every delegate the transcript named"
        );

        view.hide_done = true;
        let drawn: Vec<String> = view
            .drawn_delegates(tabs)
            .into_iter()
            .map(|tab| tab.id)
            .collect();
        assert_eq!(drawn, vec!["working", "failed", "unknown"]);
    }

    /// **A hidden delegate is not laid out.** The ring counts the arrangement packs against come
    /// from the same filter the canvas draws through, so hiding a finished delegate gives the row
    /// below the card its room back rather than leaving a gap where a box used to be.
    #[test]
    fn a_hidden_delegate_stops_taking_room_in_the_arrangement() {
        let session = SessionId::generate();
        let lead = an_agent(session, None);
        let under = an_agent(session, Some(lead.id));
        let work = WorkProjection {
            sessions: Vec::new(),
            agents: vec![lead.clone(), under.clone()],
            tasks: Vec::new(),
            loaded: true,
        };
        let tabs = vec![
            a_delegate("done", Some(ToolStatus::Completed)),
            a_delegate("working", Some(ToolStatus::InProgress)),
        ];

        let mut view = TeamsView::default();
        let ring = |view: &TeamsView| -> Vec<String> {
            view.drawn_delegates(tabs.clone())
                .into_iter()
                .map(|tab| tab.id)
                .collect()
        };

        view.rings.insert(lead.id, ring(&view));
        view.relayout(&work);
        let both = view.at(&under).1;

        view.hide_done = true;
        view.rings.insert(lead.id, ring(&view));
        view.relayout(&work);
        let one = view.at(&under).1;

        assert!(
            one < both,
            "the row under the card came up by the hidden box's height: {one} vs {both}"
        );
    }

    /// Turning the tick box on is something being hidden, so the row's one control for "show me
    /// all of it" appears — and clears it along with the session and the buckets.
    #[test]
    fn hide_done_is_a_filter_that_show_everything_clears() {
        let mut view = TeamsView::default();
        assert!(!view.filtered());

        view.toggle_hide_done();
        assert!(view.hide_done);
        assert!(view.filtered());

        view.clear_filters();
        assert!(!view.hide_done);
        assert!(!view.filtered());
    }
}
