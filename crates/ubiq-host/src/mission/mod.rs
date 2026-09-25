//! A project's missions, as the host runs them.
//!
//! **A mission is its anchor task.** Everything here is keyed by the `(ProjectId, TaskId)` of a
//! task carrying [`ubiq_proto::work::Level::Mission`], and every call refuses a task that is not
//! there or does not carry a level — `crate::plan`'s own refusal, for its reason: there is no
//! mission without a card.
//!
//! **Records are made lazily, and their phase is inferred** (M3). There is no migration pass and
//! no boot-time sweep: the first time a mission is listed, created or touched, a record appears
//! with the phase the anchor implies — `Done` is Completed, `Abandoned` is Abandoned, a plan on
//! disk is Refining, and everything else is Requirements. A mission that predates the record, one
//! promoted on the panel's level control and one made by the dialog all take the same path.
//!
//! **Demotion is the one refusal that is not about the task.** Clearing a mission's level throws
//! away its record; that is fine while nothing has happened in it and is not while agents have
//! been in it or the journal has anything in it, so [`Missions::demotion_refusal`] answers a
//! sentence and the coordinator writes nothing.
//!
//! Like [`crate::plan::Plans`] this holds a [`crate::work::Handle`] of its own — a mission's
//! refusals are the anchor task's — and is itself held behind a [`Handle`] so the coordinator and
//! (from the stage that adds `ubiq-mission`) the MCP listener share one instance (`D120`).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

use chrono::Utc;
use ubiq_proto::bus::Voice;
use ubiq_proto::ids::{ProjectId, SpawnId, TaskId};
use ubiq_proto::messages::Message;
use ubiq_proto::mission::{
    Actor, CoordinatorEntry, ExecutionMode, JournalEntry, JournalEvent, MissionField,
    MissionRecord, MissionRole, PendingPhase, PendingSpawn, Phase, PhaseEntry, RosterEntry,
    SpawnOutcome,
};
use ubiq_proto::work::{AgentId, Level, Status, TaskRecord};

use crate::reply::Reply;
use crate::store::mission::MissionStore;
use crate::store::plan::FilePlanStore;
use crate::work;

pub mod scheduler;

/// How many journal lines a page holds when the asker names no limit.
pub const JOURNAL_PAGE: usize = 50;

/// The most a page may hold, whatever an asker names — a journal grows without bound and one
/// message carrying all of it would be one message nobody can draw.
pub const JOURNAL_MAX_PAGE: usize = 500;

/// One project's missions.
pub struct Missions {
    store: MissionStore,
    /// To check the anchor task exists and carries a level, and to read the status a lazily
    /// created record's phase is inferred from.
    work: work::Handle,
    /// To ask whether a plan has been written — the other half of that inference. The store is a
    /// path calculator with no state, so holding a second one costs nothing and saves threading
    /// `crate::plan::Handle` through a lock that has no business taking it.
    plans: FilePlanStore,
    /// Where each mission's coordinator has read its feedback up to — a journal sequence, held in
    /// memory for the reason [`Missions::take_feedback`] gives.
    feedback_read: HashMap<(ProjectId, TaskId), u64>,
    /// How a line reaches a **live** agent rather than only its row.
    ///
    /// The conversation map is the coordinator's alone, and half this module runs on an MCP
    /// listener thread instead — so a prompt is *said* into the host's own inbox as
    /// [`Message::PromptAgent`] and the run loop does the driving, exactly as `ubiq-ask` says
    /// [`Message::AskUser`] (`D138`). Speaking through the inbox rather than reaching for the
    /// coordinator's state is what keeps this a bus fact and not a shared handle.
    voice: Voice,
}

/// A cloneable handle to [`Missions`], on [`crate::plan::Handle`]'s own footing.
#[derive(Clone)]
pub struct Handle(Arc<Mutex<Missions>>);

impl Handle {
    pub fn new(missions: Missions) -> Self {
        Self(Arc::new(Mutex::new(missions)))
    }

    /// A poisoned lock is taken back rather than propagated — [`crate::work::Handle::lock`]'s own
    /// reasoning.
    pub fn lock(&self) -> MutexGuard<'_, Missions> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl Missions {
    pub fn open(
        store: MissionStore,
        work: work::Handle,
        plans: FilePlanStore,
        voice: Voice,
    ) -> Self {
        Self {
            store,
            work,
            plans,
            feedback_read: HashMap::new(),
            voice,
        }
    }

    /// Why this task may not carry a mission, if there is a reason.
    fn refusal(&mut self, project: ProjectId, task: TaskId) -> Option<String> {
        let (_, tasks) = self.work.lock().tasks(project);
        let Some(record) = tasks.iter().find(|t| t.id == task) else {
            return Some("no such task".to_string());
        };
        if record.level.is_none() {
            return Some("only a task with a level can carry a mission".to_string());
        }
        None
    }

    /// The phase a record made now would start in, read off the anchor (M3).
    fn inferred_phase(&mut self, project: ProjectId, task: TaskId) -> Phase {
        let (_, tasks) = self.work.lock().tasks(project);
        let status = tasks.iter().find(|t| t.id == task).map(|t| t.status);
        match status {
            Some(Status::Done) => Phase::Completed,
            Some(Status::Abandoned) => Phase::Abandoned,
            _ => {
                let planned = self
                    .plans
                    .load(project, task)
                    .ok()
                    .flatten()
                    .is_some_and(|body| !body.trim().is_empty());
                if planned {
                    Phase::Refining
                } else {
                    Phase::Requirements
                }
            }
        }
    }

    /// The mission's record, made if it was not there — the lazy creation every entry point goes
    /// through. `Ok(None)` is a task this build may not make one for; the reason is the caller's
    /// to report.
    fn ensure(
        &mut self,
        project: ProjectId,
        task: TaskId,
    ) -> Result<(MissionRecord, bool), String> {
        match self.store.load(project, task) {
            Ok(Some(mut record)) => {
                // The directory is the truth about which documents exist, so the record is
                // refreshed from it on the way out rather than trusted — see
                // `MissionRecord::documents`. This is what carries the names to every window,
                // since `MissionChanged` already carries the record.
                record.documents = self.store.documents(project, task);
                Ok((record, false))
            }
            Ok(None) => {
                let phase = self.inferred_phase(project, task);
                let mut record = MissionRecord::new(project, task, phase, Utc::now());
                record.documents = self.store.documents(project, task);
                self.store
                    .save(&record)
                    .map_err(|error| error.to_string())?;
                Ok((record, true))
            }
            Err(error) => Err(error.to_string()),
        }
    }

    /// One mission, made if this is the first anybody has asked. Answered with
    /// [`Message::MissionChanged`] — broadcast, because every window shows the same project's
    /// missions and the bus has no way to address one project's windows.
    pub fn create(&mut self, project: ProjectId, task: TaskId) -> Vec<Reply> {
        if let Some(refusal) = self.refusal(project, task) {
            return vec![Reply::Asker(error(project, Some(task), refusal))];
        }
        match self.ensure(project, task) {
            Ok((record, _)) => vec![Reply::Everyone(Message::MissionChanged {
                project_id: project,
                mission: Box::new(record),
            })],
            Err(reason) => vec![Reply::Asker(error(project, Some(task), reason))],
        }
    }

