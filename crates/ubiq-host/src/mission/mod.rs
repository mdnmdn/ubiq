//! A project's missions, as the host runs them.
//!
//! **A mission is its parent task.** Everything here is keyed by the `(ProjectId, TaskId)` of a
//! task carrying [`Level::Mission`]: the phase is its status ([`Phase::of`]), the settings are
//! [`TaskRecord::mission`], and the log is the [`RunEvent`] comments on the tasks concerned. There
//! is no record of its own and no journal; what a window draws ([`MissionSnapshot`]) is derived
//! on every pass and never stored.
//!
//! **The runner is deterministic Ubiq code** ([`runner`]): a pure pass over the tree, carried out
//! here on the coordinator's thread — no thread, no timer, no LLM coordinator. It cannot start a
//! conversation (only a window can, and the window mints the [`AgentId`]), so a launch is a
//! [`Message::MissionLaunch`] addressed to **one** window, answered by
//! [`Message::MissionLaunched`] (contract §3.3).
//!
//! What lives only in memory — the launches in flight, the window to address them to, the tasks
//! whose launch failed, the feedback waiting for a busy worker, and the last snapshot sent — is
//! lost on a restart by design: nothing relaunches on its own.
//!
//! Like [`crate::plan::Plans`] this is held behind a [`Handle`] so the coordinator and the
//! `ubiq-mission` MCP listener share one instance (`D120`). Only the coordinator runs passes; the
//! listener writes the tree and says [`Message::MissionSchedule`] to wake it.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};

use ubiq_proto::bus::{ClientId, Voice};
use ubiq_proto::ids::{ProjectId, SpawnId, TaskId};
use ubiq_proto::messages::{Message, TaskField};
use ubiq_proto::mission::{
    HistoryEntry, HistoryKind, InitialPlan, LaunchOutcome, LaunchRequest, LaunchRole,
    MissionAction, MissionField, MissionSnapshot, MissionTaskAction, NewMission, Phase, RunEvent,
    is_human,
};
use ubiq_proto::work::{AgentId, Attachment, Level, Status, TaskRecord};

use crate::mcp::catalogue;
use crate::plan::{Saver, Target};
use crate::reply::Reply;
use crate::store::plan::FilePlanStore;
use crate::work;

pub mod prompts;
pub mod runner;

use runner::{AUTO_FAIL_CAP, Step, World, settings_of};

/// The mission document the guidance composer appends to.
pub const GUIDANCE: &str = "guidance";

/// How many times a pass may re-run over its own writes before it stops — a verdict or a phase
/// move changes what the next pass sees, and a bound keeps a bug from looping the run loop.
const MAX_PASSES: usize = 6;

/// A launch asked of a window and not answered yet.
struct PendingLaunch {
    project: ProjectId,
    mission: TaskId,
    task: TaskId,
    role: LaunchRole,
    retry: bool,
    client: ClientId,
}

/// What a caller on the coordinator's thread knows that this module cannot: which agents are
/// live, and which window a launch can be addressed to.
pub struct Ctx<'a> {
    pub live: &'a dyn Fn(AgentId) -> bool,
    pub client: Option<ClientId>,
}

/// What a mission operation wants done. [`Reply`]s are addressed as usual; a launch goes to the
/// one client named beside it; an unload is the coordinator's to perform through the agent's
/// owner.
#[derive(Default)]
pub struct Effects {
    pub replies: Vec<Reply>,
    pub launches: Vec<(ClientId, Message)>,
    pub unload: Vec<AgentId>,
}

impl Effects {
    fn error(project: ProjectId, task: Option<TaskId>, error: String) -> Self {
        Self {
            replies: vec![Reply::Asker(self::error(project, task, error))],
            ..Self::default()
        }
    }
}

