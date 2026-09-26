//! Moving a mission's phase, from the window (M5, M6).
//!
//! **Every gesture here is one message.** `Message::SetPhase` is the user's, and the host tells
//! the three acts apart by the phase named: the pending request's phase confirms it, the phase the
//! mission is already in declines it, anything else is a free move. `RequestPhase` is the
//! coordinator's and carries no requester, so the window never sends one — these tests are what
//! keeps that true. `tests/new_mission.rs`'s `Fixture` is the model this one follows.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use chrono::Utc;
use gpui::{AppContext as _, Entity, TestAppContext, WindowHandle};
use gpui_component::Root;
use ubiq::app::{AppState, BusHub};
use ubiq::state::WindowRegistry;
use ubiq::state::mission::{KindPick, KindTarget, MissionMenuRow, MissionSpawnRow};
use ubiq_proto::bus::{self, FromClient, To};
use ubiq_proto::ids::{ProjectId, SpawnId, TaskId};
use ubiq_proto::messages::{AgentDefinition, Message};
use ubiq_proto::mission::{
    Actor, AgentKind, ExecutionMode, JournalEntry, JournalEvent, MissionField, MissionRecord,
    MissionRole, PendingPhase, PendingSpawn, Phase, RosterEntry, SpawnOutcome, SpawnPolicy,
};
use ubiq_proto::projects::{ProjectHealth, ProjectRecord, ProjectSnapshot};
use ubiq_proto::work::{AgentId, TaskRecord};

const PATIENCE: Duration = Duration::from_millis(500);

struct Fixture {
    state: Entity<AppState>,
    window: WindowHandle<Root>,
    host: bus::HostEnd,
    project: ProjectId,
}

impl Fixture {
    fn open(cx: &mut TestAppContext) -> Self {
        let snapshot = a_project();
        let project = snapshot.record.id;
        let (hub, host) = bus::hub();

        cx.update(|cx| {
            gpui_component::init(cx);
            ubiq::theme::set_mode(ubiq::app::boot_theme(), cx);
            BusHub::install(hub, cx);
            WindowRegistry::install(cx);
            cx.global_mut::<WindowRegistry>().apply(snapshot);
        });

        let held: Rc<RefCell<Option<Entity<AppState>>>> = Rc::default();
        let taken = held.clone();
        let window = cx.add_window(move |window, cx| {
            let state = cx.new(|cx| AppState::for_project(Some(project), 'A', window, cx));
            *taken.borrow_mut() = Some(state.clone());
            Root::new(state, window, cx)
        });
        cx.run_until_parked();

        let state = held
            .borrow_mut()
            .take()
            .expect("the window built its state");
        Self {
            state,
            window,
            host,
            project,
        }
    }

    fn said(&self) -> Vec<Message> {
        let mut said = Vec::new();
        while let Ok(event) = self.host.recv_timeout(PATIENCE) {
            if let FromClient::Said { message, .. } = event {
                said.push(message);
            }
        }
        said
    }

    fn deliver(&self, message: Message, cx: &mut TestAppContext) {
        self.host.send(To::Everyone, message);
        cx.run_until_parked();
    }

    fn with<R>(
        &self,
        cx: &mut TestAppContext,
        f: impl FnOnce(&mut AppState, &mut gpui::Window, &mut gpui::Context<AppState>) -> R,
    ) -> R {
        let out = self
            .window
            .update(cx, |_, window, cx| {
                self.state.update(cx, |state, cx| f(state, window, cx))
            })
            .expect("the window is open");
        cx.run_until_parked();
        out
    }

    /// Put one mission's record in this window, the way the host does.
    fn mission(&self, record: MissionRecord, cx: &mut TestAppContext) {
        self.deliver(
            Message::MissionChanged {
                project_id: self.project,
                mission: Box::new(record),
            },
            cx,
        );
        self.said();
    }
}

fn a_project() -> ProjectSnapshot {
    ProjectSnapshot {
        record: ProjectRecord {
            id: ProjectId::generate(),
            name: "ubiq".to_string(),
            path: "/tmp/ubiq".to_string(),
            colour: 0,
            custom_colour: None,
            storage: Default::default(),
            temporary: false,
            created_at: Utc::now(),
            last_opened_at: None,
            search_excludes: Vec::new(),
            index: None,
            mission_term: None,
            tools: Vec::new(),
            managed_repos: Vec::new(),
            lanes: Vec::new(),
            runs_on: None,
            initials: String::new(),
        },
        health: ProjectHealth::Ok,
        open_panes: 0,
        ephemeral: false,
        workarea: "/tmp/ubiq-workarea".to_string(),
    }
}