    /// Every mission in the project, whole. **This is one of the three things that create a
    /// record**: a mission task with none gets one here, so a board that predates missions
    /// entirely answers a complete list the first time it is asked.
    pub fn list(&mut self, project: ProjectId) -> Vec<Reply> {
        let (mut replies, tasks) = self.work.lock().tasks(project);
        let anchors: Vec<TaskId> = tasks
            .iter()
            .filter(|task| task.level.is_some())
            .map(|task| task.id)
            .collect();

        let mut missions = Vec::new();
        for anchor in anchors {
            match self.ensure(project, anchor) {
                Ok((record, made)) => {
                    if made {
                        replies.push(Reply::Everyone(Message::MissionChanged {
                            project_id: project,
                            mission: Box::new(record.clone()),
                        }));
                    }
                    missions.push(record);
                }
                Err(reason) => replies.push(Reply::Asker(error(project, Some(anchor), reason))),
            }
        }
        replies.push(Reply::Asker(Message::MissionList {
            project_id: project,
            missions,
        }));
        replies
    }

    /// The record as it stands, without creating one. For the code that wants to know whether a
    /// mission has anything in it — a demotion, a deletion — rather than to show it.
    pub fn record(&self, project: ProjectId, task: TaskId) -> Option<MissionRecord> {
        let mut record = self.store.load(project, task).ok().flatten()?;
        // The same refresh [`Self::ensure`] does, for its reason: the directory is the truth about
        // which documents exist, and a reader here is a reader that will show them.
        record.documents = self.store.documents(project, task);
        Some(record)
    }

    /// The mission's record, made if this is the first anybody has asked — the same lazy creation
    /// [`Self::list`] and [`Self::create`] do, for a caller that is a **tool** rather than a
    /// window. An agent calling `mission_overview` is exactly as much "somebody opening the
    /// mission" as a board listing it is (M3).
    ///
    /// The replies carry the [`Message::MissionChanged`] a creation broadcasts and are empty when
    /// the record was already there, so a caller never has to ask whether it made one.
    pub fn touch(
        &mut self,
        project: ProjectId,
        task: TaskId,
    ) -> Result<(MissionRecord, Vec<Reply>), String> {
        if let Some(refusal) = self.refusal(project, task) {
            return Err(refusal);
        }
        let (record, made) = self.ensure(project, task)?;
        let replies = if made {
            vec![Reply::Everyone(Message::MissionChanged {
                project_id: project,
                mission: Box::new(record.clone()),
            })]
        } else {
            Vec::new()
        };
        Ok((record, replies))
    }

    /// Set one of a mission's settings. A value that already matches costs no write, the posture
    /// every other display-only edit takes.
    pub fn set_field(
        &mut self,
        project: ProjectId,
        task: TaskId,
        field: MissionField,
    ) -> Vec<Reply> {
        if let Some(refusal) = self.refusal(project, task) {
            return vec![Reply::Asker(error(project, Some(task), refusal))];
        }
        let (mut record, made) = match self.ensure(project, task) {
            Ok(found) => found,
            Err(reason) => return vec![Reply::Asker(error(project, Some(task), reason))],
        };

        let now = Utc::now();
        // Lines this edit writes, appended after the record is saved so a journal never claims
        // something the record does not say.
        let mut lines: Vec<JournalEntry> = Vec::new();
        // What to say to whom once the record is written — the coordinator handoff's two sentences
        // (M10). Collected rather than said here, so nothing is told a thing the record does not
        // yet say and a failed save tells nobody anything.
        let mut notices: Vec<(AgentId, String)> = Vec::new();
        let changed = match field {
            MissionField::Coordinator(agent) => {
                let changed = record.coordinator != agent;
                if changed {
                    // The coordinator is a roster role, so attaching one is a membership change:
                    // the one stepping down goes back to being a worker and the one taking over
                    // joins if it was not already in. Without this a mission could have a
                    // coordinator that is not on its own roster.
                    let outgoing = record.coordinator;
                    record.coordinator = agent;
                    if let Some(outgoing) = outgoing
                        && let Some(entry) = record
                            .roster
                            .iter_mut()
                            .find(|entry| entry.agent == outgoing && entry.left_at.is_none())
                    {
                        entry.role = MissionRole::Worker;
                        lines.push(JournalEntry::new(
                            now,
                            Actor::User,
                            JournalEvent::AgentRoleChanged {
                                agent: outgoing,
                                role: MissionRole::Worker,
                            },
                            format!("{outgoing} is now the mission's worker."),
                        ));
                        // **Detached, never killed** (M10). It stays on the roster, its harness
                        // keeps running and its transcript is untouched; it is simply no longer
                        // the one that plans and asks. Telling it so is the whole of the detach —
                        // an agent that goes on coordinating a mission it no longer coordinates is
                        // the failure this sentence prevents.
                        notices.push((
                            outgoing,
                            "You are no longer this mission's coordinator — another agent has \
                             taken it over. You are still on the roster as a worker. Finish what \
                             you are doing, do not start anything new for the mission, and do not \
                             request phase moves."
                                .to_string(),
                        ));
                    }
                    if let Some(agent) = agent {
                        let event = match record
                            .roster
                            .iter_mut()
                            .find(|entry| entry.agent == agent && entry.left_at.is_none())
                        {
                            Some(entry) => {
                                entry.role = MissionRole::Coordinator;
                                JournalEvent::AgentRoleChanged {
                                    agent,
                                    role: MissionRole::Coordinator,
                                }
                            }
                            None => {
                                record.roster.push(RosterEntry::new(
                                    agent,
                                    MissionRole::Coordinator,
                                    now,
                                ));
                                JournalEvent::AgentJoined {
                                    agent,
                                    role: MissionRole::Coordinator,
                                }
                            }
                        };
                        lines.push(JournalEntry::new(
                            now,
                            Actor::User,
                            event,
                            format!("{agent} is now the mission's coordinator."),
                        ));
                        // The handoff briefing M10 asks for: it points at the four things, it does
                        // not copy them. A briefing that pasted the brief would be a briefing that
                        // goes stale the moment the brief is edited, and every one of the four is
                        // a tool call away on `ubiq-mission`.
                        notices.push((
                            agent,
                            "You are now this mission's coordinator. Start by reading it: \
                             read_brief for what it is for, ubiq-plan's read_plan for the plan, \
                             list_documents and read_document for what has been written down, and \
                             mission_overview for where it has got to and the latest journal. \
                             Pick up from there — do not start the mission again."
                                .to_string(),
                        ));
                    }
                    // The history is what makes "who has been the coordinator" answerable after a
                    // handover; the current value alone cannot say it.
                    record.coordinator_history.push(CoordinatorEntry {
                        agent,
                        at: now,
                        by: Actor::User,
                    });
                }
                changed
            }
            MissionField::RequirePlan(value) => {
                let changed = record.require_plan != value;
                record.require_plan = value;
                changed
            }
            MissionField::AutoRefine(value) => {
                let changed = record.auto_refine != value;
                record.auto_refine = value;
                changed
            }
            MissionField::Execution(mode) => {
                let changed = record.execution != mode;
                record.execution = mode;
                if changed {
                    // **Journaled** (M22). Starting auto mode is the single most consequential
                    // thing anybody does to a mission — every assignment after it is the
                    // scheduler's — so the switch is the first line of the story that follows.
                    lines.push(JournalEntry::new(
                        now,
                        Actor::User,
                        JournalEvent::ExecutionChanged { mode },
                        match mode {
                            ExecutionMode::Auto => "The mission is now run automatically: the \
                                                    scheduler assigns its ready tasks."
                                .to_string(),
                            ExecutionMode::Manual => "The mission is now run by hand: readiness \
                                                      is advisory and nothing assigns itself."
                                .to_string(),
                        },
                    ));
                }
                changed
            }
            MissionField::Parallelism(value) => {
                let value = value.max(1);
                let changed = record.parallelism != value;
                record.parallelism = value;
                changed
            }
            MissionField::OnFinish(value) => {
                let changed = record.on_finish != value;
                record.on_finish = value;
                changed
            }
            MissionField::MaxTasksPerAgent(value) => {
                let value = value.max(1);
                let changed = record.max_tasks_per_agent != value;
                record.max_tasks_per_agent = value;
                changed
            }
            MissionField::MaxAttempts(value) => {
                let value = value.max(1);
                let changed = record.max_attempts != value;
                record.max_attempts = value;
                changed
            }
            MissionField::SpawnPolicy(value) => {
                let changed = record.spawn_policy != value;
                record.spawn_policy = value;
                changed
            }
            MissionField::SpawnLimit(value) => {
                let value = value.max(1);
                let changed = record.spawn_limit != value;
                record.spawn_limit = value;
                changed
            }
            MissionField::AgentKinds(kinds) => {
                // The whole table, replaced, with unnamed rows dropped and a repeated name kept
                // once — `TaskField::Labels`' own trim.
                let mut seen = std::collections::HashSet::new();
                let kinds: Vec<_> = kinds
                    .into_iter()
                    .filter_map(|mut kind| {
                        kind.name = kind.name.trim().to_string();
                        (!kind.name.is_empty() && seen.insert(kind.name.clone())).then_some(kind)
                    })
                    .collect();
                let changed = record.agent_kinds != kinds;
                record.agent_kinds = kinds;
                changed
            }
            MissionField::DefaultKind(name) => {
                let name = name
                    .map(|name| name.trim().to_string())
                    .filter(|name| !name.is_empty());
                let changed = record.default_kind != name;
                record.default_kind = name;
                changed
            }
        };

        if !changed && !made {
            return Vec::new();
        }
        if changed {
            record.updated_at = now;
            if let Err(error) = self.store.save(&record) {
                return vec![Reply::Asker(self::error(
                    project,
                    Some(task),
                    error.to_string(),
                ))];
            }
        }
        let mut replies = Vec::new();
        for line in lines {
            replies.extend(self.journal(project, task, line));
        }
        for (agent, text) in notices {
            replies.extend(self.tell_agent(project, agent, text));
        }
        replies.push(Reply::Everyone(Message::MissionChanged {
            project_id: project,
            mission: Box::new(record),
        }));
        replies
    }

