//! A project's missions: the record on disk, and the service over it.
//!
//! Two halves, because the module has two. The store half is about a real `mission.toml` — the
//! round trip, and the rule that a key this build does not understand survives a save written by
//! one that does not know it. The service half is about the three things M3 settles: a record is
//! made lazily with its phase read off the anchor, a mission that has been worked cannot quietly
//! stop being one, and a deleted task takes its directory with it.
//!
//! The interesting cases are the ones where doing the obvious thing loses something: rewriting a
//! record and dropping the half of it this build cannot read, or throwing away a roster and a
//! journal because somebody cleared a dropdown.

use std::fs;

use chrono::{TimeZone, Utc};
use tempfile::TempDir;
use ubiq_host::mission::Missions;
use ubiq_host::reply::Reply;
use ubiq_host::store::memory::MemoryTaskStore;
use ubiq_host::store::mission::MissionStore;
use ubiq_host::store::plan::FilePlanStore;
use ubiq_host::store::plan::PlanSidecar;
use ubiq_host::work::{self, Work};
use ubiq_proto::bus;
use ubiq_proto::ids::{BlockId, ProjectId, SessionId, TaskId};
use ubiq_proto::messages::Message;
use ubiq_proto::mission::{
    Actor, AgentKind, CoordinatorEntry, ExecutionMode, JournalEntry, JournalEvent, MissionField,
    MissionRecord, MissionRole, OnFinish, Phase, PhaseEntry, RosterEntry, SpawnPolicy,
};
use ubiq_proto::plan::{Annotation, AnnotationState, PlanBlock};
use ubiq_proto::work::{
    Activity, AgentId, CommentAuthor, Label, Level, Status, TaskRecord, WorkAgent, WorkSession,
};

fn anchor(title: &str, status: Status) -> TaskRecord {
    let mut task = TaskRecord::new(
        title.to_string(),
        None,
        Utc.with_ymd_and_hms(2026, 9, 24, 9, 0, 0).unwrap(),
    );
    task.level = Some(Level::Mission);
    task.status = status;
    task
}

/// The missions, over a store with a real directory under it and a board seeded exactly as given.
///
/// The [`bus::HostEnd`] comes back with it and has to be **held for the length of the test**: the
/// missions' [`ubiq_proto::bus::Voice`] is a weak sender into it, so a dropped end turns every
/// prompt `tell_agent` says into a silent no-op — which is exactly the bug a test would then fail
/// to see.
fn missions(dir: &TempDir, project: ProjectId, tasks: Vec<TaskRecord>) -> (Missions, Listener) {
    let (missions, _, host) = with_work(dir, project, tasks);
    (missions, host)
}

/// The host's own end of the bus, **with the hub still attached**.
///
/// Both halves, because a [`ubiq_proto::bus::Voice`] holds a *weak* sender: drop the hub and every
/// prompt the missions say becomes a silent no-op, which would make these tests pass for the one
/// reason they must not.
struct Listener {
    _hub: bus::Hub,
    host: bus::HostEnd,
}

/// The same, with the work handle kept — what every phase test needs, because M4's whole point is
/// that the anchor task's status followed the phase.
fn with_work(
    dir: &TempDir,
    project: ProjectId,
    tasks: Vec<TaskRecord>,
) -> (Missions, work::Handle, Listener) {
    let work = work::Handle::new(Work::open(Box::new(MemoryTaskStore::with(project, tasks))));
    let (hub, host) = bus::hub();
    let missions = Missions::open(
        MissionStore::new(dir.path().to_path_buf()),
        work.clone(),
        FilePlanStore::new(dir.path().to_path_buf()),
        host.voice(),
    );
    (missions, work, Listener { _hub: hub, host })
}

/// Every prompt the missions have said into the host's own inbox, drained — what proves a line
/// reached a **live** agent and not only its row.
fn prompts(listener: &Listener) -> Vec<(AgentId, String)> {
    let mut said = Vec::new();
    while let Ok(bus::FromClient::Said { message, .. }) = listener
        .host
        .recv_timeout(std::time::Duration::from_millis(0))
    {
        if let Message::PromptAgent { agent_id, text } = message {
            said.push((agent_id, text));
        }
    }
    said
}

/// The anchor task's status as the board would draw it.
fn status_of(work: &work::Handle, project: ProjectId, task: TaskId) -> Status {
    let (_, tasks) = work.lock().tasks(project);
    tasks
        .iter()
        .find(|record| record.id == task)
        .expect("the anchor is still on the board")
        .status
}

fn changed(replies: &[Reply]) -> Option<&MissionRecord> {
    replies.iter().find_map(|reply| match reply.message() {
        Message::MissionChanged { mission, .. } => Some(&**mission),
        _ => None,
    })
}

fn listed(replies: &[Reply]) -> Vec<MissionRecord> {
    replies
        .iter()
        .find_map(|reply| match reply.message() {
            Message::MissionList { missions, .. } => Some(missions.clone()),
            _ => None,
        })
        .expect("a list answers with a list")
}

fn refusal(replies: &[Reply]) -> Option<&str> {
    replies.iter().find_map(|reply| match reply.message() {
        Message::MissionError { error, .. } => Some(error.as_str()),
        _ => None,
    })
}

// ── the store, against a real file ──────────────────────────────────

#[test]
fn a_record_survives_the_round_trip_whole() {
    let dir = TempDir::new().unwrap();
    let store = MissionStore::new(dir.path().to_path_buf());
    let project = ProjectId::generate();
    let task = TaskId::generate();
    let at = Utc.with_ymd_and_hms(2026, 9, 24, 11, 30, 0).unwrap();
    let agent = AgentId::generate();

    let mut want = MissionRecord::new(project, task, Phase::InProgress, at);
    want.phase_history.push(PhaseEntry {
        phase: Phase::InProgress,
        at,
        by: Actor::Agent(agent),
    });
    want.coordinator = Some(agent);
    want.coordinator_history.push(CoordinatorEntry {
        agent: Some(agent),
        at,
        by: Actor::User,
    });
    let mut member = RosterEntry::new(agent, MissionRole::Coordinator, at);
    member.labels = vec!["ui".to_string(), "api".to_string()];
    member.scheduled = true;
    member.tasks_held = 2;
    member.last_held_at = Some(at);
    want.roster.push(member);
    want.require_plan = true;
    want.auto_refine = false;
    want.execution = ExecutionMode::Auto;
    want.parallelism = 4;
    want.on_finish = OnFinish::Stop;
    want.max_tasks_per_agent = 5;
    want.max_attempts = 3;
    want.spawn_policy = SpawnPolicy::Never;
    want.spawn_limit = 6;
    want.agent_kinds.push(AgentKind {
        name: "reviewer".to_string(),
        description: "reads what the workers wrote".to_string(),
        definition: Some("review".to_string()),
        labels: vec!["docs".to_string()],
        ..AgentKind::default()
    });
    want.default_kind = Some("reviewer".to_string());

    store.save(&want).unwrap();
    assert_eq!(store.load(project, task).unwrap(), Some(want));
}

/// The record lands where the plan store's sibling directory says, with `docs/` and the journal
/// beside it — the layout the stages after this one write into.
#[test]
fn a_mission_is_a_directory_under_the_project_that_owns_it() {
    let dir = TempDir::new().unwrap();
    let store = MissionStore::new(dir.path().to_path_buf());
    let project = ProjectId::generate();
    let task = TaskId::generate();

    let expected = dir
        .path()
        .join("projects")
        .join(project.to_string())
        .join("missions")
        .join(task.to_string());
    assert_eq!(store.dir(project, task), expected);
    assert_eq!(store.path(project, task), expected.join("mission.toml"));
    assert_eq!(store.docs_dir(project, task), expected.join("docs"));
    assert_eq!(
        store.journal_path(project, task),
        expected.join("journal.jsonl")
    );
}

/// A newer Ubiq's field, or the user's own note, is still there after this build has saved over
/// the file — the whole reason a save merges rather than replaces.
#[test]
fn a_key_this_build_does_not_know_survives_a_save() {
    let dir = TempDir::new().unwrap();
    let store = MissionStore::new(dir.path().to_path_buf());
    let project = ProjectId::generate();
    let task = TaskId::generate();
    let at = Utc.with_ymd_and_hms(2026, 9, 24, 11, 30, 0).unwrap();

    let mut record = MissionRecord::new(project, task, Phase::Refining, at);
    store.save(&record).unwrap();

    let path = store.path(project, task);
    let raw = fs::read_to_string(&path).unwrap();
    // At the top, because a key written after an array of tables belongs to that table rather
    // than to the document — what is preserved is the record's own unknown keys.
    fs::write(&path, format!("budget_cap = 42\n{raw}")).unwrap();

    // This build reads the record it understands and ignores the rest.
    let read = store.load(project, task).unwrap().unwrap();
    assert_eq!(read.phase, Phase::Refining);

    record.phase = Phase::InProgress;
    store.save(&record).unwrap();

    let after = fs::read_to_string(&path).unwrap();
    assert!(
        after.contains("budget_cap = 42"),
        "the unknown key was dropped by the save: {after}"
    );
    assert_eq!(
        store.load(project, task).unwrap().unwrap().phase,
        Phase::InProgress
    );
}

// ── the service ─────────────────────────────────────────────────────

