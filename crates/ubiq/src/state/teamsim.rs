//! The teams-graph testbed: a whole graph in one JSON file, and the geometry the arrangements draw
//! it as.
//!
//! **The point is that nothing here is a second layout engine.** A scenario is a description —
//! sessions, the tasks in them, the agents on those tasks, the delegates inside an agent's fence —
//! and it is projected into the very records the product's graph is drawn from: [`WorkAgent`],
//! [`TaskRecord`] and the [`Rings`] count per card. [`Layout`] then places them, through
//! [`Layout::auto`] for a tidy and [`Layout::place_new`] for an arrival, which is the same pair the
//! Teams screen calls. A page that reimplemented the packing would be testing its own copy.
//!
//! **A string id in the file, a ULID in the records.** The file has to be readable and editable by
//! hand, so it names things `a1` and `t2`; the layout keys on [`AgentId`] and [`TaskId`]. One map
//! per kind holds the correspondence for the life of the loaded scenario, so the same `a1` is the
//! same generated ULID across every re-projection — otherwise a delegate added to a card would
//! reset every position in the graph.
//!
//! **Hand positions round-trip.** A scenario may carry the places a human dragged things to, and
//! loading one restores them rather than tidying them away: the layout is seeded from the map and
//! `place_new` fills in only what the map does not name. `Tidy` is the one thing that throws them
//! away, and an arrival never does — which is what leaves `Algo::Adaptive` something to work from.
//!
//! **The geometry is computed here, not while drawing.** [`Sim::drawing`] answers every rectangle
//! and every connector endpoint the page draws, so `ui/sink/teamsim.rs` positions blocks and never
//! measures one.

use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};

use ubiq_proto::ids::{SessionId, TaskId};
use ubiq_proto::work::{Activity, AgentId, Priority, Shape, Status, TaskRecord, WorkAgent};

use super::layout::{
    Algo, CARD_HEIGHT, CARD_WIDTH, GROUP_LABEL, GROUP_PAD, Layout, Rings, SUB_BOX, fence, sub_slot,
};

/// The one format string this reader knows. A file that says anything else is refused rather than
/// read hopefully: a scenario is geometry, and a field that moved would be read as a graph nobody
/// wrote.
pub const FORMAT: &str = "ubiq.teamsim/1";

/// What a session's outline leaves round the containers in it, and the room its label takes above
/// them. The outer of the two fences, so it clears the inner one on both counts.
pub const SESSION_PAD: f32 = 20.0;
pub const SESSION_LABEL: f32 = 26.0;

/// A rectangle at 100% zoom: `(x, y, w, h)`. The same tuple `ui::kit::blocks` takes, spelled here
/// so this module names nothing in `ui/`.
pub type Rect = (f32, f32, f32, f32);

/// The rectangle readability is judged against — one screen's worth.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Viewport {
    pub w: f32,
    pub h: f32,
}

impl Default for Viewport {
    fn default() -> Self {
        Viewport {
            w: 1_600.0,
            h: 1_000.0,
        }
    }
}

/// One session: the outermost fence.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SimSession {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub branch: String,
    #[serde(default)]
    pub worktree: bool,
}

/// One task: the inner fence, and the shape its cards are arranged in.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SimTask {
    pub id: String,
    #[serde(default)]
    pub session: String,
    #[serde(default)]
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shape: Option<SimShape>,
}

/// The three task shapes, spelled as the file spells them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SimShape {
    Direct,
    Chain,
    Coordinated,
}

impl SimShape {
    /// What the container's label says, in the same words [`Shape::label`] uses.
    pub fn label(self) -> &'static str {
        self.record().label()
    }

    fn record(self) -> Shape {
        match self {
            SimShape::Direct => Shape::Direct,
            SimShape::Chain => Shape::Chain,
            SimShape::Coordinated => Shape::Coordinated,
        }
    }
}

/// What a card is doing. Only the colour of the block depends on it, but a scenario that could not
/// say it would draw every card in one state.
///
/// Read through a string rather than by serde's own naming, and forgivingly: a scenario is written
/// by hand and by the Python tool beside it, and a word this reader does not place — `idle`, or a
/// state a later [`Activity`] adds — is a card drawn as working rather than a file refused. The
/// format string is what a refusal is for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "String", into = "String")]
pub enum SimActivity {
    Thinking,
    #[default]
    Writing,
    Tools,
    NeedsYou,
    Ended,
    Failed,
}

impl From<String> for SimActivity {
    fn from(word: String) -> Self {
        match word.as_str() {
            "thinking" => SimActivity::Thinking,
            "tools" => SimActivity::Tools,
            "needs-you" | "needs_you" | "waiting" => SimActivity::NeedsYou,
            "ended" | "done" | "idle" => SimActivity::Ended,
            "failed" | "error" => SimActivity::Failed,
            _ => SimActivity::Writing,
        }
    }
}

impl From<SimActivity> for String {
    fn from(activity: SimActivity) -> Self {
        activity.word().to_string()
    }
}

impl SimActivity {
    /// What the file spells it, which is also what the block's own line says.
    pub fn word(self) -> &'static str {
        match self {
            SimActivity::Thinking => "thinking",
            SimActivity::Writing => "writing",
            SimActivity::Tools => "tools",
            SimActivity::NeedsYou => "needs-you",
            SimActivity::Ended => "ended",
            SimActivity::Failed => "failed",
        }
    }

    /// The record's own activity — what a card's colour is read from, which is the one thing a
    /// screen needs of this beyond the word.
    pub fn activity(self) -> Activity {
        self.record()
    }

    fn record(self) -> Activity {
        match self {
            SimActivity::Thinking => Activity::Thinking,
            SimActivity::Writing => Activity::Writing,
            SimActivity::Tools => Activity::Tools,
            SimActivity::NeedsYou => Activity::NeedsYou,
            SimActivity::Ended => Activity::Ended,
            SimActivity::Failed => Activity::Failed,
        }
    }
}

/// One delegate. Not an agent record — a small block inside its parent's fence, whose count is what
/// the packers leave room for.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SimSub {
    pub id: String,
    #[serde(default)]
    pub name: String,
}

