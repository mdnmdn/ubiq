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
    Activity, AgentId, Bucket, Label, Shape, Status, Step, TaskRecord, WorkAgent, WorkSession,
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

    /// Splice a task into a column, the same act a drop asks the host for.
    ///
    /// The board applies this the moment the card is put down so a reorder is visible without
    /// waiting for a `WorkList`. `before` names the card it lands in front of; `None` is the end
    /// of that column. A `before` that names the card being moved is the gap it already occupies,
    /// rewritten to the next neighbour so the host's "missing id → end of column" fallback is
    /// never what a drop onto itself becomes.
    pub fn place(&mut self, id: TaskId, status: Status, before: Option<TaskId>) {
        let Some(from) = self.tasks.iter().position(|t| t.id == id) else {
            return;
        };
        let before = match before {
            Some(named) if named == id => self.tasks[from + 1..]
                .iter()
                .find(|task| task.status == status)
                .map(|task| task.id),
            other => other,
        };
        let mut record = self.tasks.remove(from);
        record.status = status;
        let target = before
            .and_then(|named| self.tasks.iter().position(|task| task.id == named))
            .filter(|&index| self.tasks[index].status == status);
        let insert_at = target.unwrap_or_else(|| {
            self.tasks
                .iter()
                .rposition(|task| task.status == status)
                .map_or(self.tasks.len(), |index| index + 1)
        });
        self.tasks.insert(insert_at, record);
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

    /// Take an agent out of the projection outright — what `Message::ConversationDeleted` means,
    /// as opposed to [`Self::apply_agent`], which only ever replaces one still there. Answers
    /// whether one was actually held, so a caller can tell an id it never heard of from one it
    /// just dropped.
    pub fn remove_agent(&mut self, agent: AgentId) -> bool {
        let before = self.agents.len();
        self.agents.retain(|held| held.id != agent);
        self.agents.len() != before
    }

    /// The same for a session, so an agent that arrives under one nothing has
    /// heard of is still listed rather than silently dropped between headings.
    pub fn apply_session(&mut self, session: WorkSession) {
        match self.sessions.iter_mut().find(|s| s.id == session.id) {
            Some(held) => *held = session,
            None => self.sessions.push(session),
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
        if task.shape == Some(Shape::Coordinated) {
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

    /// Every task naming `id` as its [`TaskRecord::parent`]. Derived on read, the same as the host
    /// derives it: the parent carries no list of its own, so this scan is the one place either
    /// side keeps one.
    pub fn children_of(&self, id: TaskId) -> impl Iterator<Item = &TaskRecord> {
        self.tasks.iter().filter(move |t| t.parent == Some(id))
    }

    /// How many children a task has — the mission card's own count.
    pub fn child_count(&self, id: TaskId) -> usize {
        self.children_of(id).count()
    }

    /// The tasks `task` could be given as a parent, computed once so no draw site walks the task
    /// list on its own.
    ///
    /// Mirrors `crates/ubiq-host/src/work/mod.rs`'s `parent_refusal` exactly, so the picker never
    /// offers a choice the host would refuse: a candidate needs a [`ubiq_proto::work::Level`], is
    /// never `task` itself, and must not already have a parent of its own — depth is capped at
    /// one. `task` itself is excluded from having any eligible parent at all once it already has
    /// children, for the same reason: it would become both a parent and a child.
    pub fn eligible_parents(&self, task: &TaskRecord) -> Vec<&TaskRecord> {
        if self.children_of(task.id).next().is_some() {
            return Vec::new();
        }
        self.tasks
            .iter()
            .filter(|t| t.id != task.id && t.level.is_some() && t.parent.is_none())
            .collect()
    }

    /// The tasks `task` could add to its reference list: every other task it does not already
    /// name. References are symmetric and untyped, so there is no level or parent rule here — only
    /// not naming `task` itself and not naming one it already holds, which the host would drop as a
    /// no-op anyway.
    pub fn eligible_references<'a>(&'a self, task: &TaskRecord) -> Vec<&'a TaskRecord> {
        self.tasks
            .iter()
            .filter(|t| t.id != task.id && !task.references.contains(&t.id))
            .collect()
    }

    /// The tasks `task` could add to its prerequisite list: every other task it does not already
    /// name, excluding `task` itself and any task that would close a cycle in the project's
    /// prerequisite DAG. Mirrors `crates/ubiq-host/src/work/mod.rs`'s `prerequisite_cycle` walk,
    /// so the picker never offers a choice the host would refuse.
    pub fn eligible_prerequisites<'a>(&'a self, task: &TaskRecord) -> Vec<&'a TaskRecord> {
        self.tasks
            .iter()
            .filter(|t| {
                t.id != task.id
                    && !task.prerequisites.contains(&t.id)
                    && !Self::prerequisite_cycle(&self.tasks, task.id, t.id)
            })
            .collect()
    }

    /// Whether `task` is reachable by walking prerequisite edges outward from `start` — see
    /// [`Self::eligible_prerequisites`].
    fn prerequisite_cycle(tasks: &[TaskRecord], task: TaskId, start: TaskId) -> bool {
        let mut stack = vec![start];
        let mut seen = std::collections::HashSet::new();
        while let Some(id) = stack.pop() {
            if id == task {
                return true;
            }
            if !seen.insert(id) {
                continue;
            }
            if let Some(record) = tasks.iter().find(|t| t.id == id) {
                stack.extend(record.prerequisites.iter().copied());
            }
        }
        false
    }

    /// Every label anybody has used in this project, most-used first and then by name.
    ///
    /// There is no registry: a label is the name and the colour together, written on whichever
    /// tasks carry it, so the set of labels that exist *is* what the tasks say. This gathers them
    /// for the picker to suggest from, deduplicating on the name and keeping the first colour seen
    /// — the same name in two colours is one label, and the picker has to offer one swatch.
    ///
    /// Most-used first because the label somebody is about to reach for is usually the one already
    /// on half the board, and by name after that so the row does not reshuffle every time a task
    /// arrives with the counts tied.
    pub fn labels(&self) -> Vec<Label> {
        let mut found: Vec<(Label, usize)> = Vec::new();
        for label in self.tasks.iter().flat_map(|task| task.labels.iter()) {
            match found.iter_mut().find(|(held, _)| held.name == label.name) {
                Some((_, uses)) => *uses += 1,
                None => found.push((label.clone(), 1)),
            }
        }
        found.sort_by(|(a, a_uses), (b, b_uses)| {
            b_uses.cmp(a_uses).then_with(|| a.name.cmp(&b.name))
        });
        found.into_iter().map(|(label, _)| label).collect()
    }

    /// How many agents the status line counts, by bucket.
    pub fn count(&self, bucket: Bucket) -> usize {
        self.agents
            .iter()
            .filter(|a| a.activity.bucket() == bucket)
            .count()
    }

    /// What one task's [`WorkState`] is, read against the whole projection — the prerequisites
    /// come from the other tasks, and "nobody on it" from the agents.
    pub fn work_state(&self, task: &TaskRecord) -> WorkState {
        let ready = task.ready(&self.tasks);
        let held = self.members(task.id).next().is_some();
        WorkState::of(task.status, ready, held)
    }

    /// The counts one mission's progress bar is drawn from: every child of `parent`, by work
    /// state, in [`WorkState::all`]'s order. States nothing is in are kept at zero so the bar and
    /// the legend under it read the same way whatever the mission is doing.
    pub fn work_state_counts(&self, parent: TaskId) -> Vec<(WorkState, usize)> {
        let mut counts: Vec<(WorkState, usize)> =
            WorkState::all().into_iter().map(|s| (s, 0)).collect();
        for child in self.children_of(parent) {
            let state = self.work_state(child);
            if let Some((_, n)) = counts.iter_mut().find(|(s, _)| *s == state) {
                *n += 1;
            }
        }
        counts
    }
}