/// Every phase inference M3 names, in one board.
#[test]
fn a_records_phase_is_inferred_from_its_anchor() {
    let dir = TempDir::new().unwrap();
    let project = ProjectId::generate();

    let fresh = anchor("nothing yet", Status::Backlog);
    let planned = anchor("has a plan", Status::InProgress);
    let done = anchor("finished", Status::Done);
    let dropped = anchor("given up on", Status::Abandoned);
    let ordinary = {
        let mut task = anchor("not a mission", Status::Backlog);
        task.level = None;
        task
    };
    let (fresh_id, planned_id, done_id, dropped_id, ordinary_id) =
        (fresh.id, planned.id, done.id, dropped.id, ordinary.id);

    // The plan the Refining inference reads.
    FilePlanStore::new(dir.path().to_path_buf())
        .save(project, planned_id, "# a plan\n")
        .unwrap();

    let (mut missions, _host) =
        missions(&dir, project, vec![fresh, planned, done, dropped, ordinary]);
    let replies = missions.list(project);
    let list = listed(&replies);

    let phase = |id: TaskId| {
        list.iter()
            .find(|record| record.task_id == id)
            .map(|record| record.phase)
    };
    assert_eq!(phase(fresh_id), Some(Phase::Requirements));
    assert_eq!(phase(planned_id), Some(Phase::Refining));
    assert_eq!(phase(done_id), Some(Phase::Completed));
    assert_eq!(phase(dropped_id), Some(Phase::Abandoned));
    // A task with no level is not a mission and gets no record.
    assert_eq!(phase(ordinary_id), None);
    assert_eq!(list.len(), 4);

    // Listing created them, so a second list reads the same records off disk and says nothing new.
    let again = missions.list(project);
    assert_eq!(listed(&again).len(), 4);
    assert!(changed(&again).is_none());
}

#[test]
fn a_mission_cannot_be_made_for_a_task_that_is_not_one() {
    let dir = TempDir::new().unwrap();
    let project = ProjectId::generate();
    let mut ordinary = anchor("an ordinary task", Status::Ready);
    ordinary.level = None;
    let id = ordinary.id;

    let (mut missions, _host) = missions(&dir, project, vec![ordinary]);
    assert_eq!(
        refusal(&missions.create(project, id)),
        Some("only a task with a level can carry a mission")
    );
    assert_eq!(
        refusal(&missions.create(project, TaskId::generate())),
        Some("no such task")
    );
}

#[test]
fn a_setting_is_written_and_broadcast() {
    let dir = TempDir::new().unwrap();
    let project = ProjectId::generate();
    let task = anchor("a mission", Status::InProgress);
    let id = task.id;
    let (mut missions, _host) = missions(&dir, project, vec![task]);

    let replies = missions.set_field(project, id, MissionField::RequirePlan(true));
    assert!(replies.iter().all(Reply::is_broadcast));
    assert!(changed(&replies).unwrap().require_plan);
    assert!(missions.record(project, id).unwrap().require_plan);

    // A value that already matches costs no write and says nothing.
    assert!(
        missions
            .set_field(project, id, MissionField::RequirePlan(true))
            .is_empty()
    );

    // A coordinator is remembered as well as set, so a handover can be read back.
    let agent = AgentId::generate();
    let replies = missions.set_field(project, id, MissionField::Coordinator(Some(agent)));
    let record = changed(&replies).unwrap();
    assert_eq!(record.coordinator, Some(agent));
    assert_eq!(record.coordinator_history.len(), 1);
}

#[test]
fn a_demotion_is_refused_once_agents_have_been_in_the_mission() {
    let dir = TempDir::new().unwrap();
    let project = ProjectId::generate();
    let task = anchor("a mission", Status::InProgress);
    let id = task.id;
    let (mut missions, _host) = missions(&dir, project, vec![task]);
    missions.create(project, id);

    // Nothing has happened in it yet: the record goes with the demotion.
    assert_eq!(missions.demotion_refusal(project, id), None);

    let mut record = missions.record(project, id).unwrap();
    record.roster.push(RosterEntry::new(
        AgentId::generate(),
        MissionRole::Worker,
        Utc::now(),
    ));
    MissionStore::new(dir.path().to_path_buf())
        .save(&record)
        .unwrap();

    assert!(
        missions
            .demotion_refusal(project, id)
            .is_some_and(|sentence| sentence.contains("agents")),
    );

    // A journal is the other half of the same question.
    let mut empty = missions.record(project, id).unwrap();
    empty.roster.clear();
    MissionStore::new(dir.path().to_path_buf())
        .save(&empty)
        .unwrap();
    assert_eq!(missions.demotion_refusal(project, id), None);
    let store = MissionStore::new(dir.path().to_path_buf());
    fs::write(store.journal_path(project, id), "{\"event\":\"started\"}\n").unwrap();
    assert!(
        missions
            .demotion_refusal(project, id)
            .is_some_and(|sentence| sentence.contains("journal")),
    );

    // A task nobody ever made a record for has nothing to lose.
    assert_eq!(missions.demotion_refusal(project, TaskId::generate()), None);
}

#[test]
fn deleting_a_mission_takes_its_whole_directory() {
    let dir = TempDir::new().unwrap();
    let project = ProjectId::generate();
    let task = anchor("a mission", Status::InProgress);
    let id = task.id;
    let (mut missions, _host) = missions(&dir, project, vec![task]);
    missions.create(project, id);

    let store = MissionStore::new(dir.path().to_path_buf());
    fs::create_dir_all(store.docs_dir(project, id)).unwrap();
    fs::write(store.docs_dir(project, id).join("notes.md"), "# notes\n").unwrap();
    fs::write(store.journal_path(project, id), "{}\n").unwrap();
    assert!(store.dir(project, id).exists());

    let replies = missions.delete(project, id);
    assert!(matches!(
        replies.first().map(Reply::message),
        Some(Message::MissionDeleted { task_id, .. }) if *task_id == id
    ));
    assert!(!store.dir(project, id).exists());
    assert_eq!(missions.record(project, id), None);

    // Asking for an absent thing to be gone is already satisfied, and is not news.
    assert!(missions.delete(project, id).is_empty());
}

// ── the phase: M4's derivation and M5's rules ───────────────────────

/// A plan with a body, and optionally one open annotation thread on it — the two things the gate
/// reads.
fn seed_plan(dir: &TempDir, project: ProjectId, task: TaskId, body: &str, open_thread: bool) {
    let plans = FilePlanStore::new(dir.path().to_path_buf());
    plans.save(project, task, body).unwrap();
    if open_thread {
        let block = PlanBlock {
            id: BlockId::generate(),
            kind: "paragraph".to_string(),
            text: body.trim().to_string(),
        };
        let annotation = Annotation::new(
            block.id,
            None,
            CommentAuthor::User,
            "is this the whole of it?".to_string(),
            Utc::now(),
        );
        plans
            .save_sidecar(
                project,
                task,
                &PlanSidecar::new(vec![block], vec![annotation]),
            )
            .unwrap();
    }
}

#[test]
fn every_phase_writes_the_anchors_status() {
    let dir = TempDir::new().unwrap();
    let project = ProjectId::generate();
    let task = anchor("a mission", Status::Backlog);
    let id = task.id;
    let (mut missions, work, _host) = with_work(&dir, project, vec![task]);

    // The record starts in Requirements, and the first move is what writes the anchor.
    for (phase, status) in [
        (Phase::Refining, Status::InReview),
        (Phase::InProgress, Status::InProgress),
        (Phase::Completed, Status::Done),
        (Phase::Abandoned, Status::Abandoned),
        (Phase::Requirements, Status::InProgress),
    ] {
        let replies = missions.set_phase(project, id, phase);
        assert_eq!(changed(&replies).unwrap().phase, phase);
        assert_eq!(refusal(&replies), None);
        assert_eq!(status_of(&work, project, id), status, "phase {phase:?}");
    }
}

#[test]
fn the_plan_gate_refuses_until_the_plan_is_written_and_its_threads_are_closed() {
    let dir = TempDir::new().unwrap();
    let project = ProjectId::generate();
    let task = anchor("a gated mission", Status::Backlog);
    let id = task.id;
    let (mut missions, work, _host) = with_work(&dir, project, vec![task]);
    missions.set_field(project, id, MissionField::RequirePlan(true));
    missions.set_phase(project, id, Phase::Refining);

    // An empty plan is not something to commit agents and spend to.
    let replies = missions.set_phase(project, id, Phase::InProgress);
    assert!(refusal(&replies).unwrap().contains("plan is empty"));
    assert_eq!(missions.record(project, id).unwrap().phase, Phase::Refining);
    assert_eq!(status_of(&work, project, id), Status::InReview);

    // Nor is one with a thread still asking something.
    seed_plan(&dir, project, id, "# the plan\n\nfirst, this.\n", true);
    let replies = missions.set_phase(project, id, Phase::InProgress);
    assert!(
        refusal(&replies)
            .unwrap()
            .contains("1 open annotation thread")
    );
    assert_eq!(missions.record(project, id).unwrap().phase, Phase::Refining);

    // The coordinator cannot pass it either — its ask only becomes the pending request.
    let replies = missions.request_phase(project, id, Phase::InProgress, String::new());
    assert_eq!(missions.record(project, id).unwrap().phase, Phase::Refining);
    assert_eq!(
        changed(&replies)
            .unwrap()
            .pending_phase
            .as_ref()
            .unwrap()
            .phase,
        Phase::InProgress
    );

    // Resolved, and the user sends it through.
    let plans = FilePlanStore::new(dir.path().to_path_buf());
    let mut sidecar = plans.load_sidecar(project, id).unwrap().unwrap();
    sidecar.annotations[0].state = AnnotationState::Resolved;
    plans.save_sidecar(project, id, &sidecar).unwrap();

    let replies = missions.set_phase(project, id, Phase::InProgress);
    assert_eq!(refusal(&replies), None);
    assert_eq!(changed(&replies).unwrap().phase, Phase::InProgress);
    assert!(changed(&replies).unwrap().pending_phase.is_none());
    assert_eq!(status_of(&work, project, id), Status::InProgress);
}