/// One agent: a card in a task's container, or — with no task — a card above them.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SimAgent {
    pub id: String,
    #[serde(default)]
    pub session: String,
    /// The task whose container it sits in. `None` is an agent nobody gave work to.
    #[serde(default)]
    pub task: Option<String>,
    /// Who spawned it — the only edge the arrangement obeys.
    #[serde(default)]
    pub parent: Option<String>,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub harness: String,
    #[serde(default)]
    pub activity: SimActivity,
    #[serde(default)]
    pub subagents: Vec<SimSub>,
}

/// What an extra link means. Drawn and counted, never obeyed — a crossing is a readability cost,
/// not a reason to move a card.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LinkKind {
    Spawn,
    #[default]
    Handoff,
    Watch,
}

impl LinkKind {
    pub fn label(self) -> &'static str {
        match self {
            LinkKind::Spawn => "spawn",
            LinkKind::Handoff => "handoff",
            LinkKind::Watch => "watch",
        }
    }
}

/// One extra edge between two agents.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SimLink {
    pub from: String,
    pub to: String,
    #[serde(default)]
    pub kind: LinkKind,
}

/// The places a human dragged things to, at 100% zoom, in the layout's own frames: a task's is its
/// container's origin on the canvas, an agent's its offset inside its task (or its position when it
/// has none), and a delegate's its offset inside the card that spawned it, keyed
/// `<agent id>/<subagent id>`.
///
/// Sorted maps rather than hashed ones, so writing the file twice writes the same bytes and a round
/// trip can be compared.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Positions {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub tasks: BTreeMap<String, [f32; 2]>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub agents: BTreeMap<String, [f32; 2]>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub subagents: BTreeMap<String, [f32; 2]>,
}

impl Positions {
    pub fn is_empty(&self) -> bool {
        self.tasks.is_empty() && self.agents.is_empty() && self.subagents.is_empty()
    }
}

/// A whole graph, as the file holds it — and, because the lists are in arrival order, as it grew.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Scenario {
    pub format: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub note: String,
    #[serde(default)]
    pub viewport: Viewport,
    #[serde(default)]
    pub sessions: Vec<SimSession>,
    #[serde(default)]
    pub tasks: Vec<SimTask>,
    #[serde(default)]
    pub agents: Vec<SimAgent>,
    #[serde(default)]
    pub links: Vec<SimLink>,
    #[serde(default, skip_serializing_if = "Positions::is_empty")]
    pub positions: Positions,
}

impl Default for Scenario {
    fn default() -> Self {
        Scenario {
            format: FORMAT.to_string(),
            name: "empty".to_string(),
            note: "nothing in it yet".to_string(),
            viewport: Viewport::default(),
            sessions: Vec::new(),
            tasks: Vec::new(),
            agents: Vec::new(),
            links: Vec::new(),
            positions: Positions::default(),
        }
    }
}

impl Scenario {
    /// Read one, refusing a format this reader does not know.
    pub fn parse(source: &str) -> Result<Scenario, String> {
        let scenario: Scenario =
            serde_json::from_str(source).map_err(|failed| failed.to_string())?;
        if scenario.format != FORMAT {
            return Err(format!(
                "unknown format {:?} \u{2014} this reader knows {FORMAT}",
                scenario.format
            ));
        }
        Ok(scenario)
    }

    /// Write one back, pretty, so the round trip through the clipboard is lossless and the file a
    /// reader gets is one they can edit.
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    fn task(&self, id: &str) -> Option<&SimTask> {
        self.tasks.iter().find(|task| task.id == id)
    }

    fn agent(&self, id: &str) -> Option<&SimAgent> {
        self.agents.iter().find(|agent| agent.id == id)
    }
}

/// The correspondence between the file's string ids and the ULIDs the layout keys on.
///
/// Held for the life of the loaded scenario. A map rebuilt on every projection would mint new ids
/// on every edit, and every hand position in the layout would be for a card that no longer exists.
#[derive(Default)]
struct Ids {
    sessions: HashMap<String, SessionId>,
    tasks: HashMap<String, TaskId>,
    agents: HashMap<String, AgentId>,
}

impl Ids {
    fn session(&mut self, key: &str) -> SessionId {
        *self
            .sessions
            .entry(key.to_string())
            .or_insert_with(SessionId::generate)
    }

    fn task(&mut self, key: &str) -> TaskId {
        *self
            .tasks
            .entry(key.to_string())
            .or_insert_with(TaskId::generate)
    }

    fn agent(&mut self, key: &str) -> AgentId {
        *self
            .agents
            .entry(key.to_string())
            .or_insert_with(AgentId::generate)
    }
}

/// What the arrangement is given: the very records the product's graph is laid out from.
#[derive(Default)]
pub struct Projection {
    /// One per [`Scenario::agents`], in the same order, so an index serves both.
    pub agents: Vec<WorkAgent>,
    pub tasks: Vec<TaskRecord>,
    pub rings: Rings,
}

/// What a block is, for the page that draws it: where it goes, and which scenario row it came from.
pub struct Card {
    /// The index into [`Scenario::agents`].
    pub agent: usize,
    pub rect: Rect,
}

/// One delegate block, by the card that spawned it and its place in that card's list.
pub struct Sub {
    pub agent: usize,
    pub sub: usize,
    pub rect: Rect,
}

/// A grouping's outline: which list it came from, its index in it, and the box it came out as.
pub struct Group {
    pub ix: usize,
    pub rect: Rect,
}

/// Which relation a connector draws, which is what picks its colour.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum EdgeKind {
    /// `agents[].parent` — the only edge the arrangement obeys.
    Parent,
    /// A card to a delegate inside its own fence.
    Delegate,
    /// One of [`Scenario::links`].
    Extra(LinkKind),
}

/// One connector, from a block's bottom edge to another's top.
pub struct Edge {
    pub from: (f32, f32),
    pub to: (f32, f32),
    pub kind: EdgeKind,
}