fn a_mission(project: ProjectId, task: TaskId, phase: Phase) -> MissionRecord {
    MissionRecord::new(project, task, phase, Utc::now())
}

/// Clicking a step is a `SetPhase`, never a `RequestPhase`: the request is the coordinator's, and
/// a window that sent one would be putting words in an agent's mouth.
#[gpui::test]
fn a_step_on_the_stepper_moves_the_phase(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let task = TaskId::generate();
    fixture.mission(a_mission(fixture.project, task, Phase::Requirements), cx);

    fixture.with(cx, |state, _, cx| {
        state.set_mission_phase(task, Phase::InProgress, cx)
    });
    let said = fixture.said();
    assert!(said.iter().any(|m| matches!(
        m,
        Message::SetPhase { project_id, task_id, phase: Phase::InProgress }
            if *project_id == fixture.project && *task_id == task
    )));
    assert!(
        !said
            .iter()
            .any(|m| matches!(m, Message::RequestPhase { .. })),
        "the request is the coordinator's message, not the window's"
    );
}

/// Confirming names the phase that was asked for; declining names the phase the mission is
/// already in. Two answers, one message — the host tells them apart (M5).
#[gpui::test]
fn confirm_and_decline_are_both_set_phase(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let task = TaskId::generate();
    let mut record = a_mission(fixture.project, task, Phase::InProgress);
    record.pending_phase = Some(PendingPhase {
        phase: Phase::Completed,
        by: Actor::Host,
        summary: "Everything is done.".to_string(),
        at: Utc::now(),
    });
    fixture.mission(record, cx);

    fixture.with(cx, |state, _, cx| state.confirm_mission_phase(task, cx));
    assert!(fixture.said().iter().any(|m| matches!(
        m,
        Message::SetPhase { task_id, phase: Phase::Completed, .. } if *task_id == task
    )));

    fixture.with(cx, |state, _, cx| state.decline_mission_phase(task, cx));
    assert!(
        fixture.said().iter().any(|m| matches!(
            m,
            Message::SetPhase { task_id, phase: Phase::InProgress, .. } if *task_id == task
        )),
        "a decline names the phase it stays in"
    );
}

/// Nothing pending means nothing to confirm — the panel draws no action, and the app layer sends
/// nothing if one is asked for anyway.
#[gpui::test]
fn confirming_nothing_sends_nothing(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let task = TaskId::generate();
    fixture.mission(a_mission(fixture.project, task, Phase::Refining), cx);

    fixture.with(cx, |state, _, cx| state.confirm_mission_phase(task, cx));
    assert!(fixture.said().is_empty());
}

/// The `⋯`'s Complete and Abandon are phase moves and are live; Open on Teams and
/// Execution mode still are not, and a dead row never reaches the app layer.
#[gpui::test]
fn the_menu_completes_and_abandons(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let task = TaskId::generate();
    fixture.mission(a_mission(fixture.project, task, Phase::InProgress), cx);

    assert!(MissionMenuRow::Complete.enabled());
    assert!(MissionMenuRow::Abandon.enabled());
    // *Pause all agents* is live from this wave — see `stop_unloads_and_detach_clears_the_task`.
    assert!(MissionMenuRow::PauseAll.enabled());
    assert!(!MissionMenuRow::OpenOnTeams.enabled());
    assert!(!MissionMenuRow::ExecutionMode.enabled());

    let rows = MissionMenuRow::all();
    let complete = rows
        .iter()
        .position(|row| *row == MissionMenuRow::Complete)
        .expect("Complete is a row");
    fixture.with(cx, |state, _, cx| {
        state.open_mission_menu(task, (0., 0.), cx);
        state.pick_mission_menu(complete, cx);
    });
    assert!(fixture.said().iter().any(|m| matches!(
        m,
        Message::SetPhase { task_id, phase: Phase::Completed, .. } if *task_id == task
    )));

    let abandon = rows
        .iter()
        .position(|row| *row == MissionMenuRow::Abandon)
        .expect("Abandon is a row");
    fixture.with(cx, |state, _, cx| {
        state.open_mission_menu(task, (0., 0.), cx);
        state.pick_mission_menu(abandon, cx);
    });
    assert!(fixture.said().iter().any(|m| matches!(
        m,
        Message::SetPhase { task_id, phase: Phase::Abandoned, .. } if *task_id == task
    )));
}

/// A mission this window does not hold cannot be moved: the project id comes off the record, so
/// there is nothing to address the message to and nothing goes out.
#[gpui::test]
fn a_mission_nobody_holds_moves_nowhere(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    fixture.said();
    fixture.with(cx, |state, _, cx| {
        state.set_mission_phase(TaskId::generate(), Phase::Completed, cx)
    });
    assert!(fixture.said().is_empty());
}