#[test]
fn auto_refine_moves_requirements_to_refining_without_the_user() {
    let dir = TempDir::new().unwrap();
    let project = ProjectId::generate();
    let task = anchor("a mission", Status::Backlog);
    let id = task.id;
    let (mut missions, work, _host) = with_work(&dir, project, vec![task]);
    let agent = AgentId::generate();
    missions.set_field(project, id, MissionField::Coordinator(Some(agent)));

    // On by default: the ask is the move.
    let replies = missions.request_phase(project, id, Phase::Refining, String::new());
    let record = changed(&replies).unwrap();
    assert_eq!(record.phase, Phase::Refining);
    assert!(record.pending_phase.is_none());
    assert_eq!(record.phase_history.last().unwrap().by, Actor::Agent(agent));
    assert_eq!(status_of(&work, project, id), Status::InReview);

    // Off, and the same ask waits for the user instead.
    missions.set_phase(project, id, Phase::Requirements);
    missions.set_field(project, id, MissionField::AutoRefine(false));
    let replies = missions.request_phase(project, id, Phase::Refining, "ready to plan".to_string());
    let record = changed(&replies).unwrap();
    assert_eq!(record.phase, Phase::Requirements);
    let pending = record.pending_phase.as_ref().unwrap();
    assert_eq!(pending.phase, Phase::Refining);
    assert_eq!(pending.by, Actor::Agent(agent));
    assert_eq!(pending.summary, "ready to plan");
}

#[test]
fn without_require_plan_the_coordinator_moves_itself_into_the_work() {
    let dir = TempDir::new().unwrap();
    let project = ProjectId::generate();
    let task = anchor("an ungated mission", Status::Backlog);
    let id = task.id;
    let (mut missions, work, _host) = with_work(&dir, project, vec![task]);
    assert!(!missions.record(project, id).is_some_and(|r| r.require_plan));

    // No plan, no gate, no user: the move is the coordinator's to make, from either phase.
    let replies = missions.request_phase(project, id, Phase::InProgress, String::new());
    let record = changed(&replies).unwrap();
    assert_eq!(record.phase, Phase::InProgress);
    assert!(record.pending_phase.is_none());
    assert_eq!(record.phase_history.last().unwrap().by, Actor::Host);
    assert_eq!(status_of(&work, project, id), Status::InProgress);

    // Completion is never the coordinator's: it asks, and the mission reads Blocked until answered.
    let replies = missions.request_phase(project, id, Phase::Completed, "all shipped".to_string());
    assert_eq!(changed(&replies).unwrap().phase, Phase::InProgress);
    assert_eq!(status_of(&work, project, id), Status::Blocked);

    let replies = missions.set_phase(project, id, Phase::Completed);
    assert_eq!(changed(&replies).unwrap().phase, Phase::Completed);
    assert_eq!(status_of(&work, project, id), Status::Done);
}

#[test]
fn the_user_declines_by_leaving_the_mission_where_it_is() {
    let dir = TempDir::new().unwrap();
    let project = ProjectId::generate();
    let task = anchor("a mission", Status::Backlog);
    let id = task.id;
    let (mut missions, work, _host) = with_work(&dir, project, vec![task]);
    missions.set_field(project, id, MissionField::AutoRefine(false));
    missions.request_phase(project, id, Phase::Refining, "shall I plan?".to_string());
    assert!(
        missions
            .record(project, id)
            .unwrap()
            .pending_phase
            .is_some()
    );

    let replies = missions.set_phase(project, id, Phase::Requirements);
    let record = changed(&replies).unwrap();
    assert_eq!(record.phase, Phase::Requirements);
    assert!(record.pending_phase.is_none());
    assert_eq!(refusal(&replies), None);
    assert_eq!(status_of(&work, project, id), Status::InProgress);

    // Nothing pending and nowhere to go is not news.
    assert!(
        missions
            .set_phase(project, id, Phase::Requirements)
            .is_empty()
    );
}

#[test]
fn a_step_back_is_always_allowed_and_never_gated() {
    let dir = TempDir::new().unwrap();
    let project = ProjectId::generate();
    let task = anchor("a gated mission", Status::Backlog);
    let id = task.id;
    let (mut missions, work, _host) = with_work(&dir, project, vec![task]);
    missions.set_field(project, id, MissionField::RequirePlan(true));
    seed_plan(&dir, project, id, "# the plan\n", false);
    missions.set_phase(project, id, Phase::Refining);
    missions.set_phase(project, id, Phase::InProgress);

    // Out of the work and back to the requirements, through the gate's own crossing, refused
    // nowhere — and out of Abandoned again, which is a step back from past the end.
    for (phase, status) in [
        (Phase::Requirements, Status::InProgress),
        (Phase::Abandoned, Status::Abandoned),
        (Phase::Refining, Status::InReview),
    ] {
        let replies = missions.set_phase(project, id, phase);
        assert_eq!(refusal(&replies), None, "stepping to {phase:?}");
        assert_eq!(changed(&replies).unwrap().phase, phase);
        assert_eq!(status_of(&work, project, id), status);
    }
}

#[test]
fn every_transition_appends_a_phase_entry_saying_who_moved_it() {
    let dir = TempDir::new().unwrap();
    let project = ProjectId::generate();
    let task = anchor("a mission", Status::Backlog);
    let id = task.id;
    let (mut missions, _host) = missions(&dir, project, vec![task]);
    let agent = AgentId::generate();
    missions.set_field(project, id, MissionField::Coordinator(Some(agent)));

    missions.request_phase(project, id, Phase::Refining, String::new()); // agent, auto_refine
    missions.request_phase(project, id, Phase::InProgress, String::new()); // agent, no gate
    missions.set_phase(project, id, Phase::Completed); // user

    let record = missions.record(project, id).unwrap();
    let history: Vec<(Phase, Actor)> = record
        .phase_history
        .iter()
        .map(|entry| (entry.phase, entry.by))
        .collect();
    assert_eq!(
        history,
        vec![
            (Phase::Requirements, Actor::Host), // the record's own creation
            (Phase::Refining, Actor::Agent(agent)),
            (Phase::InProgress, Actor::Agent(agent)),
            (Phase::Completed, Actor::User),
        ]
    );
    // A pending request is not a transition and writes no entry.
    missions.set_field(project, id, MissionField::AutoRefine(false));
    missions.set_phase(project, id, Phase::Requirements);
    missions.request_phase(project, id, Phase::Refining, String::new());
    assert_eq!(missions.record(project, id).unwrap().phase_history.len(), 5);
}

// ── the journal (M12) ───────────────────────────────────────────────

fn appended(replies: &[Reply]) -> Vec<JournalEntry> {
    replies
        .iter()
        .filter_map(|reply| match reply.message() {
            Message::JournalAppended { entry, .. } => Some(entry.clone()),
            _ => None,
        })
        .collect()
}

fn paged(replies: &[Reply]) -> (Vec<JournalEntry>, bool) {
    replies
        .iter()
        .find_map(|reply| match reply.message() {
            Message::Journal { entries, more, .. } => Some((entries.clone(), *more)),
            _ => None,
        })
        .expect("a journal request answers with a page")
}

/// Every phase move writes exactly one line, whoever moved it — the seam `apply` calls once.
#[test]
fn every_phase_transition_writes_one_journal_line() {
    let dir = TempDir::new().unwrap();
    let project = ProjectId::generate();
    let task = anchor("Ship it", Status::InProgress);
    let id = task.id;
    let (mut missions, _host) = missions(&dir, project, vec![task]);

    let moved = missions.set_phase(project, id, Phase::Refining);
    let lines = appended(&moved);
    assert_eq!(lines.len(), 1, "one line per move");
    assert_eq!(
        lines[0].event,
        JournalEvent::PhaseChanged {
            phase: Phase::Refining
        }
    );
    assert_eq!(lines[0].by, Actor::User);
    assert_eq!(lines[0].seq, 0, "the first line is sequence zero");

    // And again, from the other direction: an agent's request that the host applies on the spot.
    let moved = missions.request_phase(project, id, Phase::InProgress, String::new());
    let kinds: Vec<&str> = appended(&moved)
        .iter()
        .map(|entry| entry.event.kind())
        .collect();
    assert_eq!(kinds, vec!["phase_changed"], "an applied move is one line");

    // A request the user has to answer is a line of its own, and says why.
    let asked = missions.request_phase(project, id, Phase::Completed, "  all done  ".to_string());
    let lines = appended(&asked);
    assert_eq!(lines.len(), 1);
    assert_eq!(
        lines[0].event,
        JournalEvent::PhaseRequested {
            phase: Phase::Completed
        }
    );
    assert!(lines[0].text.contains("all done"), "{}", lines[0].text);

    // The file on disk is the same three lines, in the order they happened.
    let raw = fs::read_to_string(
        dir.path()
            .join("projects")
            .join(project.to_string())
            .join("missions")
            .join(id.to_string())
            .join("journal.jsonl"),
    )
    .expect("the journal is a real file");
    assert_eq!(raw.lines().count(), 3);
}

