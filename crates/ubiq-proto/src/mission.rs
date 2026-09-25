//! A mission: its phase, who is on it, and how it is run.
//!
//! **A mission *is* its anchor task.** There is no `MissionId` and there never will be: the record
//! below is a sidecar keyed by the same [`TaskId`] as the task carrying
//! [`crate::work::Level::Mission`], exactly as that task's plan already is (`inbox-mission`, M1).
//! Every existing message, handle, MCP tool and board affordance keeps working, and a mission can
//! never exist without a card on the board.
//!
//! **One on-disk format, declared once.** The record holds every field the mission design names
//! across its stages, not only the ones a surface reads today, and every one of them carries a
//! serde default — so a `mission.toml` written by any build reads in any other, and a field that
//! arrives later is not a migration. The host writes it at
//! `<config root>/projects/<ProjectId>/missions/<TaskId>/mission.toml`, with `docs/` and
//! `journal.jsonl` beside it.
//!
//! **The roster is stored, not derived.** Membership is "assigned to the mission's task or one of
//! its children, *or* spawned by an agent that is in the mission" (M11), and the second half cannot
//! be recomputed after the fact: `Work::assign_agent` clears a `WorkAgent::parent` on every
//! reassignment, so the spawn link is gone the next time anybody asks. It is written down when it
//! happens instead, together with the labels each member has held — which is the affinity set the
//! scheduler reads (M24).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::ids::{ProjectId, SpawnId, TaskId};
use crate::work::AgentId;

/// Where a mission has got to.
///
/// Its own field rather than a reading of the anchor's [`crate::work::Status`]: a mission's
/// lifecycle has a gate in it the seven generic statuses have nowhere to put. The anchor's status
/// is written *from* this by the host, so every existing filter stays right (M4).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Phase {
    /// The coordinator and the user are settling what the mission is.
    #[default]
    Requirements,
    /// The plan is being refined, through annotation threads and further documents.
    Refining,
    /// Tasks are created and worked.
    InProgress,
    /// Everything is done.
    Completed,
    /// Terminal, from any phase.
    Abandoned,
}

impl Phase {
    pub fn label(self) -> &'static str {
        match self {
            Phase::Requirements => "requirements",
            Phase::Refining => "refining",
            Phase::InProgress => "in progress",
            Phase::Completed => "completed",
            Phase::Abandoned => "abandoned",
        }
    }

    pub fn all() -> [Phase; 5] {
        [
            Phase::Requirements,
            Phase::Refining,
            Phase::InProgress,
            Phase::Completed,
            Phase::Abandoned,
        ]
    }

    /// Where this phase sits in the lifecycle, for the one question the rules ask of two phases:
    /// is the move forward or back. [`Phase::Abandoned`] sits past the end, so every move out of
    /// it is a step back — which is exactly what un-abandoning a mission is.
    pub fn order(self) -> usize {
        match self {
            Phase::Requirements => 0,
            Phase::Refining => 1,
            Phase::InProgress => 2,
            Phase::Completed => 3,
            Phase::Abandoned => 4,
        }
    }

    /// The anchor task's [`crate::work::Status`] for this phase — §3's table, and the whole of M4.
    ///
    /// The phase is the truth and this is the derivation: the host writes it on every transition,
    /// so the board, the Teams tasks drawer and every existing filter stay right with no change to
    /// any of them. The one case not readable here is In progress *while it needs the user*, which
    /// the table gives as `Blocked`; the host adds that, because only the host knows whether
    /// anything is pending.
    pub fn anchor_status(self) -> crate::work::Status {
        match self {
            Phase::Requirements => crate::work::Status::InProgress,
            Phase::Refining => crate::work::Status::InReview,
            Phase::InProgress => crate::work::Status::InProgress,
            Phase::Completed => crate::work::Status::Done,
            Phase::Abandoned => crate::work::Status::Abandoned,
        }
    }

    /// What dropping a mission's card in a board column means (M4).
    ///
    /// [`Self::anchor_status`]'s inverse where it has one, and a reading where it does not: the
    /// three columns no phase writes — `Backlog`, `Ready`, `Blocked` — are the nearest phase
    /// either side of them. A mission card's drag is a phase move, never a raw status write, so
    /// this is where a column becomes a phase for both halves of the app.
    pub fn for_status(status: crate::work::Status) -> Phase {
        match status {
            crate::work::Status::Backlog | crate::work::Status::Ready => Phase::Requirements,
            crate::work::Status::InReview => Phase::Refining,
            crate::work::Status::InProgress | crate::work::Status::Blocked => Phase::InProgress,
            crate::work::Status::Done => Phase::Completed,
            crate::work::Status::Abandoned => Phase::Abandoned,
        }
    }
}

