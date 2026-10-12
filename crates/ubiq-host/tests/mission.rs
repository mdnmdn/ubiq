//! The mission runner end to end, over a real board and plan store: the user's buttons, the
//! window's answers to launches and the agents' own moves, through `Missions` and `Work` together.
//!
//! The pure pass has its own table tests in `src/mission/runner.rs`; these are the paths that
//! cross the board, the plan store and the runner.

use std::collections::HashSet;
use std::path::PathBuf;

use chrono::Utc;
use ubiq_host::mission::{Ctx, Effects, Missions};
use ubiq_host::plan::{Handle as PlanHandle, Plans, Saver, Target};
use ubiq_host::store::memory::MemoryTaskStore;
use ubiq_host::store::plan::FilePlanStore;
use ubiq_host::work::{self, Work};
use ubiq_proto::bus::{self, ClientId};
use ubiq_proto::ids::{ProjectId, SpawnId, TaskId};
use ubiq_proto::messages::Message;
use ubiq_proto::mission::{
    InitialPlan, LaunchOutcome, LaunchRole, MissionAction, MissionSettings, MissionTaskAction,
    NeedsYou, NewMission, Phase, RunEvent,
};
use ubiq_proto::work::{AgentId, Comment, Level, Status, TaskRecord};

/// A board, a plan store and a runner, with one window to ask for launches.
struct Rig {
    _dir: tempfile::TempDir,
    root: PathBuf,
    project: ProjectId,
    work: work::Handle,
    plans: PlanHandle,
    missions: Missions,
    window: ClientId,
    live: HashSet<AgentId>,
    // The bus halves are held for the test: the missions' voice is a weak sender.
    _hub: bus::Hub,
    _host: bus::HostEnd,
    _client: bus::Client,
}

impl Rig {
    fn with(tasks: Vec<TaskRecord>) -> Rig {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let project = ProjectId::generate();
        let work = work::Handle::new(Work::open(Box::new(MemoryTaskStore::with(project, tasks))));
        let plans = PlanHandle::new(Plans::open(FilePlanStore::new(root.clone()), work.clone()));
        let (hub, host) = bus::hub();
        let client = hub.connect();
        let missions = Missions::open(work.clone(), plans.clone(), root.clone(), host.voice());
        Rig {
            window: client.id(),
            _dir: dir,
            root,
            project,
            work,
            plans,
            missions,
            live: HashSet::new(),
            _hub: hub,
            _host: host,
            _client: client,
        }
    }

    fn task(&self, id: TaskId) -> TaskRecord {
        let (_, tasks) = self.work.lock().tasks(self.project);
        tasks
            .into_iter()
            .find(|t| t.id == id)
            .expect("on the board")
    }

    fn status(&self, id: TaskId) -> Status {
        self.task(id).status
    }

    fn events(&self, id: TaskId) -> Vec<RunEvent> {
        self.task(id)
            .comments
            .iter()
            .filter_map(|c: &Comment| c.event.clone())
            .collect()
    }

    fn control(&mut self, mission: TaskId, action: MissionAction) -> Effects {
        let live = &self.live;
        let ctx = Ctx {
            live: &|a| live.contains(&a),
            client: Some(self.window),
        };
        self.missions.control(self.project, mission, action, &ctx)
    }

    fn act(&mut self, task: TaskId, action: MissionTaskAction) -> Effects {
        let live = &self.live;
        let ctx = Ctx {
            live: &|a| live.contains(&a),
            client: Some(self.window),
        };
        self.missions.task_action(self.project, task, action, &ctx)
    }

    fn run(&mut self, mission: TaskId) -> Effects {
        let live = &self.live;
        let ctx = Ctx {
            live: &|a| live.contains(&a),
            client: Some(self.window),
        };
        self.missions.run(self.project, mission, &ctx)
    }

    fn answer(&mut self, mission: TaskId, id: SpawnId, outcome: LaunchOutcome) -> Effects {
        let live = &self.live;
        let ctx = Ctx {
            live: &|a| live.contains(&a),
            client: Some(self.window),
        };
        self.missions
            .launched(self.project, mission, id, outcome, &ctx)
    }