/// Every rectangle and every connector the page draws, measured once.
#[derive(Default)]
pub struct Drawing {
    pub sessions: Vec<Group>,
    pub tasks: Vec<Group>,
    /// A card's delegate ring, by the index of the card wearing it.
    pub rings: Vec<Group>,
    pub cards: Vec<Card>,
    pub subs: Vec<Sub>,
    pub edges: Vec<Edge>,
    /// The bottom-right corner of everything on it — the arrangement's own bounding box, with no
    /// margin in it, which is what the metrics are taken from.
    pub bbox: (f32, f32),
}

/// What the numbers under the toolbar say: how big the arrangement came out, and how far past one
/// screen it runs in each axis.
pub struct Metrics {
    pub w: f32,
    pub h: f32,
    pub aspect: f32,
    pub screens_x: f32,
    pub screens_y: f32,
}

/// How much the blocks that were already placed moved on the last arrival.
///
/// Zero is what an incremental arrangement is aiming for: a card that arrives should find a place
/// among what is on screen, not shuffle it. Seeing this move is the bug.
#[derive(Default, Clone, Copy)]
pub struct Drift {
    pub moved: usize,
    pub worst: f32,
}

/// The loaded scenario, the records it projects to, and where the arrangement put them.
pub struct Sim {
    pub scenario: Scenario,
    pub algo: Algo,
    ids: Ids,
    projection: Projection,
    layout: Layout,
    /// What the last add did to everything that was already placed.
    pub drift: Drift,
}

impl Default for Sim {
    fn default() -> Self {
        Sim::new(Scenario::default(), Algo::default())
    }
}

impl Sim {
    /// Load one: project it, seed the layout from whatever positions it carries, and let the
    /// arrangement place only what the map does not name.
    pub fn new(scenario: Scenario, algo: Algo) -> Self {
        let mut sim = Sim {
            scenario,
            algo,
            ids: Ids::default(),
            projection: Projection::default(),
            layout: Layout::default(),
            drift: Drift::default(),
        };
        sim.project();
        sim.seed();
        sim.fill();
        sim
    }

    pub fn projection(&self) -> &Projection {
        &self.projection
    }

    /// Throw every hand position away and arrange the whole graph from scratch. The one thing that
    /// does: an arrival places itself and leaves the rest alone.
    pub fn tidy(&mut self) {
        self.scenario.positions = Positions::default();
        self.project();
        self.layout = Layout::auto(
            &self.projection.agents,
            &self.projection.tasks,
            self.algo,
            &self.projection.rings,
        );
        self.drift = Drift::default();
    }

    /// Arrange with a different algorithm. A tidy, because that is what choosing one means.
    pub fn set_algo(&mut self, algo: Algo) {
        self.algo = algo;
        self.tidy();
    }

    /// Re-project and place whatever the layout has not seen, leaving everything placed where it
    /// is — the call an arriving card goes through in the product, and the one the incremental
    /// arrangement will live in.
    fn arrive(&mut self) {
        let before: Vec<(AgentId, (f32, f32))> = self
            .projection
            .agents
            .iter()
            .map(|agent| (agent.id, self.layout.at(agent)))
            .collect();
        self.project();
        self.fill();

        let mut drift = Drift::default();
        for (id, was) in before {
            let Some(agent) = self.projection.agents.iter().find(|a| a.id == id) else {
                continue;
            };
            let now = self.layout.at(agent);
            let moved = ((now.0 - was.0).powi(2) + (now.1 - was.1).powi(2)).sqrt();
            if moved > 0.5 {
                drift.moved += 1;
                drift.worst = drift.worst.max(moved);
            }
        }
        self.drift = drift;
    }

    /// Build the records the arrangement reads, keeping the id maps so the same string id stays the
    /// same ULID.
    fn project(&mut self) {
        let mut agents = Vec::with_capacity(self.scenario.agents.len());
        let mut rings = Rings::new();
        for row in &self.scenario.agents {
            let id = self.ids.agent(&row.id);
            let session = self.ids.session(&row.session);
            let task = row.task.as_deref().map(|key| self.ids.task(key));
            // A parent nobody declared, and one naming an agent the file does not hold, are the
            // same thing to the arrangement: a card with no edge above it.
            let parent = row
                .parent
                .as_deref()
                .filter(|key| self.scenario.agents.iter().any(|a| a.id == *key))
                .map(|key| self.ids.agent(key));
            if !row.subagents.is_empty() {
                rings.insert(id, row.subagents.len());
            }
            agents.push(WorkAgent {
                id,
                session,
                task,
                parent,
                name: row.name.clone(),
                summary: None,
                role: row.role.clone(),
                activity: row.activity.record(),
                note: String::new(),
                branch: String::new(),
                tokens: 0.0,
                harness: row.harness.clone(),
                account: String::new(),
                model: String::new(),
                context_pct: 0,
                persistent: false,
                accept_all: false,
                debug_dump: None,
                run_dir: None,
                config_dir: None,
                thread: Vec::new(),
            });
        }

        let now = chrono::Utc::now();
        let mut tasks = Vec::with_capacity(self.scenario.tasks.len());
        for row in &self.scenario.tasks {
            let id = self.ids.task(&row.id);
            let session = Some(self.ids.session(&row.session));
            tasks.push(TaskRecord {
                id,
                session,
                status: Status::InProgress,
                priority: Priority::Normal,
                shape: row.shape.map(SimShape::record),
                kind: None,
                level: None,
                parent: None,
                references: Vec::new(),
                attachments: Vec::new(),
                complexity: None,
                key: None,
                link: None,
                assigned_to: None,
                labels: Vec::new(),
                colour: None,
                title: row.title.clone(),
                description: String::new(),
                steps: Vec::new(),
                comments: Vec::new(),
                created_at: now,
                updated_at: now,
            });
        }

        self.projection = Projection {
            agents,
            tasks,
            rings,
        };
    }