/// `LoadJournal` answers newest first and pages back through the sequence — the cursor the
/// panel's *Latest* and the full view's *Activity* tab both read.
#[test]
fn a_journal_pages_back_through_its_sequence() {
    let dir = TempDir::new().unwrap();
    let project = ProjectId::generate();
    let task = anchor("Ship it", Status::InProgress);
    let id = task.id;
    let (mut missions, _host) = missions(&dir, project, vec![task]);

    for n in 0..5 {
        missions.report_progress(project, id, Actor::Host, None, format!("line {n}"));
    }

    let (newest, more) = paged(&missions.load_journal(project, id, None, Some(2)));
    assert!(more, "there are older lines");
    assert_eq!(
        newest.iter().map(|entry| entry.seq).collect::<Vec<_>>(),
        vec![4, 3],
        "newest first"
    );
    assert_eq!(newest[0].text, "line 4");

    let cursor = newest.last().unwrap().seq;
    let (next, more) = paged(&missions.load_journal(project, id, Some(cursor), Some(2)));
    assert!(more);
    assert_eq!(
        next.iter().map(|entry| entry.seq).collect::<Vec<_>>(),
        vec![2, 1],
        "the cursor neither skips nor repeats"
    );

    let cursor = next.last().unwrap().seq;
    let (last, more) = paged(&missions.load_journal(project, id, Some(cursor), Some(2)));
    assert!(!more, "nothing is older than the first line");
    assert_eq!(
        last.iter().map(|entry| entry.seq).collect::<Vec<_>>(),
        vec![0]
    );
}

/// A mission with no journal answers an empty page rather than an error — the ordinary state of
/// every mission until something happens in it.
#[test]
fn an_empty_journal_is_an_empty_page() {
    let dir = TempDir::new().unwrap();
    let project = ProjectId::generate();
    let task = anchor("Ship it", Status::InProgress);
    let id = task.id;
    let (mut missions, _host) = missions(&dir, project, vec![task]);

    let (entries, more) = paged(&missions.load_journal(project, id, None, None));
    assert!(entries.is_empty());
    assert!(!more);
}

/// The user's feedback is read once and not twice, and `read_feedback`'s watermark moves with it.
#[test]
fn feedback_is_read_once() {
    let dir = TempDir::new().unwrap();
    let project = ProjectId::generate();
    let task = anchor("Ship it", Status::InProgress);
    let id = task.id;
    let (mut missions, _host) = missions(&dir, project, vec![task]);

    missions.feedback(project, id, "try the other approach".to_string());
    let first = missions.take_feedback(project, id);
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].text, "try the other approach");

    assert!(
        missions.take_feedback(project, id).is_empty(),
        "nothing new since the last read"
    );

    missions.feedback(project, id, "and rename it".to_string());
    let second = missions.take_feedback(project, id);
    assert_eq!(second.len(), 1);
    assert_eq!(second[0].text, "and rename it");
}

// ── the roster (M11) ────────────────────────────────────────────────

/// An agent assigned to the mission's anchor, or to one of its children, is in that mission —
/// and an ordinary task with no mission over it is in none.
#[test]
fn the_roster_reads_right_for_an_assigned_agent() {
    let dir = TempDir::new().unwrap();
    let project = ProjectId::generate();
    let mission_task = anchor("Ship it", Status::InProgress);
    let mission_id = mission_task.id;

    let mut child = TaskRecord::new(
        "Wire the bus".to_string(),
        None,
        Utc.with_ymd_and_hms(2026, 9, 24, 10, 0, 0).unwrap(),
    );
    child.parent = Some(mission_id);
    let child_id = child.id;

    let stray = TaskRecord::new(
        "Something else".to_string(),
        None,
        Utc.with_ymd_and_hms(2026, 9, 24, 10, 0, 0).unwrap(),
    );
    let stray_id = stray.id;

    let (mut missions, _host) = missions(&dir, project, vec![mission_task, child, stray]);

    assert_eq!(
        missions.mission_of_task(project, mission_id),
        Some(mission_id),
        "the anchor is its own mission"
    );
    assert_eq!(
        missions.mission_of_task(project, child_id),
        Some(mission_id),
        "a child is in its parent's mission"
    );
    assert_eq!(
        missions.mission_of_task(project, stray_id),
        None,
        "an ordinary task is in no mission"
    );

    // Joining writes the roster entry and a journal line, and says so to every window.
    let agent = AgentId::generate();
    let joined = missions.join(project, mission_id, agent, MissionRole::Worker, Actor::Host);
    let record = changed(&joined).expect("joining announces the record");
    assert_eq!(record.roster.len(), 1);
    assert_eq!(record.roster[0].agent, agent);
    assert_eq!(record.roster[0].role, MissionRole::Worker);
    assert!(record.roster[0].left_at.is_none());
    assert_eq!(
        appended(&joined)[0].event,
        JournalEvent::AgentJoined {
            agent,
            role: MissionRole::Worker
        }
    );

    // Joining twice with the same role is not a second membership and not a second line.
    let again = missions.join(project, mission_id, agent, MissionRole::Worker, Actor::Host);
    assert!(again.is_empty(), "an idempotent join says nothing");

    // Promotion is a role change on the same row, not a new one.
    let promoted = missions.join(
        project,
        mission_id,
        agent,
        MissionRole::Coordinator,
        Actor::User,
    );
    let record = changed(&promoted).unwrap();
    assert_eq!(record.roster.len(), 1, "still one member");
    assert_eq!(record.roster[0].role, MissionRole::Coordinator);

    // Leaving keeps the row and stamps it, so the history can be read back.
    let left = missions.leave(project, mission_id, agent, Actor::User);
    let record = changed(&left).unwrap();
    assert_eq!(record.roster.len(), 1);
    assert!(record.roster[0].left_at.is_some());
    assert!(record.member(agent).is_none(), "a member that left is out");
}

/// Attaching a coordinator puts it on the roster: a mission cannot have a coordinator that is not
/// one of its own members.
#[test]
fn attaching_a_coordinator_rosters_it() {
    let dir = TempDir::new().unwrap();
    let project = ProjectId::generate();
    let task = anchor("Ship it", Status::InProgress);
    let id = task.id;
    let (mut missions, _host) = missions(&dir, project, vec![task]);

    let first = AgentId::generate();
    let replies = missions.set_field(project, id, MissionField::Coordinator(Some(first)));
    let record = changed(&replies).unwrap();
    assert_eq!(record.coordinator, Some(first));
    assert_eq!(
        record.member(first).map(|entry| entry.role),
        Some(MissionRole::Coordinator)
    );

    // A handover demotes the one stepping down rather than dropping it.
    let second = AgentId::generate();
    let replies = missions.set_field(project, id, MissionField::Coordinator(Some(second)));
    let record = changed(&replies).unwrap();
    assert_eq!(
        record.member(first).map(|entry| entry.role),
        Some(MissionRole::Worker)
    );
    assert_eq!(
        record.member(second).map(|entry| entry.role),
        Some(MissionRole::Coordinator)
    );
}

// ── the documents (M8) ──────────────────────────────────────────────

/// The document names ride the record, which is how a window learns which documents exist —
/// there is no listing message, and `MissionChanged` already goes to every window.
#[test]
fn a_documents_name_reaches_a_window_on_the_record() {
    let dir = TempDir::new().unwrap();
    let project = ProjectId::generate();
    let task = anchor("Ship it", Status::InProgress);
    let id = task.id;
    let (mut missions, _host) = missions(&dir, project, vec![task]);

    let docs = dir
        .path()
        .join("projects")
        .join(project.to_string())
        .join("missions")
        .join(id.to_string())
        .join("docs");
    fs::create_dir_all(&docs).unwrap();
    fs::write(docs.join("architecture.md"), "# Architecture").unwrap();
    fs::write(docs.join("research.md"), "# Research").unwrap();
    // Not markdown, so not a document.
    fs::write(docs.join("notes.txt"), "scratch").unwrap();

    let listed = listed(&missions.list(project));
    assert_eq!(listed.len(), 1);
    assert_eq!(
        listed[0].documents,
        vec!["architecture".to_string(), "research".to_string()]
    );
    assert_eq!(missions.documents(project, id), listed[0].documents);
}

// ── spawning (M13) ──────────────────────────────────────────────────

/// A mission with a worker kind on it and one member already in, which is what every spawn test
/// starts from: a spawn is asked for *by* somebody, and the answer has to reach that somebody.
fn spawnable(dir: &TempDir, project: ProjectId, task: TaskId, missions: &mut Missions) -> AgentId {
    let _ = dir;
    let asker = AgentId::generate();
    missions.set_field(
        project,
        task,
        MissionField::AgentKinds(vec![
            AgentKind {
                name: "worker".to_string(),
                description: "Does one task and stops.".to_string(),
                definition: Some("worker-definition".to_string()),
                ..Default::default()
            },
            AgentKind {
                name: "reviewer".to_string(),
                description: "Reads what a worker wrote.".to_string(),
                ..Default::default()
            },
        ]),
    );
    missions.set_field(project, task, MissionField::Coordinator(Some(asker)));
    asker
}