/// What a task is *actually* doing, as one value — the mission surfaces' own reading of a task
/// (`_docs/inbox/mission-proposal.md`, M26).
///
/// Derived rather than stored, and derived here rather than on each surface: the side panel's
/// progress bar, the full view's WBS and Tasks tab and the Teams fence all colour a task by this,
/// and a second copy of the rule is a second answer. [`crate::ui::work`] holds the token each one
/// takes; nothing in this module names a colour.
///
/// **Readiness is not a status.** M20 makes "every prerequisite is `InReview` or `Done`" a
/// question asked of the other tasks — [`TaskRecord::ready`] — so a task can be `Ready` and still
/// be waiting, which is the distinction this enum exists to draw.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkState {
    Blocked,
    /// Not ready by M20: something it waits on is neither in review nor done.
    Waiting,
    Ready,
    InProgress,
    InReview,
    Done,
    Abandoned,
}

impl WorkState {
    /// M26's table, first match wins. `ready` is [`TaskRecord::ready`] and `held` is whether any
    /// agent is on the task.
    ///
    /// The last arm is the one the table leaves implicit: a `Ready` or `Backlog` task that *is*
    /// ready and has somebody on it is not idle, so it reads as in progress — the same thing its
    /// agent's presence already says.
    pub fn of(status: Status, ready: bool, held: bool) -> Self {
        match status {
            Status::Blocked => WorkState::Blocked,
            Status::Backlog | Status::Ready | Status::InProgress if !ready => WorkState::Waiting,
            Status::Ready | Status::Backlog if !held => WorkState::Ready,
            Status::InProgress | Status::Ready | Status::Backlog => WorkState::InProgress,
            Status::InReview => WorkState::InReview,
            Status::Done => WorkState::Done,
            Status::Abandoned => WorkState::Abandoned,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            WorkState::Blocked => "blocked",
            WorkState::Waiting => "waiting",
            WorkState::Ready => "ready",
            WorkState::InProgress => "run",
            WorkState::InReview => "review",
            WorkState::Done => "done",
            WorkState::Abandoned => "abandoned",
        }
    }

