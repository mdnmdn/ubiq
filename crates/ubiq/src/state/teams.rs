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

use ubiq_proto::ids::{ProjectId, SessionId, TaskId};
use ubiq_proto::work::{AgentId, Bucket, TaskRecord, WorkAgent};

use super::conversation::SubagentTab;
use super::work::WorkProjection;

pub use super::layout::{
    Algo, CARD_WIDTH, GROUP_LABEL, GROUP_PAD, Layout, MISSION_BAND, MISSION_HANDLE, MISSION_MIN,
    RING_PAD, Rings, SUB_BOX, SUB_GAP, SUB_HEIGHT, SUB_WIDTH, TEAMS_CARD_HEIGHT, fence, sub_slot,
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
/// **Not stored anywhere.** The span is which rail entry the window is on — `RailMode::Teams` for
/// the project, `RailMode::TeamsAll` for the window — and `AppState::teams_span` derives it. So
/// there is no switch on the canvas to get out of step with the rail, and no second answer to
/// reconcile; nothing outside this window has an opinion about it, and it is not sent anywhere.
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

/// What a card says it is doing — the *shared* vocabulary, not one of this mode's own.
///
/// **An agent and a delegate say the same things now.** They used to have an enum each, and the
/// two disagreed about the one fact that matters most: a delegate reading `Done` was still counted
/// as running. [`super::status`] holds the pair both speak, and this mode re-exports it so a
/// reader of the canvas has one place to look.
pub use super::status::{Doing, Lifecycle, Status, agent_status, delegate_status};

/// What the right dock's conversation and the tasks strip are about. A session, an agent and one
/// of an agent's delegates are all selectable, and the three answer the same questions at
/// different scales.
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
    Subagent {
        agent: AgentId,
        subagent: String,
    },
    /// A mission, by the anchor task it is (M1). Selected by its fence handle, and the one
    /// selection that opens no conversation — a mission is not a workspace, so what the handle
    /// puts in the right dock is the mission panel.
    Mission(TaskId),
}

/// Which half of `[Teams]`'s inspector is showing. `Teams` has no inspector of its own — a
/// selection opens a `Chat` panel in the right dock instead — but keeps the field for the link
/// grammar `ubiq://` deep links share with `[Teams]` (`state/nav/text.rs`), and sets it to `Chat`
/// wherever it points at an agent.
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
    /// A mission's fence, by its anchor task. Held by its handle alone — the ground inside a
    /// mission fence stays free, because that ground is what a card is dropped onto to join the
    /// mission (M15).
    Mission(TaskId),
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

    /// Which sessions the graph is drawing, or every one of them when empty. Its own field rather
    /// than a reading of `selection`, because which sessions are *shown* and which is *selected*
    /// are two questions: the drawer reports on the second, and narrowing the first must not throw
    /// the second away.
    ///
    /// **A set, not a choice** — several sessions on at once, the same shape `buckets` already is
    /// and for the same reason: under the window span each session is a project, and asking "which
    /// projects" is asking for a union, not a single answer. **Empty is no filter**, exactly as it
    /// was when this held one `Option<SessionId>` — narrowing to nothing is what an untouched
    /// control already does.
    pub sessions: Vec<SessionId>,
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
    /// Which missions the graph is drawing, or every one of them when empty — the third filter
    /// beside the sessions and the buckets, and the same shape both of them are. A mission is
    /// named by its anchor task (M1).
    pub mission_filter: Vec<TaskId>,
    pub zoom: f32,
    pub selection: Option<TeamsSelection>,
    pub tab: TeamsInspectorTab,
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

    /// Which missions the canvas fences, and who is in each — the anchor task the mission *is*,
    /// against every agent on it.
    ///
    /// **Copied here for [`Self::rings`]'s reason.** Membership is "assigned to the mission's task
    /// or one of its children, *or* spawned by an agent that is in the mission" (M11), and the
    /// second half is stored on `MissionRecord::roster` rather than derivable — `WorkAgent::parent`
    /// is cleared by every reassignment. The record lives on the project, which this module
    /// deliberately knows nothing about, so the window answers the question once a frame in
    /// `settle_teams` and every geometry reader below works from the answer. An entry with an
    /// empty vector is a mission with nobody on it, which still draws its handle (M15).
    pub missions: HashMap<TaskId, Vec<AgentId>>,

    pub carry: Option<TeamsCarry>,
    pub sand: Vec<TeamsGrain>,
}