    // ── the phase (M4, M5) ──────────────────────────────────────────

    /// An agent asks for a phase move.
    ///
    /// Who asked is not the caller's to say: it is the mission's coordinator, read off the record,
    /// and [`Actor::Host`] when there is not one attached yet. Two moves have nothing at stake and
    /// are applied on the spot — Requirements → Refining under `auto_refine`, and the move into In
    /// progress when `require_plan` is off. Everything else becomes the mission's one pending
    /// request, which is what the panel's *Needs you* draws (M6) and what
    /// [`Self::set_phase`] answers.
    pub fn request_phase(
        &mut self,
        project: ProjectId,
        task: TaskId,
        phase: Phase,
        summary: String,
    ) -> Vec<Reply> {
        if let Some(refusal) = self.refusal(project, task) {
            return vec![Reply::Asker(error(project, Some(task), refusal))];
        }
        let (mut record, _) = match self.ensure(project, task) {
            Ok(found) => found,
            Err(reason) => return vec![Reply::Asker(error(project, Some(task), reason))],
        };
        if phase == record.phase {
            return Vec::new();
        }

        let by = record.coordinator.map_or(Actor::Host, Actor::Agent);
        let now = Utc::now();
        let automatic = match (record.phase, phase) {
            // Nothing is at stake in starting to refine, so the setting says whether to ask.
            (Phase::Requirements, Phase::Refining) => record.auto_refine,
            // Without the plan gate the coordinator moves itself into the work; the move is
            // journaled and broadcast, and the user can step it back at any time.
            (Phase::Requirements | Phase::Refining, Phase::InProgress) => !record.require_plan,
            _ => false,
        };
        if automatic {
            return self.apply(record, phase, by, now);
        }

        let summary = summary.trim().to_string();
        record.pending_phase = Some(PendingPhase {
            phase,
            by,
            summary: summary.clone(),
            at: now,
        });
        record.updated_at = now;
        let text = if summary.is_empty() {
            format!("Asked to move to {}.", phase.label())
        } else {
            format!("Asked to move to {}: {summary}", phase.label())
        };
        let mut replies = self.journal(
            project,
            task,
            JournalEntry::new(now, by, JournalEvent::PhaseRequested { phase }, text),
        );
        replies.extend(self.save_and_announce(record));
        replies
    }

    /// The user moves a mission's phase — a confirmation, a decline or a step back.
    ///
    /// The phase named is what tells the three apart: the pending request's phase confirms it, the
    /// phase the mission is already in declines it, and anything else is the user moving the
    /// mission where they want it. The plan gate is the only refusal, and only on the way in.
    pub fn set_phase(&mut self, project: ProjectId, task: TaskId, phase: Phase) -> Vec<Reply> {
        if let Some(refusal) = self.refusal(project, task) {
            return vec![Reply::Asker(error(project, Some(task), refusal))];
        }
        let (mut record, _) = match self.ensure(project, task) {
            Ok(found) => found,
            Err(reason) => return vec![Reply::Asker(error(project, Some(task), reason))],
        };
        let now = Utc::now();

        // The mission is already there: a decline of whatever was pending, or nothing at all.
        if phase == record.phase {
            let Some(pending) = record.pending_phase.take() else {
                return Vec::new();
            };
            record.updated_at = now;
            let mut replies = self.tell_coordinator(
                &record,
                format!(
                    "The user declined the move to {}. The mission stays in {}.",
                    pending.phase.label(),
                    record.phase.label()
                ),
            );
            replies.extend(self.save_and_announce(record));
            return replies;
        }

        // The plan gate: only with `require_plan`, only crossing forward into the work, never on
        // the way back out.
        let crossing_in = phase == Phase::InProgress && record.phase.order() < phase.order();
        if record.require_plan
            && crossing_in
            && let Some(refusal) = self.plan_gate_refusal(project, task)
        {
            return vec![Reply::Asker(error(project, Some(task), refusal))];
        }

        let answered = record
            .pending_phase
            .as_ref()
            .is_some_and(|pending| pending.phase == phase);
        let mut replies = if answered {
            self.tell_coordinator(
                &record,
                format!("The user confirmed the move to {}.", phase.label()),
            )
        } else {
            Vec::new()
        };
        replies.extend(self.apply(record, phase, Actor::User, now));
        replies
    }

    /// Why the plan gate refuses, if it does (M5).
    ///
    /// **An open annotation thread is a blocking one.** `AnnotationState` has exactly two states,
    /// `Open` and `Resolved`, and nothing anywhere marks a thread blocking or not — so "open
    /// blocking threads" is read as "open threads", which is what the plan surface already means
    /// by one (`D157`–`D161`).
    fn plan_gate_refusal(&self, project: ProjectId, task: TaskId) -> Option<String> {
        let body = self.plans.load(project, task).ok().flatten();
        if !body.is_some_and(|body| !body.trim().is_empty()) {
            return Some(
                "this mission's plan is empty — the plan is what the work is approved from"
                    .to_string(),
            );
        }
        let open = self
            .plans
            .load_sidecar(project, task)
            .ok()
            .flatten()
            .map_or(0, |sidecar| {
                sidecar
                    .annotations
                    .iter()
                    .filter(|annotation| annotation.is_open())
                    .count()
            });
        (open > 0).then(|| {
            format!(
                "this mission's plan has {open} open annotation thread{} — resolve them before the work starts",
                if open == 1 { "" } else { "s" }
            )
        })
    }