/// The whole of the relay: an agent asks for a kind, gets an id back at once, and what it asked
/// for is on the record, in the journal and on the wire for the window that will launch it.
///
/// **Nothing here launches.** The reply carries no `StartConversation` and no `AgentId`, because
/// minting one is the window's and the host has no business having an opinion about it.
#[test]
fn a_spawn_request_answers_an_id_and_launches_nothing() {
    let dir = TempDir::new().unwrap();
    let project = ProjectId::generate();
    let task = anchor("Ship it", Status::InProgress);
    let id = task.id;
    let (mut missions, _host) = missions(&dir, project, vec![task]);
    let asker = spawnable(&dir, project, id, &mut missions);

    let (request_id, replies) = missions
        .request_spawn(
            project,
            id,
            Actor::Agent(asker),
            "worker",
            None,
            None,
            "Read the contract and wire the bus.".to_string(),
            "The task splits in two and I cannot do both.".to_string(),
        )
        .expect("a known kind is relayed");

    // The record holds it, exactly as `pending_phase` holds a phase ask — this is what the
    // panel's *Needs you* draws and what survives a host restart.
    let record = changed(&replies).expect("the request announces the record");
    assert_eq!(record.pending_spawns.len(), 1);
    let pending = &record.pending_spawns[0];
    assert_eq!(pending.id, request_id);
    assert_eq!(pending.kind, "worker");
    assert_eq!(pending.by, Actor::Agent(asker));
    assert_eq!(pending.prompt, "Read the contract and wire the bus.");
    assert_eq!(
        missions.record(project, id).unwrap().pending_spawns.len(),
        1,
        "and it is on disk, not only in the reply"
    );

    // The relay message the window acts on.
    let relayed = replies
        .iter()
        .find_map(|reply| match reply.message() {
            Message::MissionSpawnRequest { request, .. } => Some((**request).clone()),
            _ => None,
        })
        .expect("the window is told");
    assert_eq!(relayed.id, request_id);
    assert_eq!(relayed.kind, "worker");

    // And the journal says a spawn was asked for, in the requester's words.
    let line = appended(&replies)
        .into_iter()
        .find(|entry| matches!(entry.event, JournalEvent::SpawnRequested { .. }))
        .expect("the ask is journaled");
    assert_eq!(
        line.event,
        JournalEvent::SpawnRequested {
            agent_kind: "worker".to_string()
        }
    );
    assert!(line.text.contains("cannot do both"), "{}", line.text);

    // Nothing in the whole exchange named an agent that does not exist yet.
    assert!(
        !replies
            .iter()
            .any(|reply| matches!(reply.message(), Message::StartConversation { .. })),
        "the host relays; it never launches"
    );
}

/// A kind the mission does not have is refused with a sentence naming the ones it does, and an
/// empty table says so rather than pretending any name would work.
#[test]
fn an_unknown_kind_is_refused_with_a_sentence() {
    let dir = TempDir::new().unwrap();
    let project = ProjectId::generate();
    let task = anchor("Ship it", Status::InProgress);
    let id = task.id;
    let (mut missions, _host) = missions(&dir, project, vec![task]);

    // Before any kinds are set up at all: honest about the table being empty.
    let empty = missions
        .request_spawn(
            project,
            id,
            Actor::Host,
            "worker",
            None,
            None,
            "go".to_string(),
            "because".to_string(),
        )
        .unwrap_err();
    assert!(empty.contains("no agent kinds yet"), "{empty}");
    assert!(
        missions
            .record(project, id)
            .unwrap()
            .pending_spawns
            .is_empty(),
        "a refused request is not pending"
    );

    let asker = spawnable(&dir, project, id, &mut missions);
    let unknown = missions
        .request_spawn(
            project,
            id,
            Actor::Agent(asker),
            "architect",
            None,
            None,
            "go".to_string(),
            "because".to_string(),
        )
        .unwrap_err();
    assert!(unknown.contains("'architect'"), "{unknown}");
    assert!(
        unknown.contains("'worker'") && unknown.contains("'reviewer'"),
        "{unknown}"
    );

    // A name cased differently is the row it meant, not a fourth kind.
    let (_, replies) = missions
        .request_spawn(
            project,
            id,
            Actor::Agent(asker),
            "Reviewer",
            None,
            None,
            "go".to_string(),
            "because".to_string(),
        )
        .expect("a kind named in another case is still that kind");
    assert_eq!(
        changed(&replies).unwrap().pending_spawns[0].kind,
        "reviewer"
    );

    // `custom` is the escape hatch, and it is nothing without a definition.
    let bare = missions
        .request_spawn(
            project,
            id,
            Actor::Agent(asker),
            "custom",
            None,
            None,
            "go".to_string(),
            "because".to_string(),
        )
        .unwrap_err();
    assert!(bare.contains("needs a definition"), "{bare}");
}

/// The outcome the window reports reaches all three places it has to: the roster, the journal and
/// the requester's own next prompt — and the **kind actually used** is what is said, not the kind
/// that was asked for.
#[test]
fn a_spawn_outcome_reaches_the_roster_the_journal_and_the_requester() {
    let dir = TempDir::new().unwrap();
    let project = ProjectId::generate();
    let task = anchor("Ship it", Status::InProgress);
    let id = task.id;
    let (mut missions, host) = missions(&dir, project, vec![task]);
    let asker = spawnable(&dir, project, id, &mut missions);
    let _ = prompts(&host);

    let (request_id, _) = missions
        .request_spawn(
            project,
            id,
            Actor::Agent(asker),
            "worker",
            None,
            None,
            "go".to_string(),
            "because".to_string(),
        )
        .unwrap();

    // The user changed the kind in the *Needs you* row before allowing — which is exactly why the
    // outcome carries one at all.
    let spawned = AgentId::generate();
    let replies = missions.answer_spawn(
        project,
        id,
        request_id,
        ubiq_proto::mission::SpawnOutcome::Launched {
            agent: spawned,
            kind: "reviewer".to_string(),
        },
    );

    let record = missions.record(project, id).unwrap();
    assert!(
        record.pending_spawns.is_empty(),
        "an answered request stops being pending"
    );
    assert_eq!(
        record.member(spawned).map(|entry| entry.role),
        Some(MissionRole::Worker),
        "the new agent is on the roster — the half M11 says cannot be recomputed later"
    );

    let line = appended(&replies)
        .into_iter()
        .find(|entry| matches!(entry.event, JournalEvent::SpawnAnswered { .. }))
        .expect("the outcome is journaled");
    assert_eq!(
        line.event,
        JournalEvent::SpawnAnswered {
            agent_kind: "reviewer".to_string(),
            agent: Some(spawned),
        }
    );

    // And the requester is told, as its next prompt, which kind it actually got.
    let told = prompts(&host);
    let answer = told
        .iter()
        .find(|(agent, _)| *agent == asker)
        .map(|(_, text)| text.clone())
        .expect("the requester hears the outcome");
    assert!(answer.contains(&spawned.to_string()), "{answer}");
    assert!(answer.contains("'reviewer'"), "{answer}");

    // Answering the same request twice is harmless — the row is already gone.
    let again = missions.answer_spawn(
        project,
        id,
        request_id,
        ubiq_proto::mission::SpawnOutcome::Declined {
            reason: "no".to_string(),
        },
    );
    assert!(again.is_empty(), "a stale answer is discarded");
}

/// A decline is a real answer, not a failure: it is journaled and the requester is told why, so it
/// can carry on rather than wait for something that will never come.
#[test]
fn a_declined_spawn_tells_the_requester_why() {
    let dir = TempDir::new().unwrap();
    let project = ProjectId::generate();
    let task = anchor("Ship it", Status::InProgress);
    let id = task.id;
    let (mut missions, host) = missions(&dir, project, vec![task]);
    let asker = spawnable(&dir, project, id, &mut missions);
    let _ = prompts(&host);

    let (request_id, _) = missions
        .request_spawn(
            project,
            id,
            Actor::Agent(asker),
            "worker",
            None,
            None,
            "go".to_string(),
            "because".to_string(),
        )
        .unwrap();
    let replies = missions.answer_spawn(
        project,
        id,
        request_id,
        ubiq_proto::mission::SpawnOutcome::Declined {
            reason: "two agents is enough for this mission".to_string(),
        },
    );

    assert!(
        missions
            .record(project, id)
            .unwrap()
            .pending_spawns
            .is_empty()
    );
    let line = appended(&replies)
        .into_iter()
        .find(|entry| matches!(entry.event, JournalEvent::SpawnAnswered { .. }))
        .unwrap();
    assert_eq!(
        line.event,
        JournalEvent::SpawnAnswered {
            agent_kind: "worker".to_string(),
            agent: None,
        }
    );
    let told = prompts(&host);
    let answer = told
        .iter()
        .find(|(agent, _)| *agent == asker)
        .map(|(_, text)| text.clone())
        .expect("a decline is still an answer");
    assert!(answer.contains("two agents is enough"), "{answer}");
}

// ── the coordinator handoff (M10) ───────────────────────────────────

/// Handing the mission to a second coordinator **detaches** the first: it stays on the roster, it
/// is told it is no longer coordinating, and nothing anywhere kills it. The history carries both.
#[test]
fn a_coordinator_handoff_detaches_rather_than_kills() {
    let dir = TempDir::new().unwrap();
    let project = ProjectId::generate();
    let task = anchor("Ship it", Status::InProgress);
    let id = task.id;
    let (mut missions, host) = missions(&dir, project, vec![task]);

    let first = AgentId::generate();
    missions.set_field(project, id, MissionField::Coordinator(Some(first)));
    let _ = prompts(&host);

    let second = AgentId::generate();
    let replies = missions.set_field(project, id, MissionField::Coordinator(Some(second)));
    let record = changed(&replies).expect("the handoff announces the record");

    assert_eq!(record.coordinator, Some(second));
    // Detached, not removed: the outgoing one is still a member, as a worker.
    assert_eq!(
        record.member(first).map(|entry| entry.role),
        Some(MissionRole::Worker),
        "the previous coordinator is detached, never killed and never dropped"
    );
    assert_eq!(
        record.member(second).map(|entry| entry.role),
        Some(MissionRole::Coordinator)
    );
    // Nothing in the reply ends a conversation — a detach is a role change and nothing else.
    assert!(!replies.iter().any(|reply| matches!(
        reply.message(),
        Message::EndConversation { .. } | Message::AbortConversation { .. }
    )));

    // Both steps are in the history, oldest first.
    let handed: Vec<Option<AgentId>> = record
        .coordinator_history
        .iter()
        .map(|entry| entry.agent)
        .collect();
    assert_eq!(handed, vec![Some(first), Some(second)]);

    // The outgoing one is told to stand down, and the new one is pointed at the four things M10
    // names rather than handed a copy of any of them.
    let told = prompts(&host);
    let stand_down = told
        .iter()
        .find(|(agent, _)| *agent == first)
        .map(|(_, text)| text.clone())
        .expect("the outgoing coordinator is told");
    assert!(stand_down.contains("no longer"), "{stand_down}");

    let briefing = told
        .iter()
        .find(|(agent, _)| *agent == second)
        .map(|(_, text)| text.clone())
        .expect("the incoming coordinator is briefed");
    for pointer in [
        "read_brief",
        "read_plan",
        "list_documents",
        "mission_overview",
    ] {
        assert!(
            briefing.contains(pointer),
            "{briefing} is missing {pointer}"
        );
    }
}