    /// The order every mission surface lists the states in: done first, then what is still moving,
    /// then what is stuck — the reading order of the panel's own counts line.
    pub fn all() -> [WorkState; 7] {
        [
            WorkState::Done,
            WorkState::InReview,
            WorkState::InProgress,
            WorkState::Waiting,
            WorkState::Blocked,
            WorkState::Ready,
            WorkState::Abandoned,
        ]
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
    format_tokens(agent.tokens.max(0.0).round() as u64)
}

/// A token count in its shortest legible form: a plain integer under a thousand, then a compact
/// suffix scaled to one decimal place — `k` for thousands, `M` for millions, `G` for billions.
///
/// The one formatting a raw token count goes through everywhere a ring or a pill states one: the
/// footer's `tot` and `ctx` pills and their tooltips, and this card's own reading. A second
/// spelling of "how big is this number" per surface is exactly the drift a shared helper exists to
/// rule out.
pub fn format_tokens(value: u64) -> String {
    const UNITS: [(u64, &str); 3] = [(1_000_000_000, "G"), (1_000_000, "M"), (1_000, "k")];
    for (scale, suffix) in UNITS {
        if value >= scale {
            return format!("{:.1}{suffix}", value as f64 / scale as f64);
        }
    }
    value.to_string()
}

#[cfg(test)]
mod format_tokens_tests {
    use super::format_tokens;

    #[test]
    fn a_count_under_a_thousand_is_the_plain_number() {
        assert_eq!(format_tokens(0), "0");
        assert_eq!(format_tokens(999), "999");
    }

    #[test]
    fn a_count_of_a_thousand_or_more_takes_a_compact_suffix() {
        assert_eq!(format_tokens(1_000), "1.0k");
        assert_eq!(format_tokens(12_345), "12.3k");
        assert_eq!(format_tokens(1_000_000), "1.0M");
        assert_eq!(format_tokens(2_500_000), "2.5M");
        assert_eq!(format_tokens(1_000_000_000), "1.0G");
        assert_eq!(format_tokens(3_400_000_000), "3.4G");
    }
}

/// The word this project uses for [`ubiq_proto::work::Level::Mission`] — the project's own
/// override if it has one, else the application-wide default. Resolved once, here, so the board
/// reads a word and nothing else in the tree learns it exists.
pub fn mission_term(project_override: Option<&str>, host_default: &str) -> String {
    project_override
        .map(str::to_string)
        .unwrap_or_else(|| host_default.to_string())
}