// ── M13: the spawn policy is the window's ───────────────────────────
//
// **The host relays and the window launches**, so every one of these drives `AppState` and reads
// what went out on the bus. What a policy does is the whole of what these assert: the host has no
// opinion, and `MissionRecord::spawn_policy` is read nowhere else.

fn a_definition(id: &str) -> AgentDefinition {
    AgentDefinition {
        id: id.to_string(),
        description: None,
        agent_type: "claude-code".to_string(),
        account: None,
        model: None,
        mode: None,
        thinking: None,
        max_subagents: None,
        prompt: None,
        mcps: Vec::new(),
        mission_assistant: Some(true),
        mission_coordinator: false,
        mission_worker: false,
        disabled: false,
        project: None,
    }
}

fn a_request(kind: &str) -> PendingSpawn {
    PendingSpawn {
        id: SpawnId::generate(),
        by: Actor::Agent(AgentId::generate()),
        kind: kind.to_string(),
        definition: None,
        task: None,
        prompt: "Fix the retries.".to_string(),
        reason: "I need a second pair of hands.".to_string(),
        at: Utc::now(),
        auto: false,
    }
}

/// A mission whose table knows one kind, resolving to one definition the window has been told about.
fn a_mission_with_kinds(
    fixture: &Fixture,
    task: TaskId,
    policy: SpawnPolicy,
    cx: &mut TestAppContext,
) -> MissionRecord {
    fixture.deliver(
        Message::AgentDefinitions {
            definitions: vec![a_definition("worker")],
        },
        cx,
    );
    let mut record = a_mission(fixture.project, task, Phase::InProgress);
    record.spawn_policy = policy;
    record.agent_kinds = vec![AgentKind {
        name: "reviewer".to_string(),
        definition: Some("worker".to_string()),
        ..AgentKind::default()
    }];
    record
}

/// Under `ask`, nothing is launched and nothing is answered: the request sits on the record and
/// the panel draws it. A window that answered on its own would be answering for the user.
#[gpui::test]
fn ask_launches_nothing_and_answers_nothing(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let task = TaskId::generate();
    let mut record = a_mission_with_kinds(&fixture, task, SpawnPolicy::Ask, cx);
    let request = a_request("reviewer");
    record.pending_spawns = vec![request.clone()];
    fixture.mission(record, cx);

    fixture.deliver(
        Message::MissionSpawnRequest {
            project_id: fixture.project,
            task_id: task,
            request: Box::new(request),
        },
        cx,
    );
    let said = fixture.said();
    assert!(
        !said
            .iter()
            .any(|m| matches!(m, Message::StartConversation { .. })),
        "ask does not launch"
    );
    assert!(
        !said
            .iter()
            .any(|m| matches!(m, Message::AnswerSpawn { .. })),
        "the row is the answer, and the user has not given one"
    );
}

/// Under `auto` and below the cap, the window launches and answers — `StartConversation` first,
/// so the outcome names an agent that already exists, and the kind actually used is on it.
#[gpui::test]
fn auto_launches_under_the_cap_and_answers_with_the_kind_used(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let task = TaskId::generate();
    let mut record = a_mission_with_kinds(&fixture, task, SpawnPolicy::Auto, cx);
    record.spawn_limit = 2;
    let request = a_request("reviewer");
    let asked_by = match request.by {
        Actor::Agent(id) => id,
        _ => unreachable!("the fixture asks as an agent"),
    };
    record.pending_spawns = vec![request.clone()];
    fixture.mission(record, cx);

    fixture.deliver(
        Message::MissionSpawnRequest {
            project_id: fixture.project,
            task_id: task,
            request: Box::new(request.clone()),
        },
        cx,
    );
    let said = fixture.said();

    let started = said.iter().position(|m| {
        matches!(
            m,
            Message::StartConversation { spawned_by, definition, .. }
                if *spawned_by == Some(asked_by) && definition.as_deref() == Some("worker")
        )
    });
    let answered = said.iter().position(|m| {
        matches!(
            m,
            Message::AnswerSpawn { request_id, outcome: SpawnOutcome::Launched { kind, .. }, .. }
                if *request_id == request.id && kind == "reviewer"
        )
    });
    let (Some(started), Some(answered)) = (started, answered) else {
        panic!("auto launches and answers: {said:#?}");
    };
    assert!(
        started < answered,
        "the launch comes first, so the outcome names a real agent"
    );

    // The agent the outcome names is the one the window minted and started.
    let (Message::StartConversation { agent_id, .. }, Message::AnswerSpawn { outcome, .. }) =
        (&said[started], &said[answered])
    else {
        unreachable!()
    };
    assert!(matches!(
        outcome,
        SpawnOutcome::Launched { agent, .. } if agent == agent_id
    ));
}