/// A line to a roster member reaches its row **and** its live conversation; a line to a stranger
/// reaches neither.
#[test]
fn a_message_to_a_member_reaches_a_running_agent() {
    let dir = TempDir::new().unwrap();
    let project = ProjectId::generate();
    let task = anchor("Ship it", Status::InProgress);
    let id = task.id;
    let (mut missions, work, host) = with_work(&dir, project, vec![task]);

    // A member that is actually on the board, so the durable half has a row to land on — the mock
    // agents `Work::prepare` mints are the only ones a test can have without a harness.
    let member = work.lock().agents(project).1[0].id;
    missions.join(project, id, member, MissionRole::Worker, Actor::Host);
    let _ = prompts(&host);

    let replies = missions
        .message_member(
            project,
            id,
            member,
            "Look at the contract first.".to_string(),
        )
        .expect("a roster member can be messaged");
    // The durable half: the line is on the agent's row.
    assert!(
        replies
            .iter()
            .any(|reply| matches!(reply.message(), Message::AgentChanged { .. })),
        "the line is in the agent's thread"
    );
    // The live half: a prompt is on its way to the conversation.
    assert_eq!(
        prompts(&host),
        vec![(member, "Look at the contract first.".to_string())]
    );

    let stranger = AgentId::generate();
    let refused = missions
        .message_member(project, id, stranger, "psst".to_string())
        .unwrap_err();
    assert!(
        refused.contains("not on this mission's roster"),
        "{refused}"
    );
    assert!(
        prompts(&host).is_empty(),
        "a refused message reaches nobody at all"
    );
}

// ── the scheduler (M22, M23, M24) ───────────────────────────────────
//
// **The one piece here that can do real damage if it is wrong.** An autonomous scheduler that
// picks the wrong task hands a person's work to a model at three in the morning, and one that
// releases a task an agent is halfway through throws that work away — so every rule M23 states
// gets a test that fails when the rule is broken, not one that passes when it happens to hold.
//
// Every test drives `Missions::schedule_at` with a stated clock: the idle grace is the one rule
// that depends on time, and a grace-period rule tested by sleeping is a rule nobody runs.

/// The moment every scheduler test starts from, so `held_since + IDLE_GRACE` is arithmetic rather
/// than a race.
fn at(minutes: i64) -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 9, 25, 9, 0, 0).unwrap() + chrono::Duration::minutes(minutes)
}

/// A child of `mission`, in a column and with labels — the whole of what the pool reads.
fn child(mission: TaskId, title: &str, status: Status, labels: &[&str]) -> TaskRecord {
    let mut task = TaskRecord::new(title.to_string(), None, at(0));
    task.parent = Some(mission);
    task.status = status;
    task.key = Some(title.to_string());
    task.labels = labels
        .iter()
        .map(|name| Label::new((*name).to_string(), 0))
        .collect();
    task
}