    /// Answer the one launch `effects` carries with a fresh live agent.
    fn launch_answered(&mut self, mission: TaskId, effects: &Effects) -> (AgentId, Effects) {
        let (id, _, _) = launch_of(effects);
        let agent = AgentId::generate();
        self.live.insert(agent);
        let after = self.answer(mission, id, LaunchOutcome::Launched(agent));
        (agent, after)
    }

    /// The agent says it is done: what `use-task`'s `change_state` does.
    fn to_review(&mut self, task: TaskId) {
        self.work
            .lock()
            .move_task(self.project, task, Status::InReview, None);
    }

    /// A reviewer's verdict, as `submit_review` records it.
    fn verdict(&mut self, review: TaskId, pass: bool, findings: &str) {
        self.work.lock().add_event(
            self.project,
            review,
            findings.to_string(),
            RunEvent::ReviewVerdict { pass },
        );
    }

    /// A task the planner drafts: in backlog, under the mission.
    fn add_child(&mut self, mission: TaskId, title: &str) -> TaskId {
        let mut work = self.work.lock();
        let replies = work.create(self.project, title.to_string(), None);
        let id = replies
            .iter()
            .find_map(|r| match r.message() {
                Message::TaskCreated { task, .. } => Some(task.id),
                _ => None,
            })
            .expect("created");
        work.edit(self.project, id, |t| {
            t.parent = Some(mission);
            true
        });
        id
    }

    /// The mission as a window would be told it on connecting: derived now, launching nothing.
    fn snapshot(&mut self, mission: TaskId) -> ubiq_proto::mission::MissionSnapshot {
        let live = &self.live;
        let ctx = Ctx {
            live: &|a| live.contains(&a),
            client: Some(self.window),
        };
        let listed = self.missions.list(self.project, &ctx);
        listed
            .replies
            .iter()
            .find_map(|r| match r.message() {
                Message::MissionList { missions, .. } => {
                    missions.iter().find(|m| m.task_id == mission).cloned()
                }
                _ => None,
            })
            .expect("the mission is listed")
    }
}

fn mission_parent(phase: Phase, settings: MissionSettings) -> TaskRecord {
    let mut parent = TaskRecord::new("ship it".into(), None, Utc::now());
    parent.level = Some(Level::Mission);
    parent.status = phase.status();
    parent.mission = Some(settings);
    parent
}

fn child(parent: &TaskRecord, title: &str, status: Status) -> TaskRecord {
    let mut task = TaskRecord::new(title.into(), None, Utc::now());
    task.parent = Some(parent.id);
    task.status = status;
    task
}

/// A review task of `of`, waiting on it, assigned to a person when `human`.
fn review(parent: &TaskRecord, of: &TaskRecord, human: bool) -> TaskRecord {
    let mut task = child(parent, "review", Status::Ready);
    task.review_of = Some(of.id);
    task.prerequisites = vec![of.id];
    if human {
        task.assigned_to = Some("human".into());
    }
    task
}

fn refused(effects: &Effects) -> bool {
    effects
        .replies
        .iter()
        .any(|r| matches!(r.message(), Message::MissionError { .. }))
}

/// The one launch the effects carry: its id, the task and the role it asks for.
fn launch_of(effects: &Effects) -> (SpawnId, TaskId, LaunchRole) {
    assert_eq!(effects.launches.len(), 1, "exactly one launch");
    match &effects.launches[0].1 {
        Message::MissionLaunch { request, .. } => (request.id, request.task, request.role),
        other => panic!("not a launch: {other:?}"),
    }
}