/// Under `auto` and at the cap, the request falls back to being asked rather than refused. Over
/// the cap is a reason to ask, not a reason to say no.
#[gpui::test]
fn auto_over_the_cap_falls_back_to_asking(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let task = TaskId::generate();
    let mut record = a_mission_with_kinds(&fixture, task, SpawnPolicy::Auto, cx);
    record.spawn_limit = 1;
    record.roster = vec![RosterEntry::new(
        AgentId::generate(),
        MissionRole::Worker,
        Utc::now(),
    )];
    let request = a_request("reviewer");
    record.pending_spawns = vec![request.clone()];
    fixture.mission(record, cx);

    fixture.deliver(
        Message::MissionSpawnRequest {
            project_id: fixture.project,
            task_id: task,
            request: Box::new(request),
        },
        cx,
    );
    let said = fixture.said();
    assert!(
        said.is_empty(),
        "at the cap the row waits on the user: {said:#?}"
    );
}

/// Under `never` the request is declined with a sentence. A decline is a real answer, which is
/// why the requester is told something rather than left waiting.
#[gpui::test]
fn never_declines_with_a_sentence(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let task = TaskId::generate();
    let mut record = a_mission_with_kinds(&fixture, task, SpawnPolicy::Never, cx);
    let request = a_request("reviewer");
    record.pending_spawns = vec![request.clone()];
    fixture.mission(record, cx);

    fixture.deliver(
        Message::MissionSpawnRequest {
            project_id: fixture.project,
            task_id: task,
            request: Box::new(request.clone()),
        },
        cx,
    );
    let said = fixture.said();
    assert!(
        !said
            .iter()
            .any(|m| matches!(m, Message::StartConversation { .. })),
        "never launches nothing"
    );
    assert!(said.iter().any(|m| matches!(
        m,
        Message::AnswerSpawn { request_id, outcome: SpawnOutcome::Declined { reason }, .. }
            if *request_id == request.id && !reason.is_empty()
    )));
}

/// The scheduler's own request bypasses `ask` — choosing auto execution *was* the consent (M23) —
/// and is still refused by `never`.
#[gpui::test]
fn the_schedulers_request_bypasses_ask_but_not_never(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let task = TaskId::generate();
    let mut record = a_mission_with_kinds(&fixture, task, SpawnPolicy::Ask, cx);
    let mut request = a_request("reviewer");
    request.auto = true;
    record.pending_spawns = vec![request.clone()];
    fixture.mission(record.clone(), cx);

    fixture.deliver(
        Message::MissionSpawnRequest {
            project_id: fixture.project,
            task_id: task,
            request: Box::new(request.clone()),
        },
        cx,
    );
    assert!(
        fixture
            .said()
            .iter()
            .any(|m| matches!(m, Message::StartConversation { .. })),
        "the scheduler is not asked"
    );

    let mut refusing = record;
    refusing.spawn_policy = SpawnPolicy::Never;
    fixture.mission(refusing, cx);
    fixture.deliver(
        Message::MissionSpawnRequest {
            project_id: fixture.project,
            task_id: task,
            request: Box::new(request),
        },
        cx,
    );
    let said = fixture.said();
    assert!(
        !said
            .iter()
            .any(|m| matches!(m, Message::StartConversation { .. })),
        "never refuses the scheduler too"
    );
    assert!(said.iter().any(|m| matches!(
        m,
        Message::AnswerSpawn {
            outcome: SpawnOutcome::Declined { .. },
            ..
        }
    )));
}