/// Who moved something — a phase, a coordinator, a roster entry.
///
/// Three cases rather than an `Option<AgentId>`, because "the host did it on its own" and "the
/// person at the keyboard did it" are different answers and a history that cannot tell them apart
/// cannot be read back.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Actor {
    /// The person at the keyboard, through a window.
    #[default]
    User,
    /// An agent, through the mission's MCP surface.
    Agent(AgentId),
    /// The host itself — an inferred phase, a lazily created record.
    Host,
}

/// One entry in a mission's phase history: what it moved to, when, and who moved it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhaseEntry {
    pub phase: Phase,
    pub at: DateTime<Utc>,
    #[serde(default)]
    pub by: Actor,
}

/// A phase move somebody has asked for and nobody has answered yet (M5, M6).
///
/// **It lives on the record**, not in a side table in the host: it is the thing the panel's *Needs
/// you* section draws, so it has to cross the bus — and [`crate::messages::Message::MissionChanged`]
/// already carries the whole record to every window, so storing it here costs no new message. It
/// is on disk for the same reason a phase is: a request outstanding when the host stops is still
/// outstanding when it starts again.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingPhase {
    /// The phase asked for.
    pub phase: Phase,
    /// Who asked. Never [`Actor::User`]: the user does not ask, the user moves.
    #[serde(default)]
    pub by: Actor,
    /// Why, in the requester's words — the sentence the *Needs you* row shows. A completion
    /// request carries the summary M5 asks it for; the other requests may leave it empty.
    #[serde(default)]
    pub summary: String,
    pub at: DateTime<Utc>,
}

/// An agent a member of the mission has asked for and nobody has launched yet (M13).
///
/// **[`PendingPhase`]'s shape, and it lives on the record for [`PendingPhase`]'s reason**: it is
/// what the panel's *Needs you* draws, [`crate::messages::Message::MissionChanged`] already carries
/// the whole record to every window, and a request outstanding when the host stops is still
/// outstanding when it starts again.
///
/// **Plural where the phase is singular.** At most one phase move can be pending because a
/// coordinator that has changed its mind is not two questions; two members each waiting on a worker
/// *are* two questions, and answering one must not throw the other away.
///
/// Nothing here is a launch. The host relays what was asked for; the window resolves the kind to a
/// composition, applies the spawn policy and mints the [`AgentId`] — so this names a **kind**, and
/// a definition only for the `custom` case where the requester named one outright.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingSpawn {
    /// Minted host-side at the `spawn_agent` call and handed straight back to the requester, which
    /// is why the tool can answer without waiting for anything.
    pub id: SpawnId,
    /// Who asked. Never [`Actor::User`]: the user does not ask for an agent, the user spawns one.
    #[serde(default)]
    pub by: Actor,
    /// One of [`MissionRecord::agent_kinds`] by name, or `custom` beside a [`Self::definition`].
    pub kind: String,
    /// A definition named outright, for the `custom` kind. The window still resolves it — an agent
    /// naming a definition is naming a saved setup, never a harness, an account or a credential.
    ///
    /// `alias` because this field was called `profile` before the rename (`D174`): a mission
    /// record written by an older build still says so, and a spawn request that lost its
    /// definition would silently launch the wrong thing.
    #[serde(default, alias = "profile")]
    pub definition: Option<String>,
    /// The task the new agent is for, when the requester had one in mind.
    #[serde(default)]
    pub task: Option<TaskId>,
    /// The opening prompt the new agent should be launched with.
    #[serde(default)]
    pub prompt: String,
    /// Why, in the requester's words — the sentence the *Needs you* row shows.
    #[serde(default)]
    pub reason: String,
    /// Whether the mission's scheduler asked for this one rather than an agent (M23).
    ///
    /// **It bypasses [`SpawnPolicy::Ask`]** — choosing [`ExecutionMode::Auto`] *is* the consent,
    /// and a scheduler that stopped at every launch to ask would not be a scheduler. It is capped
    /// by [`MissionRecord::parallelism`] instead, which the host applies before the request is
    /// ever made. The window still mints the [`AgentId`] and still refuses under
    /// [`SpawnPolicy::Never`]; only the *asking* is skipped.
    #[serde(default)]
    pub auto: bool,
    pub at: DateTime<Utc>,
}