    /// Put the file's hand positions into the layout, before anything is placed automatically.
    fn seed(&mut self) {
        for (key, at) in &self.scenario.positions.tasks {
            if let Some(id) = self.ids.tasks.get(key) {
                self.layout.place_task(*id, (at[0], at[1]));
            }
        }
        for (key, at) in &self.scenario.positions.agents {
            if let Some(id) = self.ids.agents.get(key) {
                self.layout.place_agent(*id, (at[0], at[1]));
            }
        }
        for (key, at) in &self.scenario.positions.subagents {
            let Some((agent, sub)) = key.split_once('/') else {
                continue;
            };
            if let Some(id) = self.ids.agents.get(agent) {
                self.layout.place_sub(*id, sub.to_string(), (at[0], at[1]));
            }
        }
    }

    /// Place what the layout has never seen, without moving what it has.
    fn fill(&mut self) {
        self.layout.place_new(
            &self.projection.agents,
            &self.projection.tasks,
            self.algo,
            &self.projection.rings,
        );
    }

    /// The scenario as it stands, with every position the layout is holding written into it — what
    /// `Copy JSON` puts on the clipboard, so a saved scenario reproduces the picture and not only
    /// the graph.
    pub fn snapshot(&self) -> Scenario {
        let mut scenario = self.scenario.clone();
        let mut positions = Positions::default();
        for row in &self.scenario.tasks {
            if let Some(id) = self.ids.tasks.get(&row.id) {
                let at = self.layout.task_origin(*id);
                positions.tasks.insert(row.id.clone(), [at.0, at.1]);
            }
        }
        for row in &self.scenario.agents {
            let Some(id) = self.ids.agents.get(&row.id) else {
                continue;
            };
            let at = self.layout.offset(*id);
            positions.agents.insert(row.id.clone(), [at.0, at.1]);
            for sub in &row.subagents {
                // Only a delegate that was actually moved: the rest are where `sub_slot` puts them,
                // and writing that out would freeze the one part of the fence the algorithms own.
                if let Some(at) = self.layout.sub_offset(*id, &sub.id) {
                    positions
                        .subagents
                        .insert(format!("{}/{}", row.id, sub.id), [at.0, at.1]);
                }
            }
        }
        scenario.positions = positions;
        scenario
    }

    pub fn to_json(&self) -> String {
        self.snapshot().to_json()
    }

    // ---- geometry -------------------------------------------------------------------------

    /// Where one card is drawn.
    pub fn card_at(&self, ix: usize) -> Option<(f32, f32)> {
        Some(self.layout.at(self.projection.agents.get(ix)?))
    }

    /// Where one delegate is drawn: the offset a drag wrote for it, or the slot it starts in.
    fn sub_at(&self, ix: usize, sub: usize) -> Option<(f32, f32)> {
        let row = self.scenario.agents.get(ix)?;
        let id = self.projection.agents.get(ix)?.id;
        let key = &row.subagents.get(sub)?.id;
        let at = self.card_at(ix)?;
        let offset = self
            .layout
            .sub_offset(id, key)
            .unwrap_or_else(|| self.slot(ix, sub));
        Some((at.0 + offset.0, at.1 + offset.1))
    }

    /// The slot a delegate starts in, in the shape the chosen arrangement asks for — the same
    /// reading the teams canvas makes, so the sink shows what the application draws.
    fn slot(&self, ix: usize, sub: usize) -> (f32, f32) {
        let count = self
            .scenario
            .agents
            .get(ix)
            .map(|row| row.subagents.len())
            .unwrap_or(0);
        let slots = (self.algo.ring())(count);
        slots.get(sub).copied().unwrap_or_else(|| sub_slot(sub))
    }

    /// What one delegate box measures. [`SUB_BOX`] under every arrangement, because no ring shape
    /// scales a block.
    pub fn sub_box(&self) -> (f32, f32) {
        SUB_BOX
    }

    /// Every delegate of one card, where each is drawn.
    fn subs_at(&self, ix: usize) -> Vec<(f32, f32)> {
        let count = self
            .scenario
            .agents
            .get(ix)
            .map(|row| row.subagents.len())
            .unwrap_or(0);
        (0..count).filter_map(|sub| self.sub_at(ix, sub)).collect()
    }

    /// What a card takes on the canvas, its ring included: `(x0, y0, x1, y1)`.
    fn card_bounds(&self, ix: usize) -> Option<(f32, f32, f32, f32)> {
        let at = self.card_at(ix)?;
        Some(match fence(at, &self.subs_at(ix)) {
            Some((x, y, w, h)) => (x, y, x + w, y + h),
            None => (at.0, at.1, at.0 + CARD_WIDTH, at.1 + CARD_HEIGHT),
        })
    }

    /// The container a task is drawn in: the box round its cards, with room for the label. `None`
    /// is a task with nothing in it, which is drawn as no box at all.
    pub fn task_fence(&self, task: &str) -> Option<Rect> {
        let members: Vec<usize> = self
            .scenario
            .agents
            .iter()
            .enumerate()
            .filter(|(_, row)| row.task.as_deref() == Some(task))
            .map(|(ix, _)| ix)
            .collect();
        let hull = hull(members.iter().filter_map(|ix| self.card_bounds(*ix)))?;
        Some((
            hull.0 - GROUP_PAD,
            hull.1 - GROUP_PAD - GROUP_LABEL,
            (hull.2 - hull.0) + GROUP_PAD * 2.0,
            (hull.3 - hull.1) + GROUP_PAD * 2.0 + GROUP_LABEL,
        ))
    }

    /// A session's outline: the box round its containers and the cards nobody gave work to.
    pub fn session_fence(&self, session: &str) -> Option<Rect> {
        let tasks = self
            .scenario
            .tasks
            .iter()
            .filter(|task| task.session == session)
            .filter_map(|task| self.task_fence(&task.id))
            .map(|(x, y, w, h)| (x, y, x + w, y + h));
        let loose = self
            .scenario
            .agents
            .iter()
            .enumerate()
            .filter(|(_, row)| row.session == session && row.task.is_none())
            .filter_map(|(ix, _)| self.card_bounds(ix));
        let hull = hull(tasks.chain(loose))?;
        Some((
            hull.0 - SESSION_PAD,
            hull.1 - SESSION_PAD - SESSION_LABEL,
            (hull.2 - hull.0) + SESSION_PAD * 2.0,
            (hull.3 - hull.1) + SESSION_PAD * 2.0 + SESSION_LABEL,
        ))
    }