#[test]
fn planning_to_review_end_to_end() {
    let mut rig = Rig::with(vec![]);
    let new = NewMission {
        title: "ship it".into(),
        description: String::new(),
        attachments: vec![],
        include: vec![],
        initial_plan: InitialPlan::Paste {
            body: "# Plan\n\nDo the thing.\n".into(),
        },
        settings: MissionSettings::default(),
    };
    let effects = {
        let live = &rig.live;
        let ctx = Ctx {
            live: &|a| live.contains(&a),
            client: Some(rig.window),
        };
        rig.missions.create(rig.project, new, None, &ctx)
    };

    // Created in Planning, the pasted plan is revision 1, and the planner is asked for.
    let (id, task, role) = launch_of(&effects);
    assert_eq!(role, LaunchRole::Planner);
    let mission = task;
    assert_eq!(rig.status(mission), Status::Backlog);
    let store = FilePlanStore::new(rig.root.clone());
    assert_eq!(
        store.load(rig.project, mission).unwrap().as_deref(),
        Some("# Plan\n\nDo the thing.\n")
    );
    assert_eq!(
        store
            .load_sidecar(rig.project, mission)
            .unwrap()
            .expect("a sidecar")
            .revision,
        1
    );
    let planner = AgentId::generate();
    rig.live.insert(planner);
    rig.answer(mission, id, LaunchOutcome::Launched(planner));

    // Approve plan: Breakdown, and the live planner gets its next turn.
    rig.control(mission, MissionAction::ApprovePlan);
    assert_eq!(rig.status(mission), Status::Ready);
    let (_, told) = rig
        .missions
        .take_feedback(planner)
        .expect("the planner is told");
    assert_eq!(told.len(), 1);

    // The planner drafts a task (backlog), the user starts the mission: the task is ready, the
    // mission runs, the planner is unloaded and a worker is asked for.
    let work_task = rig.add_child(mission, "the work");
    let effects = rig.control(mission, MissionAction::StartMission);
    assert_eq!(rig.status(mission), Status::InProgress);
    assert!(effects.unload.contains(&planner));
    let (_, asked_for, role) = launch_of(&effects);
    assert_eq!((asked_for, role), (work_task, LaunchRole::Worker));
    assert_eq!(effects.launches[0].0, rig.window, "addressed to the window");

    // The window answers: the task is the worker's and in progress.
    let (worker, after) = rig.launch_answered(mission, &effects);
    assert!(
        after.launches.is_empty(),
        "parallel 1: nothing else to launch"
    );
    assert_eq!(rig.status(work_task), Status::InProgress);
    assert_eq!(rig.task(work_task).worker, Some(worker));

    // The worker moves its task to review and the runner wakes: the mission is in review.
    rig.to_review(work_task);
    let effects = rig.run(mission);
    assert_eq!(rig.status(mission), Status::InReview);
    let snapshot = effects
        .replies
        .iter()
        .find_map(|reply| match reply.message() {
            Message::MissionChanged { mission, .. } => Some(mission.clone()),
            _ => None,
        })
        .expect("the changed mission is broadcast");
    assert_eq!(snapshot.phase, Phase::Review);
    assert!(snapshot.needs_you.contains(&NeedsYou::ReadyForReview {
        left_in_review: vec![work_task],
    }));

    // Complete: done, the worker unloaded.
    let effects = rig.control(mission, MissionAction::Complete);
    assert_eq!(rig.status(mission), Status::Done);
    assert!(effects.unload.contains(&worker));
}

#[test]
fn an_initial_plan_file_is_saved_as_revision_one_and_a_bad_one_is_reported() {
    let new = |target: &str| NewMission {
        title: "from a file".into(),
        description: String::new(),
        attachments: vec![],
        include: vec![],
        initial_plan: InitialPlan::File {
            target: target.into(),
        },
        settings: MissionSettings::default(),
    };
    let mut rig = Rig::with(vec![]);
    let effects = {
        let live = &rig.live;
        let ctx = Ctx {
            live: &|a| live.contains(&a),
            client: Some(rig.window),
        };
        rig.missions.create(
            rig.project,
            new("docs/plan.md"),
            Some(Ok("# From a file\n".into())),
            &ctx,
        )
    };
    let (_, mission, _) = launch_of(&effects);
    let store = FilePlanStore::new(rig.root.clone());
    assert_eq!(
        store.load(rig.project, mission).unwrap().as_deref(),
        Some("# From a file\n")
    );
    assert_eq!(
        store
            .load_sidecar(rig.project, mission)
            .unwrap()
            .unwrap()
            .revision,
        1
    );
    assert!(
        rig.task(mission)
            .attachments
            .iter()
            .any(|a| a.target == "docs/plan.md"),
        "the file is attached too"
    );

    let effects = {
        let live = &rig.live;
        let ctx = Ctx {
            live: &|a| live.contains(&a),
            client: Some(rig.window),
        };
        rig.missions.create(
            rig.project,
            new("gone.md"),
            Some(Err("no such file".into())),
            &ctx,
        )
    };
    assert!(refused(&effects), "the unreadable file is said");
    let (_, other, _) = launch_of(&effects);
    assert_eq!(store.load(rig.project, other).unwrap(), None);
}