/// The screen as it opens: every session and every state showing, zoomed out far enough to see the
/// work whole and the tasks drawer shut. Written out rather than derived, because the derived zero
/// of a zoom is a graph nobody can see.
impl Default for TeamsView {
    fn default() -> Self {
        Self {
            layout: Layout::default(),
            algo: Algo::default(),
            sessions: Vec::new(),
            buckets: Bucket::all().to_vec(),
            hide_done: false,
            mission_filter: Vec::new(),
            zoom: 0.8,
            // Nothing is selected until there is something to select; the window points the
            // selection at the first agent the moment the work arrives.
            selection: None,
            tab: TeamsInspectorTab::Chat,
            tasks_open: false,
            rings: HashMap::new(),
            missions: HashMap::new(),
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
        self.layout = Layout::auto(
            &work.agents,
            &work.tasks,
            self.algo,
            &self.ring_counts(),
            (CARD_WIDTH, TEAMS_CARD_HEIGHT),
        );
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
        self.layout.place_new(
            &work.agents,
            &work.tasks,
            self.algo,
            &self.ring_counts(),
            (CARD_WIDTH, TEAMS_CARD_HEIGHT),
        );
    }

    /// How many delegates each card wears, which is all the arrangement needs of them.
    fn ring_counts(&self) -> Rings {
        self.rings
            .iter()
            .map(|(agent, subs)| (*agent, subs.len()))
            .collect()
    }

    /// Which workspace the screen is about — the selected card, or the card whose delegate is
    /// selected. **The one answer**, so the right dock's conversation and the link a card copies
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
            None => (at.0, at.1, at.0 + CARD_WIDTH, at.1 + TEAMS_CARD_HEIGHT),
        }
    }

    /// The cards the canvas fences on their own in their project's colour: every visible card no
    /// container encloses, in the projection's order.
    ///
    /// **Empty under [`TeamsSpan::Project`]**, where one project is the whole canvas and a colour
    /// per card would be one colour repeated — the project span's canvas is what it was before
    /// the fences existed, and the span is in here rather than only at the call site so that is a
    /// claim a test can make without a frame.
    ///
    /// **A card is in a container when a container is actually drawn round it**, which is more
    /// than `task.is_some()`: a task the projection does not carry has no box on the canvas, and a
    /// card serving it is as loose as a card serving nothing. Nothing is fenced twice, so a card
    /// inside a container is not here.
    pub fn fenced_alone(&self, work: &WorkProjection, span: TeamsSpan) -> Vec<AgentId> {
        if span != TeamsSpan::Window {
            return Vec::new();
        }
        work.agents
            .iter()
            .filter(|agent| self.visible(agent))
            // A card inside a mission fence is already fenced, whether or not a container reaches
            // it: a spawn-only member sits in its mission's fence with no task of its own, and a
            // loose fence round it would be the second outline `T-105` forbids.
            .filter(|agent| self.mission_of(agent.id).is_none())
            .filter(|agent| match agent.task {
                None => true,
                Some(task) => !work.tasks.iter().any(|record| record.id == task),
            })
            .map(|agent| agent.id)
            .collect()
    }

    /// The containers the canvas colours by project, each with the card whose project answers for
    /// it — a task is minted inside a project and every card serving it is that project's, so the
    /// first one drawn speaks for the box.
    ///
    /// **Empty under [`TeamsSpan::Project`]** for the same reason [`Self::fenced_alone`] is, and a
    /// container with no visible card is not here because it has no outline on the canvas either.
    pub fn fenced_tasks(&self, work: &WorkProjection, span: TeamsSpan) -> Vec<(TaskId, AgentId)> {
        if span != TeamsSpan::Window {
            return Vec::new();
        }
        work.tasks
            .iter()
            .filter_map(|task| {
                let owner = work
                    .agents
                    .iter()
                    .find(|agent| agent.task == Some(task.id) && self.visible(agent))?;
                Some((task.id, owner.id))
            })
            .collect()
    }

    /// The rectangle a lone card's own fence takes, `(x, y, w, h)`.
    ///
    /// **The inner distance, not the outer one.** A lone card can carry a ring of its own —
    /// [`Self::card_bounds`] already wraps that at [`RING_PAD`] — and this fence is the only
    /// outline drawn round it, so it sits [`RING_PAD`] off the ring rather than adding a second,
    /// looser margin on top of one already there. No label, so no [`GROUP_LABEL`] strip at the
    /// top.
    pub fn solo_bounds(&self, agent: AgentId, at: (f32, f32)) -> (f32, f32, f32, f32) {
        let (x0, y0, x1, y1) = self.card_bounds(agent, at);
        (
            x0 - RING_PAD,
            y0 - RING_PAD,
            (x1 - x0) + RING_PAD * 2.0,
            (y1 - y0) + RING_PAD * 2.0,
        )
    }

    // ── missions ────────────────────────────────────────────────────

    /// Which mission the handle last clicked belongs to, where one is what is selected.
    pub fn mission_in_focus(&self) -> Option<TaskId> {
        match &self.selection {
            Some(TeamsSelection::Mission(id)) => Some(*id),
            _ => None,
        }
    }

    /// Everybody on one mission, as the window last wrote it down. Empty for a mission nobody is
    /// on, and for a task that is not a mission at all.
    pub fn mission_members(&self, mission: TaskId) -> &[AgentId] {
        self.missions
            .get(&mission)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    /// Which mission an agent is on, where it is on one.
    pub fn mission_of(&self, agent: AgentId) -> Option<TaskId> {
        self.missions
            .iter()
            .find(|(_, members)| members.contains(&agent))
            .map(|(mission, _)| *mission)
    }

    /// The task containers a mission fence is the union of: its anchor's own, and its children's
    /// (M15). The anchor is first, which is what makes its container **merge into** the fence
    /// rather than draw inside it — the canvas skips the box for a task in this list's head.
    pub fn mission_tasks(&self, work: &WorkProjection, mission: TaskId) -> Vec<TaskId> {
        std::iter::once(mission)
            .chain(work.children_of(mission).map(|task| task.id))
            .collect()
    }

    /// The fence round a mission: `(x, y, w, h)` at 100% zoom.
    ///
    /// **Derived every frame, never stored** (M15, `D41`). It is the union of the containers
    /// [`Self::mission_tasks`] names and of the cards on the mission by spawn alone — an agent
    /// with no task of its own sits inside the fence beside its spawner — padded by [`GROUP_PAD`]
    /// and given [`MISSION_BAND`] at the top for the handle and the coordinator.
    ///
    /// A mission with nothing visible on it answers [`MISSION_MIN`] at its anchor's own layout
    /// origin, so its handle is still there to be grabbed and to be dropped onto.
    pub fn mission_bounds(&self, work: &WorkProjection, mission: TaskId) -> (f32, f32, f32, f32) {
        self.mission_bounds_excluding(work, mission, None)
    }

    fn mission_bounds_excluding(
        &self,
        work: &WorkProjection,
        mission: TaskId,
        skip: Option<AgentId>,
    ) -> (f32, f32, f32, f32) {
        let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        let mut any = false;
        let mut grow = |(ax0, ay0, ax1, ay1): (f32, f32, f32, f32)| {
            any = true;
            x0 = x0.min(ax0);
            y0 = y0.min(ay0);
            x1 = x1.max(ax1);
            y1 = y1.max(ay1);
        };

        for task in self.mission_tasks(work, mission) {
            if let Some((x, y, w, h)) = self.bounds_excluding(work, task, skip) {
                grow((x, y, x + w, y + h));
            }
        }
        // The spawn-only members: on the mission by M11's second half, holding no task of their
        // own, and so reached by no container above.
        for id in self.mission_members(mission) {
            if Some(*id) == skip {
                continue;
            }
            let Some(agent) = work.agent(*id) else {
                continue;
            };
            if agent.task.is_some() || !self.visible(agent) {
                continue;
            }
            grow(self.card_bounds(*id, self.layout.at(agent)));
        }

        if !any {
            let (x, y) = self.layout.task_origin(mission);
            return (
                x - GROUP_PAD,
                y - GROUP_PAD - MISSION_BAND,
                MISSION_MIN.0,
                MISSION_MIN.1,
            );
        }
        (
            x0 - GROUP_PAD,
            y0 - GROUP_PAD - MISSION_BAND,
            (x1 - x0) + GROUP_PAD * 2.0,
            (y1 - y0) + GROUP_PAD * 2.0 + MISSION_BAND,
        )
    }

    /// Which mission fence the carried card's centre is inside, where no container claimed it
    /// first — which is what makes dropping a card on a mission's open ground attach it to the
    /// mission itself (M15). The tightest fence wins, so a reading is the same every frame.
    fn mission_at(
        &self,
        work: &WorkProjection,
        carried: AgentId,
        at: (f32, f32),
    ) -> Option<TaskId> {
        let centre = (at.0 + CARD_WIDTH / 2.0, at.1 + TEAMS_CARD_HEIGHT / 2.0);
        self.missions
            .keys()
            .filter(|mission| work.task(**mission).is_some())
            .filter_map(|mission| {
                let (x, y, w, h) = self.mission_bounds_excluding(work, *mission, Some(carried));
                (centre.0 >= x && centre.0 <= x + w && centre.1 >= y && centre.1 <= y + h)
                    .then_some((*mission, w * h))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(mission, _)| mission)
    }

    /// Which session the screen is *about*: the one selected, or the one the selected agent runs
    /// in, falling back to the first so the drawer always has something to report. What the canvas
    /// *draws* is `session`, which is a separate question.
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
            .filter(|tab| delegate_status(tab).doing != Doing::Done)
            .collect()
    }

    /// Whether a card is drawn at all, given the three filters. An empty list is no filter, for
    /// each of them: `sessions` empty is every session, `mission_filter` empty is every mission
    /// *and* everything on none.
    pub fn visible(&self, agent: &WorkAgent) -> bool {
        self.showing(agent.activity.bucket())
            && (self.sessions.is_empty() || self.sessions.contains(&agent.session))
            && self.in_missions(agent.id)
    }

    /// Whether the mission filter lets this card through. Off — the empty list — is everything;
    /// on, it is the cards on one of the missions named, and a card on no mission at all is not
    /// one of them.
    fn in_missions(&self, agent: AgentId) -> bool {
        self.mission_filter.is_empty()
            || self
                .mission_of(agent)
                .is_some_and(|mission| self.mission_filter.contains(&mission))
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

    /// Turn one session's tick on or off. Any of them may be the last: with none ticked the row is
    /// not filtering, which is the way back from having turned them all on. It leaves the
    /// selection alone: narrowing which is shown is not "stop looking at this".
    pub fn toggle_session(&mut self, session: SessionId) {
        if let Some(ix) = self.sessions.iter().position(|held| *held == session) {
            self.sessions.remove(ix);
        } else {
            self.sessions.push(session);
        }
    }

    /// Put every filter back, which is the toolbar's one control for "show everything".
    /// Turn one mission's tick on or off, on [`Self::toggle_session`]'s rule exactly: any of them
    /// may be the last, and none ticked is not filtering.
    pub fn toggle_mission(&mut self, mission: TaskId) {
        if let Some(ix) = self.mission_filter.iter().position(|held| *held == mission) {
            self.mission_filter.remove(ix);
        } else {
            self.mission_filter.push(mission);
        }
    }

    pub fn clear_filters(&mut self) {
        self.sessions.clear();
        self.buckets = Bucket::all().to_vec();
        self.hide_done = false;
        self.mission_filter.clear();
    }

    /// Whether anything is being hidden, so the control that clears the filters can say whether it
    /// has anything to do.
    pub fn filtered(&self) -> bool {
        !self.sessions.is_empty()
            || self.buckets.len() < Bucket::all().len()
            || self.hide_done
            || !self.mission_filter.is_empty()
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
                // A container first, the mission fence round it second: a card let go over a
                // child's box joins that child, and one let go on the mission's own open ground
                // joins the mission itself (M15) — which is its anchor task, so both answers are
                // the same `AssignAgent`.
                let over = self
                    .task_at(work, id, at)
                    .or_else(|| self.mission_at(work, id, at))
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
            // A mission's handle moves the whole fence, which is every container it is the union
            // of and every card on it by spawn alone. Containers come along by their origin, the
            // way a task container's own drag already moves them; a member with no task has an
            // absolute position instead, so it is moved by the same difference.
            TeamsHeld::Mission(id) => {
                let (x, y, _, _) = self.mission_bounds(work, id);
                let (dx, dy) = (at.0 - x, at.1 - y);
                for task in self.mission_tasks(work, id) {
                    let origin = self.layout.task_origin(task);
                    self.layout.place_task(task, (origin.0 + dx, origin.1 + dy));
                }
                let loose: Vec<AgentId> = self
                    .mission_members(id)
                    .iter()
                    .filter(|id| work.agent(**id).is_some_and(|agent| agent.task.is_none()))
                    .copied()
                    .collect();
                for agent in loose {
                    let offset = self.layout.offset(agent);
                    self.layout
                        .place_agent(agent, (offset.0 + dx, offset.1 + dy));
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
        let centre = (at.0 + CARD_WIDTH / 2.0, at.1 + TEAMS_CARD_HEIGHT / 2.0);
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
        // The inner distance, not the outer one: `card_bounds` already wraps a ringed member at
        // `RING_PAD`, and this is the *only* fence drawn round the task — so it sits `RING_PAD`
        // off the tightest thing inside it rather than doubling the margin with `GROUP_PAD` on
        // top of a ring's own. One fence, one distance (`T-105`).
        Some((
            x0 - RING_PAD,
            y0 - RING_PAD - GROUP_LABEL,
            (x1 - x0) + RING_PAD * 2.0,
            (y1 - y0) + RING_PAD * 2.0 + GROUP_LABEL,
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
    use ubiq_proto::conversation::ToolStatus;
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
            doing: Doing::Unknown,
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

    // ── the mission fence ───────────────────────────────────────────

    fn a_task(parent: Option<TaskId>) -> TaskRecord {
        let mut task = TaskRecord::new("work".to_string(), None, chrono::Utc::now());
        task.parent = parent;
        task
    }

    /// A mission, its anchor and one child, with one card on each — the shape M15's sketch draws.
    fn a_mission() -> (TeamsView, WorkProjection, TaskId) {
        let session = SessionId::generate();
        let anchor = a_task(None);
        let child = a_task(Some(anchor.id));
        let mut lead = an_agent(session, None);
        lead.task = Some(anchor.id);
        let mut worker = an_agent(session, None);
        worker.task = Some(child.id);

        let work = WorkProjection {
            sessions: Vec::new(),
            agents: vec![lead.clone(), worker.clone()],
            tasks: vec![anchor.clone(), child.clone()],
            loaded: true,
        };
        let mut view = TeamsView::default();
        view.missions.insert(anchor.id, vec![lead.id, worker.id]);
        view.relayout(&work);
        (view, work, anchor.id)
    }

    /// **The fence is the union of the containers, not a rectangle anybody wrote down.** It holds
    /// the mission's own container and its children's, and it still holds them after one of them is
    /// dragged away — which is the whole of the derived rule.
    #[test]
    fn a_mission_fence_is_the_union_of_its_containers() {
        let (mut view, work, mission) = a_mission();
        let child = work.tasks.iter().find(|t| t.parent.is_some()).unwrap().id;

        let holds = |view: &TeamsView, task: TaskId| {
            let (fx, fy, fw, fh) = view.mission_bounds(&work, mission);
            let (x, y, w, h) = view.bounds_of(&work, task).expect("a drawn container");
            x >= fx && y >= fy && x + w <= fx + fw && y + h <= fy + fh
        };

        assert!(holds(&view, mission), "the anchor's container is inside");
        assert!(holds(&view, child), "the child's container is inside");

        // Scattered far apart is still one fence: it is the bounding box of wherever the members
        // ended up, so it grows to reach them rather than leaving one outside.
        let snug = view.mission_bounds(&work, mission);
        view.layout.place_task(child, (4_000.0, 3_000.0));
        let wide = view.mission_bounds(&work, mission);
        assert!(
            wide.2 > snug.2 && wide.3 > snug.3,
            "the fence grew round the moved container: {wide:?} from {snug:?}"
        );
        assert!(holds(&view, child), "and still holds it");
    }

    /// **A mission with no agent still draws its handle** (M15), so it can be moved and dropped
    /// onto — an empty union would otherwise be no fence at all.
    #[test]
    fn a_mission_with_nobody_on_it_keeps_a_minimum_fence() {
        let anchor = a_task(None);
        let work = WorkProjection {
            sessions: Vec::new(),
            agents: Vec::new(),
            tasks: vec![anchor.clone()],
            loaded: true,
        };
        let mut view = TeamsView::default();
        view.missions.insert(anchor.id, Vec::new());

        let (_, _, w, h) = view.mission_bounds(&work, anchor.id);
        assert_eq!((w, h), MISSION_MIN);
    }

    /// The handle translates everything inside: every container comes along by its origin, which is
    /// the same move a task container's own ground drag already makes.
    #[test]
    fn dragging_the_handle_moves_every_card_inside() {
        let (mut view, work, mission) = a_mission();
        let before: Vec<(f32, f32)> = work.agents.iter().map(|a| view.at(a)).collect();
        let (x, y, _, _) = view.mission_bounds(&work, mission);

        view.start_carry(TeamsHeld::Mission(mission), (0.0, 0.0));
        view.carry_to(&work, (x + 300.0, y + 200.0), None, Instant::now(), None);

        for (agent, was) in work.agents.iter().zip(before) {
            let now = view.at(agent);
            assert!(
                (now.0 - was.0 - 300.0).abs() < 0.5 && (now.1 - was.1 - 200.0).abs() < 0.5,
                "every card moved by the one translation: {now:?} from {was:?}"
            );
        }
    }

    /// A card let go on the mission's open ground — inside the fence, in no child's container —
    /// joins the mission itself, which is its anchor task (M15). Out on open ground is still a
    /// no-op.
    #[test]
    fn a_card_dropped_in_the_fence_attaches_to_the_mission() {
        let (mut view, work, mission) = a_mission();
        let loose = work
            .agents
            .iter()
            .find(|a| a.task != Some(mission))
            .unwrap()
            .id;
        let (x, y, w, _) = view.mission_bounds(&work, mission);

        // The band along the top of the fence is inside it and inside no container.
        view.start_carry(TeamsHeld::Agent(loose), (0.0, 0.0));
        view.carry_to(
            &work,
            (
                x + w / 2.0 - CARD_WIDTH / 2.0,
                y - TEAMS_CARD_HEIGHT / 2.0 + MISSION_BAND / 2.0,
            ),
            None,
            Instant::now(),
            None,
        );
        assert_eq!(
            view.carry.as_ref().and_then(|carry| carry.over),
            Some(mission),
            "the drop names the anchor task, which is what attaching to the mission is"
        );

        view.carry_to(&work, (9_000.0, 9_000.0), None, Instant::now(), None);
        assert_eq!(
            view.carry.as_ref().and_then(|carry| carry.over),
            None,
            "open ground stays a no-op"
        );
    }

    /// The Missions filter narrows the canvas the way the session row does: the cards on the
    /// missions named and nothing else, and "show everything" puts it back.
    #[test]
    fn the_missions_filter_narrows_the_canvas() {
        let (mut view, work, mission) = a_mission();
        let session = SessionId::generate();
        let outsider = an_agent(session, None);

        assert!(view.visible(&outsider), "no filter is everything");

        view.toggle_mission(mission);
        assert!(view.filtered());
        assert!(
            work.agents.iter().all(|agent| view.visible(agent)),
            "the mission's own cards stay"
        );
        assert!(
            !view.visible(&outsider),
            "a card on no mission is narrowed away"
        );

        view.clear_filters();
        assert!(view.visible(&outsider));
        assert!(!view.filtered());
    }
}