    /// Every rectangle and every connector the page draws.
    pub fn drawing(&self) -> Drawing {
        let mut drawing = Drawing::default();

        for (ix, session) in self.scenario.sessions.iter().enumerate() {
            if let Some(rect) = self.session_fence(&session.id) {
                drawing.sessions.push(Group { ix, rect });
            }
        }
        for (ix, task) in self.scenario.tasks.iter().enumerate() {
            if let Some(rect) = self.task_fence(&task.id) {
                drawing.tasks.push(Group { ix, rect });
            }
        }

        for (ix, row) in self.scenario.agents.iter().enumerate() {
            let Some(at) = self.card_at(ix) else { continue };
            drawing.cards.push(Card {
                agent: ix,
                rect: (at.0, at.1, CARD_WIDTH, CARD_HEIGHT),
            });

            let spots = self.subs_at(ix);
            if let Some(rect) = fence(at, &spots) {
                drawing.rings.push(Group { ix, rect });
            }
            for (sub, spot) in spots.iter().enumerate() {
                drawing.subs.push(Sub {
                    agent: ix,
                    sub,
                    rect: (spot.0, spot.1, self.sub_box().0, self.sub_box().1),
                });
                drawing.edges.push(Edge {
                    from: (at.0 + CARD_WIDTH / 2.0, at.1 + CARD_HEIGHT),
                    to: (spot.0 + self.sub_box().0 / 2.0, spot.1),
                    kind: EdgeKind::Delegate,
                });
            }

            // The spawn edge, from the parent's bottom to this card's top.
            if let Some(parent) = row.parent.as_deref()
                && let Some(from) = self.index_of(parent).and_then(|up| self.card_at(up))
            {
                drawing.edges.push(Edge {
                    from: (from.0 + CARD_WIDTH / 2.0, from.1 + CARD_HEIGHT),
                    to: (at.0 + CARD_WIDTH / 2.0, at.1),
                    kind: EdgeKind::Parent,
                });
            }
        }

        for link in &self.scenario.links {
            // A `spawn` link that only repeats a parent says nothing the parent edge did not.
            if link.kind == LinkKind::Spawn
                && self
                    .scenario
                    .agent(&link.to)
                    .and_then(|row| row.parent.clone())
                    .as_deref()
                    == Some(link.from.as_str())
            {
                continue;
            }
            let (Some(from), Some(to)) = (
                self.index_of(&link.from).and_then(|ix| self.card_at(ix)),
                self.index_of(&link.to).and_then(|ix| self.card_at(ix)),
            ) else {
                continue;
            };
            drawing.edges.push(Edge {
                from: (from.0 + CARD_WIDTH / 2.0, from.1 + CARD_HEIGHT),
                to: (to.0 + CARD_WIDTH / 2.0, to.1),
                kind: EdgeKind::Extra(link.kind),
            });
        }

        let mut bbox = (0.0f32, 0.0f32);
        let rects = drawing
            .sessions
            .iter()
            .chain(&drawing.tasks)
            .chain(&drawing.rings)
            .map(|group| group.rect)
            .chain(drawing.cards.iter().map(|card| card.rect))
            .chain(drawing.subs.iter().map(|sub| sub.rect));
        for (x, y, w, h) in rects {
            bbox = (bbox.0.max(x + w), bbox.1.max(y + h));
        }
        drawing.bbox = bbox;
        drawing
    }

    /// How big the arrangement came out, and how far past one screen it runs in each axis — the
    /// numbers that say whether it is readable at all.
    pub fn metrics(&self, bbox: (f32, f32)) -> Metrics {
        let (w, h) = (bbox.0.max(1.0), bbox.1.max(1.0));
        Metrics {
            w,
            h,
            aspect: w / h,
            screens_x: w / self.scenario.viewport.w.max(1.0),
            screens_y: h / self.scenario.viewport.h.max(1.0),
        }
    }

    fn index_of(&self, agent: &str) -> Option<usize> {
        self.scenario.agents.iter().position(|row| row.id == agent)
    }

    // ---- moving things by hand ------------------------------------------------------------

    /// Put a container's origin somewhere. Every card in it follows, because a card's position is
    /// an offset inside it.
    pub fn move_task(&mut self, task: &str, at: (f32, f32)) {
        if let Some(id) = self.ids.tasks.get(task) {
            self.layout.place_task(*id, at);
        }
    }

    /// Put a card somewhere on the canvas. What is written is its offset inside its container, or
    /// its position when it has none.
    pub fn move_agent(&mut self, agent: &str, at: (f32, f32)) {
        let Some(ix) = self.index_of(agent) else {
            return;
        };
        let Some(record) = self.projection.agents.get(ix) else {
            return;
        };
        let origin = record
            .task
            .map(|task| self.layout.task_origin(task))
            .unwrap_or((0.0, 0.0));
        let id = record.id;
        self.layout
            .place_agent(id, (at.0 - origin.0, at.1 - origin.1));
    }

    /// Put a delegate somewhere. What is written is its offset inside the card that spawned it, so
    /// moving the card takes the delegate with it and the fence grows round wherever it landed.
    pub fn move_sub(&mut self, agent: &str, sub: &str, at: (f32, f32)) {
        let Some(ix) = self.index_of(agent) else {
            return;
        };
        let Some(card) = self.card_at(ix) else { return };
        let Some(id) = self.projection.agents.get(ix).map(|a| a.id) else {
            return;
        };
        self.layout
            .place_sub(id, sub.to_string(), (at.0 - card.0, at.1 - card.1));
    }

    // ---- editing the scenario -------------------------------------------------------------