/// What became of a [`PendingSpawn`], as the window reports it back (M13).
///
/// The kind is carried on both arms rather than read back off the request, because the user may
/// **change** it in the *Needs you* row before allowing: what the requester is told is the kind
/// actually used, which is only knowable here.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SpawnOutcome {
    /// The window launched one. The id is the window's own mint, as every [`AgentId`] is.
    Launched { agent: AgentId, kind: String },
    /// The policy refused it, or the user did. `reason` is the sentence the requester reads.
    Declined { reason: String },
}

/// One entry in a mission's coordinator history. `agent` absent is the coordinator being cleared.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoordinatorEntry {
    #[serde(default)]
    pub agent: Option<AgentId>,
    pub at: DateTime<Utc>,
    #[serde(default)]
    pub by: Actor,
}

/// What an agent is in a mission for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum MissionRole {
    /// The one agent that plans, asks and requests the phase moves.
    Coordinator,
    /// Everybody else.
    #[default]
    Worker,
}

/// One agent's membership of a mission, as it was written down when it happened.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RosterEntry {
    pub agent: AgentId,
    #[serde(default)]
    pub role: MissionRole,
    /// The labels of every task this agent has held in this mission — its affinity set (M24).
    /// Accumulated, never recomputed: the tasks it held may since have been relabelled or deleted.
    #[serde(default)]
    pub labels: Vec<String>,
    pub joined_at: DateTime<Utc>,
    /// When it left, absent while it is still in. A member is kept rather than removed so the
    /// history can be read back.
    #[serde(default)]
    pub left_at: Option<DateTime<Utc>>,

    // ── the scheduler's bookkeeping (M23, M24) ──────────────────────
    /// Whether the mission's scheduler is the one that gave this agent work.
    ///
    /// The scheduler only counts, reuses and retires **its own** agents: a coordinator, an agent a
    /// person attached by hand and an agent a member spawned for its own reasons are none of its
    /// business, and an autonomous scheduler that stops an agent nobody gave it is the failure
    /// this flag prevents.
    #[serde(default)]
    pub scheduled: bool,
    /// The task this member is holding for the scheduler right now, if any — its slot.
    #[serde(default)]
    pub current_task: Option<TaskId>,
    /// When [`Self::current_task`] was handed over, which is what the idle grace is measured from.
    #[serde(default)]
    pub held_since: Option<DateTime<Utc>>,
    /// When this member was last handed anything — the tie-break when two agents have the same
    /// affinity, "the most recent holder" (M24).
    #[serde(default)]
    pub last_held_at: Option<DateTime<Utc>>,
    /// How many tasks this member has been handed in this mission, against
    /// [`MissionRecord::max_tasks_per_agent`].
    #[serde(default)]
    pub tasks_held: usize,
}

impl RosterEntry {
    /// A fresh membership, holding nothing.
    pub fn new(agent: AgentId, role: MissionRole, joined_at: DateTime<Utc>) -> Self {
        Self {
            agent,
            role,
            labels: Vec::new(),
            joined_at,
            left_at: None,
            scheduled: false,
            current_task: None,
            held_since: None,
            last_held_at: None,
            tasks_held: 0,
        }
    }

    /// How well this member fits a task: how many of the task's labels are in its affinity set
    /// (M24). **Zero never reuses an agent** — that rule is the caller's, because zero is still a
    /// meaningful answer to "how well".
    pub fn affinity(&self, labels: &[String]) -> usize {
        labels
            .iter()
            .filter(|label| {
                self.labels
                    .iter()
                    .any(|held| held.eq_ignore_ascii_case(label))
            })
            .count()
    }