    /// Move the phase, whoever moved it: the history entry, the journal, the anchor's status and
    /// the broadcast, in that order. Any pending request is answered by the move and cleared.
    fn apply(
        &mut self,
        mut record: MissionRecord,
        phase: Phase,
        by: Actor,
        now: chrono::DateTime<Utc>,
    ) -> Vec<Reply> {
        record.phase = phase;
        let entry = PhaseEntry { phase, at: now, by };
        record.phase_history.push(entry.clone());
        record.pending_phase = None;
        record.updated_at = now;
        let mut replies = self.journal_phase(&record, &entry);
        replies.extend(self.save_and_announce(record));
        replies
    }

    /// **The journal seam (S3).** Every phase move calls this exactly once, with the record as it
    /// now stands and the entry that moved it, before anything is written or announced.
    fn journal_phase(&mut self, record: &MissionRecord, entry: &PhaseEntry) -> Vec<Reply> {
        self.journal(
            record.project_id,
            record.task_id,
            JournalEntry::new(
                entry.at,
                entry.by,
                JournalEvent::PhaseChanged { phase: entry.phase },
                format!("The mission moved to {}.", entry.phase.label()),
            ),
        )
    }

    // ── the journal (M12) ───────────────────────────────────────────

    /// Write one line and tell every window about it.
    ///
    /// **The one writer.** Everything that journals goes through here, so a line is written and
    /// announced in one act and no caller can do half of it. A journal that will not be written is
    /// logged and swallowed rather than failing the thing it was recording: a phase that moved and
    /// a line that could not be appended is still a phase that moved, and refusing the move would
    /// be the worse failure.
    fn journal(&mut self, project: ProjectId, task: TaskId, entry: JournalEntry) -> Vec<Reply> {
        match self.store.append_journal(project, task, entry) {
            Ok(entry) => vec![Reply::Everyone(Message::JournalAppended {
                project_id: project,
                task_id: task,
                entry,
            })],
            Err(error) => {
                tracing::warn!(%project, %task, "a mission journal line was lost: {error}");
                Vec::new()
            }
        }
    }

    /// One page of a mission's journal, newest first — [`Message::LoadJournal`]'s answer.
    ///
    /// No lazy record creation and no refusal beyond the anchor's: a journal is read, never
    /// written, so asking for one is not one of the three things that brings a record into being.
    pub fn load_journal(
        &mut self,
        project: ProjectId,
        task: TaskId,
        before: Option<u64>,
        limit: Option<usize>,
    ) -> Vec<Reply> {
        if let Some(refusal) = self.refusal(project, task) {
            return vec![Reply::Asker(error(project, Some(task), refusal))];
        }
        let limit = limit.unwrap_or(JOURNAL_PAGE).clamp(1, JOURNAL_MAX_PAGE);
        let (entries, more) = self.store.read_journal(project, task, before, limit);
        vec![Reply::Asker(Message::Journal {
            project_id: project,
            task_id: task,
            entries,
            more,
        })]
    }

    /// The tail of the journal, for the overview an agent reads. Not a message — a direct read,
    /// because the caller is a tool handler answering itself.
    pub fn journal_tail(
        &self,
        project: ProjectId,
        task: TaskId,
        limit: usize,
    ) -> Vec<JournalEntry> {
        self.store.read_journal(project, task, None, limit).0
    }

    /// An agent reports progress in its own words (M12, M16). The one journal line a tool writes
    /// directly.
    pub fn report_progress(
        &mut self,
        project: ProjectId,
        task: TaskId,
        by: Actor,
        about: Option<TaskId>,
        text: String,
    ) -> Vec<Reply> {
        let text = text.trim().to_string();
        if text.is_empty() {
            return Vec::new();
        }
        self.journal(
            project,
            task,
            JournalEntry::new(Utc::now(), by, JournalEvent::Progress { task: about }, text),
        )
    }

    /// The user said something to the mission — journaled where it happens, which is the
    /// coordinator's own thread (M12; the panel's *Feedback* line is the surface of it).
    pub fn feedback(&mut self, project: ProjectId, task: TaskId, text: String) -> Vec<Reply> {
        let text = text.trim().to_string();
        if text.is_empty() {
            return Vec::new();
        }
        self.journal(
            project,
            task,
            JournalEntry::new(Utc::now(), Actor::User, JournalEvent::Feedback, text),
        )
    }

    /// Feedback the coordinator has not read yet, and the watermark moved past it.
    ///
    /// **The watermark is in memory**, keyed by mission: it is a read position, not a fact about
    /// the mission, and a host that restarted has no coordinator left holding the old one either —
    /// so the first call of a run answers with everything and every call after it with what
    /// arrived since. Nothing on disk changes, which is why `read_feedback` is a read.
    pub fn take_feedback(&mut self, project: ProjectId, task: TaskId) -> Vec<JournalEntry> {
        let since = self.feedback_read.get(&(project, task)).copied();
        let next = self.store.journal_len(project, task);
        self.feedback_read.insert((project, task), next);
        let (mut lines, _) = self
            .store
            .read_journal(project, task, None, usize::MAX >> 1);
        lines.retain(|entry| {
            matches!(entry.event, JournalEvent::Feedback)
                && since.is_none_or(|since| entry.seq >= since)
        });
        lines
    }

    // ── the documents (M8) ──────────────────────────────────────────

    /// The names of a mission's documents, without the `.md` — what `list_documents` answers and
    /// what [`MissionRecord::documents`] carries to a window.
    pub fn documents(&self, project: ProjectId, task: TaskId) -> Vec<String> {
        self.store.documents(project, task)
    }

    /// Note that a document was written, and tell every window the list has changed.
    ///
    /// Two things in one act because they are one act: the journal line and the refreshed record,
    /// which is how a name reaches a window that never asked for a listing.
    pub fn document_written(
        &mut self,
        project: ProjectId,
        task: TaskId,
        by: Actor,
        name: &str,
    ) -> Vec<Reply> {
        let mut replies = self.journal(
            project,
            task,
            JournalEntry::new(
                Utc::now(),
                by,
                JournalEvent::DocumentWritten {
                    name: name.to_string(),
                },
                format!("Wrote the document '{name}'."),
            ),
        );
        if let Ok((record, _)) = self.ensure(project, task) {
            replies.push(Reply::Everyone(Message::MissionChanged {
                project_id: project,
                mission: Box::new(record),
            }));
        }
        replies
    }

    // ── the roster (M11) ────────────────────────────────────────────

    /// The mission an agent working `task` belongs to: the task itself when it is a mission, else
    /// its parent when *that* is (M11's first half — "assigned to the mission's task or one of its
    /// children"). Spawn membership is the other half and is not derivable; it is written into the
    /// roster at the spawn instead.
    pub fn mission_of_task(&mut self, project: ProjectId, task: TaskId) -> Option<TaskId> {
        let (_, tasks) = self.work.lock().tasks(project);
        let record = tasks.iter().find(|candidate| candidate.id == task)?;
        if record.level == Some(Level::Mission) {
            return Some(record.id);
        }
        let parent = record.parent?;
        tasks
            .iter()
            .find(|candidate| candidate.id == parent && candidate.level == Some(Level::Mission))
            .map(|candidate| candidate.id)
    }

    /// Put an agent on a mission's roster, or move it back in if it had left.
    ///
    /// Idempotent: an agent already in with the same role is no change and no line. A role that
    /// differs is a role change rather than a second joining, because an agent promoted to
    /// coordinator is the same member.
    pub fn join(
        &mut self,
        project: ProjectId,
        task: TaskId,
        agent: AgentId,
        role: MissionRole,
        by: Actor,
    ) -> Vec<Reply> {
        if self.refusal(project, task).is_some() {
            return Vec::new();
        }
        let (mut record, _) = match self.ensure(project, task) {
            Ok(found) => found,
            Err(_) => return Vec::new(),
        };
        let now = Utc::now();
        let (event, text) = match record
            .roster
            .iter_mut()
            .find(|entry| entry.agent == agent && entry.left_at.is_none())
        {
            Some(entry) if entry.role == role => return Vec::new(),
            Some(entry) => {
                entry.role = role;
                (
                    JournalEvent::AgentRoleChanged { agent, role },
                    format!("{} is now the mission's {}.", agent, role_label(role)),
                )
            }
            None => {
                record.roster.push(RosterEntry::new(agent, role, now));
                (
                    JournalEvent::AgentJoined { agent, role },
                    format!("{} joined the mission as {}.", agent, role_label(role)),
                )
            }
        };
        record.updated_at = now;
        let mut replies = self.journal(project, task, JournalEntry::new(now, by, event, text));
        replies.extend(self.save_and_announce(record));
        replies
    }

