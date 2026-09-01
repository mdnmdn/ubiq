//! The agents screen's own view of the work: what is selected in the orchestration graph, which
//! session and which states it is showing, how far in it is zoomed, and what the pointer has hold
//! of.
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
//! Nothing here draws and nothing here names a colour — an activity says what it *is*, and
//! `ui::agents` decides which token that reads in. Nothing here says where anything sits either:
//! a record and its position are separate, and [`super::layout`] owns the second half.
//!
//! **Position is the interface's own fact, membership is the host's.** A drag moves a card on the
//! canvas, which nothing outside this window has an opinion about; which task that card *serves* is
//! written down, so a drop answers the pair and the caller sends `AssignAgent`.

use std::time::{Duration, Instant};

use ubiq_proto::ids::{SessionId, TaskId};
use ubiq_proto::work::{AgentId, Bucket, TaskRecord, WorkAgent};

use super::work::WorkProjection;

pub use super::layout::{CARD_HEIGHT, CARD_WIDTH, GROUP_LABEL, GROUP_PAD, Layout};

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

pub struct GraphView {
    /// Where the work is drawn. Thrown away and recomputed whole by `relayout`, and topped up one
    /// arriving card at a time by `Layout::place_new`.
    pub layout: Layout,

    /// Which session the graph is drawing, or every one of them. Its own field rather than a
    /// reading of `selection`, because which session is *shown* and which is *selected* are two
    /// questions: the inspector and the drawer report on the second, and clearing the first must
    /// not throw the second away.
    pub session: Option<SessionId>,
    /// Which buckets the graph is showing. **Empty is no filter, not nothing** — a card in a hidden
    /// bucket is not drawn, and neither are the connectors into it, so a row with every pill off
    /// would otherwise be an empty screen with no way back.
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

/// The screen as it opens: every session and every state showing, zoomed out far enough to see the
/// work whole, the inspector up on the thread and the tasks drawer shut. Written out rather than
/// derived, because the derived zero of a zoom is a graph nobody can see.
impl Default for GraphView {
    fn default() -> Self {
        Self {
            layout: Layout::default(),
            session: None,
            buckets: Bucket::all().to_vec(),
            zoom: 0.8,
            // Nothing is selected until there is something to select; the window points the
            // selection at the first agent the moment the work arrives.
            selection: None,
            tab: InspectorTab::Chat,
            show_inspector: true,
            tasks_open: false,
            draft: String::new(),
            carry: None,
            sand: Vec::new(),
        }
    }
}

/// The zoom range and the step the toolbar's `−` and `+` move in.
pub const ZOOM_MIN: f32 = 0.5;
pub const ZOOM_MAX: f32 = 1.6;
pub const ZOOM_STEP: f32 = 0.1;

impl GraphView {
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

    /// Throw the arrangement away and compute it again from the records.
    pub fn relayout(&mut self, work: &WorkProjection) {
        self.layout = Layout::auto(&work.agents, &work.tasks);
    }

    /// The selected agent, when an agent is what is selected.
    pub fn selected_agent<'a>(&self, work: &'a WorkProjection) -> Option<&'a WorkAgent> {
        match self.selection {
            Some(Selection::Agent(id)) => work.agent(id),
            _ => None,
        }
    }

    /// Which session the screen is *about*: the one selected, or the one the selected agent runs
    /// in, falling back to the first so the inspector and the drawer always have something to
    /// report. What the canvas *draws* is `session`, which is a separate question.
    pub fn active_session(&self, work: &WorkProjection) -> Option<SessionId> {
        match self.selection {
            Some(Selection::Session(id)) => Some(id),
            Some(Selection::Agent(id)) => work.agent(id).map(|a| a.session),
            None => work.sessions.first().map(|s| s.id),
        }
    }

    /// Whether one bucket is drawn. **No pill lit is no filter**: the row means "narrow it to
    /// these", and narrowing to nothing is what an untouched row already does.
    pub fn showing(&self, bucket: Bucket) -> bool {
        self.buckets.is_empty() || self.buckets.contains(&bucket)
    }

    /// Whether a card is drawn at all, given the two filters. `session` absent is every session.
    pub fn visible(&self, agent: &WorkAgent) -> bool {
        self.showing(agent.activity.bucket()) && self.session.is_none_or(|id| agent.session == id)
    }

    /// The tasks the strip lists: every task in the session, or the ones the selected agent has a
    /// step in.
    pub fn listed_tasks<'a>(&self, work: &'a WorkProjection) -> Vec<&'a TaskRecord> {
        match self.selection {
            Some(Selection::Agent(id)) => work
                .tasks
                .iter()
                .filter(|t| {
                    t.steps.iter().any(|s| s.owner == Some(id))
                        || work.agent(id).and_then(|a| a.task) == Some(t.id)
                })
                .collect(),
            _ => {
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

    /// Show one session, or every one. It leaves the selection alone: "show me all of it" is not
    /// "stop looking at this".
    pub fn show_session(&mut self, session: Option<SessionId>) {
        self.session = session;
    }

    /// Put every filter back, which is the toolbar's one control for "show everything".
    pub fn clear_filters(&mut self) {
        self.session = None;
        self.buckets = Bucket::all().to_vec();
    }

    /// Whether anything is being hidden, so the control that clears the filters can say whether it
    /// has anything to do.
    pub fn filtered(&self) -> bool {
        self.session.is_some() || self.buckets.len() < Bucket::all().len()
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
    pub fn carry_to(
        &mut self,
        work: &WorkProjection,
        at: (f32, f32),
        pointer: Option<(f32, f32)>,
        now: Instant,
    ) {
        let Some(carry) = self.carry else { return };
        match carry.held {
            Held::Agent(id) => {
                self.place(work, id, at);
                // Which container the pointer is over decides what a drop means, and is what the
                // canvas lights up while the card is in the air.
                let over = self.task_at(work, id, at);
                if let Some(carry) = self.carry.as_mut() {
                    carry.over = over;
                }
            }
            // A container has no position of its own — its box is the box round its cards — so it
            // is moved by the difference between where the box is and where the pointer wants it,
            // and every card in it comes along because none of them was ever placed absolutely.
            Held::Task(id) => {
                if let Some((x, y, _, _)) = self.bounds_of(work, id) {
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
        let Held::Agent(id) = carry.held else {
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