    /// Fold a task's labels into this member's affinity set, keeping it a set.
    pub fn learn(&mut self, labels: &[String]) {
        for label in labels {
            if !self
                .labels
                .iter()
                .any(|held| held.eq_ignore_ascii_case(label))
            {
                self.labels.push(label.clone());
            }
        }
    }
}

/// How many times one task has been released back to `Ready` by a failed attempt (M23).
///
/// A list rather than a map because `mission.toml` is TOML: a table keyed by a raw id reads
/// badly and a key that is not a bare word has to be quoted, and this is read far more often by a
/// person debugging a mission than by anything else.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskAttempts {
    pub task: TaskId,
    pub attempts: usize,
}

/// The agent kind the planner suggested for one task (M21).
///
/// **On the mission, not on the task.** A [`crate::work::TaskRecord`] is a board fact and a
/// mission's agent kinds are the mission's own vocabulary — a task carrying the name of a kind
/// that only one mission has ever heard of would be a board field nothing else can read. It is
/// [`TaskAttempts`]' shape and lives beside it for the same reason.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskKind {
    pub task: TaskId,
    pub kind: String,
}

/// How the In-progress phase is run (M22).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExecutionMode {
    /// Tasks are completed by the user, or by agents the coordinator spawns and assigns.
    /// Readiness is advisory: shown everywhere, enforced nowhere.
    #[default]
    Manual,
    /// The mission scheduler hands ready tasks to agents, at [`MissionRecord::parallelism`].
    Auto,
}

/// What becomes of a scheduler's agent when its task is finished (M23).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum OnFinish {
    /// Hand it another task with enough affinity, else stop it.
    #[default]
    ReuseOrStop,
    /// Always stop it. Its conversation stays resumable either way.
    Stop,
}

/// What happens when an agent in the mission asks for another agent (M13).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SpawnPolicy {
    /// A row in the panel's *Needs you*, where the user may change the kind before allowing.
    #[default]
    Ask,
    /// Launched without asking, up to [`MissionRecord::spawn_limit`] concurrent.
    Auto,
    /// Refused.
    Never,
}

/// One of the kinds of agent a mission may spawn: what to call it, what it resolves to, and a
/// line the requesting agent reads to choose between them (M13).
///
/// The launch itself is still the window's — an agent asks for a *kind*, never for a concrete
/// composition, so the one-minter rule for an [`AgentId`] holds.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentKind {
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// The saved setup it resolves to. The four fields below override it, exactly as a launch's
    /// picks already outrank a definition's. `alias` reads what older records call `profile`
    /// (`D174`).
    #[serde(default, alias = "profile")]
    pub definition: Option<String>,
    #[serde(default)]
    pub agent_type: Option<String>,
    #[serde(default)]
    pub account: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub permission_mode: Option<String>,
    /// What this kind is good at, in the task labels' own vocabulary (M24).
    #[serde(default)]
    pub labels: Vec<String>,
}

fn default_true() -> bool {
    true
}

fn default_parallelism() -> usize {
    2
}

fn default_spawn_limit() -> usize {
    2
}

fn default_max_tasks_per_agent() -> usize {
    3
}

fn default_max_attempts() -> usize {
    2
}