    /// Take an agent off a mission's roster. The row stays, stamped `left_at`, so the history can
    /// be read back — the whole reason the roster is stored rather than derived.
    pub fn leave(
        &mut self,
        project: ProjectId,
        task: TaskId,
        agent: AgentId,
        by: Actor,
    ) -> Vec<Reply> {
        let Some(mut record) = self.record(project, task) else {
            return Vec::new();
        };
        let now = Utc::now();
        let Some(entry) = record
            .roster
            .iter_mut()
            .find(|entry| entry.agent == agent && entry.left_at.is_none())
        else {
            return Vec::new();
        };
        entry.left_at = Some(now);
        // The mission's coordinator leaving is the mission having no coordinator: a name that
        // points at a member who is gone is worse than none.
        if record.coordinator == Some(agent) {
            record.coordinator = None;
            record.coordinator_history.push(CoordinatorEntry {
                agent: None,
                at: now,
                by,
            });
        }
        record.updated_at = now;
        let mut replies = self.journal(
            project,
            task,
            JournalEntry::new(
                now,
                by,
                JournalEvent::AgentLeft { agent },
                format!("{agent} left the mission."),
            ),
        );
        replies.extend(self.save_and_announce(record));
        replies
    }

    // ── spawning (M13) ──────────────────────────────────────────────

    /// A member asks the mission for another agent.
    ///
    /// **The host relays and the window launches.** Nothing here composes, resolves or mints
    /// anything: the request is written onto the record as a [`PendingSpawn`], journaled, and
    /// broadcast as [`Message::MissionSpawnRequest`] for the window that has the project open —
    /// which is the half that applies [`ubiq_proto::mission::SpawnPolicy`], picks the composition
    /// and mints the [`AgentId`]. The [`SpawnId`] comes back at once, so `spawn_agent` never waits
    /// on a launch.
    ///
    /// **The spawn policy is not read here.** Even `Never` is the window's refusal to make, because
    /// a refusal the user can see and override in the same row is worth more than one the host
    /// swallows — and splitting the policy across both halves is how the two come to disagree.
    ///
    /// The kind is the one thing checked, because it is the one thing the requester can get wrong
    /// in a way it can fix: an unknown name is refused with a sentence naming what there is, and a
    /// mission whose table is empty says so rather than pretending any name would do.
    #[allow(clippy::too_many_arguments)]
    pub fn request_spawn(
        &mut self,
        project: ProjectId,
        mission: TaskId,
        by: Actor,
        kind: &str,
        profile: Option<String>,
        task: Option<TaskId>,
        prompt: String,
        reason: String,
    ) -> Result<(SpawnId, Vec<Reply>), String> {
        let (mut record, mut replies) = self.touch(project, mission)?;
        let kind = self.resolve_kind(&record, kind, profile.as_deref())?;

        let now = Utc::now();
        let request = PendingSpawn {
            id: SpawnId::generate(),
            by,
            kind: kind.clone(),
            profile,
            task,
            prompt: prompt.trim().to_string(),
            reason: reason.trim().to_string(),
            // A member asking is never automatic: the policy is the window's to apply. Only the
            // scheduler's own request carries `auto`, and it builds one in [`Self::schedule_at`].
            auto: false,
            at: now,
        };
        let id = request.id;
        record.pending_spawns.push(request.clone());
        record.updated_at = now;

        let text = if request.reason.is_empty() {
            format!("Asked for a '{kind}' agent.")
        } else {
            format!("Asked for a '{kind}' agent: {}", request.reason)
        };
        replies.extend(self.journal(
            project,
            mission,
            JournalEntry::new(
                now,
                by,
                JournalEvent::SpawnRequested {
                    agent_kind: kind.clone(),
                },
                text,
            ),
        ));
        replies.extend(self.save_and_announce(record));
        // After the record, so a window drawing the row from the broadcast finds the request
        // already on the copy it holds.
        replies.push(Reply::Everyone(Message::MissionSpawnRequest {
            project_id: project,
            task_id: mission,
            request: Box::new(request),
        }));
        Ok((id, replies))
    }

    /// Which kind a request resolves to, or why it does not.
    ///
    /// `custom` is the one name that need not be in the table — it is M13's escape hatch for a
    /// request that names a profile outright — and it still names a profile, never a harness, an
    /// account or anything carrying credential material.
    fn resolve_kind(
        &self,
        record: &MissionRecord,
        kind: &str,
        profile: Option<&str>,
    ) -> Result<String, String> {
        let asked = kind.trim();
        if asked.eq_ignore_ascii_case("custom") {
            return match profile {
                Some(profile) if !profile.trim().is_empty() => Ok("custom".to_string()),
                _ => Err(
                    "the 'custom' kind needs a profile — name one, or call list_agent_kinds and \
                     ask for a kind this mission already has"
                        .to_string(),
                ),
            };
        }
        let asked = if asked.is_empty() {
            match record.default_kind.as_deref() {
                Some(default) => default,
                None if record.agent_kinds.len() == 1 => record.agent_kinds[0].name.as_str(),
                None => {
                    return Err(no_kinds(record));
                }
            }
        } else {
            asked
        };
        match record.kind_named(asked) {
            Some(found) => Ok(found.name.clone()),
            None if record.agent_kinds.is_empty() => Err(no_kinds(record)),
            None => Err(format!(
                "this mission has no agent kind called '{asked}' — it has {}. Call \
                 list_agent_kinds to read what each one is for.",
                known_kinds(record)
            )),
        }
    }

    /// What the window did with a spawn request: the roster, the journal, and the sentence the
    /// requester reads next.
    ///
    /// A request the record no longer holds is discarded rather than refused — a second window
    /// answering an already-answered row is harmless and is not a failure anybody can act on.
    pub fn answer_spawn(
        &mut self,
        project: ProjectId,
        mission: TaskId,
        request_id: SpawnId,
        outcome: SpawnOutcome,
    ) -> Vec<Reply> {
        let Some(mut record) = self.record(project, mission) else {
            return Vec::new();
        };
        let Some(position) = record
            .pending_spawns
            .iter()
            .position(|spawn| spawn.id == request_id)
        else {
            return Vec::new();
        };
        let request = record.pending_spawns.remove(position);
        let now = Utc::now();
        record.updated_at = now;

        // The kind **actually used**, which the user may have changed in the *Needs you* row: that
        // is what the requester is told and what the journal records, never what was asked for.
        let (event, line, answer) = match &outcome {
            SpawnOutcome::Launched { agent, kind } => (
                JournalEvent::SpawnAnswered {
                    agent_kind: kind.clone(),
                    agent: Some(*agent),
                },
                format!("Spawned {agent} as a '{kind}' agent."),
                format!(
                    "The agent you asked for was launched: {agent}, of kind '{kind}'. It is on \
                     the mission's roster — message_agent reaches it and list_agents shows it."
                ),
            ),
            SpawnOutcome::Declined { reason } => {
                let reason = reason.trim();
                let said = if reason.is_empty() {
                    "no reason was given".to_string()
                } else {
                    reason.to_string()
                };
                (
                    JournalEvent::SpawnAnswered {
                        agent_kind: request.kind.clone(),
                        agent: None,
                    },
                    format!("Declined the '{}' agent: {said}", request.kind),
                    format!(
                        "The '{}' agent you asked for was not launched: {said}. Carry on without \
                         it, or say why it is needed.",
                        request.kind
                    ),
                )
            }
        };
        let mut replies = self.journal(
            project,
            mission,
            JournalEntry::new(now, Actor::User, event, line),
        );
        replies.extend(self.save_and_announce(record));
        // The roster after the record, so `join`'s own save is the one that lands last and the
        // window's copy carries both the cleared request and the new member.
        if let SpawnOutcome::Launched { agent, .. } = &outcome {
            replies.extend(self.join(project, mission, *agent, MissionRole::Worker, Actor::User));
            // **A scheduler's spawn arrives already holding its task.** The slot was committed
            // when the request was made, so the roster has to say so the moment the agent exists
            // — otherwise the next wake sees a free slot and commits it twice. `join` reloads the
            // record, so this is written after it rather than onto the copy above.
            if request.auto
                && let Some(task) = request.task
            {
                replies.extend(self.hold(project, mission, *agent, task, now));
            }
        }
        // And the requester hears the outcome as its next prompt, which is the whole point of
        // having relayed rather than refused.
        if let Actor::Agent(requester) = request.by {
            replies.extend(self.tell_agent(project, requester, answer));
        }
        replies
    }