    /// Give a card one more delegate. Appended, because the list is the order they arrived in.
    pub fn add_subagent(&mut self, agent: &str) {
        let taken: Vec<String> = self
            .scenario
            .agents
            .iter()
            .flat_map(|row| row.subagents.iter().map(|sub| sub.id.clone()))
            .collect();
        let id = fresh("d", &taken);
        let Some(row) = self.scenario.agents.iter_mut().find(|row| row.id == agent) else {
            return;
        };
        let name = format!("delegate {}", row.subagents.len() + 1);
        row.subagents.push(SimSub { id, name });
        self.arrive();
    }

    pub fn remove_subagent(&mut self, agent: &str, sub: &str) {
        if let Some(row) = self.scenario.agents.iter_mut().find(|row| row.id == agent) {
            row.subagents.retain(|held| held.id != sub);
        }
        self.scenario
            .positions
            .subagents
            .remove(&format!("{agent}/{sub}"));
        self.arrive();
    }

    /// One more card in a container, parented to whoever is already leading it — a task with a card
    /// in it gains a worker, not a second root, which is what makes the shapes grow the way a real
    /// one does.
    pub fn add_agent(&mut self, task: &str) {
        let Some(row) = self.scenario.task(task) else {
            return;
        };
        let session = row.session.clone();
        let lead = self
            .scenario
            .agents
            .iter()
            .find(|agent| agent.task.as_deref() == Some(task))
            .map(|agent| agent.id.clone());
        let taken: Vec<String> = self
            .scenario
            .agents
            .iter()
            .map(|agent| agent.id.clone())
            .collect();
        let id = fresh("a", &taken);
        let name = format!("agent {}", self.scenario.agents.len() + 1);
        self.scenario.agents.push(SimAgent {
            id,
            session,
            task: Some(task.to_string()),
            parent: lead,
            name,
            role: "Implementer".to_string(),
            harness: "claude-code".to_string(),
            activity: SimActivity::Writing,
            subagents: Vec::new(),
        });
        self.arrive();
    }

    /// One more container in a session, and one card in it — an empty container is drawn nowhere,
    /// so adding one on its own would look like the button did nothing.
    pub fn add_task(&mut self, session: &str) {
        let taken: Vec<String> = self
            .scenario
            .tasks
            .iter()
            .map(|task| task.id.clone())
            .collect();
        let id = fresh("t", &taken);
        let title = format!("task {}", self.scenario.tasks.len() + 1);
        self.scenario.tasks.push(SimTask {
            id: id.clone(),
            session: session.to_string(),
            title,
            shape: Some(SimShape::Direct),
        });
        self.add_agent(&id);
    }

    /// Drop a card, and with it every edge that named it. The delegates go with it, because they
    /// were never records of their own.
    pub fn remove_agent(&mut self, agent: &str) {
        self.scenario.agents.retain(|row| row.id != agent);
        for row in &mut self.scenario.agents {
            if row.parent.as_deref() == Some(agent) {
                row.parent = None;
            }
        }
        self.scenario
            .links
            .retain(|link| link.from != agent && link.to != agent);
        self.scenario.positions.agents.remove(agent);
        self.arrive();
    }

    /// Join two cards. A link to itself and a link that already exists are both no-ops rather than
    /// a second line over the first.
    pub fn link(&mut self, from: &str, to: &str, kind: LinkKind) {
        if from == to
            || self.scenario.agent(from).is_none()
            || self.scenario.agent(to).is_none()
            || self
                .scenario
                .links
                .iter()
                .any(|link| link.from == from && link.to == to)
        {
            return;
        }
        self.scenario.links.push(SimLink {
            from: from.to_string(),
            to: to.to_string(),
            kind,
        });
    }

    pub fn unlink(&mut self, index: usize) {
        if index < self.scenario.links.len() {
            self.scenario.links.remove(index);
        }
    }
}

/// The box round a set of `(x0, y0, x1, y1)` corners, or `None` for none of them.
fn hull(rects: impl Iterator<Item = (f32, f32, f32, f32)>) -> Option<(f32, f32, f32, f32)> {
    let mut hull: Option<(f32, f32, f32, f32)> = None;
    for (x0, y0, x1, y1) in rects {
        hull = Some(match hull {
            None => (x0, y0, x1, y1),
            Some(held) => (
                held.0.min(x0),
                held.1.min(y0),
                held.2.max(x1),
                held.3.max(y1),
            ),
        });
    }
    hull
}

/// The next id of a kind nobody has used: `a1`, `a2`, and so on. Counted rather than generated,
/// because the file is meant to be read and edited by hand.
fn fresh(prefix: &str, taken: &[String]) -> String {
    (1..)
        .map(|n| format!("{prefix}{n}"))
        .find(|candidate| !taken.contains(candidate))
        .unwrap_or_else(|| format!("{prefix}1"))
}

/// One preset: the file's stem, which is also its label, and the JSON embedded from it.
pub struct Preset {
    pub name: &'static str,
    pub source: &'static str,
}

/// Every scenario in `_tools/teamsim/scenarios/`, embedded. The Python tool beside them reads the
/// same files, so a scenario added there is a preset in both halves of the spike.
pub const PRESETS: [Preset; 7] = [
    Preset {
        name: "solo",
        source: include_str!("../../../../_tools/teamsim/scenarios/solo.json"),
    },
    Preset {
        name: "chain",
        source: include_str!("../../../../_tools/teamsim/scenarios/chain.json"),
    },
    Preset {
        name: "wide-coordination",
        source: include_str!("../../../../_tools/teamsim/scenarios/wide-coordination.json"),
    },
    Preset {
        name: "deep-delegation",
        source: include_str!("../../../../_tools/teamsim/scenarios/deep-delegation.json"),
    },
    Preset {
        name: "many-tasks",
        source: include_str!("../../../../_tools/teamsim/scenarios/many-tasks.json"),
    },
    Preset {
        name: "two-sessions",
        source: include_str!("../../../../_tools/teamsim/scenarios/two-sessions.json"),
    },
    Preset {
        name: "kitchen",
        source: include_str!("../../../../_tools/teamsim/scenarios/kitchen.json"),
    },
];

/// What is being carried on the testbed's canvas.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum TeamsimHeld {
    Task(String),
    Agent(String),
    Sub { agent: String, sub: String },
}