/// A mission, whole — everything about one that is not already on its anchor task.
///
/// The brief (description, attachments, references), the children and the plan are **not** here:
/// they are the task's own fields and the plan store's, reused rather than copied (M7, M9).
///
/// Every field carries a serde default except the two ids, which have none by construction — an
/// id that was not minted is a nil id that looks real — and which the store writes on every save.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MissionRecord {
    /// The project the anchor task belongs to.
    pub project_id: ProjectId,
    /// The anchor task — the mission's identity (M1).
    pub task_id: TaskId,

    // ── the lifecycle ───────────────────────────────────────────────
    #[serde(default)]
    pub phase: Phase,
    /// Every phase this mission has been in, oldest first, including the one it is in now.
    #[serde(default)]
    pub phase_history: Vec<PhaseEntry>,
    /// The phase move waiting on the user, if one is. At most one at a time: a second request
    /// replaces the first, because a coordinator that has changed its mind is not two questions.
    #[serde(default)]
    pub pending_phase: Option<PendingPhase>,

    // ── who is on it ────────────────────────────────────────────────
    /// The agent that plans, asks and requests the phase moves. Absent until one is attached.
    #[serde(default)]
    pub coordinator: Option<AgentId>,
    /// Every agent that has been the coordinator, oldest first.
    #[serde(default)]
    pub coordinator_history: Vec<CoordinatorEntry>,
    /// Who is in the mission, and who has been — stored rather than derived, for the reason this
    /// module's doc comment gives.
    #[serde(default)]
    pub roster: Vec<RosterEntry>,

    // ── the gates ───────────────────────────────────────────────────
    /// Whether Refining → In progress is a human gate: plan approval, the moment the user commits
    /// agents and spend (M5).
    #[serde(default)]
    pub require_plan: bool,
    /// Whether Requirements → Refining moves without asking. Nothing is at stake there, so it is
    /// on by default.
    #[serde(default = "default_true")]
    pub auto_refine: bool,

    // ── how it is run ───────────────────────────────────────────────
    #[serde(default)]
    pub execution: ExecutionMode,
    /// How many tasks the scheduler may have in flight at once, in [`ExecutionMode::Auto`].
    #[serde(default = "default_parallelism")]
    pub parallelism: usize,
    #[serde(default)]
    pub on_finish: OnFinish,
    /// How many tasks one agent may be handed in this mission before a fresh one is spawned
    /// instead — a fresh context beats a bloated one.
    #[serde(default = "default_max_tasks_per_agent")]
    pub max_tasks_per_agent: usize,
    /// How many times a task may be released back to `Ready` by a failed attempt before it is
    /// blocked and the user is told.
    #[serde(default = "default_max_attempts")]
    pub max_attempts: usize,
    /// What each task has cost the scheduler in failed attempts so far, against
    /// [`Self::max_attempts`]. Only tasks that have failed at least once appear.
    #[serde(default, rename = "attempt")]
    pub attempts: Vec<TaskAttempts>,
    /// The agent kind the planner suggested for a task, where it suggested one (M21).
    #[serde(default, rename = "task_kind")]
    pub task_kinds: Vec<TaskKind>,
    /// Whether the scheduler has already said the work is finished, so it says it once rather
    /// than on every wake. Cleared the moment it hands anything out again.
    #[serde(default)]
    pub scheduler_done: bool,

    // ── spawning ────────────────────────────────────────────────────
    #[serde(default)]
    pub spawn_policy: SpawnPolicy,
    /// The concurrency cap for [`SpawnPolicy::Auto`], ignored by the other two.
    #[serde(default = "default_spawn_limit")]
    pub spawn_limit: usize,
    /// The kinds of agent this mission may spawn, seeded from the project's definitions.
    #[serde(default)]
    pub agent_kinds: Vec<AgentKind>,
    /// Which of [`Self::agent_kinds`] a request that names none resolves to, by name.
    #[serde(default)]
    pub default_kind: Option<String>,
    /// The agents members have asked for and nobody has launched yet — [`Self::pending_phase`]'s
    /// place in *Needs you*, and see [`PendingSpawn`] for why this one is a list.
    #[serde(default)]
    pub pending_spawns: Vec<PendingSpawn>,

    // ── the documents ───────────────────────────────────────────────
    /// The names of the documents in `missions/<TaskId>/docs/`, without the `.md`, sorted.
    ///
    /// **Derived, not authored** — the host refreshes it from the directory every time it reads
    /// the record, so a file the user dropped in by hand is listed and a stale name never is. It
    /// lives here rather than in a listing message of its own because
    /// [`crate::messages::Message::MissionChanged`] already carries the whole record to every
    /// window on every mutation: a window learns which documents exist for free, and the *Plan &
    /// docs* tab needs no second round trip (M8, M16).
    ///
    /// It is serialised like every other field — the same derive carries the record over the bus
    /// as well as into `mission.toml` — but the copy in the file is never read as truth: the host
    /// overwrites it from the directory on every load.
    #[serde(default)]
    pub documents: Vec<String>,

    // ── stamps ──────────────────────────────────────────────────────
    #[serde(default)]
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub updated_at: DateTime<Utc>,
}