    // ── the scheduler (M22, M23) ────────────────────────────────────

    /// Run the mission's scheduler once, now. [`Self::schedule_at`] with the wall clock.
    ///
    /// **This is the whole entry point.** The coordinator calls it on the events it already
    /// receives — a task changed, an agent's activity or lifecycle changed, the mode or the
    /// parallelism changed — and it is a no-op for every mission that is not in auto *and* in the
    /// In-progress phase, so calling it more often than necessary costs a record read.
    pub fn schedule(&mut self, project: ProjectId, mission: TaskId) -> Vec<Reply> {
        self.schedule_at(project, mission, Utc::now())
    }

    /// One pass of the scheduler, at a stated time — [`scheduler::plan`] decided it and this
    /// carries it out.
    ///
    /// The clock is a parameter because the idle grace is the one rule that depends on it, and a
    /// grace-period rule that can only be tested by waiting is a rule nobody tests.
    ///
    /// Everything is applied against **one** copy of the record and saved once, so a pass that
    /// assigned three tasks writes `mission.toml` once and broadcasts one
    /// [`Message::MissionChanged`]. The journal lines go out as they are decided, which is the
    /// order they are read back in.
    pub fn schedule_at(
        &mut self,
        project: ProjectId,
        mission: TaskId,
        now: chrono::DateTime<Utc>,
    ) -> Vec<Reply> {
        let Some(mut record) = self.record(project, mission) else {
            return Vec::new();
        };
        if !record.scheduling() {
            return Vec::new();
        }
        let (tasks, agents) = {
            let mut board = self.work.lock();
            let (_, tasks) = board.tasks(project);
            let (_, agents) = board.agents(project);
            (tasks, agents)
        };
        let decisions = scheduler::plan(&record, &tasks, &agents, now);
        if decisions.is_empty() {
            return Vec::new();
        }

        let mut replies = Vec::new();
        // Said after the record is saved, for [`Self::set_field`]'s reason: nothing is told a
        // thing the record does not yet say.
        let mut notices: Vec<(AgentId, String)> = Vec::new();
        let mut coordinator_notices: Vec<String> = Vec::new();
        let mut lines: Vec<JournalEntry> = Vec::new();
        let mut requests: Vec<PendingSpawn> = Vec::new();
        let mut unload: Vec<AgentId> = Vec::new();

        for decision in decisions {
            match decision {
                scheduler::Decision::Finished { task, agent } => {
                    if let Some(entry) = record.member_mut(agent) {
                        entry.current_task = None;
                        entry.held_since = None;
                    }
                    lines.push(JournalEntry::new(
                        now,
                        Actor::Host,
                        JournalEvent::TaskFinished { task },
                        format!("{agent} finished {}.", task_name(&tasks, task)),
                    ));
                }
                scheduler::Decision::Release {
                    task,
                    agent,
                    attempt,
                } => {
                    record.note_attempt(task);
                    if let Some(agent) = agent
                        && let Some(entry) = record.member_mut(agent)
                    {
                        entry.current_task = None;
                        entry.held_since = None;
                    }
                    // `move_task` is the host's one status writer, here as everywhere.
                    replies.extend(
                        self.work
                            .lock()
                            .move_task(project, task, Status::Ready, None),
                    );
                    if let Some(agent) = agent {
                        replies.extend(self.work.lock().assign_agent(project, agent, None));
                    }
                    lines.push(JournalEntry::new(
                        now,
                        Actor::Host,
                        JournalEvent::TaskReleased {
                            task,
                            agent,
                            attempt,
                        },
                        format!("Released {}, attempt {attempt}.", task_name(&tasks, task)),
                    ));
                }
                scheduler::Decision::Block { task, attempts } => {
                    replies.extend(self.work.lock().move_task(
                        project,
                        task,
                        Status::Blocked,
                        None,
                    ));
                    lines.push(JournalEntry::new(
                        now,
                        Actor::Host,
                        JournalEvent::TaskBlocked { task, attempts },
                        format!(
                            "Blocked {} after {attempts} attempts.",
                            task_name(&tasks, task)
                        ),
                    ));
                    coordinator_notices.push(format!(
                        "The scheduler gave up on {}: {attempts} attempts ended without it moving \
                         off In progress, so it is now Blocked and the user can see it. Look at \
                         what it asks for — it may need splitting, re-labelling or a different \
                         kind of agent.",
                        task_name(&tasks, task)
                    ));
                }
                scheduler::Decision::Assign {
                    task,
                    agent,
                    affinity,
                } => {
                    let labels = labels_of(&tasks, task);
                    record.scheduler_done = false;
                    if let Some(entry) = record.member_mut(agent) {
                        entry.scheduled = true;
                        entry.current_task = Some(task);
                        entry.held_since = Some(now);
                        entry.last_held_at = Some(now);
                        entry.tasks_held += 1;
                        entry.learn(&labels);
                    }
                    replies.extend(self.work.lock().assign_agent(project, agent, Some(task)));
                    lines.push(JournalEntry::new(
                        now,
                        Actor::Host,
                        JournalEvent::TaskScheduled {
                            task,
                            agent,
                            labels: affinity.clone(),
                        },
                        format!(
                            "Scheduled {} → {agent}{}.",
                            task_name(&tasks, task),
                            affinity_note(&affinity)
                        ),
                    ));
                    notices.push((agent, briefing(&tasks, task)));
                }
                scheduler::Decision::Spawn { task, kind } => {
                    record.scheduler_done = false;
                    let request = PendingSpawn {
                        id: SpawnId::generate(),
                        by: Actor::Host,
                        kind: kind.clone(),
                        profile: None,
                        task: Some(task),
                        prompt: briefing(&tasks, task),
                        reason: format!(
                            "The mission is in auto mode and {} is ready with nobody free to \
                             take it.",
                            task_name(&tasks, task)
                        ),
                        // **The consent was given when auto was chosen** (M23), so the window
                        // does not ask again. `parallelism` is what caps this instead, and it was
                        // applied before the decision was made.
                        auto: true,
                        at: now,
                    };
                    record.pending_spawns.push(request.clone());
                    lines.push(JournalEntry::new(
                        now,
                        Actor::Host,
                        JournalEvent::SpawnRequested {
                            agent_kind: kind.clone(),
                        },
                        format!(
                            "Asked for a '{kind}' agent for {}.",
                            task_name(&tasks, task)
                        ),
                    ));
                    requests.push(request);
                }
                scheduler::Decision::Retire { agent } => {
                    if let Some(entry) = record.member_mut(agent) {
                        entry.current_task = None;
                        entry.held_since = None;
                    }
                    lines.push(JournalEntry::new(
                        now,
                        Actor::Host,
                        JournalEvent::AgentRetired { agent },
                        format!("Retired {agent}."),
                    ));
                    unload.push(agent);
                }
                scheduler::Decision::Done => {
                    record.scheduler_done = true;
                    lines.push(JournalEntry::new(
                        now,
                        Actor::Host,
                        JournalEvent::SchedulerFinished,
                        "Every task in the breakdown is in review or done.".to_string(),
                    ));
                    coordinator_notices.push(
                        "Every task in this mission's breakdown is in review or done, so the \
                         scheduler has stopped. Check the work over and, if it is what the brief \
                         asked for, call request_phase for 'completed' with a summary."
                            .to_string(),
                    );
                }
            }
        }

        record.updated_at = now;
        for line in lines {
            replies.extend(self.journal(project, mission, line));
        }
        replies.extend(self.save_and_announce(record.clone()));
        for request in requests {
            replies.push(Reply::Everyone(Message::MissionSpawnRequest {
                project_id: project,
                task_id: mission,
                request: Box::new(request),
            }));
        }
        for (agent, text) in notices {
            replies.extend(self.tell_agent(project, agent, text));
        }
        for text in coordinator_notices {
            replies.extend(self.tell_coordinator(&record, text));
        }
        // **Unloaded, never ended** — the conversation stays resumable, which is the whole
        // difference between retiring an agent and throwing its transcript away. Said into the
        // host's own inbox for [`Self::tell_agent`]'s reason: only the run loop holds the
        // conversation map.
        for agent in unload {
            self.voice
                .say(Message::UnloadConversation { agent_id: agent });
        }
        replies
    }