/// One project's missions.
pub struct Missions {
    work: work::Handle,
    /// Reads of a plan body and its sidecar (open threads, revision log) — a path calculator.
    plan_store: FilePlanStore,
    /// Writes through the plan service: the first revision of a plan, the guidance document.
    plans: crate::plan::Handle,
    /// The config root: where the docs directory and the agent definitions are read from.
    root: PathBuf,
    /// How the MCP listener's half wakes the runner: [`Message::MissionSchedule`] said into the
    /// host's own inbox (`D138`).
    voice: Voice,
    launches: HashMap<SpawnId, PendingLaunch>,
    /// The window that last sent a mission message for each mission.
    launch_client: HashMap<(ProjectId, TaskId), ClientId>,
    /// Tasks whose launch failed, until a manual Start or Retry.
    launch_failed: HashSet<(ProjectId, TaskId)>,
    /// Feedback and guidance waiting for a live worker to be between turns.
    feedback_queue: HashMap<AgentId, (ProjectId, Vec<String>)>,
    /// The last snapshot broadcast per mission, so an unchanged one is not sent again.
    sent: HashMap<(ProjectId, TaskId), MissionSnapshot>,
    /// Projects whose restart scan has run.
    scanned: HashSet<ProjectId>,
    /// The agent definition ids each project can see, read once and dropped by
    /// [`Missions::definitions_changed`].
    definitions: HashMap<ProjectId, Vec<String>>,
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
        work: work::Handle,
        plans: crate::plan::Handle,
        root: PathBuf,
        voice: Voice,
    ) -> Self {
        Self {
            work,
            plan_store: FilePlanStore::new(root.clone()),
            plans,
            root,
            voice,
            launches: HashMap::new(),
            launch_client: HashMap::new(),
            launch_failed: HashSet::new(),
            feedback_queue: HashMap::new(),
            sent: HashMap::new(),
            scanned: HashSet::new(),
            definitions: HashMap::new(),
        }
    }

    // ── reading ─────────────────────────────────────────────────────────

    fn tasks(&self, project: ProjectId) -> Vec<TaskRecord> {
        self.work.lock().tasks(project).1
    }

    /// The mission's parent and the project's tasks, or why `task` is not a mission.
    fn refusal(
        &self,
        project: ProjectId,
        task: TaskId,
    ) -> Result<(TaskRecord, Vec<TaskRecord>), String> {
        let tasks = self.tasks(project);
        let Some(parent) = tasks.iter().find(|t| t.id == task).cloned() else {
            return Err("no such task".to_string());
        };
        if parent.level != Some(Level::Mission) {
            return Err("that task is not a mission".to_string());
        }
        Ok((parent, tasks))
    }

    /// The mission a task belongs to: itself when it is one, else its parent when that is.
    pub fn mission_of_task(&self, project: ProjectId, task: TaskId) -> Option<TaskId> {
        let tasks = self.tasks(project);
        mission_in(&tasks, task)
    }

    /// Whether this task carries a mission.
    pub fn is_mission(&self, project: ProjectId, task: TaskId) -> bool {
        self.tasks(project)
            .iter()
            .any(|t| t.id == task && t.level == Some(Level::Mission))
    }

    /// The names of a mission's documents, without the `.md`, sorted.
    pub fn documents(&self, project: ProjectId, task: TaskId) -> Vec<String> {
        let dir = crate::store::project_dir::ProjectData::under_config(&self.root, project)
            .mission_docs(task);
        let Ok(entries) = std::fs::read_dir(dir) else {
            return Vec::new();
        };
        let mut names: Vec<String> = entries
            .flatten()
            .filter_map(|entry| {
                let name = entry.file_name().to_str()?.to_string();
                name.strip_suffix(".md").map(str::to_string)
            })
            .collect();
        names.sort();
        names
    }

    /// A document was written from the MCP listener: wake the runner so the snapshot (and its
    /// document list) goes out again.
    pub fn document_written(&self, project: ProjectId, task: TaskId) {
        self.voice.say(Message::MissionSchedule {
            project_id: project,
            task_id: task,
        });
    }

    /// The planner's conversation, for a comment addressed to the agent on a mission's plan.
    pub fn planner_of(&self, project: ProjectId, task: TaskId) -> Option<AgentId> {
        self.tasks(project)
            .iter()
            .find(|t| t.id == task && t.level == Some(Level::Mission))
            .and_then(|t| settings_of(t).planner_agent)
    }

    /// Every mission an agent is the planner, a worker or the final reviewer of.
    pub fn missions_of_agent(&self, project: ProjectId, agent: AgentId) -> Vec<TaskId> {
        let tasks = self.tasks(project);
        let mut out = Vec::new();
        for t in &tasks {
            let mission = if t.worker == Some(agent) {
                mission_in(&tasks, t.id)
            } else if t.level == Some(Level::Mission)
                && (settings_of(t).planner_agent == Some(agent)
                    || runner::mission_reviewer(t, &|a| a == agent).is_some())
            {
                Some(t.id)
            } else {
                None
            };
            if let Some(mission) = mission
                && !out.contains(&mission)
            {
                out.push(mission);
            }
        }
        out
    }

    /// Every agent the mission has launched, live or not — what the coordinator looks an owner
    /// window up from when no window has spoken for the mission yet.
    pub fn agents_of(&self, project: ProjectId, mission: TaskId) -> Vec<AgentId> {
        let tasks = self.tasks(project);
        let mut out: Vec<AgentId> = tasks
            .iter()
            .filter(|t| t.parent == Some(mission))
            .filter_map(|t| t.worker)
            .collect();
        if let Some(parent) = tasks.iter().find(|t| t.id == mission) {
            out.extend(settings_of(parent).planner_agent);
            out.extend(runner::mission_reviewer(parent, &|_| true));
        }
        out
    }

    /// Remember which window spoke for a mission last — where its launches go.
    pub fn note_client(&mut self, project: ProjectId, mission: TaskId, client: ClientId) {
        if !client.is_host_voice() {
            self.launch_client.insert((project, mission), client);
        }
    }

    /// The window that last spoke for the mission.
    pub fn client_of(&self, project: ProjectId, mission: TaskId) -> Option<ClientId> {
        self.launch_client.get(&(project, mission)).copied()
    }

    /// A window has gone: its launches will never be answered, and it can be addressed no more.
    pub fn client_gone(&mut self, client: ClientId) {
        self.launches.retain(|_, launch| launch.client != client);
        self.launch_client.retain(|_, held| *held != client);
    }

    /// Why this mission may not be demoted, if there is a reason.
    pub fn demotion_refusal(
        &self,
        project: ProjectId,
        task: TaskId,
        live: &dyn Fn(AgentId) -> bool,
    ) -> Option<String> {
        let (parent, tasks) = self.refusal(project, task).ok()?;
        if tasks.iter().any(|t| t.parent == Some(task)) {
            return Some(
                "this mission has tasks in it — move them out before it stops being a mission"
                    .to_string(),
            );
        }
        let no = HashSet::new();
        let world = World {
            parent: &parent,
            tasks: &tasks,
            live,
            in_flight: &no,
            launch_failed: &no,
            definitions: &[],
        };
        if !runner::agents(&world).is_empty() {
            return Some(
                "this mission has a live agent — stop it before it stops being a mission"
                    .to_string(),
            );
        }
        None
    }

    /// A mission was demoted (its settings are cleared) or deleted (its docs directory goes too).
    pub fn gone(&mut self, project: ProjectId, task: TaskId, deleted: bool) -> Vec<Reply> {
        let mut replies = Vec::new();
        if deleted {
            let dir = crate::store::project_dir::ProjectData::under_config(&self.root, project)
                .missions()
                .join(task.to_string());
            let _ = std::fs::remove_dir_all(dir);
        } else {
            replies.extend(
                self.work
                    .lock()
                    .edit(project, task, |t| t.mission.take().is_some()),
            );
        }
        self.sent.remove(&(project, task));
        self.launches
            .retain(|_, l| !(l.project == project && l.mission == task));
        replies.push(Reply::Everyone(Message::MissionDeleted {
            project_id: project,
            task_id: task,
        }));
        replies
    }

    /// The definition ids a project can see — what tells a definition from a person in
    /// `assigned_to` — cached per project.
    fn definition_ids(&mut self, project: ProjectId) -> Vec<String> {
        let root = &self.root;
        self.definitions
            .entry(project)
            .or_insert_with(|| crate::agent::Agents::definition_ids(root, project))
            .clone()
    }

    /// An agent definition was saved, cloned or deleted: read the ids again next time.
    pub fn definitions_changed(&mut self) {
        self.definitions.clear();
    }

    // ── the feedback queue ──────────────────────────────────────────────

    fn deliver(&mut self, project: ProjectId, agent: AgentId, text: String) {
        self.feedback_queue
            .entry(agent)
            .or_insert_with(|| (project, Vec::new()))
            .1
            .push(text);
    }

    /// Agents with feedback waiting.
    pub fn queued_agents(&self) -> Vec<AgentId> {
        self.feedback_queue.keys().copied().collect()
    }

    /// Take an agent's waiting feedback, to deliver or to drop.
    pub fn take_feedback(&mut self, agent: AgentId) -> Option<(ProjectId, Vec<String>)> {
        self.feedback_queue.remove(&agent)
    }

    // ── the listing and the restart scan ────────────────────────────────

    /// Every mission in the project, as snapshots — answered to the asker. The first listing of
    /// a project after a start is the restart scan (contract §3.5): a pass with launches off over
    /// every mission, which writes the `Gone` lines and derives *Worker gone* and *Planner
    /// stopped*; nothing is launched.
    pub fn list(&mut self, project: ProjectId, ctx: &Ctx) -> Effects {
        let mut eff = Effects::default();
        let missions: Vec<TaskId> = self
            .tasks(project)
            .iter()
            .filter(|t| t.level == Some(Level::Mission))
            .map(|t| t.id)
            .collect();
        if self.scanned.insert(project) {
            for mission in &missions {
                self.run_into(&mut eff, project, *mission, ctx, false);
            }
        }
        let tasks = self.tasks(project);
        let mut snapshots = Vec::new();
        for mission in missions {
            if let Some(parent) = tasks.iter().find(|t| t.id == mission) {
                let snapshot = self.snapshot_of(project, parent, &tasks, ctx);
                self.sent.insert((project, mission), snapshot.clone());
                snapshots.push(snapshot);
            }
        }
        eff.replies.push(Reply::Asker(Message::MissionList {
            project_id: project,
            missions: snapshots,
        }));
        eff
    }

    // ── the pass ────────────────────────────────────────────────────────

    /// Run the mission's runner now, and broadcast the snapshot if it changed.
    pub fn run(&mut self, project: ProjectId, mission: TaskId, ctx: &Ctx) -> Effects {
        let mut eff = Effects::default();
        self.run_into(&mut eff, project, mission, ctx, true);
        eff
    }

    fn sets(&self, project: ProjectId, mission: TaskId) -> (HashSet<TaskId>, HashSet<TaskId>) {
        let in_flight = self
            .launches
            .values()
            .filter(|l| l.project == project && l.mission == mission)
            .map(|l| l.task)
            .collect();
        let failed = self
            .launch_failed
            .iter()
            .filter(|(p, _)| *p == project)
            .map(|(_, t)| *t)
            .collect();
        (in_flight, failed)
    }

    fn run_into(
        &mut self,
        eff: &mut Effects,
        project: ProjectId,
        mission: TaskId,
        ctx: &Ctx,
        launch: bool,
    ) {
        let definitions = self.definition_ids(project);
        let mut unloaded: HashSet<AgentId> = HashSet::new();
        for _ in 0..MAX_PASSES {
            let Ok((parent, tasks)) = self.refusal(project, mission) else {
                return;
            };
            let (in_flight, failed) = self.sets(project, mission);
            let steps = runner::pass(&World {
                parent: &parent,
                tasks: &tasks,
                live: ctx.live,
                in_flight: &in_flight,
                launch_failed: &failed,
                definitions: &definitions,
            });
            if steps.is_empty() {
                break;
            }
            let mut again = false;
            for step in steps {
                match step {
                    Step::Launch { task, role } => {
                        if !launch {
                            continue;
                        }
                        let Some(record) = tasks.iter().find(|t| t.id == task) else {
                            continue;
                        };
                        let settings = settings_of(&parent);
                        let prompt = self.task_prompt(project, &parent, record, &tasks, false);
                        let definition = runner::definition_for(record, &settings, &definitions);
                        self.launch(
                            eff, ctx, project, mission, task, role, prompt, definition, None, false,
                        );
                    }
                    Step::ParentTo(status) => {
                        self.set_phase(eff, project, mission, Phase::of(status));
                        again = true;
                    }
                    Step::Verdict {
                        review,
                        pass,
                        findings,
                    } => {
                        self.apply_verdict(eff, ctx, project, &tasks, review, pass, findings);
                        again = true;
                    }
                    Step::Finished { task, agent } => self.event(
                        eff,
                        project,
                        task,
                        String::new(),
                        RunEvent::Finished { agent: Some(agent) },
                    ),
                    Step::Gone { task, agent } => {
                        self.event(eff, project, task, String::new(), RunEvent::Gone { agent })
                    }
                    Step::Unload { task, agent } => {
                        if unloaded.insert(agent) {
                            self.stop(eff, project, task, agent);
                        }
                    }
                    Step::Abandon { task } => {
                        self.move_to(eff, project, task, Status::Abandoned);
                        again = true;
                    }
                }
            }
            if !again {
                break;
            }
        }
        self.announce(eff, project, mission, ctx);
    }

    /// Recompute the snapshot and broadcast it if it differs from the last one sent.
    fn announce(&mut self, eff: &mut Effects, project: ProjectId, mission: TaskId, ctx: &Ctx) {
        let Ok((parent, tasks)) = self.refusal(project, mission) else {
            return;
        };
        let snapshot = self.snapshot_of(project, &parent, &tasks, ctx);
        if self.sent.get(&(project, mission)) != Some(&snapshot) {
            self.sent.insert((project, mission), snapshot.clone());
            eff.replies.push(Reply::Everyone(Message::MissionChanged {
                project_id: project,
                mission: Box::new(snapshot),
            }));
        }
    }

    fn snapshot_of(
        &mut self,
        project: ProjectId,
        parent: &TaskRecord,
        tasks: &[TaskRecord],
        ctx: &Ctx,
    ) -> MissionSnapshot {
        let (in_flight, failed) = self.sets(project, parent.id);
        let definitions = self.definition_ids(project);
        runner::snapshot(
            project,
            &World {
                parent,
                tasks,
                live: ctx.live,
                in_flight: &in_flight,
                launch_failed: &failed,
                definitions: &definitions,
            },
            self.documents(project, parent.id),
            self.open_threads(project, parent.id),
        )
    }

    fn open_threads(&self, project: ProjectId, task: TaskId) -> usize {
        self.plan_store
            .load_sidecar(project, task)
            .ok()
            .flatten()
            .map_or(0, |sidecar| {
                sidecar
                    .annotations
                    .iter()
                    .filter(|annotation| annotation.is_open())
                    .count()
            })
    }

    fn plan_body(&self, project: ProjectId, task: TaskId) -> String {
        self.plan_store
            .load(project, task)
            .ok()
            .flatten()
            .unwrap_or_default()
    }

    fn guidance_body(&self, project: ProjectId, task: TaskId) -> String {
        std::fs::read_to_string(crate::plan::mission_doc_path(
            &self.root, project, task, GUIDANCE,
        ))
        .unwrap_or_default()
    }

    // ── writing ─────────────────────────────────────────────────────────

    /// Hand the board's replies on: every change to every window (a runner write has no asker),
    /// a refusal to the log.
    fn spread(eff: &mut Effects, replies: Vec<Reply>) {
        for reply in replies {
            match reply.into_message() {
                Message::WorkError { error, .. } => tracing::warn!("mission runner: {error}"),
                Message::PlanError { error, .. } => tracing::warn!("mission runner: {error}"),
                message @ (Message::TaskCreated { .. }
                | Message::TaskChanged { .. }
                | Message::TaskDeleted { .. }
                | Message::AgentChanged { .. }
                | Message::PlanChanged { .. }
                | Message::PlanAnnotationsChanged { .. }) => {
                    eff.replies.push(Reply::Everyone(message));
                }
                _ => {}
            }
        }
    }

    fn move_to(&self, eff: &mut Effects, project: ProjectId, task: TaskId, status: Status) {
        let replies = self.work.lock().move_task(project, task, status, None);
        Self::spread(eff, replies);
    }

    fn event(
        &self,
        eff: &mut Effects,
        project: ProjectId,
        task: TaskId,
        text: String,
        event: RunEvent,
    ) {
        let replies = self.work.lock().add_event(project, task, text, event);
        Self::spread(eff, replies);
    }

    fn edit(
        &self,
        eff: &mut Effects,
        project: ProjectId,
        task: TaskId,
        change: impl FnOnce(&mut TaskRecord) -> bool,
    ) {
        let replies = self.work.lock().edit(project, task, change);
        Self::spread(eff, replies);
    }

    /// Move the mission to a phase: the parent's status and the `Phase` line.
    fn set_phase(&self, eff: &mut Effects, project: ProjectId, mission: TaskId, phase: Phase) {
        self.move_to(eff, project, mission, phase.status());
        self.event(
            eff,
            project,
            mission,
            String::new(),
            RunEvent::Phase { to: phase },
        );
    }

    /// Stop an agent: the `Stopped` line, and the unload the coordinator performs.
    fn stop(&self, eff: &mut Effects, project: ProjectId, task: TaskId, agent: AgentId) {
        self.event(
            eff,
            project,
            task,
            String::new(),
            RunEvent::Stopped { agent },
        );
        eff.unload.push(agent);
    }

    /// Ask a window to launch an agent. Nothing happens without a window to address: the task
    /// stays ready and the next message from a window retries.
    #[allow(clippy::too_many_arguments)]
    fn launch(
        &mut self,
        eff: &mut Effects,
        ctx: &Ctx,
        project: ProjectId,
        mission: TaskId,
        task: TaskId,
        role: LaunchRole,
        prompt: String,
        definition: Option<String>,
        resume: Option<AgentId>,
        retry: bool,
    ) -> bool {
        let Some(client) = ctx.client else {
            tracing::debug!(%project, %task, "no window to launch a mission agent in");
            return false;
        };
        let id = SpawnId::generate();
        let mcps = match role {
            LaunchRole::Planner => catalogue::LAUNCH_PLANNER_MCPS,
            LaunchRole::Worker | LaunchRole::Reviewer => catalogue::LAUNCH_WORKER_MCPS,
            LaunchRole::MissionReviewer => catalogue::LAUNCH_MISSION_REVIEWER_MCPS,
        };
        self.launches.insert(
            id,
            PendingLaunch {
                project,
                mission,
                task,
                role,
                retry,
                client,
            },
        );
        eff.launches.push((
            client,
            Message::MissionLaunch {
                project_id: project,
                task_id: mission,
                request: Box::new(LaunchRequest {
                    id,
                    role,
                    task,
                    definition,
                    mcps: mcps.iter().map(|m| (*m).to_string()).collect(),
                    prompt,
                    resume,
                    persistent: role == LaunchRole::Planner,
                }),
            },
        ));
        true
    }

    /// A worker's or reviewer's prompt; `retry` wraps it in the "continue…" prompt with the
    /// feedback since the last launch.
    fn task_prompt(
        &self,
        project: ProjectId,
        parent: &TaskRecord,
        task: &TaskRecord,
        tasks: &[TaskRecord],
        retry: bool,
    ) -> String {
        let plan = self.plan_body(project, parent.id);
        let guidance = self.guidance_body(project, parent.id);
        let opening = match task.review_of {
            Some(of) => {
                prompts::reviewer(task, tasks.iter().find(|t| t.id == of), &plan, &guidance)
            }
            None => prompts::worker(task, &plan, &guidance),
        };
        if retry {
            prompts::continuing(&opening, task)
        } else {
            opening
        }
    }

    fn brief(&self, project: ProjectId, parent: &TaskRecord, tasks: &[TaskRecord]) -> String {
        let included: Vec<&TaskRecord> = tasks
            .iter()
            .filter(|t| t.parent == Some(parent.id))
            .collect();
        let has_plan = !self.plan_body(project, parent.id).trim().is_empty();
        prompts::planner(parent, &included, has_plan)
    }

    fn launch_planner(
        &mut self,
        eff: &mut Effects,
        ctx: &Ctx,
        project: ProjectId,
        parent: &TaskRecord,
        prompt: String,
        resume: bool,
    ) {
        let settings = settings_of(parent);
        let resume = if resume { settings.planner_agent } else { None };
        self.launch_failed.remove(&(project, parent.id));
        if !self.launch(
            eff,
            ctx,
            project,
            parent.id,
            parent.id,
            LaunchRole::Planner,
            prompt,
            settings.planner,
            resume,
            false,
        ) {
            eff.replies.push(Reply::Asker(error(
                project,
                Some(parent.id),
                "no window to start the planner in".to_string(),
            )));
        }
    }

    /// Say something to the planner: as its next turn when it is live, else by reviving it (or
    /// starting a fresh one with the brief) with the same line.
    fn tell_planner(
        &mut self,
        eff: &mut Effects,
        ctx: &Ctx,
        project: ProjectId,
        parent: &TaskRecord,
        tasks: &[TaskRecord],
        line: String,
    ) {
        match settings_of(parent).planner_agent.filter(|a| (ctx.live)(*a)) {
            Some(agent) => self.deliver(project, agent, line),
            None => {
                let brief = self.brief(project, parent, tasks);
                let prompt = prompts::planner_with_brief(&line, &brief);
                self.launch_planner(eff, ctx, project, parent, prompt, true);
            }
        }
    }

    /// The one rule (note §Reject): the feedback is a comment on the target, the target goes back
    /// in progress, and a live worker gets it as its next turn. A gone worker reads as *Worker
    /// gone*, and Retry carries the feedback in its prompt.
    #[allow(clippy::too_many_arguments)]
    fn one_rule(
        &mut self,
        eff: &mut Effects,
        ctx: &Ctx,
        project: ProjectId,
        target: &TaskRecord,
        feedback: String,
        event: RunEvent,
        reset: bool,
    ) {
        self.event(eff, project, target.id, feedback.clone(), event);
        self.move_to(eff, project, target.id, Status::InProgress);
        if reset {
            self.edit(eff, project, target.id, |t| {
                let changed = t.auto_fails != 0;
                t.auto_fails = 0;
                changed
            });
        }
        if let Some(agent) = target.worker.filter(|a| (ctx.live)(*a)) {
            self.deliver(project, agent, prompts::feedback(target, &feedback));
        }
    }

    /// Contract §3.6.
    #[allow(clippy::too_many_arguments)]
    fn apply_verdict(
        &mut self,
        eff: &mut Effects,
        ctx: &Ctx,
        project: ProjectId,
        tasks: &[TaskRecord],
        review: TaskId,
        pass: bool,
        findings: String,
    ) {
        let Some(review) = tasks.iter().find(|t| t.id == review).cloned() else {
            return;
        };
        let reviewed = review
            .review_of
            .and_then(|of| tasks.iter().find(|t| t.id == of).cloned());
        // The verdict is consumed: the reviewer is done with this task either way.
        let worker = review.worker;
        self.edit(eff, project, review.id, |t| t.worker.take().is_some());
        if let Some(agent) = worker.filter(|a| (ctx.live)(*a)) {
            self.stop(eff, project, review.id, agent);
        }
        if pass {
            self.move_to(eff, project, review.id, Status::Done);
            if let Some(t) = reviewed {
                self.edit(eff, project, t.id, |t| {
                    let changed = t.auto_fails != 0;
                    t.auto_fails = 0;
                    changed
                });
                if t.status == Status::InReview && !t.approve {
                    self.move_to(eff, project, t.id, Status::Done);
                }
            }
            return;
        }
        if let Some(t) = reviewed {
            let fails = t.auto_fails.saturating_add(1);
            self.edit(eff, project, t.id, |t| {
                t.auto_fails = fails;
                true
            });
            if fails < AUTO_FAIL_CAP {
                self.one_rule(
                    eff,
                    ctx,
                    project,
                    &t,
                    findings,
                    RunEvent::ReviewVerdict { pass: false },
                    false,
                );
            }
        }
        // The review task waits to run again once the reviewed task is back in review.
        self.move_to(eff, project, review.id, Status::Ready);
    }

    // ── the messages ────────────────────────────────────────────────────

    /// `CreateMission`: the parent, the adopted tasks, the first plan and the planner's launch
    /// (contract §3.4). `file_plan` is the initial plan file as the coordinator read it.
    pub fn create(
        &mut self,
        project: ProjectId,
        new: NewMission,
        file_plan: Option<Result<String, String>>,
        ctx: &Ctx,
    ) -> Effects {
        let mut eff = Effects::default();
        let title = new.title.trim().to_string();
        if title.is_empty() {
            return Effects::error(project, None, "a mission needs a title".to_string());
        }
        let replies = self.work.lock().create(project, title, None);
        let Some(id) = replies.iter().find_map(|r| match r.message() {
            Message::TaskCreated { task, .. } => Some(task.id),
            _ => None,
        }) else {
            eff.replies.extend(replies);
            return eff;
        };
        Self::spread(&mut eff, replies);

        let mut attachments: Vec<Attachment> = new
            .attachments
            .iter()
            .map(|target| Attachment {
                target: target.clone(),
                label: None,
            })
            .collect();
        if let InitialPlan::File { target } = &new.initial_plan
            && !attachments.iter().any(|a| &a.target == target)
        {
            attachments.push(Attachment {
                target: target.clone(),
                label: None,
            });
        }
        let mut settings = new.settings.clone();
        settings.parallel = settings.parallelism();
        settings.planner_agent = None;
        {
            let mut work = self.work.lock();
            let mut replies = work.set_field(project, id, TaskField::Level(Some(Level::Mission)));
            if !new.description.trim().is_empty() {
                replies.extend(work.update(project, id, None, Some(new.description.clone()), None));
            }
            if !attachments.is_empty() {
                replies.extend(work.set_field(project, id, TaskField::Attachments(attachments)));
            }
            replies.extend(work.edit(project, id, |t| {
                t.mission = Some(settings);
                true
            }));
            for included in &new.include {
                if *included == id {
                    continue;
                }
                replies.extend(work.set_field(project, *included, TaskField::Parent(Some(id))));
                let status = work
                    .tasks(project)
                    .1
                    .iter()
                    .find(|t| t.id == *included)
                    .map(|t| t.status);
                // Kept as it is unless it had not started: never reopen done or running work.
                if status == Some(Status::Ready) {
                    replies.extend(work.move_task(project, *included, Status::Backlog, None));
                }
            }
            drop(work);
            Self::spread(&mut eff, replies);
        }
        self.event(
            &mut eff,
            project,
            id,
            String::new(),
            RunEvent::Phase {
                to: Phase::Planning,
            },
        );

        let body = match (&new.initial_plan, file_plan) {
            (InitialPlan::Paste { body }, _) => Some(body.clone()),
            (InitialPlan::File { .. }, Some(Ok(body))) => Some(body),
            (InitialPlan::File { target }, Some(Err(reason))) => {
                eff.replies.push(Reply::Asker(error(
                    project,
                    Some(id),
                    format!("the initial plan `{target}` could not be read: {reason}"),
                )));
                None
            }
            _ => None,
        };
        if let Some(body) = body.filter(|b| !b.trim().is_empty()) {
            let replies =
                self.plans
                    .lock()
                    .save(&Target::plan(project, id), body, &Saver::human(), None);
            Self::spread(&mut eff, replies);
        }

        if let Some(client) = ctx.client {
            self.note_client(project, id, client);
        }
        if let Ok((parent, tasks)) = self.refusal(project, id) {
            let brief = self.brief(project, &parent, &tasks);
            self.launch_planner(&mut eff, ctx, project, &parent, brief, false);
        }
        self.run_into(&mut eff, project, id, ctx, true);
        eff
    }

    /// `SetMissionField`.
    pub fn set_field(
        &mut self,
        project: ProjectId,
        task: TaskId,
        field: MissionField,
        ctx: &Ctx,
    ) -> Effects {
        let (parent, _) = match self.refusal(project, task) {
            Ok(found) => found,
            Err(reason) => return Effects::error(project, Some(task), reason),
        };
        let mut eff = Effects::default();
        let mut settings = settings_of(&parent);
        match field {
            MissionField::Driver(driver) => settings.driver = driver,
            MissionField::Planner(planner) => settings.planner = trimmed(planner),
            MissionField::Worker(worker) => settings.worker = trimmed(worker),
            MissionField::Parallel(parallel) => settings.parallel = parallel.max(1),
            MissionField::Approve(approve) => settings.approve = approve,
        }
        self.edit(&mut eff, project, task, |t| {
            let changed = t.mission.as_ref() != Some(&settings);
            t.mission = Some(settings);
            changed
        });
        self.run_into(&mut eff, project, task, ctx, true);
        eff
    }

    /// `MissionControl` (contract §3.4).
    pub fn control(
        &mut self,
        project: ProjectId,
        mission: TaskId,
        action: MissionAction,
        ctx: &Ctx,
    ) -> Effects {
        let (parent, tasks) = match self.refusal(project, mission) {
            Ok(found) => found,
            Err(reason) => return Effects::error(project, Some(mission), reason),
        };
        let mut eff = Effects::default();
        let phase = Phase::of(parent.status);
        let refuse = |what: &str| Effects::error(project, Some(mission), what.to_string());
        match action {
            MissionAction::ApprovePlan => {
                if phase != Phase::Planning {
                    return refuse("the plan is approved while the mission is planning");
                }
                if self.plan_body(project, mission).trim().is_empty() {
                    return refuse(
                        "this mission's plan is empty — the plan is what the work is approved from",
                    );
                }
                // Open annotation threads only warn (they show on *Plan ready*); they no longer
                // block the gate.
                self.set_phase(&mut eff, project, mission, Phase::Breakdown);
                self.tell_planner(
                    &mut eff,
                    ctx,
                    project,
                    &parent,
                    &tasks,
                    prompts::PLAN_APPROVED.to_string(),
                );
            }
            MissionAction::StartMission => {
                if phase != Phase::Breakdown {
                    return refuse("a mission is started from its breakdown");
                }
                for child in tasks.iter().filter(|t| t.parent == Some(mission)) {
                    if child.status == Status::Backlog {
                        self.move_to(&mut eff, project, child.id, Status::Ready);
                    }
                }
                self.set_phase(&mut eff, project, mission, Phase::Running);
                if let Some(planner) = settings_of(&parent)
                    .planner_agent
                    .filter(|a| (ctx.live)(*a))
                {
                    self.stop(&mut eff, project, mission, planner);
                }
            }
            MissionAction::RedoBreakdown { note } => {
                if phase != Phase::Breakdown {
                    return refuse("the breakdown is redone while the mission is in breakdown");
                }
                // A fresh `Phase` line clears the stale *Tasks drafted*.
                self.event(
                    &mut eff,
                    project,
                    mission,
                    note.trim().to_string(),
                    RunEvent::Phase {
                        to: Phase::Breakdown,
                    },
                );
                self.tell_planner(
                    &mut eff,
                    ctx,
                    project,
                    &parent,
                    &tasks,
                    prompts::redo_breakdown(&note),
                );
            }
            MissionAction::Pause => {
                if phase != Phase::Running {
                    return refuse("only a running mission can be paused");
                }
                self.set_phase(&mut eff, project, mission, Phase::Paused);
            }
            MissionAction::Resume => {
                if phase != Phase::Paused {
                    return refuse("only a paused mission can be resumed");
                }
                self.set_phase(&mut eff, project, mission, Phase::Running);
            }
            MissionAction::Replan { note } => {
                if !matches!(phase, Phase::Running | Phase::Paused | Phase::Review) {
                    return refuse("a mission is replanned once it has started");
                }
                self.replan(&mut eff, ctx, project, &parent, &tasks, &note);
            }
            MissionAction::RejectMission { feedback } => {
                if phase != Phase::Review {
                    return refuse("a mission is rejected from its review");
                }
                self.event(
                    &mut eff,
                    project,
                    mission,
                    feedback.trim().to_string(),
                    RunEvent::Rejected,
                );
                self.replan(&mut eff, ctx, project, &parent, &tasks, &feedback);
            }
            MissionAction::RunAgentReview => {
                if phase != Phase::Review {
                    return refuse("the agent review runs in the mission's review");
                }
                let settings = settings_of(&parent);
                if !self.launch(
                    &mut eff,
                    ctx,
                    project,
                    mission,
                    mission,
                    LaunchRole::MissionReviewer,
                    prompts::mission_reviewer(&parent),
                    settings.worker,
                    None,
                    false,
                ) {
                    return refuse("no window to start the reviewer in");
                }
            }
            MissionAction::Complete => {
                self.stop_all(&mut eff, ctx, project, &parent, &tasks);
                self.set_phase(&mut eff, project, mission, Phase::Done);
            }
            MissionAction::Abandon => {
                self.stop_all(&mut eff, ctx, project, &parent, &tasks);
                for child in tasks.iter().filter(|t| t.parent == Some(mission)) {
                    if !matches!(child.status, Status::Done | Status::Abandoned) {
                        self.move_to(&mut eff, project, child.id, Status::Abandoned);
                    }
                }
                self.set_phase(&mut eff, project, mission, Phase::Abandoned);
            }
            MissionAction::ResumePlanner => {
                if !matches!(phase, Phase::Planning | Phase::Breakdown) {
                    return refuse("the planner is resumed while planning or in breakdown");
                }
                let brief = self.brief(project, &parent, &tasks);
                let prompt = prompts::planner_with_brief(prompts::RESTARTED, &brief);
                self.launch_planner(&mut eff, ctx, project, &parent, prompt, true);
            }
        }
        self.run_into(&mut eff, project, mission, ctx, true);
        eff
    }

    /// Replan and reject-mission: unstarted tasks back to backlog, the mission back to
    /// breakdown, and the planner told why.
    fn replan(
        &mut self,
        eff: &mut Effects,
        ctx: &Ctx,
        project: ProjectId,
        parent: &TaskRecord,
        tasks: &[TaskRecord],
        note: &str,
    ) {
        for child in tasks.iter().filter(|t| t.parent == Some(parent.id)) {
            if child.status == Status::Ready && child.worker.is_none() {
                self.move_to(eff, project, child.id, Status::Backlog);
            }
        }
        self.set_phase(eff, project, parent.id, Phase::Breakdown);
        self.tell_planner(eff, ctx, project, parent, tasks, prompts::replan(note));
    }

    /// Stop every live agent of the mission.
    fn stop_all(
        &mut self,
        eff: &mut Effects,
        ctx: &Ctx,
        project: ProjectId,
        parent: &TaskRecord,
        tasks: &[TaskRecord],
    ) {
        let no = HashSet::new();
        let world = World {
            parent,
            tasks,
            live: ctx.live,
            in_flight: &no,
            launch_failed: &no,
            definitions: &[],
        };
        for agent in runner::agents(&world) {
            let task = tasks
                .iter()
                .find(|t| t.parent == Some(parent.id) && t.worker == Some(agent))
                .map_or(parent.id, |t| t.id);
            self.stop(eff, project, task, agent);
        }
        self.launches
            .retain(|_, l| !(l.project == project && l.mission == parent.id));
    }

    /// `MissionTask` (contract §3.4) — `task` is the subtask.
    pub fn task_action(
        &mut self,
        project: ProjectId,
        task: TaskId,
        action: MissionTaskAction,
        ctx: &Ctx,
    ) -> Effects {
        let tasks = self.tasks(project);
        let Some(record) = tasks.iter().find(|t| t.id == task).cloned() else {
            return Effects::error(project, Some(task), "no such task".to_string());
        };
        let Some(mission) = record.parent.filter(|p| {
            tasks
                .iter()
                .any(|t| t.id == *p && t.level == Some(Level::Mission))
        }) else {
            return Effects::error(
                project,
                Some(task),
                "that task is not in a mission".to_string(),
            );
        };
        let Some(parent) = tasks.iter().find(|t| t.id == mission).cloned() else {
            return Effects::default();
        };
        if let Some(client) = ctx.client {
            self.note_client(project, mission, client);
        }
        let mut eff = Effects::default();
        let refuse = |what: &str| Effects::error(project, Some(task), what.to_string());
        let definitions = self.definition_ids(project);
        let in_flight = self.sets(project, mission).0.contains(&task);
        let worker_live = record.worker.is_some_and(|a| (ctx.live)(a));

        match action {
            MissionTaskAction::Start | MissionTaskAction::Retry => {
                let retry = matches!(action, MissionTaskAction::Retry);
                if in_flight || worker_live {
                    return refuse("that task already has an agent");
                }
                if is_human(&record, &definitions) {
                    return refuse("that task is a person's — mark it done instead");
                }
                if !retry && record.status != Status::Ready {
                    return refuse("only a ready task can be started");
                }
                if matches!(record.status, Status::Done | Status::Abandoned) {
                    return refuse("that task is closed");
                }
                self.launch_failed.remove(&(project, task));
                let settings = settings_of(&parent);
                let prompt = self.task_prompt(project, &parent, &record, &tasks, retry);
                let definition = runner::definition_for(&record, &settings, &definitions);
                if !self.launch(
                    &mut eff,
                    ctx,
                    project,
                    mission,
                    task,
                    runner::role_for(&record),
                    prompt,
                    definition,
                    None,
                    retry,
                ) {
                    return refuse("no window to start the agent in");
                }
            }
            MissionTaskAction::Confirm => {
                let capped = record.auto_fails >= AUTO_FAIL_CAP;
                if record.status != Status::InReview || !(record.approve || capped) {
                    return refuse("only a task waiting in review can be confirmed");
                }
                self.event(&mut eff, project, task, String::new(), RunEvent::Confirmed);
                self.move_to(&mut eff, project, task, Status::Done);
                self.edit(&mut eff, project, task, |t| {
                    let changed = t.auto_fails != 0;
                    t.auto_fails = 0;
                    changed
                });
                if capped {
                    // *Confirm anyway*: the review that kept failing is closed with it.
                    for review in tasks.iter().filter(|t| {
                        t.review_of == Some(task)
                            && !matches!(t.status, Status::Done | Status::Abandoned)
                    }) {
                        self.move_to(&mut eff, project, review.id, Status::Done);
                    }
                }
            }
            MissionTaskAction::Reject { feedback } => {
                if feedback.trim().is_empty() {
                    return refuse("a rejection needs feedback");
                }
                if matches!(record.status, Status::Done | Status::Abandoned) {
                    return refuse("that task is closed");
                }
                self.one_rule(
                    &mut eff,
                    ctx,
                    project,
                    &record,
                    feedback.trim().to_string(),
                    RunEvent::Rejected,
                    true,
                );
            }
            MissionTaskAction::Done => {
                if !is_human(&record, &definitions) {
                    return refuse("only a task assigned to a person is marked done by hand");
                }
                self.event(&mut eff, project, task, String::new(), RunEvent::HumanDone);
                self.move_to(&mut eff, project, task, Status::Done);
                if let Some(target) = record
                    .review_of
                    .and_then(|of| tasks.iter().find(|t| t.id == of))
                    && target.status == Status::InReview
                    && !target.approve
                {
                    self.move_to(&mut eff, project, target.id, Status::Done);
                }
            }
            MissionTaskAction::Fail { feedback } => {
                if !is_human(&record, &definitions) {
                    return refuse("only a task assigned to a person can be failed by hand");
                }
                let Some(target) = record
                    .review_of
                    .and_then(|of| tasks.iter().find(|t| t.id == of))
                    .cloned()
                else {
                    return refuse("this check reviews no task — mark it done instead");
                };
                if feedback.trim().is_empty() {
                    return refuse("a fail needs feedback");
                }
                self.one_rule(
                    &mut eff,
                    ctx,
                    project,
                    &target,
                    feedback.trim().to_string(),
                    RunEvent::HumanFailed,
                    true,
                );
                if record.status != Status::Ready {
                    self.move_to(&mut eff, project, task, Status::Ready);
                }
            }
            MissionTaskAction::AddReview { human } => {
                let replies =
                    self.work
                        .lock()
                        .create(project, format!("Review: {}", record.title), None);
                let Some(id) = replies.iter().find_map(|r| match r.message() {
                    Message::TaskCreated { task, .. } => Some(task.id),
                    _ => None,
                }) else {
                    eff.replies.extend(replies);
                    return eff;
                };
                Self::spread(&mut eff, replies);
                let replies =
                    self.work
                        .lock()
                        .set_field(project, id, TaskField::Parent(Some(mission)));
                Self::spread(&mut eff, replies);
                self.edit(&mut eff, project, id, |t| {
                    t.review_of = Some(task);
                    t.prerequisites = vec![task];
                    if human {
                        t.assigned_to = Some("human".to_string());
                    }
                    true
                });
                if matches!(
                    Phase::of(parent.status),
                    Phase::Running | Phase::Paused | Phase::Review
                ) {
                    self.move_to(&mut eff, project, id, Status::Ready);
                }
            }
        }
        self.run_into(&mut eff, project, mission, ctx, true);
        eff
    }

    /// `AddGuidance`: appended to `guidance.md` as a person's save, a `Guidance` line on the
    /// parent, and — when asked — a turn for every live worker.
    pub fn guidance(
        &mut self,
        project: ProjectId,
        mission: TaskId,
        text: String,
        send_to_running: bool,
        ctx: &Ctx,
    ) -> Effects {
        let (_, tasks) = match self.refusal(project, mission) {
            Ok(found) => found,
            Err(reason) => return Effects::error(project, Some(mission), reason),
        };
        let text = text.trim().to_string();
        if text.is_empty() {
            return Effects::error(
                project,
                Some(mission),
                "guidance needs some text".to_string(),
            );
        }
        let mut eff = Effects::default();
        let target = Target::MissionDoc {
            project,
            task: mission,
            name: GUIDANCE.to_string(),
        };
        let stamp = chrono::Local::now().format("%Y-%m-%d %H:%M");
        let replies = {
            let mut plans = self.plans.lock();
            let current = plans.body(&target).unwrap_or_default();
            let head = if current.trim().is_empty() {
                "# Guidance".to_string()
            } else {
                current.trim_end().to_string()
            };
            plans.save(
                &target,
                format!("{head}\n\n### {stamp}\n{text}\n"),
                &Saver::human(),
                None,
            )
        };
        Self::spread(&mut eff, replies);
        self.event(
            &mut eff,
            project,
            mission,
            text.clone(),
            RunEvent::Guidance {
                sent_to_running: send_to_running,
            },
        );
        if send_to_running {
            for child in tasks.iter().filter(|t| {
                t.parent == Some(mission)
                    && matches!(t.status, Status::InProgress | Status::InReview)
            }) {
                if let Some(agent) = child.worker.filter(|a| (ctx.live)(*a)) {
                    self.deliver(project, agent, prompts::guidance(&text));
                }
            }
        }
        self.run_into(&mut eff, project, mission, ctx, true);
        eff
    }

    /// `MissionLaunched`: the window's answer to a launch (contract §3.3, 3).
    pub fn launched(
        &mut self,
        project: ProjectId,
        mission: TaskId,
        id: SpawnId,
        outcome: LaunchOutcome,
        ctx: &Ctx,
    ) -> Effects {
        let mut eff = Effects::default();
        // An agent nobody is waiting for any more — a launch dropped by a Complete, an Abandon or
        // a closed window — is stopped rather than left loaded with nothing to do.
        let orphan = |eff: &mut Effects| {
            if let LaunchOutcome::Launched(agent) = &outcome {
                eff.unload.push(*agent);
            }
        };
        let Some(launch) = self.launches.remove(&id) else {
            orphan(&mut eff);
            return eff;
        };
        if launch.project != project || launch.mission != mission {
            tracing::warn!(%project, %mission, "a launch answered for another mission");
            orphan(&mut eff);
            return eff;
        }
        // Never reopen what moved on while the window was launching: a task back in backlog or
        // closed, or a mission that is over.
        let tasks = self.tasks(project);
        let status = |id: TaskId| tasks.iter().find(|t| t.id == id).map(|t| t.status);
        let stale = match launch.role {
            LaunchRole::Worker | LaunchRole::Reviewer => !matches!(
                status(launch.task),
                Some(Status::Ready | Status::InProgress | Status::InReview | Status::Blocked)
            ),
            LaunchRole::Planner | LaunchRole::MissionReviewer => {
                matches!(
                    status(mission),
                    None | Some(Status::Done | Status::Abandoned)
                )
            }
        };
        if stale {
            if let LaunchOutcome::Launched(agent) = &outcome {
                self.stop(&mut eff, project, launch.task, *agent);
            }
            self.run_into(&mut eff, project, mission, ctx, true);
            return eff;
        }
        match outcome {
            LaunchOutcome::Launched(agent) => {
                match launch.role {
                    LaunchRole::Planner => self.edit(&mut eff, project, mission, |t| {
                        let mut settings = settings_of(t);
                        settings.planner_agent = Some(agent);
                        t.mission = Some(settings);
                        true
                    }),
                    LaunchRole::MissionReviewer => {}
                    LaunchRole::Worker | LaunchRole::Reviewer => {
                        self.edit(&mut eff, project, launch.task, |t| {
                            t.worker = Some(agent);
                            true
                        });
                        self.move_to(&mut eff, project, launch.task, Status::InProgress);
                    }
                }
                self.event(
                    &mut eff,
                    project,
                    launch.task,
                    String::new(),
                    RunEvent::Spawned {
                        agent,
                        role: launch.role,
                        retry: launch.retry,
                    },
                );
            }
            LaunchOutcome::Failed(reason) => {
                self.event(
                    &mut eff,
                    project,
                    launch.task,
                    reason,
                    RunEvent::LaunchFailed,
                );
                self.launch_failed.insert((project, launch.task));
            }
        }
        self.run_into(&mut eff, project, mission, ctx, true);
        eff
    }

    /// The planner's `request_phase` (MCP): a `PhaseRequested` line on the parent. Never moves
    /// the phase. Returns the board's replies for the caller to broadcast.
    pub fn request_phase(
        &self,
        project: ProjectId,
        mission: TaskId,
        to: Phase,
        summary: String,
    ) -> Result<Vec<Reply>, String> {
        let (parent, _) = self.refusal(project, mission)?;
        let now = Phase::of(parent.status);
        match (now, to) {
            (Phase::Planning, Phase::Breakdown) | (Phase::Breakdown, Phase::Running) => {}
            _ => {
                return Err(format!(
                    "the mission is in {}: ask for 'breakdown' while planning, or 'running' in breakdown",
                    now.label()
                ));
            }
        }
        let summary = summary.trim().to_string();
        let replies = self.work.lock().add_event(
            project,
            mission,
            summary.clone(),
            RunEvent::PhaseRequested { to, summary },
        );
        self.voice.say(Message::MissionSchedule {
            project_id: project,
            task_id: mission,
        });
        Ok(replies)
    }

    /// `LoadHistory`: the mission's timeline, computed — the parent's and the subtasks' comments
    /// and the plan's revision log, newest first.
    pub fn history(&self, project: ProjectId, mission: TaskId) -> Vec<Reply> {
        let (parent, tasks) = match self.refusal(project, mission) {
            Ok(found) => found,
            Err(reason) => return vec![Reply::Asker(error(project, Some(mission), reason))],
        };
        let mut entries: Vec<HistoryEntry> = Vec::new();
        for task in
            std::iter::once(&parent).chain(tasks.iter().filter(|t| t.parent == Some(mission)))
        {
            let child = task.id != mission;
            for comment in &task.comments {
                let (kind, agent, line) = describe(comment);
                let text = if child {
                    format!("{}: {line}", prompts::name(task))
                } else {
                    line
                };
                entries.push(HistoryEntry {
                    at: comment.created_at,
                    kind,
                    task: Some(task.id),
                    agent,
                    plan_revision: None,
                    text,
                });
            }
        }
        if let Some(sidecar) = self
            .plan_store
            .load_sidecar(project, mission)
            .ok()
            .flatten()
        {
            for revision in &sidecar.history {
                let who = revision.author.as_deref().unwrap_or("you");
                entries.push(HistoryEntry {
                    at: revision.at,
                    kind: HistoryKind::Plan,
                    task: Some(mission),
                    agent: revision.author.as_deref().and_then(|a| a.parse().ok()),
                    plan_revision: Some(revision.revision),
                    text: format!(
                        "Plan revision {} by {who} (+{} −{} ~{})",
                        revision.revision, revision.added, revision.removed, revision.modified
                    ),
                });
            }
        }
        entries.sort_by(|a, b| b.at.cmp(&a.at));
        vec![Reply::Asker(Message::MissionHistory {
            project_id: project,
            task_id: mission,
            entries,
        })]
    }
}