/// The user may change the kind before allowing, and **that** is what the outcome names — the
/// whole reason `SpawnOutcome::Launched` carries a kind at all.
#[gpui::test]
fn a_changed_kind_is_what_the_outcome_names(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let task = TaskId::generate();
    let mut record = a_mission_with_kinds(&fixture, task, SpawnPolicy::Ask, cx);
    record.agent_kinds.push(AgentKind {
        name: "tester".to_string(),
        definition: Some("worker".to_string()),
        ..AgentKind::default()
    });
    let request = a_request("reviewer");
    record.pending_spawns = vec![request.clone()];
    fixture.mission(record, cx);

    // The kind picker, in the order `mission_kind_picks` answers: the mission's kinds first.
    let picked = fixture.with(cx, |state, _, cx| {
        let rows = state.mission_kind_picks(task, KindTarget::Pending(request.id));
        let at = rows
            .iter()
            .position(|pick| *pick == KindPick::Kind("tester".to_string()))
            .expect("tester is a kind");
        state.open_mission_kind_menu(task, KindTarget::Pending(request.id), (0., 0.), cx);
        state.pick_mission_kind_menu(at, cx);
        state.mission_spawn_pick(task, &request)
    });
    assert_eq!(
        picked.kind, "tester",
        "the pick is the window's, not the record's"
    );

    fixture.said();
    fixture.with(cx, |state, _, cx| {
        state.allow_mission_spawn(task, request.id, cx)
    });
    assert!(fixture.said().iter().any(|m| matches!(
        m,
        Message::AnswerSpawn { outcome: SpawnOutcome::Launched { kind, .. }, .. }
            if kind == "tester"
    )));
}

/// A kind nothing resolves to is a decline with a sentence, not a launch that fails as a spawn.
#[gpui::test]
fn a_kind_that_resolves_to_nothing_declines(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let task = TaskId::generate();
    let mut record = a_mission(fixture.project, task, Phase::InProgress);
    record.spawn_policy = SpawnPolicy::Auto;
    let request = a_request("nobody");
    record.pending_spawns = vec![request.clone()];
    fixture.mission(record, cx);

    fixture.deliver(
        Message::MissionSpawnRequest {
            project_id: fixture.project,
            task_id: task,
            request: Box::new(request.clone()),
        },
        cx,
    );
    let said = fixture.said();
    assert!(
        !said
            .iter()
            .any(|m| matches!(m, Message::StartConversation { .. })),
    );
    assert!(said.iter().any(|m| matches!(
        m,
        Message::AnswerSpawn { outcome: SpawnOutcome::Declined { reason }, .. }
            if reason.contains("nobody")
    )));
}

// ── the roster's actions (§6.2) ─────────────────────────────────────

/// Crowning a member is **one** message. The handoff — the outgoing coordinator put back to
/// worker, told so, and left running — is the host's, and a window that detached the old one
/// itself would be killing what M10 says to leave alone.
#[gpui::test]
fn make_coordinator_is_one_message_and_detaches_nobody(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let task = TaskId::generate();
    let outgoing = AgentId::generate();
    let incoming = AgentId::generate();
    let mut record = a_mission(fixture.project, task, Phase::InProgress);
    record.coordinator = Some(outgoing);
    fixture.mission(record, cx);

    fixture.with(cx, |state, _, cx| {
        state.make_mission_coordinator(task, incoming, cx)
    });
    let said = fixture.said();
    assert!(said.iter().any(|m| matches!(
        m,
        Message::SetMissionField { task_id, field: MissionField::Coordinator(Some(agent)), .. }
            if *task_id == task && *agent == incoming
    )));
    assert!(
        !said.iter().any(|m| matches!(
            m,
            Message::UnloadConversation { .. }
                | Message::EndConversation { .. }
                | Message::AssignAgent { .. }
        )),
        "the previous coordinator is detached by the host, and never killed: {said:#?}"
    );
}

/// Stop unloads and never ends: the transcript and the run directory stay, so the agent resumes.
/// Detach points it at no task, which is what membership is read from.
#[gpui::test]
fn stop_unloads_and_detach_clears_the_task(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let task = TaskId::generate();
    let agent = AgentId::generate();
    let mut record = a_mission(fixture.project, task, Phase::InProgress);
    record.roster = vec![RosterEntry::new(agent, MissionRole::Worker, Utc::now())];
    fixture.mission(record, cx);

    fixture.with(cx, |state, _, cx| state.stop_mission_agent(agent, cx));
    let said = fixture.said();
    assert!(
        said.iter()
            .any(|m| matches!(m, Message::UnloadConversation { agent_id } if *agent_id == agent))
    );
    assert!(
        !said
            .iter()
            .any(|m| matches!(m, Message::EndConversation { .. })),
        "a mission surface deletes nobody's transcript"
    );

    // *Pause all agents* is the same verb over the whole roster.
    let rows = MissionMenuRow::all();
    let pause = rows
        .iter()
        .position(|row| *row == MissionMenuRow::PauseAll)
        .expect("Pause all is a row");
    assert!(MissionMenuRow::PauseAll.enabled());
    fixture.with(cx, |state, _, cx| {
        state.open_mission_menu(task, (0., 0.), cx);
        state.pick_mission_menu(pause, cx);
    });
    assert!(
        fixture
            .said()
            .iter()
            .any(|m| matches!(m, Message::UnloadConversation { agent_id } if *agent_id == agent))
    );
}

