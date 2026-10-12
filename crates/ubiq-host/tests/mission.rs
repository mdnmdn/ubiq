//! The mission runner end to end, over a real board and plan store: Start mission → the runner
//! asks one window for a worker → the window answers → the worker moves its task to review → the
//! mission moves to Review.
//!
//! The pure pass has its own table tests in `src/mission/runner.rs`; this is the one path that
//! goes through `Missions` and `Work` together. The rest of the coverage is MS6's.

use std::collections::HashSet;

use chrono::Utc;
use ubiq_host::mission::{Ctx, Effects, Missions};
use ubiq_host::plan::{Handle as PlanHandle, Plans};
use ubiq_host::store::memory::MemoryTaskStore;
use ubiq_host::store::plan::FilePlanStore;
use ubiq_host::work::{self, Work};
use ubiq_proto::bus;
use ubiq_proto::ids::{ProjectId, SpawnId, TaskId};
use ubiq_proto::messages::Message;
use ubiq_proto::mission::{
    LaunchOutcome, LaunchRole, MissionAction, MissionSettings, NeedsYou, Phase,
};
use ubiq_proto::work::{AgentId, Level, Status, TaskRecord};

fn status_of(work: &work::Handle, project: ProjectId, task: TaskId) -> TaskRecord {
    let (_, tasks) = work.lock().tasks(project);
    tasks
        .into_iter()
        .find(|t| t.id == task)
        .expect("on the board")
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
fn start_mission_launches_a_worker_and_a_finished_task_moves_the_mission_to_review() {
    let dir = tempfile::tempdir().unwrap();
    let project = ProjectId::generate();
    let now = Utc::now();

    let mut parent = TaskRecord::new("ship it".into(), None, now);
    parent.level = Some(Level::Mission);
    parent.status = Phase::Breakdown.status();
    parent.mission = Some(MissionSettings::default());
    let mut child = TaskRecord::new("the work".into(), None, now);
    child.parent = Some(parent.id);
    let (mission, task) = (parent.id, child.id);

    let work = work::Handle::new(Work::open(Box::new(MemoryTaskStore::with(
        project,
        vec![parent, child],
    ))));
    let plans = PlanHandle::new(Plans::open(
        FilePlanStore::new(dir.path().to_path_buf()),
        work.clone(),
    ));
    // Both bus halves held for the test: the missions' voice is a weak sender.
    let (hub, host) = bus::hub();
    let window = hub.connect();
    let mut missions = Missions::open(work.clone(), plans, dir.path().to_path_buf(), host.voice());

    let mut live: HashSet<AgentId> = HashSet::new();

    // Start mission: the draft goes ready, the mission runs, and one worker is asked for.
    let effects = {
        let ctx = Ctx {
            live: &|a| live.contains(&a),
            client: Some(window.id()),
        };
        missions.control(project, mission, MissionAction::StartMission, &ctx)
    };
    assert_eq!(
        status_of(&work, project, mission).status,
        Status::InProgress
    );
    let (id, asked_for, role) = launch_of(&effects);
    assert_eq!((asked_for, role), (task, LaunchRole::Worker));
    assert_eq!(
        effects.launches[0].0,
        window.id(),
        "addressed to the one window"
    );

    // The window answers: the task is the worker's and in progress.
    let agent = AgentId::generate();
    live.insert(agent);
    let effects = {
        let ctx = Ctx {
            live: &|a| live.contains(&a),
            client: Some(window.id()),
        };
        missions.launched(project, mission, id, LaunchOutcome::Launched(agent), &ctx)
    };
    assert!(
        effects.launches.is_empty(),
        "parallel 1: nothing else to launch"
    );
    let worked = status_of(&work, project, task);
    assert_eq!(worked.status, Status::InProgress);
    assert_eq!(worked.worker, Some(agent));

    // The worker moves its task to review (use-task's change_state), and the runner wakes.
    work.lock().move_task(project, task, Status::InReview, None);
    let effects = {
        let ctx = Ctx {
            live: &|a| live.contains(&a),
            client: Some(window.id()),
        };
        missions.run(project, mission, &ctx)
    };
    assert_eq!(status_of(&work, project, mission).status, Status::InReview);
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
        left_in_review: vec![task],
    }));
    drop(host);
}