#[test]
fn open_threads_warn_but_do_not_block_approve_plan() {
    let parent = mission_parent(Phase::Planning, MissionSettings::default());
    let mission = parent.id;
    let mut rig = Rig::with(vec![parent]);
    let target = Target::plan(rig.project, mission);
    rig.plans.lock().save(
        &target,
        "# Plan\n\nDo the thing.\n".into(),
        &Saver::human(),
        None,
    );
    let block = {
        let mut plans = rig.plans.lock();
        let (blocks, _, _) = plans.annotation_list(&target).unwrap();
        blocks[0].id
    };
    rig.plans.lock().annotate(
        &target,
        block,
        None,
        ubiq_proto::work::CommentAuthor::User,
        "is this right?".into(),
        vec![],
        None,
    );
    assert_eq!(rig.snapshot(mission).open_threads, 1);

    let effects = rig.control(mission, MissionAction::ApprovePlan);
    assert!(!refused(&effects));
    assert_eq!(rig.status(mission), Status::Ready);
}

#[test]
fn approve_plan_refuses_an_empty_plan() {
    let parent = mission_parent(Phase::Planning, MissionSettings::default());
    let mission = parent.id;
    let mut rig = Rig::with(vec![parent]);
    let effects = rig.control(mission, MissionAction::ApprovePlan);
    assert!(refused(&effects));
    assert_eq!(rig.status(mission), Status::Backlog);
}

#[test]
fn an_approved_task_waits_for_confirm_and_a_reject_while_alive_is_its_next_turn() {
    let parent = mission_parent(Phase::Running, MissionSettings::default());
    let mission = parent.id;
    let mut task = child(&parent, "the work", Status::Ready);
    task.approve = true;
    let id = task.id;
    let mut rig = Rig::with(vec![parent, task]);

    let effects = rig.run(mission);
    let (worker, _) = rig.launch_answered(mission, &effects);
    rig.to_review(id);
    rig.run(mission);
    // Approval on: the mission does not move to review over a task waiting for the user.
    assert_eq!(rig.status(mission), Status::InProgress);
    assert!(
        rig.snapshot(mission)
            .needs_you
            .contains(&NeedsYou::AwaitingApproval { task: id })
    );

    // Reject while the worker is alive: back in progress, the feedback its next turn.
    rig.act(
        id,
        MissionTaskAction::Reject {
            feedback: "use the other name".into(),
        },
    );
    assert_eq!(rig.status(id), Status::InProgress);
    let (_, queued) = rig
        .missions
        .take_feedback(worker)
        .expect("queued for the worker");
    assert!(
        queued
            .iter()
            .any(|line| line.contains("use the other name"))
    );
    assert!(rig.events(id).contains(&RunEvent::Rejected));

    // It comes back and the user confirms: done, and the mission follows.
    rig.to_review(id);
    rig.act(id, MissionTaskAction::Confirm);
    assert_eq!(rig.status(id), Status::Done);
    assert_eq!(rig.status(mission), Status::InReview);
}

#[test]
fn a_task_whose_worker_is_gone_is_retried() {
    let parent = mission_parent(Phase::Running, MissionSettings::default());
    let mission = parent.id;
    let task = child(&parent, "the work", Status::Ready);
    let id = task.id;
    let mut rig = Rig::with(vec![parent, task]);

    let effects = rig.run(mission);
    let (worker, _) = rig.launch_answered(mission, &effects);
    rig.live.remove(&worker);
    rig.run(mission);
    assert!(
        rig.snapshot(mission)
            .needs_you
            .contains(&NeedsYou::WorkerGone { task: id })
    );
    assert!(rig.events(id).contains(&RunEvent::Gone { agent: worker }));

    let effects = rig.act(id, MissionTaskAction::Retry);
    let (_, asked_for, role) = launch_of(&effects);
    assert_eq!((asked_for, role), (id, LaunchRole::Worker));
    let (again, _) = rig.launch_answered(mission, &effects);
    assert_eq!(rig.task(id).worker, Some(again));
    assert!(rig.events(id).contains(&RunEvent::Spawned {
        agent: again,
        role: LaunchRole::Worker,
        retry: true,
    }));
}