/// Attaching a running agent points it at the anchor; the host settles the roster from that.
#[gpui::test]
fn attaching_points_the_agent_at_the_anchor(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let task = TaskId::generate();
    let agent = AgentId::generate();
    fixture.mission(a_mission(fixture.project, task, Phase::InProgress), cx);

    fixture.with(cx, |state, _, cx| {
        state.attach_mission_agent(task, agent, cx)
    });
    assert!(fixture.said().iter().any(|m| matches!(
        m,
        Message::AssignAgent { agent_id, task_id: Some(held), .. }
            if *agent_id == agent && *held == task
    )));
}

// ── feedback (M12) ──────────────────────────────────────────────────

/// The user talking to a mission **is** `SendToAgent` at its coordinator — the message the host
/// already journals as feedback and `read_feedback` reads back. No second path, and no second
/// message.
#[gpui::test]
fn feedback_goes_to_the_coordinator_as_send_to_agent(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let task = TaskId::generate();
    let coordinator = AgentId::generate();
    let mut record = a_mission(fixture.project, task, Phase::InProgress);
    record.coordinator = Some(coordinator);
    fixture.mission(record, cx);

    let sent = fixture.with(cx, |state, _, cx| {
        state.send_mission_feedback(task, "  the retries are the wrong ones  ".to_string(), cx)
    });
    assert!(sent);
    assert!(fixture.said().iter().any(|m| matches!(
        m,
        Message::SendToAgent { agent_id, text, .. }
            if *agent_id == coordinator && text == "the retries are the wrong ones"
    )));
}

/// A coordinator on the record with no harness loaded is prompted all the same — the host's
/// `PromptAgent` arm relaunches it. Journaled *and* delivered, not journaled and dropped (T-212).
#[gpui::test]
fn feedback_prompts_an_unloaded_coordinator(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let task = TaskId::generate();
    let coordinator = AgentId::generate();
    let mut record = a_mission(fixture.project, task, Phase::InProgress);
    record.coordinator = Some(coordinator);
    fixture.mission(record, cx);

    let sent = fixture.with(cx, |state, _, cx| {
        state.send_mission_feedback(task, "anyone there?".to_string(), cx)
    });
    assert!(sent);
    assert!(fixture.said().iter().any(|m| matches!(
        m,
        Message::PromptAgent { agent_id, text } if *agent_id == coordinator && text == "anyone there?"
    )));
}

/// A mission with no coordinator gets one: the send spawns it, crowns it, and the line goes both
/// into its briefing and into the journal (T-212).
#[gpui::test]
fn feedback_with_no_coordinator_spawns_one(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let task = TaskId::generate();
    fixture.deliver(
        Message::AgentDefinitions {
            definitions: vec![a_definition("worker")],
        },
        cx,
    );
    fixture.mission(a_mission(fixture.project, task, Phase::InProgress), cx);
    fixture.said();

    let sent = fixture.with(cx, |state, _, cx| {
        state.send_mission_feedback(task, "anyone there?".to_string(), cx)
    });
    assert!(sent);
    let said = fixture.said();
    let crowned = said.iter().find_map(|m| match m {
        Message::SetMissionField {
            task_id,
            field: MissionField::Coordinator(Some(agent)),
            ..
        } if *task_id == task => Some(*agent),
        _ => None,
    });
    let crowned = crowned.expect("the send crowned a coordinator");
    assert!(said.iter().any(|m| matches!(
        m,
        Message::StartConversation { agent_id, .. } if *agent_id == crowned
    )));
    assert!(said.iter().any(|m| matches!(
        m,
        Message::PromptAgent { agent_id, text } if *agent_id == crowned && text.contains("anyone there?")
    )));
    assert!(said.iter().any(|m| matches!(
        m,
        Message::SendToAgent { agent_id, text, .. }
            if *agent_id == crowned && text == "anyone there?"
    )));
}

/// Nothing to run a coordinator *as* is still a refusal with a sentence — the one case where the
/// line does not go anywhere, and the composer says why rather than swallowing it.
#[gpui::test]
fn feedback_with_nothing_to_spawn_says_so(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let task = TaskId::generate();
    fixture.mission(a_mission(fixture.project, task, Phase::InProgress), cx);
    fixture.said();

    let sent = fixture.with(cx, |state, _, cx| {
        state.send_mission_feedback(task, "anyone there?".to_string(), cx)
    });
    assert!(!sent);
    assert!(fixture.said().is_empty());
}

// ── the journal (M12) ───────────────────────────────────────────────