    /// Write onto the roster that an agent now holds a task for the scheduler.
    ///
    /// Its own method because two paths reach it: an [`scheduler::Decision::Assign`] above, and a
    /// scheduler's spawn coming back through [`Self::answer_spawn`] — and a slot recorded in only
    /// one of them is a slot committed twice.
    fn hold(
        &mut self,
        project: ProjectId,
        mission: TaskId,
        agent: AgentId,
        task: TaskId,
        now: chrono::DateTime<Utc>,
    ) -> Vec<Reply> {
        let Some(mut record) = self.record(project, mission) else {
            return Vec::new();
        };
        let (labels, name) = {
            let (_, tasks) = self.work.lock().tasks(project);
            (labels_of(&tasks, task), task_name(&tasks, task))
        };
        let Some(entry) = record.member_mut(agent) else {
            return Vec::new();
        };
        entry.scheduled = true;
        entry.current_task = Some(task);
        entry.held_since = Some(now);
        entry.last_held_at = Some(now);
        entry.tasks_held += 1;
        entry.learn(&labels);
        record.updated_at = now;
        let mut replies = self.work.lock().assign_agent(project, agent, Some(task));
        replies.extend(self.journal(
            project,
            mission,
            JournalEntry::new(
                now,
                Actor::Host,
                JournalEvent::TaskScheduled {
                    task,
                    agent,
                    labels: labels.clone(),
                },
                format!("Scheduled {name} → {agent} (new agent)."),
            ),
        ));
        replies.extend(self.save_and_announce(record));
        replies
    }

    /// Record the agent kind the planner suggested for each of a batch's tasks (M21).
    ///
    /// **On the mission, not on the card.** A mission's agent kinds are its own vocabulary, and a
    /// board field naming one would be a field nothing outside that mission can read. One save for
    /// the whole batch, because a breakdown is written in one call.
    pub fn set_task_kinds(
        &mut self,
        project: ProjectId,
        mission: TaskId,
        kinds: Vec<(TaskId, Option<String>)>,
    ) -> Vec<Reply> {
        // Through `touch`, not `record`: a coordinator writing the breakdown may be the first
        // thing that ever opens the mission, and a suggested kind dropped because no record
        // existed yet is a scheduler that later spawns the wrong agent.
        let Ok((mut record, mut replies)) = self.touch(project, mission) else {
            return Vec::new();
        };
        for (task, kind) in kinds {
            record.set_task_kind(task, kind);
        }
        record.updated_at = Utc::now();
        replies.extend(self.save_and_announce(record));
        replies
    }

    /// What the scheduler would do right now, without doing any of it — what `mission_overview`
    /// reports as the pool and the slots (M25).
    ///
    /// A read, so it creates nothing and journals nothing: a coordinator asking what auto mode is
    /// up to must not be the thing that makes it act.
    pub fn scheduler_view(&mut self, project: ProjectId, mission: TaskId) -> SchedulerView {
        let Some(record) = self.record(project, mission) else {
            return SchedulerView::default();
        };
        let (tasks, agents) = {
            let mut board = self.work.lock();
            let (_, tasks) = board.tasks(project);
            let (_, agents) = board.agents(project);
            (tasks, agents)
        };
        let pool = tasks
            .iter()
            .filter(|task| {
                task.parent == Some(mission)
                    && task.status == Status::Ready
                    && task.ready(&tasks)
                    && !agents.iter().any(|agent| agent.task == Some(task.id))
            })
            .map(|task| (task.id, task.title.clone()))
            .collect();
        let holding: Vec<(AgentId, TaskId)> = record
            .scheduler_members()
            .filter_map(|entry| entry.current_task.map(|task| (entry.agent, task)))
            .collect();
        let used = holding.len()
            + record
                .pending_spawns
                .iter()
                .filter(|spawn| spawn.auto)
                .count();
        SchedulerView {
            auto: record.execution == ExecutionMode::Auto,
            running: record.scheduling(),
            parallelism: record.parallelism,
            free_slots: record.parallelism.saturating_sub(used),
            pool,
            holding,
        }
    }

    /// Note that a task was created inside a mission (M12).
    pub fn task_created(
        &mut self,
        project: ProjectId,
        task: TaskId,
        by: Actor,
        child: TaskId,
        title: &str,
    ) -> Vec<Reply> {
        self.journal(
            project,
            task,
            JournalEntry::new(
                Utc::now(),
                by,
                JournalEvent::TaskCreated { task: child },
                format!("Created the task '{title}'."),
            ),
        )
    }

    /// Write the record, derive the anchor's status from it (M4) and broadcast the result.
    ///
    /// The status derivation is the whole of why a phase can be inferred lazily and still agree
    /// with the board: `Phase::anchor_status`, with In progress reading `Blocked` while something
    /// is waiting on the user — §3's table, and the only part of it the phase alone cannot say.
    fn save_and_announce(&mut self, record: MissionRecord) -> Vec<Reply> {
        if let Err(error) = self.store.save(&record) {
            return vec![Reply::Asker(self::error(
                record.project_id,
                Some(record.task_id),
                error.to_string(),
            ))];
        }
        let status = if record.phase == Phase::InProgress && record.pending_phase.is_some() {
            Status::Blocked
        } else {
            record.phase.anchor_status()
        };
        // `move_task` is the host's one status writer — it keeps the column's order right, which a
        // bare field write would not. `None` drops the card at the end of its new column.
        let mut replies =
            self.work
                .lock()
                .move_task(record.project_id, record.task_id, status, None);
        replies.push(Reply::Everyone(Message::MissionChanged {
            project_id: record.project_id,
            mission: Box::new(record),
        }));
        replies
    }