#[test]
fn a_person_marking_a_review_done_closes_the_task_under_review() {
    let parent = mission_parent(Phase::Running, MissionSettings::default());
    let mission = parent.id;
    let reviewed = child(&parent, "the work", Status::InReview);
    let check = review(&parent, &reviewed, true);
    let (work_id, check_id) = (reviewed.id, check.id);
    let mut rig = Rig::with(vec![parent, reviewed, check]);

    let snapshot = rig.snapshot(mission);
    assert!(
        snapshot
            .needs_you
            .contains(&NeedsYou::HumanTask { task: check_id })
    );
    assert!(
        rig.run(mission).launches.is_empty(),
        "a person's task is never launched"
    );

    rig.act(check_id, MissionTaskAction::Done);
    assert_eq!(rig.status(check_id), Status::Done);
    assert_eq!(rig.status(work_id), Status::Done);
    assert_eq!(rig.status(mission), Status::InReview);
}

#[test]
fn a_person_failing_a_review_sends_the_feedback_to_the_task_under_review() {
    let parent = mission_parent(Phase::Running, MissionSettings::default());
    let reviewed = child(&parent, "the work", Status::InReview);
    let check = review(&parent, &reviewed, true);
    let (work_id, check_id, mission) = (reviewed.id, check.id, parent.id);
    let mut rig = Rig::with(vec![parent, reviewed, check]);

    // A fail needs feedback.
    assert!(refused(&rig.act(
        check_id,
        MissionTaskAction::Fail {
            feedback: "  ".into()
        }
    )));
    rig.act(
        check_id,
        MissionTaskAction::Fail {
            feedback: "the edge case is missing".into(),
        },
    );
    assert_eq!(rig.status(work_id), Status::InProgress);
    let failed = rig
        .task(work_id)
        .comments
        .into_iter()
        .find(|c| c.event == Some(RunEvent::HumanFailed))
        .expect("a HumanFailed line on the reviewed task");
    assert_eq!(failed.text, "the edge case is missing");
    assert_eq!(
        rig.status(check_id),
        Status::Ready,
        "the check waits to run again"
    );
    assert_eq!(rig.status(mission), Status::InProgress);
}

#[test]
fn two_failed_agent_reviews_hit_the_cap_and_confirm_anyway_closes_both() {
    let parent = mission_parent(Phase::Running, MissionSettings::default());
    let mission = parent.id;
    let reviewed = child(&parent, "the work", Status::InReview);
    let check = review(&parent, &reviewed, false);
    let (work_id, check_id) = (reviewed.id, check.id);
    let mut rig = Rig::with(vec![parent, reviewed, check]);

    // First review: the reviewer is launched on the review task and fails the work.
    let effects = rig.run(mission);
    let (_, asked_for, role) = launch_of(&effects);
    assert_eq!((asked_for, role), (check_id, LaunchRole::Reviewer));
    let (reviewer, _) = rig.launch_answered(mission, &effects);
    rig.verdict(check_id, false, "no tests");
    let effects = rig.run(mission);
    assert!(
        effects.unload.contains(&reviewer),
        "the reviewer is done with the task"
    );
    assert_eq!(rig.task(work_id).auto_fails, 1);
    assert_eq!(
        rig.status(work_id),
        Status::InProgress,
        "the worker gets the findings"
    );
    assert_eq!(rig.status(check_id), Status::Ready);

    // The worker fixes it and goes back to review: the reviewer runs again and fails it again.
    rig.to_review(work_id);
    let effects = rig.run(mission);
    let (_, asked_for, _) = launch_of(&effects);
    assert_eq!(asked_for, check_id);
    rig.launch_answered(mission, &effects);
    rig.verdict(check_id, false, "still no tests");
    let effects = rig.run(mission);
    assert!(
        effects.launches.is_empty(),
        "no third review: the cap waits for the user"
    );
    assert_eq!(rig.task(work_id).auto_fails, 2);
    assert_eq!(rig.status(work_id), Status::InReview);
    assert!(
        rig.snapshot(mission)
            .needs_you
            .contains(&NeedsYou::ReviewFailedTwice { task: work_id })
    );
    assert_eq!(
        rig.status(mission),
        Status::InProgress,
        "a capped task holds the mission"
    );

    // Confirm anyway: the task and the review that kept failing are closed, the count reset.
    rig.act(work_id, MissionTaskAction::Confirm);
    assert_eq!(rig.status(work_id), Status::Done);
    assert_eq!(rig.status(check_id), Status::Done);
    assert_eq!(rig.task(work_id).auto_fails, 0);
    assert_eq!(rig.status(mission), Status::InReview);
}