/// A page lands newest first and a later line goes on the front of it — including the two kinds
/// M13 adds, which are rows like any other.
#[gpui::test]
fn the_journal_pages_newest_first_and_takes_appends(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let task = TaskId::generate();
    fixture.mission(a_mission(fixture.project, task, Phase::InProgress), cx);

    let line = |seq: u64, event: JournalEvent, text: &str| JournalEntry {
        seq,
        at: Utc::now(),
        by: Actor::Host,
        event,
        text: text.to_string(),
    };
    fixture.deliver(
        Message::Journal {
            project_id: fixture.project,
            task_id: task,
            entries: vec![
                line(
                    2,
                    JournalEvent::SpawnAnswered {
                        agent_kind: "reviewer".to_string(),
                        agent: None,
                    },
                    "declined a reviewer",
                ),
                line(
                    1,
                    JournalEvent::SpawnRequested {
                        agent_kind: "reviewer".to_string(),
                    },
                    "asked for a reviewer",
                ),
            ],
            more: true,
        },
        cx,
    );
    fixture.deliver(
        Message::JournalAppended {
            project_id: fixture.project,
            task_id: task,
            entry: line(3, JournalEvent::Feedback, "you said something"),
        },
        cx,
    );

    fixture.with(cx, |state, _, _| {
        let journal = state.mission_journal(task).expect("a journal");
        let seqs: Vec<u64> = journal.entries.iter().map(|entry| entry.seq).collect();
        assert_eq!(seqs, vec![3, 2, 1], "newest first");
        assert!(journal.more);
        assert_eq!(journal.oldest(), Some(1), "the cursor is the oldest held");
        assert_eq!(journal.entries[1].event.kind(), "spawn_answered");
        assert_eq!(journal.entries[2].event.kind(), "spawn_requested");
    });
}

/// *Spawn ▾*'s rows, and the fact a click resolves against the same list the menu draws.
#[gpui::test]
fn spawn_offers_the_coordinator_every_kind_any_agent_and_attach(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let task = TaskId::generate();
    let record = a_mission_with_kinds(&fixture, task, SpawnPolicy::Ask, cx);
    fixture.mission(record, cx);

    let rows = fixture.with(cx, |state, _, _| state.mission_spawn_rows(task));
    assert_eq!(
        rows,
        vec![
            MissionSpawnRow::Coordinator,
            MissionSpawnRow::Kind("reviewer".to_string()),
            MissionSpawnRow::Any,
            MissionSpawnRow::Attach,
        ]
    );

    fixture.said();
    let kind = rows
        .iter()
        .position(|row| *row == MissionSpawnRow::Kind("reviewer".to_string()))
        .expect("the kind is a row");
    fixture.with(cx, |state, window, cx| {
        state.open_mission_spawn_menu(task, (0., 0.), cx);
        state.pick_mission_spawn_menu(kind, window, cx);
    });
    let said = fixture.said();
    assert!(
        said.iter().any(|m| matches!(
            m,
            Message::StartConversation { definition, spawned_by: None, .. }
                if definition.as_deref() == Some("worker")
        )),
        "the user's own spawn asks nobody and answers nobody: {said:#?}"
    );
    assert!(
        !said
            .iter()
            .any(|m| matches!(m, Message::AnswerSpawn { .. })),
        "there was no request to answer"
    );
}

// ── the WBS and the settings (S6) ───────────────────────────────────

/// A task of the mission's breakdown, waiting on whatever it is given.
fn a_task(title: &str, prerequisites: Vec<TaskId>) -> TaskRecord {
    let mut task = TaskRecord::new(title.to_string(), None, Utc::now());
    task.prerequisites = prerequisites;
    task
}

/// The layering is a longest path, not a depth: a task waiting on two chains sits past the deeper
/// of them, which is what makes a level mean "not before this round".
#[gpui::test]
fn the_wbs_lays_tasks_out_by_dependency_level() {
    let first = a_task("schema", Vec::new());
    let second = a_task("api", vec![first.id]);
    let third = a_task("ui", vec![second.id]);
    // Waits on the first only, so it is on level 1 beside the api — and the join below is what
    // makes the longest path visible.
    let aside = a_task("docs", vec![first.id]);
    let join = a_task("release", vec![third.id, aside.id]);

    let tasks = vec![&first, &second, &third, &aside, &join];
    let arranged = ubiq::state::wbs::layers(&tasks);

    assert_eq!(arranged.level_of(first.id), 0, "nothing comes before it");
    assert_eq!(arranged.level_of(second.id), 1);
    assert_eq!(arranged.level_of(aside.id), 1, "a second chain, same level");
    assert_eq!(arranged.level_of(third.id), 2);
    assert_eq!(
        arranged.level_of(join.id),
        3,
        "past the deeper of the two it waits on, not the shallower"
    );
    assert_eq!(arranged.rows.len(), 4);
    assert_eq!(arranged.rows[1].len(), 2, "level 1 holds both chains");
    assert_eq!(
        arranged.critical,
        vec![first.id, second.id, third.id, join.id],
        "the longest chain, level 0 first"
    );
}