/// The mission `task` belongs to, in a task list.
fn mission_in(tasks: &[TaskRecord], task: TaskId) -> Option<TaskId> {
    let record = tasks.iter().find(|t| t.id == task)?;
    if record.level == Some(Level::Mission) {
        return Some(record.id);
    }
    let parent = record.parent?;
    tasks
        .iter()
        .find(|t| t.id == parent && t.level == Some(Level::Mission))
        .map(|t| t.id)
}

fn trimmed(value: Option<String>) -> Option<String> {
    value
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// One comment as a history line: its kind, the agent it names, and the sentence.
fn describe(comment: &ubiq_proto::work::Comment) -> (HistoryKind, Option<AgentId>, String) {
    let text = comment.text.trim();
    let with = |head: &str| {
        if text.is_empty() {
            head.to_string()
        } else {
            format!("{head}: {text}")
        }
    };
    let role = |role: LaunchRole| match role {
        LaunchRole::Planner => "planner",
        LaunchRole::Worker => "worker",
        LaunchRole::Reviewer => "reviewer",
        LaunchRole::MissionReviewer => "mission reviewer",
    };
    match &comment.event {
        None => (HistoryKind::Comment, None, with(comment.author.label())),
        Some(event) => match event {
            RunEvent::Phase { to } => (
                HistoryKind::Phase,
                None,
                with(&format!("Phase → {}", to.label())),
            ),
            RunEvent::PhaseRequested { to, .. } => (
                HistoryKind::Phase,
                None,
                with(&format!("Planner asked for {}", to.label())),
            ),
            RunEvent::Spawned {
                agent,
                role: r,
                retry,
            } => (
                HistoryKind::Agent,
                Some(*agent),
                format!(
                    "Started the {}{}",
                    role(*r),
                    if *retry { " (retry)" } else { "" }
                ),
            ),
            RunEvent::Finished { agent } => (
                HistoryKind::Agent,
                *agent,
                "Finished — in review".to_string(),
            ),
            RunEvent::Gone { agent } => {
                (HistoryKind::Agent, Some(*agent), "Worker gone".to_string())
            }
            RunEvent::Stopped { agent } => (
                HistoryKind::Agent,
                Some(*agent),
                "Agent stopped".to_string(),
            ),
            RunEvent::LaunchFailed => (HistoryKind::Agent, None, with("Launch failed")),
            RunEvent::Confirmed => (HistoryKind::Review, None, "Confirmed".to_string()),
            RunEvent::HumanDone => (HistoryKind::Review, None, "Checked and done".to_string()),
            RunEvent::ReviewVerdict { pass: true } => {
                (HistoryKind::Review, None, with("Review passed"))
            }
            RunEvent::ReviewVerdict { pass: false } => {
                (HistoryKind::Feedback, None, with("Review failed"))
            }
            RunEvent::Rejected => (HistoryKind::Feedback, None, with("Rejected")),
            RunEvent::HumanFailed => (HistoryKind::Feedback, None, with("Failed")),
            RunEvent::Guidance { .. } => (HistoryKind::Guidance, None, with("Guidance")),
            RunEvent::Moved { from, to } => (
                HistoryKind::Status,
                None,
                format!("{} → {}", from.label(), to.label()),
            ),
        },
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