impl MissionRecord {
    /// A record for a task, in a phase somebody else inferred. The host stamps both times and
    /// opens the phase history with the phase it starts in, so a record always says how it got
    /// where it is.
    pub fn new(project_id: ProjectId, task_id: TaskId, phase: Phase, at: DateTime<Utc>) -> Self {
        Self {
            project_id,
            task_id,
            phase,
            phase_history: vec![PhaseEntry {
                phase,
                at,
                by: Actor::Host,
            }],
            pending_phase: None,
            coordinator: None,
            coordinator_history: Vec::new(),
            roster: Vec::new(),
            require_plan: false,
            auto_refine: true,
            execution: ExecutionMode::default(),
            parallelism: default_parallelism(),
            on_finish: OnFinish::default(),
            max_tasks_per_agent: default_max_tasks_per_agent(),
            max_attempts: default_max_attempts(),
            attempts: Vec::new(),
            task_kinds: Vec::new(),
            scheduler_done: false,
            spawn_policy: SpawnPolicy::default(),
            spawn_limit: default_spawn_limit(),
            agent_kinds: Vec::new(),
            default_kind: None,
            pending_spawns: Vec::new(),
            documents: Vec::new(),
            created_at: at,
            updated_at: at,
        }
    }

    /// Whether anything has happened in this mission that a demotion would lose — the test M3
    /// refuses a demotion on. The journal is a file rather than a field, so its half of the
    /// question is the store's.
    pub fn has_members(&self) -> bool {
        !self.roster.is_empty()
    }

    /// This agent's roster entry while it is still in — a member that has left is history, not
    /// membership, which is the whole reason `left_at` is kept rather than the row removed.
    pub fn member(&self, agent: AgentId) -> Option<&RosterEntry> {
        self.roster
            .iter()
            .find(|entry| entry.agent == agent && entry.left_at.is_none())
    }

    /// One of [`Self::agent_kinds`] by name, matched case-insensitively — an agent writing
    /// `Reviewer` for a row called `reviewer` has named the row it meant.
    pub fn kind_named(&self, name: &str) -> Option<&AgentKind> {
        let name = name.trim();
        self.agent_kinds
            .iter()
            .find(|kind| kind.name.eq_ignore_ascii_case(name))
    }

    /// The spawn request this id names, while it is still outstanding.
    pub fn pending_spawn(&self, id: SpawnId) -> Option<&PendingSpawn> {
        self.pending_spawns.iter().find(|spawn| spawn.id == id)
    }

    // ── the scheduler's state (M23) ─────────────────────────────────

    /// Whether the scheduler is the one assigning this mission's work right now.
    ///
    /// **Auto only runs in the In-progress phase** (M22): leaving it pauses the scheduler rather
    /// than switching the mode, so the setting is still what the user chose when they come back.
    pub fn scheduling(&self) -> bool {
        self.execution == ExecutionMode::Auto && self.phase == Phase::InProgress
    }

    /// The members the scheduler may reuse, retire and count against its slots — its own, still
    /// in, and never the coordinator.
    pub fn scheduler_members(&self) -> impl Iterator<Item = &RosterEntry> {
        self.roster.iter().filter(|entry| {
            entry.left_at.is_none() && entry.scheduled && entry.role != MissionRole::Coordinator
        })
    }

    /// One member's row while it is still in, to write the scheduler's bookkeeping onto.
    pub fn member_mut(&mut self, agent: AgentId) -> Option<&mut RosterEntry> {
        self.roster
            .iter_mut()
            .find(|entry| entry.agent == agent && entry.left_at.is_none())
    }

    /// How many failed attempts this task has cost so far.
    pub fn attempts_of(&self, task: TaskId) -> usize {
        self.attempts
            .iter()
            .find(|row| row.task == task)
            .map_or(0, |row| row.attempts)
    }

    /// Count one more failed attempt against a task, and answer the new total.
    pub fn note_attempt(&mut self, task: TaskId) -> usize {
        match self.attempts.iter_mut().find(|row| row.task == task) {
            Some(row) => {
                row.attempts += 1;
                row.attempts
            }
            None => {
                self.attempts.push(TaskAttempts { task, attempts: 1 });
                1
            }
        }
    }

    /// The agent kind the planner asked for on this task, if it asked for one.
    pub fn task_kind(&self, task: TaskId) -> Option<&str> {
        self.task_kinds
            .iter()
            .find(|row| row.task == task)
            .map(|row| row.kind.as_str())
    }