/// **A mission with no prerequisites at all does not collapse**: it is one level holding every
/// task, which is the arrangement saying they can all be done at once.
#[gpui::test]
fn a_mission_with_no_prerequisites_is_one_level() {
    let one = a_task("one", Vec::new());
    let two = a_task("two", Vec::new());
    let three = a_task("three", Vec::new());
    let arranged = ubiq::state::wbs::layers(&[&one, &two, &three]);

    assert_eq!(arranged.rows.len(), 1, "one level");
    assert_eq!(arranged.rows[0].len(), 3, "everything on it");
    assert_eq!(arranged.widest(), 3);
    assert!(
        arranged.critical.is_empty(),
        "no chain is not a chain of one"
    );
}

/// **A disconnected graph reads as two chains against the same levels**, not as one run-on
/// arrangement — and a prerequisite outside the set is a root here rather than a dangling edge.
#[gpui::test]
fn a_disconnected_wbs_keeps_its_islands_on_the_same_levels() {
    let outside = TaskId::generate();
    let a0 = a_task("a0", Vec::new());
    let a1 = a_task("a1", vec![a0.id]);
    let b0 = a_task("b0", vec![outside]);
    let b1 = a_task("b1", vec![b0.id]);

    let arranged = ubiq::state::wbs::layers(&[&a0, &a1, &b0, &b1]);

    assert_eq!(arranged.rows.len(), 2);
    assert_eq!(
        arranged.level_of(b0.id),
        0,
        "what it waits on is not in the mission, so it is a root here"
    );
    assert_eq!(arranged.level_of(a1.id), 1);
    assert_eq!(arranged.level_of(b1.id), 1);
    assert_eq!(arranged.rows[0].len(), 2, "two islands, two roots");
    assert_eq!(arranged.rows[1].len(), 2);
}

/// Every Settings control is one `SetMissionField`, and the window never writes the record it is
/// drawing — the host does, and `MissionChanged` is how the tab learns it landed.
#[gpui::test]
fn a_settings_control_is_one_set_mission_field(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let task = TaskId::generate();
    let record = a_mission(fixture.project, task, Phase::InProgress);
    let was = record.parallelism;
    fixture.mission(record, cx);

    fixture.with(cx, |state, _, cx| {
        state.set_mission_field(task, MissionField::Execution(ExecutionMode::Auto), cx);
        state.set_mission_field(task, MissionField::Parallelism(was + 1), cx);
    });

    let said = fixture.said();
    assert!(
        said.iter().any(|m| matches!(
            m,
            Message::SetMissionField {
                project_id,
                task_id,
                field: MissionField::Execution(ExecutionMode::Auto),
            } if *project_id == fixture.project && *task_id == task
        )),
        "the mode switch is a field write: {said:#?}"
    );
    assert!(said.iter().any(|m| matches!(
        m,
        Message::SetMissionField { field: MissionField::Parallelism(n), .. } if *n == was + 1
    )));
    fixture.with(cx, |state, _, cx| {
        assert_eq!(
            state.mission(task, cx).map(|record| record.parallelism),
            Some(was),
            "nothing on this side wrote the record"
        );
    });
}

/// Picking a node on the WBS is the *board's* selection, because the detail beside the graph is
/// the board's own — one selection, so what is open and which field is being edited cannot drift.
#[gpui::test]
fn picking_a_wbs_node_selects_it_for_the_board_detail(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let task = TaskId::generate();
    fixture.mission(a_mission(fixture.project, task, Phase::InProgress), cx);
    let child = TaskId::generate();

    fixture.with(cx, |state, _, cx| state.select_wbs_task(child, cx));
    fixture.with(cx, |state, _, cx| {
        assert_eq!(
            state.mission_view(cx).and_then(|view| view.selected),
            Some(child)
        );
        assert_eq!(
            state.board(cx).and_then(|board| board.selected),
            Some(child)
        );
    });

    fixture.with(cx, |state, _, cx| state.close_task_detail(cx));
    fixture.with(cx, |state, _, cx| {
        assert_eq!(
            state.mission_view(cx).and_then(|view| view.selected),
            None,
            "the detail's close clears the WBS selection with it"
        );
    });
}