/// A live agent as the board holds one: an id, what it is doing, and what it has.
fn live(id: AgentId, activity: Activity, task: Option<TaskId>) -> WorkAgent {
    WorkAgent {
        id,
        session: SessionId::generate(),
        task,
        parent: None,
        name: format!("worker-{id}"),
        summary: None,
        role: "worker".to_string(),
        activity,
        note: String::new(),
        branch: "main".to_string(),
        tokens: 0.0,
        harness: "Claude Code".to_string(),
        account: String::new(),
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

/// A mission in auto and In progress, with one agent kind, and the agents named put on the board
/// and on the roster as the scheduler's own.
fn auto_mission(
    dir: &TempDir,
    project: ProjectId,
    tasks: Vec<TaskRecord>,
    anchor_id: TaskId,
) -> (Missions, work::Handle, Listener) {
    let (mut missions, work, host) = with_work(dir, project, tasks);
    missions.set_field(
        project,
        anchor_id,
        MissionField::AgentKinds(vec![AgentKind {
            name: "worker".to_string(),
            description: "Does one task.".to_string(),
            labels: vec!["ui".to_string()],
            ..Default::default()
        }]),
    );
    missions.set_field(
        project,
        anchor_id,
        MissionField::DefaultKind(Some("worker".to_string())),
    );
    missions.set_field(
        project,
        anchor_id,
        MissionField::Execution(ExecutionMode::Auto),
    );
    // A record is made lazily in the phase its anchor implies (M3), which for a fresh board is
    // Requirements — and **auto only runs in In progress** (M22), so the scheduler would answer
    // nothing at all without this.
    missions.set_phase(project, anchor_id, Phase::InProgress);
    (missions, work, host)
}

/// Put an agent on the board and on the mission's roster as one of the scheduler's, holding
/// nothing, with the affinity labels it has picked up so far.
#[allow(clippy::too_many_arguments)]
fn rostered(
    dir: &TempDir,
    missions: &mut Missions,
    work: &work::Handle,
    project: ProjectId,
    mission: TaskId,
    labels: &[&str],
    held: usize,
    last: Option<chrono::DateTime<Utc>>,
) -> AgentId {
    let agent = AgentId::generate();
    work.lock().add_live_agent(
        project,
        live(agent, Activity::Thinking, None),
        WorkSession {
            id: SessionId::generate(),
            name: "main".to_string(),
            branch: "main".to_string(),
            worktree: false,
        },
    );
    missions.join(project, mission, agent, MissionRole::Worker, Actor::Host);
    let mut record = missions.record(project, mission).unwrap();
    let entry = record.member_mut(agent).unwrap();
    entry.scheduled = true;
    entry.tasks_held = held;
    entry.last_held_at = last;
    entry.labels = labels.iter().map(|name| (*name).to_string()).collect();
    MissionStore::new(dir.path().to_path_buf())
        .save(&record)
        .unwrap();
    agent
}

/// The journal lines one call wrote, by kind.
fn kinds(replies: &[Reply]) -> Vec<&'static str> {
    appended(replies)
        .iter()
        .map(|entry| entry.event.kind())
        .collect::<Vec<_>>()
}

/// Every task the board now holds, by key.
fn status_by_key(work: &work::Handle, project: ProjectId, key: &str) -> Status {
    let (_, tasks) = work.lock().tasks(project);
    tasks
        .iter()
        .find(|task| task.key.as_deref() == Some(key))
        .expect("the task is on the board")
        .status
}

/// **The pool is `Ready`, ready and unassigned — nothing else.** Three tasks the scheduler must
/// leave alone sit beside the one it must take: a `Backlog` task (promoting is how work is
/// released), one waiting on a prerequisite, and one a person has already given to an agent.
#[test]
fn the_pool_is_ready_tasks_that_are_ready_and_nobody_has() {
    let dir = TempDir::new().unwrap();
    let project = ProjectId::generate();
    let anchor_task = anchor("Ship it", Status::InProgress);
    let mission = anchor_task.id;

    let backlog = child(mission, "BACKLOG", Status::Backlog, &["ui"]);
    let mut waiting = child(mission, "WAITING", Status::Ready, &["ui"]);
    waiting.prerequisites = vec![backlog.id];
    let taken = child(mission, "TAKEN", Status::Ready, &["ui"]);
    let takeable = child(mission, "TAKEABLE", Status::Ready, &["ui"]);

    let (mut missions, work, host) = auto_mission(
        &dir,
        project,
        vec![
            anchor_task,
            backlog,
            waiting,
            taken.clone(),
            takeable.clone(),
        ],
        mission,
    );
    let hand = rostered(
        &dir,
        &mut missions,
        &work,
        project,
        mission,
        &["ui"],
        0,
        None,
    );
    // The hand assignment: a person gave `TAKEN` to an agent, which takes it out of the pool.
    work.lock().assign_agent(project, hand, Some(taken.id));
    let worker = rostered(
        &dir,
        &mut missions,
        &work,
        project,
        mission,
        &["ui"],
        0,
        Some(at(0)),
    );

    let replies = missions.schedule_at(project, mission, at(1));
    let scheduled: Vec<(TaskId, AgentId)> = appended(&replies)
        .iter()
        .filter_map(|entry| match entry.event {
            JournalEvent::TaskScheduled { task, agent, .. } => Some((task, agent)),
            _ => None,
        })
        .collect();
    assert_eq!(
        scheduled,
        vec![(takeable.id, worker)],
        "only the ready, unblocked, unheld task is picked — and the free agent gets it"
    );
    // The pool the overview reports agrees: one task, and it is not the backlog one.
    let view = missions.scheduler_view(project, mission);
    assert_eq!(view.pool.len(), 0, "the one task in it has just been taken");
    assert!(view.running);
    let _ = host;
}

/// **Slots cap concurrency.** Four ready tasks and a parallelism of two: exactly two go out, and
/// the third only once a slot comes back.
#[test]
fn parallelism_is_how_many_tasks_can_be_out_at_once() {
    let dir = TempDir::new().unwrap();
    let project = ProjectId::generate();
    let anchor_task = anchor("Ship it", Status::InProgress);
    let mission = anchor_task.id;
    let tasks: Vec<TaskRecord> = (1..=4)
        .map(|n| child(mission, &format!("T{n}"), Status::Ready, &["ui"]))
        .collect();
    let mut all = vec![anchor_task];
    all.extend(tasks.clone());

    let (mut missions, work, _host) = auto_mission(&dir, project, all, mission);
    for _ in 0..4 {
        rostered(
            &dir,
            &mut missions,
            &work,
            project,
            mission,
            &["ui"],
            0,
            Some(at(0)),
        );
    }

    let replies = missions.schedule_at(project, mission, at(1));
    let out = appended(&replies)
        .iter()
        .filter(|entry| matches!(entry.event, JournalEvent::TaskScheduled { .. }))
        .count();
    assert_eq!(out, 2, "parallelism is 2, so two tasks go out and no more");

    // A second pass with nothing changed hands out nothing: the slots are still full.
    let again = missions.schedule_at(project, mission, at(2));
    assert!(
        !kinds(&again).contains(&"task_scheduled"),
        "a pass with no free slot schedules nothing: {:?}",
        kinds(&again)
    );
}

/// **Affinity picks the agent, and zero overlap never reuses one.** Two idle agents, one sharing
/// both of the task's labels and one sharing neither: the first gets it. With *no* agent sharing
/// anything, the scheduler spawns rather than reusing the one that is sitting there.
#[test]
fn affinity_picks_the_agent_and_zero_overlap_never_reuses_one() {
    let dir = TempDir::new().unwrap();
    let project = ProjectId::generate();
    let anchor_task = anchor("Ship it", Status::InProgress);
    let mission = anchor_task.id;
    let task = child(mission, "T1", Status::Ready, &["ui", "api"]);
    let (mut missions, work, _host) =
        auto_mission(&dir, project, vec![anchor_task, task.clone()], mission);
    let stranger = rostered(
        &dir,
        &mut missions,
        &work,
        project,
        mission,
        &["docs"],
        1,
        Some(at(0)),
    );
    let fit = rostered(
        &dir,
        &mut missions,
        &work,
        project,
        mission,
        &["ui", "api"],
        1,
        Some(at(0)),
    );

    let replies = missions.schedule_at(project, mission, at(1));
    let line = appended(&replies)
        .into_iter()
        .find(|entry| matches!(entry.event, JournalEvent::TaskScheduled { .. }))
        .expect("the task was scheduled");
    let JournalEvent::TaskScheduled { agent, labels, .. } = &line.event else {
        unreachable!()
    };
    assert_eq!(*agent, fit, "the overlapping agent gets it, not {stranger}");
    assert_eq!(labels, &vec!["ui".to_string(), "api".to_string()]);
    assert!(
        line.text.contains("affinity ui, api"),
        "the line says why: {}",
        line.text
    );
}

/// The other half of the same rule, on its own so a failure names which half broke: an agent with
/// **no** shared label is never reused, and the scheduler asks for a fresh one instead.
#[test]
fn a_task_with_nothing_in_common_spawns_rather_than_reusing() {
    let dir = TempDir::new().unwrap();
    let project = ProjectId::generate();
    let anchor_task = anchor("Ship it", Status::InProgress);
    let mission = anchor_task.id;
    let task = child(mission, "T1", Status::Ready, &["ui"]);
    let (mut missions, work, _host) =
        auto_mission(&dir, project, vec![anchor_task, task.clone()], mission);
    let stranger = rostered(
        &dir,
        &mut missions,
        &work,
        project,
        mission,
        &["docs"],
        1,
        Some(at(0)),
    );

    let replies = missions.schedule_at(project, mission, at(1));
    assert!(
        !kinds(&replies).contains(&"task_scheduled"),
        "zero overlap must never reuse {stranger}: {:?}",
        kinds(&replies)
    );
    let request = replies
        .iter()
        .find_map(|reply| match reply.message() {
            Message::MissionSpawnRequest { request, .. } => Some((**request).clone()),
            _ => None,
        })
        .expect("a fresh agent was asked for instead");
    assert_eq!(request.task, Some(task.id));
    assert_eq!(request.kind, "worker");
    assert!(
        request.auto,
        "a scheduler's spawn bypasses the policy's ask — choosing auto is the consent"
    );
}

/// **A finished task frees the slot, and `on_finish` says what becomes of the agent.** Under
/// `ReuseOrStop` with another matching task waiting, the agent is handed it; with nothing left, it
/// is retired.
#[test]
fn a_finished_task_hands_over_or_retires_per_on_finish() {
    let dir = TempDir::new().unwrap();
    let project = ProjectId::generate();
    let anchor_task = anchor("Ship it", Status::InProgress);
    let mission = anchor_task.id;
    let first = child(mission, "FIRST", Status::Ready, &["ui"]);
    let second = child(mission, "SECOND", Status::Ready, &["ui"]);
    let (mut missions, work, _host) = auto_mission(
        &dir,
        project,
        vec![anchor_task, first.clone(), second.clone()],
        mission,
    );
    let worker = rostered(
        &dir,
        &mut missions,
        &work,
        project,
        mission,
        &["ui"],
        0,
        Some(at(0)),
    );

    // It takes the first, works it, and moves it to In review — which is what "done" means here.
    missions.schedule_at(project, mission, at(1));
    work.lock()
        .move_task(project, first.id, Status::InReview, None);

    let replies = missions.schedule_at(project, mission, at(2));
    let lines = kinds(&replies);
    assert!(lines.contains(&"task_finished"), "{lines:?}");
    assert!(
        lines.contains(&"task_scheduled"),
        "the slot came back and the second task went to the same agent: {lines:?}"
    );
    assert!(
        !lines.contains(&"agent_retired"),
        "an agent handed more work is not retired: {lines:?}"
    );

    // Nothing left: finishing the second retires it.
    work.lock()
        .move_task(project, second.id, Status::InReview, None);
    let replies = missions.schedule_at(project, mission, at(3));
    let lines = kinds(&replies);
    assert!(
        lines.contains(&"agent_retired"),
        "with nothing left the agent is stopped: {lines:?}"
    );
    assert!(
        appended(&replies)
            .iter()
            .any(|entry| entry.event == JournalEvent::AgentRetired { agent: worker })
    );
    assert!(
        lines.contains(&"scheduler_finished"),
        "and the scheduler says the work is done: {lines:?}"
    );
}

/// **`OnFinish::Stop` never hands over**, even with a matching task waiting and a slot free.
#[test]
fn on_finish_stop_retires_instead_of_handing_over() {
    let dir = TempDir::new().unwrap();
    let project = ProjectId::generate();
    let anchor_task = anchor("Ship it", Status::InProgress);
    let mission = anchor_task.id;
    let first = child(mission, "FIRST", Status::Ready, &["ui"]);
    let second = child(mission, "SECOND", Status::Ready, &["ui"]);
    let (mut missions, work, _host) = auto_mission(
        &dir,
        project,
        vec![anchor_task, first.clone(), second],
        mission,
    );
    missions.set_field(project, mission, MissionField::OnFinish(OnFinish::Stop));
    let worker = rostered(
        &dir,
        &mut missions,
        &work,
        project,
        mission,
        &["ui"],
        1,
        Some(at(0)),
    );
    // It is holding the first task already.
    let mut record = missions.record(project, mission).unwrap();
    let entry = record.member_mut(worker).unwrap();
    entry.current_task = Some(first.id);
    entry.held_since = Some(at(0));
    MissionStore::new(dir.path().to_path_buf())
        .save(&record)
        .unwrap();
    work.lock()
        .move_task(project, first.id, Status::InReview, None);

    let replies = missions.schedule_at(project, mission, at(1));
    let lines = kinds(&replies);
    assert!(lines.contains(&"agent_retired"), "{lines:?}");
    assert!(
        !lines.contains(&"task_scheduled"),
        "under Stop the second task waits for a fresh agent rather than going to this one: \
         {lines:?}"
    );
}

/// **A failed attempt releases the task; the second blocks it.** The agent ends without moving the
/// task off `InProgress`, twice — and `max_attempts` is 2, so the second time the task goes to
/// `Blocked` and the coordinator is told.
#[test]
fn a_failed_attempt_releases_the_task_and_the_second_blocks_it() {
    let dir = TempDir::new().unwrap();
    let project = ProjectId::generate();
    let anchor_task = anchor("Ship it", Status::InProgress);
    let mission = anchor_task.id;
    let task = child(mission, "T1", Status::Ready, &["ui"]);
    let (mut missions, work, host) =
        auto_mission(&dir, project, vec![anchor_task, task.clone()], mission);
    let coordinator = AgentId::generate();
    missions.set_field(
        project,
        mission,
        MissionField::Coordinator(Some(coordinator)),
    );
    let _ = prompts(&host);

    for attempt in 1..=2 {
        let worker = rostered(
            &dir,
            &mut missions,
            &work,
            project,
            mission,
            &["ui"],
            0,
            Some(at(0)),
        );
        missions.schedule_at(project, mission, at(attempt * 10));
        // The worker starts the task and then its harness ends without finishing.
        work.lock()
            .move_task(project, task.id, Status::InProgress, None);
        work.lock().remove_live_agent(project, worker);

        let replies = missions.schedule_at(project, mission, at(attempt * 10 + 1));
        let released = appended(&replies)
            .into_iter()
            .find_map(|entry| match entry.event {
                JournalEvent::TaskReleased {
                    task: id, attempt, ..
                } if id == task.id => Some((attempt, entry.text.clone())),
                _ => None,
            })
            .unwrap_or_else(|| panic!("attempt {attempt} must be released: {:?}", kinds(&replies)));
        assert_eq!(released.0, attempt as usize);
        assert!(
            released.1.contains(&format!("attempt {attempt}")),
            "the line says which attempt: {}",
            released.1
        );

        if attempt == 1 {
            assert_eq!(
                status_by_key(&work, project, "T1"),
                Status::Ready,
                "a first failure puts the task back in the pool"
            );
            assert!(!kinds(&replies).contains(&"task_blocked"));
        } else {
            assert_eq!(
                status_by_key(&work, project, "T1"),
                Status::Blocked,
                "out of attempts, the task is a person's problem now"
            );
            assert!(kinds(&replies).contains(&"task_blocked"));
            assert!(
                prompts(&host)
                    .iter()
                    .any(|(who, text)| *who == coordinator && text.contains("gave up")),
                "the coordinator is told"
            );
        }
    }
}

/// **An agent waiting on a person keeps its slot.** `Needs you` is not a failure: the task stays
/// with it, no attempt is counted, and the second task waits rather than being handed out.
#[test]
fn an_agent_waiting_on_a_person_keeps_its_slot() {
    let dir = TempDir::new().unwrap();
    let project = ProjectId::generate();
    let anchor_task = anchor("Ship it", Status::InProgress);
    let mission = anchor_task.id;
    let first = child(mission, "FIRST", Status::Ready, &["ui"]);
    let second = child(mission, "SECOND", Status::Ready, &["ui"]);
    let (mut missions, work, _host) = auto_mission(
        &dir,
        project,
        vec![anchor_task, first.clone(), second],
        mission,
    );
    missions.set_field(project, mission, MissionField::Parallelism(1));
    let worker = rostered(
        &dir,
        &mut missions,
        &work,
        project,
        mission,
        &["ui"],
        0,
        Some(at(0)),
    );
    missions.schedule_at(project, mission, at(1));
    work.lock()
        .move_task(project, first.id, Status::InProgress, None);
    if let Some(agent) = work.lock().live_agent_mut(project, worker) {
        agent.activity = Activity::NeedsYou;
    }

    // Long past the grace, and still nothing is taken away.
    let replies = missions.schedule_at(project, mission, at(120));
    let lines = kinds(&replies);
    assert!(
        !lines.contains(&"task_released"),
        "waiting on a person is not a failed attempt: {lines:?}"
    );
    assert!(
        !lines.contains(&"task_scheduled"),
        "and it still holds the only slot: {lines:?}"
    );
}

/// **An agent that never starts its task loses it after the grace, and not a minute before.**
#[test]
fn a_stalled_agent_loses_its_task_only_after_the_grace() {
    let dir = TempDir::new().unwrap();
    let project = ProjectId::generate();
    let anchor_task = anchor("Ship it", Status::InProgress);
    let mission = anchor_task.id;
    let task = child(mission, "T1", Status::Ready, &["ui"]);
    let (mut missions, work, _host) = auto_mission(&dir, project, vec![anchor_task, task], mission);
    rostered(
        &dir,
        &mut missions,
        &work,
        project,
        mission,
        &["ui"],
        0,
        Some(at(0)),
    );
    missions.schedule_at(project, mission, at(0));

    // Four minutes in, still within the five-minute grace: nothing is taken back.
    let early = missions.schedule_at(project, mission, at(4));
    assert!(
        !kinds(&early).contains(&"task_released"),
        "inside the grace an agent is given the benefit of the doubt: {:?}",
        kinds(&early)
    );
    // Six minutes in, it has plainly not started.
    let late = missions.schedule_at(project, mission, at(6));
    assert!(
        kinds(&late).contains(&"task_released"),
        "past the grace the slot is taken back: {:?}",
        kinds(&late)
    );
}

/// **Leaving In progress pauses the scheduler**, and the mode is still what the user chose when it
/// comes back — M22's rule, and the one that stops a mission moved to Refining from carrying on
/// assigning work behind the user's back.
#[test]
fn leaving_in_progress_pauses_the_scheduler() {
    let dir = TempDir::new().unwrap();
    let project = ProjectId::generate();
    let anchor_task = anchor("Ship it", Status::InProgress);
    let mission = anchor_task.id;
    let task = child(mission, "T1", Status::Ready, &["ui"]);
    let (mut missions, work, _host) = auto_mission(&dir, project, vec![anchor_task, task], mission);
    rostered(
        &dir,
        &mut missions,
        &work,
        project,
        mission,
        &["ui"],
        0,
        Some(at(0)),
    );

    missions.set_phase(project, mission, Phase::Refining);
    let paused = missions.schedule_at(project, mission, at(1));
    assert!(paused.is_empty(), "a paused scheduler decides nothing");
    let view = missions.scheduler_view(project, mission);
    assert!(view.auto, "the mode is still auto — it is only paused");
    assert!(!view.running);

    // Back in, and it picks up where it left off without the user re-choosing anything.
    missions.set_phase(project, mission, Phase::InProgress);
    let resumed = missions.schedule_at(project, mission, at(2));
    assert!(
        kinds(&resumed).contains(&"task_scheduled"),
        "back in progress it runs again: {:?}",
        kinds(&resumed)
    );
}

/// **Switching the mode is journaled** (M22). Auto mode with no line saying when it started is the
/// thing that makes the rest of the journal unreadable.
#[test]
fn switching_the_execution_mode_writes_a_journal_line() {
    let dir = TempDir::new().unwrap();
    let project = ProjectId::generate();
    let anchor_task = anchor("Ship it", Status::InProgress);
    let mission = anchor_task.id;
    let (mut missions, _host) = missions(&dir, project, vec![anchor_task]);

    let replies = missions.set_field(
        project,
        mission,
        MissionField::Execution(ExecutionMode::Auto),
    );
    let line = appended(&replies)
        .into_iter()
        .find(|entry| matches!(entry.event, JournalEvent::ExecutionChanged { .. }))
        .expect("the switch is journaled");
    assert_eq!(
        line.event,
        JournalEvent::ExecutionChanged {
            mode: ExecutionMode::Auto
        }
    );
    assert!(line.text.contains("scheduler"), "{}", line.text);

    // Back to manual, and that is journaled too.
    let replies = missions.set_field(
        project,
        mission,
        MissionField::Execution(ExecutionMode::Manual),
    );
    assert!(kinds(&replies).contains(&"execution_changed"));
    // A write that changes nothing writes nothing.
    let again = missions.set_field(
        project,
        mission,
        MissionField::Execution(ExecutionMode::Manual),
    );
    assert!(again.is_empty(), "a no-op switch is not a journal line");
}

/// **A hand assignment removes the task from the pool** (M22) even while the scheduler is running,
/// and the scheduler does not take it back.
#[test]
fn a_hand_assignment_takes_a_task_out_of_the_pool() {
    let dir = TempDir::new().unwrap();
    let project = ProjectId::generate();
    let anchor_task = anchor("Ship it", Status::InProgress);
    let mission = anchor_task.id;
    let task = child(mission, "T1", Status::Ready, &["ui"]);
    let (mut missions, work, _host) =
        auto_mission(&dir, project, vec![anchor_task, task.clone()], mission);
    let theirs = AgentId::generate();
    work.lock().add_live_agent(
        project,
        live(theirs, Activity::Thinking, Some(task.id)),
        WorkSession {
            id: SessionId::generate(),
            name: "main".to_string(),
            branch: "main".to_string(),
            worktree: false,
        },
    );
    rostered(
        &dir,
        &mut missions,
        &work,
        project,
        mission,
        &["ui"],
        0,
        Some(at(0)),
    );

    let replies = missions.schedule_at(project, mission, at(1));
    assert!(
        !kinds(&replies).contains(&"task_scheduled"),
        "a task somebody already has is not the scheduler's to give: {:?}",
        kinds(&replies)
    );
}

/// **Every decision can be read back.** One pass, several decisions, and the journal on disk says
/// all of them in order with a sentence a person can read.
#[test]
fn every_decision_the_scheduler_makes_is_in_the_journal() {
    let dir = TempDir::new().unwrap();
    let project = ProjectId::generate();
    let anchor_task = anchor("Ship it", Status::InProgress);
    let mission = anchor_task.id;
    let first = child(mission, "FIRST", Status::Ready, &["ui"]);
    let second = child(mission, "SECOND", Status::Ready, &["ui"]);
    let (mut missions, work, _host) = auto_mission(
        &dir,
        project,
        vec![anchor_task, first.clone(), second.clone()],
        mission,
    );
    let worker = rostered(
        &dir,
        &mut missions,
        &work,
        project,
        mission,
        &["ui"],
        0,
        Some(at(0)),
    );

    missions.schedule_at(project, mission, at(1));
    work.lock()
        .move_task(project, first.id, Status::InReview, None);
    missions.schedule_at(project, mission, at(2));
    work.lock()
        .move_task(project, second.id, Status::Done, None);
    missions.schedule_at(project, mission, at(3));

    // Read back off disk, not out of the replies: the journal is the record.
    let (page, _) = paged(&missions.load_journal(project, mission, None, Some(100)));
    let story: Vec<&'static str> = page.iter().rev().map(|entry| entry.event.kind()).collect();
    for wanted in [
        "execution_changed",
        "task_scheduled",
        "task_finished",
        "agent_retired",
        "scheduler_finished",
    ] {
        assert!(
            story.contains(&wanted),
            "{wanted} is missing from {story:?}"
        );
    }
    let scheduled: Vec<String> = page
        .iter()
        .filter(|entry| matches!(entry.event, JournalEvent::TaskScheduled { .. }))
        .map(|entry| entry.text.clone())
        .collect();
    assert!(
        scheduled
            .iter()
            .any(|text| text.contains("FIRST") && text.contains(&worker.to_string())),
        "a line names the task and the agent: {scheduled:?}"
    );
}