    /// Record the planner's suggested kind for a task, replacing any earlier one. An empty name
    /// clears it.
    pub fn set_task_kind(&mut self, task: TaskId, kind: Option<String>) {
        self.task_kinds.retain(|row| row.task != task);
        if let Some(kind) = kind
            .map(|kind| kind.trim().to_string())
            .filter(|k| !k.is_empty())
        {
            self.task_kinds.push(TaskKind { task, kind });
        }
    }

    /// The agent kind that best fits a task's labels, for a fresh spawn (M24): the most overlap,
    /// and the mission's default kind when nothing overlaps at all.
    pub fn kind_for(&self, labels: &[String]) -> Option<String> {
        let best = self
            .agent_kinds
            .iter()
            .map(|kind| {
                let score = labels
                    .iter()
                    .filter(|label| {
                        kind.labels
                            .iter()
                            .any(|held| held.eq_ignore_ascii_case(label))
                    })
                    .count();
                (score, kind)
            })
            .filter(|(score, _)| *score > 0)
            .max_by_key(|(score, _)| *score)
            .map(|(_, kind)| kind.name.clone());
        best.or_else(|| self.default_kind.clone())
            .or_else(|| (self.agent_kinds.len() == 1).then(|| self.agent_kinds[0].name.clone()))
    }
}

/// What happened in a mission, as one journal line names it (M12).
///
/// A closed set rather than free text, so the *Activity* tab can filter by kind and by agent
/// without parsing sentences. The sentence a surface draws is [`JournalEntry::text`], written by
/// the host beside the kind — one reading, so a window and an agent never disagree about what a
/// line says.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum JournalEvent {
    /// Somebody asked for a phase move and it became the mission's pending request.
    PhaseRequested {
        phase: Phase,
    },
    /// The phase moved.
    PhaseChanged {
        phase: Phase,
    },
    AgentJoined {
        agent: AgentId,
        role: MissionRole,
    },
    AgentLeft {
        agent: AgentId,
    },
    /// A member asked the mission for another agent (M13).
    ///
    /// `agent_kind` rather than `kind`: this enum is serialised with `tag = "kind"`, so a field
    /// spelled that way would collide with the discriminant the *Activity* tab filters on.
    SpawnRequested {
        agent_kind: String,
    },
    /// The window answered a spawn request. `agent` absent is a decline — the kind is still named,
    /// because the kind the user allowed may not be the kind that was asked for.
    SpawnAnswered {
        agent_kind: String,
        #[serde(default)]
        agent: Option<AgentId>,
    },
    AgentRoleChanged {
        agent: AgentId,
        role: MissionRole,
    },
    TaskCreated {
        task: TaskId,
    },
    TaskFinished {
        task: TaskId,
    },
    DocumentWritten {
        name: String,
    },
    /// `report_progress`, and every other line an agent writes about its own work.
    Progress {
        #[serde(default)]
        task: Option<TaskId>,
    },
    /// The user said something to the mission — the panel's *Feedback* line.
    Feedback,
    AskRaised,
    AskAnswered,

    // ── the scheduler (M22, M23) ────────────────────────────────────
    /// The execution mode was switched. Journaled wherever it is switched from, because auto mode
    /// starting is the single most consequential thing anybody does to a mission.
    ExecutionChanged {
        mode: ExecutionMode,
    },
    /// The scheduler handed a task to an agent — *scheduled T-45 → worker-2 (affinity ui, api)*.
    /// `labels` is the overlap it chose on, not the task's whole label set: what is read back has
    /// to say *why*, or auto mode cannot be audited.
    TaskScheduled {
        task: TaskId,
        agent: AgentId,
        #[serde(default)]
        labels: Vec<String>,
    },
    /// A failed attempt put a task back in the pool — *released T-47, attempt 2*.
    TaskReleased {
        task: TaskId,
        agent: Option<AgentId>,
        attempt: usize,
    },
    /// A task ran out of attempts and is now `Blocked` for a person to look at.
    TaskBlocked {
        task: TaskId,
        attempts: usize,
    },
    /// The scheduler stopped one of its own agents — *retired worker-3*. Unloaded, never deleted:
    /// its conversation stays resumable.
    AgentRetired {
        agent: AgentId,
    },
    /// Every child is in review or done, so the scheduler has nothing left to hand out.
    SchedulerFinished,
}