/// A carry in progress: what is held, and where inside it the pointer went down. Keeping the grab
/// point is what stops the block jumping under the cursor on the first move.
#[derive(Clone)]
pub struct TeamsimCarry {
    pub held: TeamsimHeld,
    pub grab: (f32, f32),
}

/// The kitchen sink's teamsim page: the loaded scenario and everything the page remembers about
/// looking at it.
///
/// The heavy part is [`Sim`], beside the scenario it was loaded from, so `state/sink.rs` holds one
/// field rather than a second layout engine.
pub struct TeamsimDemo {
    pub sim: Sim,
    /// Which of [`PRESETS`] the picker last chose.
    pub preset: usize,
    pub zoom: f32,
    /// The card a link is armed from, where one is. The next card clicked becomes the target, and
    /// clicking the armed card again disarms.
    pub arming: Option<String>,
    pub selected: Option<String>,
    /// What the last paste said, where it failed. Shown inline rather than swallowed.
    pub error: Option<String>,
    pub carry: Option<TeamsimCarry>,
}

impl Default for TeamsimDemo {
    fn default() -> Self {
        // The first preset, so the page draws something the moment it is opened. A file that does
        // not parse leaves an empty scenario and says so, rather than refusing to open the page.
        let (sim, error) = match Scenario::parse(PRESETS[0].source) {
            Ok(scenario) => (Sim::new(scenario, Algo::default()), None),
            Err(failed) => (Sim::default(), Some(failed)),
        };
        TeamsimDemo {
            sim,
            preset: 0,
            zoom: 1.0,
            arming: None,
            selected: None,
            error,
            carry: None,
        }
    }
}

impl TeamsimDemo {
    pub fn zoom_pct(&self) -> u32 {
        (self.zoom * 100.0).round() as u32
    }

    /// The same steps and the same range the Teams canvas zooms in, because the bench is judged at
    /// the zooms the screen it benches is read at.
    pub fn zoom_by(&mut self, delta: f32) {
        self.zoom =
            (self.zoom + delta).clamp(crate::state::teams::ZOOM_MIN, crate::state::teams::ZOOM_MAX);
    }

    /// Load one of the presets, hand positions and all.
    pub fn load(&mut self, preset: usize) {
        let Some(row) = PRESETS.get(preset) else {
            return;
        };
        self.preset = preset;
        match Scenario::parse(row.source) {
            Ok(scenario) => {
                self.sim = Sim::new(scenario, self.sim.algo);
                self.error = None;
            }
            Err(failed) => self.error = Some(failed),
        }
        self.arming = None;
        self.selected = None;
        self.carry = None;
    }