#[test]
fn a_passing_review_closes_the_reviewed_task_unless_it_needs_approval() {
    let parent = mission_parent(Phase::Running, MissionSettings::default());
    let mission = parent.id;
    let reviewed = child(&parent, "the work", Status::InReview);
    let check = review(&parent, &reviewed, false);
    let (work_id, check_id) = (reviewed.id, check.id);
    let mut rig = Rig::with(vec![parent, reviewed, check]);

    let effects = rig.run(mission);
    rig.launch_answered(mission, &effects);
    rig.verdict(check_id, true, "");
    rig.run(mission);
    assert_eq!(rig.status(check_id), Status::Done);
    assert_eq!(rig.status(work_id), Status::Done);
    assert_eq!(rig.status(mission), Status::InReview);
}

#[test]
fn pause_stops_launches_and_resume_starts_them_again() {
    let parent = mission_parent(Phase::Running, MissionSettings::default());
    let mission = parent.id;
    let task = child(&parent, "the work", Status::Ready);
    let mut rig = Rig::with(vec![parent, task]);

    rig.control(mission, MissionAction::Pause);
    assert_eq!(rig.status(mission), Status::Blocked);
    assert!(rig.run(mission).launches.is_empty());

    let effects = rig.control(mission, MissionAction::Resume);
    assert_eq!(rig.status(mission), Status::InProgress);
    assert_eq!(launch_of(&effects).2, LaunchRole::Worker);
}

#[test]
fn a_manual_driver_launches_nothing_until_a_task_is_started() {
    let settings = MissionSettings {
        driver: ubiq_proto::mission::Driver::Manual,
        ..MissionSettings::default()
    };
    let parent = mission_parent(Phase::Running, settings);
    let mission = parent.id;
    let task = child(&parent, "the work", Status::Ready);
    let id = task.id;
    let mut rig = Rig::with(vec![parent, task]);

    assert!(rig.run(mission).launches.is_empty());
    assert_eq!(rig.snapshot(mission).next, vec![id]);
    let effects = rig.act(id, MissionTaskAction::Start);
    assert_eq!(launch_of(&effects).1, id);
}

#[test]
fn replan_returns_unstarted_tasks_to_backlog_and_asks_the_planner_again() {
    let parent = mission_parent(Phase::Running, MissionSettings::default());
    let mission = parent.id;
    let waiting = child(&parent, "not yet", Status::Ready);
    let mut working = child(&parent, "under way", Status::InProgress);
    working.worker = Some(AgentId::generate());
    let (waiting_id, working_id) = (waiting.id, working.id);
    let mut rig = Rig::with(vec![parent, waiting, working]);
    rig.live.insert(rig.task(working_id).worker.unwrap());

    let effects = rig.control(
        mission,
        MissionAction::Replan {
            note: "the API changed".into(),
        },
    );
    assert_eq!(rig.status(mission), Status::Ready, "back to breakdown");
    assert_eq!(rig.status(waiting_id), Status::Backlog);
    assert_eq!(
        rig.status(working_id),
        Status::InProgress,
        "started work is left alone"
    );
    let (_, task, role) = launch_of(&effects);
    assert_eq!((task, role), (mission, LaunchRole::Planner));
}

#[test]
fn abandon_cascades_to_the_review_tasks_and_unloads_the_worker() {
    let parent = mission_parent(Phase::Running, MissionSettings::default());
    let mission = parent.id;
    let mut working = child(&parent, "under way", Status::InProgress);
    let agent = AgentId::generate();
    working.worker = Some(agent);
    let check = review(&parent, &working, false);
    let (working_id, check_id) = (working.id, check.id);
    let mut rig = Rig::with(vec![parent, working, check]);
    rig.live.insert(agent);

    let effects = rig.control(mission, MissionAction::Abandon);
    assert_eq!(rig.status(mission), Status::Abandoned);
    assert_eq!(rig.status(working_id), Status::Abandoned);
    assert_eq!(rig.status(check_id), Status::Abandoned);
    assert!(effects.unload.contains(&agent));
}