    /// Say something to one agent: its row **and**, if it is live, its conversation.
    ///
    /// **Both halves, always.** The row is the durable copy — it is what the agent's column draws
    /// and what survives a harness that is parked or has not launched yet — and the prompt is what
    /// actually reaches a model. Neither alone is delivery: a row with no prompt is a line nobody
    /// reads, and a prompt with no row is a line nobody can look back at.
    ///
    /// The prompt goes out through [`Self::voice`] rather than being driven here, because this is
    /// called from the MCP listener's thread as often as from the run loop's, and only the run loop
    /// holds the conversation map. A conversation that has not launched gets the row and nothing
    /// else — a mission line is not a reason to start a harness the user has not started.
    fn tell_agent(&mut self, project: ProjectId, agent: AgentId, text: String) -> Vec<Reply> {
        let text = text.trim().to_string();
        if text.is_empty() {
            return Vec::new();
        }
        let replies = self.work.lock().send_to_agent(project, agent, text.clone());
        self.voice.say(Message::PromptAgent {
            agent_id: agent,
            text,
        });
        replies
    }

    /// Put the user's answer in the coordinator's thread and in front of it, so it is the next
    /// thing it reads (M5). A mission with no coordinator attached has nobody to tell.
    fn tell_coordinator(&mut self, record: &MissionRecord, text: String) -> Vec<Reply> {
        let Some(agent) = record.coordinator else {
            return Vec::new();
        };
        self.tell_agent(record.project_id, agent, text)
    }

    /// Say something to a roster member, for `message_agent` (M16).
    ///
    /// **Roster members only** — an agent that can message any agent in the project is an agent
    /// that can reach outside its own mission, which is the one thing the mission's identity model
    /// is for. The check is here rather than in the tool so the rule has one reading.
    pub fn message_member(
        &mut self,
        project: ProjectId,
        mission: TaskId,
        agent: AgentId,
        text: String,
    ) -> Result<Vec<Reply>, String> {
        let (record, mut replies) = self.touch(project, mission)?;
        if record.member(agent).is_none() {
            return Err(
                "that agent is not on this mission's roster — list_agents says who is".to_string(),
            );
        }
        replies.extend(self.tell_agent(project, agent, text));
        Ok(replies)
    }

    /// Whether this task carries a mission — the question the board's drag asks before deciding
    /// whether a column change is a status write or a phase move.
    pub fn is_mission(&mut self, project: ProjectId, task: TaskId) -> bool {
        let (_, tasks) = self.work.lock().tasks(project);
        tasks
            .iter()
            .any(|record| record.id == task && record.level.is_some())
    }

    /// Why this mission may not be demoted away, if there is a reason (M3).
    ///
    /// A record nobody has been in and nothing has been written to is thrown away with the
    /// demotion; one with a roster or a journal is what the sentence protects. No record at all is
    /// no refusal — there is nothing to lose.
    pub fn demotion_refusal(&self, project: ProjectId, task: TaskId) -> Option<String> {
        let record = self.record(project, task)?;
        if record.has_members() {
            return Some(
                "this mission has agents in it — remove them before it stops being a mission"
                    .to_string(),
            );
        }
        if self.store.has_journal(project, task) {
            return Some(
                "this mission has a journal — it cannot stop being a mission without losing it"
                    .to_string(),
            );
        }
        None
    }

    /// Drop a mission's whole directory. Said only when there was something there: a task with no
    /// record deleted or demoted is not news.
    pub fn delete(&mut self, project: ProjectId, task: TaskId) -> Vec<Reply> {
        let existed = self.store.path(project, task).exists();
        if let Err(reason) = self.store.delete(project, task) {
            return vec![Reply::Asker(error(project, Some(task), reason.to_string()))];
        }
        if !existed {
            return Vec::new();
        }
        vec![Reply::Everyone(Message::MissionDeleted {
            project_id: project,
            task_id: task,
        })]
    }
}

/// The sentence a mission with no agent kinds answers a spawn request with.
///
/// It says what is true — the table is empty — rather than inventing a kind, because the table is
/// the user's to seed from the project's profiles and nothing the host could guess belongs in it.
fn no_kinds(record: &MissionRecord) -> String {
    if record.agent_kinds.is_empty() {
        return "this mission has no agent kinds yet, so there is nothing to spawn — a person has \
                to add them in the mission panel. Say what kind of agent you need and why."
            .to_string();
    }
    format!(
        "this mission names no default agent kind — ask for one of {} by name.",
        known_kinds(record)
    )
}

/// The mission's agent kinds, quoted, for a sentence a model reads.
fn known_kinds(record: &MissionRecord) -> String {
    record
        .agent_kinds
        .iter()
        .map(|kind| format!("'{}'", kind.name))
        .collect::<Vec<_>>()
        .join(", ")
}

/// What the scheduler is doing right now, for a reader rather than for the scheduler (M25).
///
/// Not a message: it is what [`crate::mcp::mission`] turns into the `scheduler` block of
/// `mission_overview`, which is how an agent — coordinator or worker — asks what auto mode is
/// holding and what is waiting.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SchedulerView {
    /// Whether the mission is set to auto at all.
    pub auto: bool,
    /// Whether it is actually running — auto **and** In progress (M22).
    pub running: bool,
    pub parallelism: usize,
    pub free_slots: usize,
    /// The ready, unassigned children the scheduler would draw from, with their titles.
    pub pool: Vec<(TaskId, String)>,
    /// Who holds what, of the scheduler's own agents.
    pub holding: Vec<(AgentId, TaskId)>,
}

/// What a task is called in a journal line a person reads: the key they say out loud, else the
/// title. Never the raw id — a journal nobody can match to a card is a journal nobody reads.
fn task_name(tasks: &[TaskRecord], task: TaskId) -> String {
    match tasks.iter().find(|record| record.id == task) {
        Some(record) => record
            .key
            .clone()
            .unwrap_or_else(|| format!("'{}'", record.title)),
        None => task.to_string(),
    }
}

/// One task's label names — the affinity vocabulary (M24). Nothing new on the task: this is the
/// list the board already carries.
fn labels_of(tasks: &[TaskRecord], task: TaskId) -> Vec<String> {
    tasks
        .iter()
        .find(|record| record.id == task)
        .map(|record| {
            record
                .labels
                .iter()
                .map(|label| label.name.clone())
                .collect()
        })
        .unwrap_or_default()
}

/// The *(affinity ui, api)* half of a scheduled line, or nothing when the agent was fresh.
fn affinity_note(labels: &[String]) -> String {
    if labels.is_empty() {
        String::new()
    } else {
        format!(" (affinity {})", labels.join(", "))
    }
}

/// What an agent is told when the scheduler hands it a task.
///
/// It names the task and points at the tools rather than pasting the brief: a briefing that copied
/// the description goes stale the moment the card is edited, and `get_task` is one call away.
fn briefing(tasks: &[TaskRecord], task: TaskId) -> String {
    let record = tasks.iter().find(|record| record.id == task);
    let title = record.map_or_else(String::new, |record| record.title.clone());
    format!(
        "You have been given the task {} — \"{title}\". Read it with use-task's get_task ({task}), \
         then work it. Move it to In progress when you start and to In review when it is done, \
         with change_state; report anything a person has to decide with ubiq-mission's \
         report_progress. Do not pick up anything else from this mission.",
        task_name(tasks, task),
    )
}

/// What a role is called in a journal line a person reads.
fn role_label(role: MissionRole) -> &'static str {
    match role {
        MissionRole::Coordinator => "coordinator",
        MissionRole::Worker => "worker",
    }
}

/// The family's one failure message.
fn error(project: ProjectId, task: Option<TaskId>, error: String) -> Message {
    Message::MissionError {
        project_id: project,
        task_id: task,
        error,
    }
}