    /// Read a scenario from text — the clipboard's, for the page. A failure leaves what is on
    /// screen alone and shows the reason: a half-typed scenario is not a reason to lose the one
    /// being looked at.
    pub fn paste(&mut self, source: &str) {
        match Scenario::parse(source) {
            Ok(scenario) => {
                self.sim = Sim::new(scenario, self.sim.algo);
                self.error = None;
                self.arming = None;
                self.selected = None;
                self.carry = None;
            }
            Err(failed) => self.error = Some(failed),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn every_preset() -> Vec<Scenario> {
        PRESETS
            .iter()
            .map(|preset| {
                Scenario::parse(preset.source)
                    .unwrap_or_else(|failed| panic!("{}: {failed}", preset.name))
            })
            .collect()
    }

    /// A scenario written out and read back is the same scenario, positions included — which is
    /// what makes the clipboard a save file.
    #[test]
    fn a_scenario_round_trips() {
        for scenario in every_preset() {
            let json = scenario.to_json();
            let back = Scenario::parse(&json).expect("what we just wrote");
            assert_eq!(
                back.to_json(),
                json,
                "{} changed on a round trip",
                back.name
            );
        }

        // And the positions a drag wrote come back with it.
        let mut sim = Sim::new(every_preset().pop().expect("a preset"), Algo::Flow);
        let card = sim.scenario.agents[0].id.clone();
        sim.move_agent(&card, (640.0, 480.0));
        let json = sim.to_json();
        let back = Scenario::parse(&json).expect("readable");
        assert!(
            back.positions.agents.contains_key(&card),
            "the dragged card's offset was written: {:?}",
            back.positions
        );
        let restored = Sim::new(back, Algo::Flow);
        assert_eq!(
            restored.card_at(0),
            sim.card_at(0),
            "and loading it puts the card back where it was"
        );
    }

    /// A format this reader does not know is refused rather than read hopefully.
    #[test]
    fn an_unknown_format_is_refused() {
        let failed = Scenario::parse(r#"{"format": "ubiq.teamsim/2"}"#)
            .expect_err("a format from the future");
        assert!(failed.contains("ubiq.teamsim/2"), "{failed}");
        assert!(Scenario::parse("{").is_err(), "and so is broken JSON");
    }

    /// The projection is the records the arrangement reads: one per row, in the same order, with
    /// the delegate count per card the packers leave room for.
    #[test]
    fn a_projection_counts_the_delegates() {
        for scenario in every_preset() {
            let name = scenario.name.clone();
            let sim = Sim::new(scenario, Algo::Flow);
            let projection = sim.projection();
            assert_eq!(
                projection.agents.len(),
                sim.scenario.agents.len(),
                "{name}: one record per agent row"
            );
            for (ix, row) in sim.scenario.agents.iter().enumerate() {
                let id = projection.agents[ix].id;
                let counted = projection.rings.get(&id).copied().unwrap_or(0);
                assert_eq!(counted, row.subagents.len(), "{name}: ring of {}", row.id);
            }
            // A card the file gives no task sits above the containers, which is the one case the
            // offset is read as a point on the canvas.
            for (ix, row) in sim.scenario.agents.iter().enumerate() {
                assert_eq!(
                    projection.agents[ix].task.is_none(),
                    row.task.is_none(),
                    "{name}: {} kept its container",
                    row.id
                );
            }
        }
    }

    /// **Every arrangement keeps a card inside the container it serves.** A card drawn outside its
    /// own fence is the failure the spike exists to catch, and it is a test rather than something
    /// somebody notices on screen.
    #[test]
    fn every_arrangement_keeps_a_card_in_its_container() {
        for scenario in every_preset() {
            let name = scenario.name.clone();
            for algo in Algo::ALL {
                let mut sim = Sim::new(scenario.clone(), algo);
                sim.tidy();
                for (ix, row) in sim.scenario.agents.iter().enumerate() {
                    let Some(task) = row.task.as_deref() else {
                        continue;
                    };
                    let fence = sim.task_fence(task).expect("a container with a card in it");
                    let card = sim.card_bounds(ix).expect("a placed card");
                    assert!(
                        card.0 >= fence.0 - EPS
                            && card.1 >= fence.1 - EPS
                            && card.2 <= fence.0 + fence.2 + EPS
                            && card.3 <= fence.1 + fence.3 + EPS,
                        "{name}/{}: {} at {card:?} is outside its container {fence:?}",
                        algo.label(),
                        row.id
                    );
                }
            }
        }
    }

    /// And no two cards of one session are drawn on top of each other. Rings included: a fence is
    /// part of what a card draws, so a card under somebody else's delegates overlaps them.
    #[test]
    fn every_arrangement_keeps_two_cards_apart() {
        for scenario in every_preset() {
            let name = scenario.name.clone();
            for algo in Algo::ALL {
                let mut sim = Sim::new(scenario.clone(), algo);
                sim.tidy();
                let placed: Vec<(usize, (f32, f32, f32, f32))> = sim
                    .scenario
                    .agents
                    .iter()
                    .enumerate()
                    .filter_map(|(ix, _)| Some((ix, sim.card_bounds(ix)?)))
                    .collect();
                for (i, (one, a)) in placed.iter().enumerate() {
                    for (two, b) in &placed[i + 1..] {
                        if sim.scenario.agents[*one].session != sim.scenario.agents[*two].session {
                            continue;
                        }
                        let apart_x = a.2 <= b.0 + EPS || b.2 <= a.0 + EPS;
                        let apart_y = a.3 <= b.1 + EPS || b.3 <= a.1 + EPS;
                        assert!(
                            apart_x || apart_y,
                            "{name}/{}: {} at {a:?} overlaps {} at {b:?}",
                            algo.label(),
                            sim.scenario.agents[*one].id,
                            sim.scenario.agents[*two].id
                        );
                    }
                }
            }
        }
    }

    /// An arrival places itself and leaves what is on screen alone — the property the incremental
    /// arrangement is judged on, asserted rather than eyeballed.
    #[test]
    fn an_arrival_places_only_itself() {
        let mut sim = Sim::new(
            Scenario::parse(PRESETS[0].source).expect("the first preset"),
            Algo::Flow,
        );
        let card = sim.scenario.agents[0].id.clone();
        let was = sim.card_at(0).expect("placed");

        sim.add_subagent(&card);
        assert_eq!(sim.scenario.agents[0].subagents.len(), 1);
        assert_eq!(sim.card_at(0), Some(was), "the card it was added to stayed");

        let before: Vec<Option<(f32, f32)>> = (0..sim.scenario.agents.len())
            .map(|ix| sim.card_at(ix))
            .collect();
        let task = sim.scenario.tasks[0].id.clone();
        sim.add_agent(&task);
        for (ix, at) in before.iter().enumerate() {
            assert_eq!(&sim.card_at(ix), at, "card {ix} moved on an arrival");
        }
        assert_eq!(sim.drift.moved, 0, "and the drift says so");
        assert!(
            sim.card_at(sim.scenario.agents.len() - 1).is_some(),
            "while the arriving card was placed"
        );
    }

    /// Editing is editing the scenario: the JSON stays the one source of truth, and the geometry
    /// follows it.
    #[test]
    fn the_editor_keeps_the_scenario_true() {
        let mut sim = Sim::new(
            Scenario::parse(PRESETS[0].source).expect("the first preset"),
            Algo::Flow,
        );
        let session = sim.scenario.sessions[0].id.clone();
        let tasks = sim.scenario.tasks.len();
        sim.add_task(&session);
        assert_eq!(sim.scenario.tasks.len(), tasks + 1);
        // Appended, not sorted: the list is the order things arrived in.
        assert_eq!(sim.scenario.tasks[tasks].session, session);
        assert!(
            sim.task_fence(&sim.scenario.tasks[tasks].id.clone())
                .is_some(),
            "a new container is drawn, because it was given a card"
        );

        let one = sim.scenario.agents[0].id.clone();
        let two = sim.scenario.agents[sim.scenario.agents.len() - 1]
            .id
            .clone();
        sim.link(&one, &two, LinkKind::Handoff);
        sim.link(&one, &two, LinkKind::Handoff);
        sim.link(&one, &one, LinkKind::Watch);
        assert_eq!(sim.scenario.links.len(), 1, "no duplicate and no self-link");

        sim.remove_agent(&two);
        assert!(sim.scenario.agents.iter().all(|row| row.id != two));
        assert!(
            sim.scenario.links.is_empty(),
            "and the edge that named it went with it"
        );
    }

    /// The metrics are the arrangement measured against one screen, which is the number that says
    /// how far past it the canvas runs.
    #[test]
    fn the_metrics_measure_against_the_viewport() {
        let sim = Sim::new(
            Scenario::parse(PRESETS[0].source).expect("the first preset"),
            Algo::Flow,
        );
        let drawing = sim.drawing();
        let metrics = sim.metrics(drawing.bbox);
        assert!(metrics.w > 0.0 && metrics.h > 0.0);
        assert!((metrics.aspect - metrics.w / metrics.h).abs() < EPS);
        assert!(
            (metrics.screens_x - metrics.w / sim.scenario.viewport.w).abs() < EPS,
            "measured against the scenario's own viewport"
        );
    }

    /// The slack a float comparison is allowed: geometry is accumulated by addition, so two edges
    /// that should meet rarely meet exactly.
    const EPS: f32 = 0.01;
}