impl JournalEvent {
    /// The filter key the *Activity* tab groups by.
    pub fn kind(&self) -> &'static str {
        match self {
            JournalEvent::PhaseRequested { .. } => "phase_requested",
            JournalEvent::PhaseChanged { .. } => "phase_changed",
            JournalEvent::AgentJoined { .. } => "agent_joined",
            JournalEvent::AgentLeft { .. } => "agent_left",
            JournalEvent::SpawnRequested { .. } => "spawn_requested",
            JournalEvent::SpawnAnswered { .. } => "spawn_answered",
            JournalEvent::AgentRoleChanged { .. } => "agent_role_changed",
            JournalEvent::TaskCreated { .. } => "task_created",
            JournalEvent::TaskFinished { .. } => "task_finished",
            JournalEvent::DocumentWritten { .. } => "document_written",
            JournalEvent::Progress { .. } => "progress",
            JournalEvent::Feedback => "feedback",
            JournalEvent::AskRaised => "ask_raised",
            JournalEvent::AskAnswered => "ask_answered",
            JournalEvent::ExecutionChanged { .. } => "execution_changed",
            JournalEvent::TaskScheduled { .. } => "task_scheduled",
            JournalEvent::TaskReleased { .. } => "task_released",
            JournalEvent::TaskBlocked { .. } => "task_blocked",
            JournalEvent::AgentRetired { .. } => "agent_retired",
            JournalEvent::SchedulerFinished => "scheduler_finished",
        }
    }
}

/// One line of `missions/<TaskId>/journal.jsonl`.
///
/// **`seq` is the line's own index in the file**, assigned by the store at the append and never
/// rewritten — the journal is append-only, so a line's position is stable for the life of the
/// mission. It is what [`crate::messages::Message::LoadJournal`] pages back through: a timestamp
/// would collide within a millisecond and could not be used as a cursor at all.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalEntry {
    #[serde(default)]
    pub seq: u64,
    pub at: DateTime<Utc>,
    #[serde(default)]
    pub by: Actor,
    pub event: JournalEvent,
    /// The one sentence a surface draws, written where the event was.
    #[serde(default)]
    pub text: String,
}

impl JournalEntry {
    /// A line with no sequence yet — the store assigns one as it appends.
    pub fn new(at: DateTime<Utc>, by: Actor, event: JournalEvent, text: impl Into<String>) -> Self {
        Self {
            seq: 0,
            at,
            by,
            event,
            text: text.into(),
        }
    }

    /// The agent this line is about or from, where there is one — what the *Activity* tab's
    /// per-agent filter reads.
    pub fn agent(&self) -> Option<AgentId> {
        match &self.event {
            JournalEvent::AgentJoined { agent, .. }
            | JournalEvent::AgentLeft { agent }
            | JournalEvent::AgentRetired { agent }
            | JournalEvent::TaskScheduled { agent, .. }
            | JournalEvent::AgentRoleChanged { agent, .. } => Some(*agent),
            JournalEvent::TaskReleased { agent, .. } => *agent,
            _ => match self.by {
                Actor::Agent(agent) => Some(agent),
                _ => None,
            },
        }
    }
}

/// One of a mission's settings, with the value to set it to.
///
/// [`crate::messages::TaskField`]'s shape, for its reason: one variant on the wire rather than a
/// message per setting, and an arm's `None` is the cleared state.
///
/// **The phase is not here.** Moving a mission is a request and a confirmation, not a field write.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MissionField {
    Coordinator(Option<AgentId>),
    RequirePlan(bool),
    AutoRefine(bool),
    Execution(ExecutionMode),
    Parallelism(usize),
    OnFinish(OnFinish),
    MaxTasksPerAgent(usize),
    MaxAttempts(usize),
    SpawnPolicy(SpawnPolicy),
    SpawnLimit(usize),
    /// The whole table, replaced — [`crate::messages::TaskField::Labels`]'s posture: a short list
    /// edited as a set, where a delta would cost a second message and an ordering rule.
    AgentKinds(Vec<AgentKind>),
    DefaultKind(Option<String>),
}
