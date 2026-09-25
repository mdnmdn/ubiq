//! The new-mission dialog's app-level wiring: `open_new_mission` asks for the definition list, the
//! coordinator picker only ever offers a definition carrying `mission_assistant` (plus the agents
//! already running, M10), and `start_new_mission` composes a `CreateTask` whose answer —
//! `TaskCreated` — is what promotes the task to a mission, writes the **whole brief** onto it
//! (description, attachments, linked tasks — M7, with no message of their own), brings the record
//! into being with `CreateMission`, seeds the plan, and puts a coordinator on it with the briefing
//! as its opening turn. `tests/plan.rs`'s `Fixture` is the model this one follows.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use chrono::Utc;
use gpui::{AppContext as _, Entity, TestAppContext, WindowHandle};
use gpui_component::Root;
use ubiq::app::{AppState, BusHub};
use ubiq::state::WindowRegistry;
use ubiq::state::new_mission::Coordinator;
use ubiq_proto::bus::{self, FromClient, To};
use ubiq_proto::ids::{ProjectId, TaskId};
use ubiq_proto::messages::{AgentDefinition, Message, TaskField};
use ubiq_proto::mission::{MissionField, Phase};
use ubiq_proto::plan::DocumentHandle;
use ubiq_proto::projects::{ProjectHealth, ProjectRecord, ProjectSnapshot};
use ubiq_proto::work::{AgentId, Level, Priority, Status, TaskRecord};

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

    /// Deliver the definition list a `ListAgentDefinitions` would be answered with.
    fn definitions(&self, definitions: Vec<AgentDefinition>, cx: &mut TestAppContext) {
        self.deliver(Message::AgentDefinitions { definitions }, cx);
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

fn a_definition(id: &str, mission_assistant: Option<bool>) -> AgentDefinition {
    AgentDefinition {
        id: id.to_string(),
        description: None,
        agent_type: "claude-code".to_string(),
        account: Some("work".to_string()),
        model: None,
        mode: None,
        thinking: None,
        max_subagents: None,
        prompt: None,
        mcps: Vec::new(),
        mission_assistant,
        mission_coordinator: false,
        mission_worker: false,
        disabled: false,
        project: None,
    }
}

fn a_task(id: TaskId) -> TaskRecord {
    TaskRecord {
        id,
        session: None,
        status: Status::Backlog,
        priority: Priority::Normal,
        shape: None,
        kind: None,
        level: None,
        parent: None,
        references: Vec::new(),
        prerequisites: Vec::new(),
        attachments: Vec::new(),
        complexity: None,
        key: None,
        link: None,
        assigned_to: None,
        labels: Vec::new(),
        colour: None,
        title: "Ship v2".to_string(),
        description: String::new(),
        steps: Vec::new(),
        comments: Vec::new(),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

/// Opening the dialog asks the host for the definition list, so a definition ticked `mission assistant`
/// since the window opened is offered without a restart.
#[gpui::test]
fn opening_the_dialog_asks_for_definitions(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    fixture.said(); // drain boot noise
    fixture.with(cx, |state, window, cx| state.open_new_mission(window, cx));

    let said = fixture.said();
    assert!(
        said.iter()
            .any(|m| matches!(m, Message::ListAgentDefinitions))
    );
    fixture.state.read_with(cx, |state, _| {
        assert!(state.workbench.new_mission.is_some());
    });
}

/// The picker's own list — `state::new_mission::assistants` — offers only a definition ticked
/// `mission_assistant`; `None` and `Some(false)` are both left out.
#[gpui::test]
fn only_mission_assistants_are_offered(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    fixture.with(cx, |state, window, cx| state.open_new_mission(window, cx));
    fixture.said();
    fixture.definitions(
        vec![
            a_definition("reviewer", Some(true)),
            a_definition("writer", None),
            a_definition("planner", Some(false)),
        ],
        cx,
    );

    fixture.state.read_with(cx, |state, _| {
        let offered = ubiq::state::new_mission::assistants(&state.workbench.settings.definitions);
        assert_eq!(offered.len(), 1);
        assert_eq!(offered[0].id, "reviewer");
    });
}

/// A definition scoped to the dialog's own project is offered alongside the global ones — G331: the
/// picker used to read the global list alone, so a `mission_assistant` definition filed under a
/// project could never be picked from inside that very project.
#[gpui::test]
fn a_project_scoped_assistant_is_offered_in_its_own_project(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    fixture.with(cx, |state, window, cx| state.open_new_mission(window, cx));
    fixture.said();
    let mut scoped = a_definition("scoped-reviewer", Some(true));
    scoped.project = Some(fixture.project);
    fixture.definitions(
        vec![a_definition("global-reviewer", Some(true)), scoped],
        cx,
    );

    fixture.state.read_with(cx, |state, cx| {
        let project = state.project(cx);
        let offered = state.workbench.settings.definitions_in(project);
        let ids: Vec<&str> = ubiq::state::new_mission::assistants(&offered)
            .into_iter()
            .map(|p| p.id.as_str())
            .collect();
        assert!(ids.contains(&"scoped-reviewer"));
        assert!(ids.contains(&"global-reviewer"));
    });
}

/// Start is refused with no title or no assistant chosen: nothing is sent, and the dialog stays
/// up rather than composing a launch out of half an answer.
#[gpui::test]
fn start_is_refused_without_a_title_or_an_assistant(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    fixture.with(cx, |state, window, cx| state.open_new_mission(window, cx));
    fixture.definitions(vec![a_definition("reviewer", Some(true))], cx);
    fixture.said();

    // An assistant with no title.
    fixture.with(cx, |state, _, cx| {
        state.pick_new_mission_coordinator(Coordinator::AgentDefinition("reviewer".to_string()), cx)
    });
    fixture.with(cx, |state, _, cx| state.start_new_mission(cx));
    assert!(fixture.said().is_empty());
    fixture.state.read_with(cx, |state, _| {
        assert!(state.workbench.new_mission.is_some())
    });

    // A title with no assistant.
    fixture.with(cx, |state, _, _cx| {
        let form = state.workbench.new_mission.as_mut().expect("still open");
        form.title = "Ship v2".to_string();
        form.coordinator = None;
    });
    fixture.with(cx, |state, _, cx| state.start_new_mission(cx));
    assert!(fixture.said().is_empty());
}

/// The whole flow: `CreateTask` first, and once its `TaskCreated` answer names an id, the
/// promotion to `Level::Mission`, the description, the assistant's `StartConversation` — both
/// planning MCP servers ticked — and the briefing as its opening `PromptAgent`, in that order.
#[gpui::test]
fn starting_a_mission_creates_promotes_and_launches_the_assistant(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    fixture.with(cx, |state, window, cx| state.open_new_mission(window, cx));
    fixture.definitions(vec![a_definition("reviewer", Some(true))], cx);
    fixture.said();

    let linked = TaskId::generate();
    fixture.with(cx, |state, _, _cx| {
        let form = state.workbench.new_mission.as_mut().expect("open");
        form.title = "Ship v2".to_string();
        form.description = "Get it out the door.".to_string();
        form.attachments = vec![
            "docs/spec.md".to_string(),
            "kb:handbook/release".to_string(),
        ];
        form.references = vec![linked];
        form.require_plan = true;
    });
    fixture.with(cx, |state, _, cx| {
        state.pick_new_mission_coordinator(Coordinator::AgentDefinition("reviewer".to_string()), cx)
    });
    fixture.with(cx, |state, _, cx| state.start_new_mission(cx));

    let said = fixture.said();
    assert_eq!(said.len(), 1, "only the CreateTask goes out up front");
    assert!(matches!(
        &said[0],
        Message::CreateTask { project_id, title, .. }
            if *project_id == fixture.project && title == "Ship v2"
    ));
    // The dialog closes the moment its own part is done; the rest waits for the id.
    fixture.state.read_with(cx, |state, _| {
        assert!(state.workbench.new_mission.is_none())
    });

    let task_id = TaskId::generate();
    fixture.deliver(
        Message::TaskCreated {
            project_id: fixture.project,
            task: a_task(task_id),
        },
        cx,
    );

    let said = fixture.said();
    assert!(said.iter().any(|m| matches!(
        m,
        Message::SetTaskField { project_id, task_id: id, field: TaskField::Level(Some(Level::Mission)) }
            if *project_id == fixture.project && *id == task_id
    )));
    assert!(said.iter().any(|m| matches!(
        m,
        Message::UpdateTask { project_id, task_id: id, description: Some(d), .. }
            if *project_id == fixture.project && *id == task_id && d == "Get it out the door."
    )));
    // The brief's other two halves, on the anchor's own fields and with no message of their own
    // (M7) — a project path and a `kb:` address go up as one list.
    assert!(said.iter().any(|m| matches!(
        m,
        Message::SetTaskField { task_id: id, field: TaskField::Attachments(list), .. }
            if *id == task_id
                && list.iter().map(|a| a.target.as_str()).collect::<Vec<_>>()
                    == vec!["docs/spec.md", "kb:handbook/release"]
    )));
    assert!(said.iter().any(|m| matches!(
        m,
        Message::SetTaskField { task_id: id, field: TaskField::References(list), .. }
            if *id == task_id && list.as_slice() == [linked]
    )));
    // The record exists in the same act as the card (M17), and the gate is written onto it.
    assert!(said.iter().any(|m| matches!(
        m,
        Message::CreateMission { project_id, task_id: id }
            if *project_id == fixture.project && *id == task_id
    )));
    assert!(said.iter().any(|m| matches!(
        m,
        Message::SetMissionField { task_id: id, field: MissionField::RequirePlan(true), .. }
            if *id == task_id
    )));
    // No plan was supplied, so the mission stays in its opening phase: nothing moves it.
    assert!(
        !said.iter().any(|m| matches!(m, Message::SetPhase { .. })),
        "a mission with no plan seed opens where the host put it"
    );
    let start = said
        .iter()
        .find_map(|m| match m {
            Message::StartConversation {
                agent_id,
                project_id,
                definition,
                agent_type,
                mcps,
                ..
            } if *project_id == fixture.project => Some((
                *agent_id,
                definition.clone(),
                agent_type.clone(),
                mcps.clone(),
            )),
            _ => None,
        })
        .expect("the assistant is started");
    assert_eq!(start.1, Some("reviewer".to_string()));
    assert_eq!(start.2, "claude-code");
    // The coordinator's set, named once in `state::mission::COORDINATOR_MCPS` — the panel's
    // *Spawn ▾* composes a coordinator from the same list, and this asserts the dialog reads it
    // rather than keeping a second copy.
    assert_eq!(
        start.3,
        ubiq::state::mission::COORDINATOR_MCPS
            .iter()
            .map(|it| it.to_string())
            .collect::<Vec<_>>()
    );
    // Whoever runs it is on the record, not only in the window.
    assert!(said.iter().any(|m| matches!(
        m,
        Message::SetMissionField { task_id: id, field: MissionField::Coordinator(Some(agent)), .. }
            if *id == task_id && *agent == start.0
    )));

    let briefing = said
        .iter()
        .find_map(|m| match m {
            Message::PromptAgent { agent_id, text } if *agent_id == start.0 => Some(text.clone()),
            _ => None,
        })
        .expect("the briefing is the opening turn");
    assert!(briefing.contains("Ship v2"));
    assert!(briefing.contains("Get it out the door."));
    assert!(briefing.contains(&task_id.to_string()));
    assert!(
        briefing.contains("ubiq-mission"),
        "the briefing names the surface the phase is moved through: {briefing}"
    );
    assert!(
        briefing.contains("A plan is required"),
        "require_plan now reads as the gate it is: {briefing}"
    );
}

/// A coordinator that is already running is **adopted, not launched** (M10): the record names it,
/// the briefing is its next turn, and no second harness is started for it.
#[gpui::test]
fn a_running_agent_is_adopted_as_the_coordinator(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    fixture.with(cx, |state, window, cx| state.open_new_mission(window, cx));
    fixture.definitions(vec![a_definition("reviewer", Some(true))], cx);
    fixture.said();

    let running = AgentId::generate();
    fixture.with(cx, |state, _, _cx| {
        let form = state.workbench.new_mission.as_mut().expect("open");
        form.title = "Ship v2".to_string();
        form.coordinator = Some(Coordinator::Running(running));
    });
    fixture.with(cx, |state, _, cx| state.start_new_mission(cx));
    fixture.said();

    let task_id = TaskId::generate();
    fixture.deliver(
        Message::TaskCreated {
            project_id: fixture.project,
            task: a_task(task_id),
        },
        cx,
    );
    let said = fixture.said();
    assert!(
        !said
            .iter()
            .any(|m| matches!(m, Message::StartConversation { .. })),
        "an agent already running is not started again"
    );
    assert!(said.iter().any(|m| matches!(
        m,
        Message::SetMissionField { field: MissionField::Coordinator(Some(agent)), .. }
            if *agent == running
    )));
    assert!(said.iter().any(|m| matches!(
        m,
        Message::PromptAgent { agent_id, .. } if *agent_id == running
    )));
}

/// A mission started **from a plan** saves that markdown as the plan's first revision and opens in
/// refining (M9) — one `SavePlan` at the watermark of a plan nobody has written, and one
/// `SetPhase`.
#[gpui::test]
fn a_plan_seed_is_saved_and_starts_the_mission_in_refining(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    fixture.with(cx, |state, window, cx| state.open_new_mission(window, cx));
    fixture.definitions(vec![a_definition("reviewer", Some(true))], cx);
    fixture.said();

    fixture.with(cx, |state, _, _cx| {
        let form = state.workbench.new_mission.as_mut().expect("open");
        form.title = "Ship v2".to_string();
        form.plan_seed = "# Plan\n\nOne step.".to_string();
    });
    fixture.with(cx, |state, _, cx| {
        state.pick_new_mission_coordinator(Coordinator::AgentDefinition("reviewer".to_string()), cx)
    });
    fixture.with(cx, |state, _, cx| state.start_new_mission(cx));
    fixture.said();

    let task_id = TaskId::generate();
    fixture.deliver(
        Message::TaskCreated {
            project_id: fixture.project,
            task: a_task(task_id),
        },
        cx,
    );
    let said = fixture.said();
    assert!(said.iter().any(|m| matches!(
        m,
        Message::SavePlan { doc: DocumentHandle::Plan { task_id: id, .. }, body, expected: 0 }
            if *id == task_id && body == "# Plan\n\nOne step."
    )));
    assert!(said.iter().any(|m| matches!(
        m,
        Message::SetPhase { task_id: id, phase: Phase::Refining, .. } if *id == task_id
    )));
    let briefing = said
        .iter()
        .find_map(|m| match m {
            Message::PromptAgent { text, .. } => Some(text.clone()),
            _ => None,
        })
        .expect("the briefing is the opening turn");
    assert!(
        briefing.contains("refining"),
        "the coordinator is told to refine the plan rather than start one: {briefing}"
    );
}

/// A mission written with only requirements is named from their first line — the board's own
/// rule for a draft with no title, one level up. Start is not refused for it.
#[gpui::test]
fn a_blank_title_is_taken_from_the_requirements(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    fixture.with(cx, |state, window, cx| state.open_new_mission(window, cx));
    fixture.definitions(vec![a_definition("reviewer", Some(true))], cx);
    fixture.said();

    fixture.with(cx, |state, _, _cx| {
        let form = state.workbench.new_mission.as_mut().expect("open");
        form.description = "# Payment retries\n\nThey fail twice.".to_string();
    });
    fixture.with(cx, |state, _, cx| {
        state.pick_new_mission_coordinator(Coordinator::AgentDefinition("reviewer".to_string()), cx)
    });
    fixture.with(cx, |state, _, cx| state.start_new_mission(cx));

    let said = fixture.said();
    assert!(said.iter().any(|m| matches!(
        m,
        Message::CreateTask { title, .. } if title == "Payment retries"
    )));
}

/// The attachment list is a set: a target already on the draft is not added twice, whichever way
/// it arrived — the picker, or the clipboard.
#[gpui::test]
fn an_attachment_is_never_added_twice(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    fixture.with(cx, |state, window, cx| state.open_new_mission(window, cx));
    fixture.said();

    fixture.with(cx, |state, _, cx| {
        state.add_new_mission_attachments(vec!["docs/spec.md".to_string()], cx);
        state.add_new_mission_attachments(
            vec!["docs/spec.md".to_string(), "kb:handbook".to_string()],
            cx,
        );
    });
    fixture.state.read_with(cx, |state, _| {
        let form = state.workbench.new_mission.as_ref().expect("open");
        assert_eq!(form.attachments, vec!["docs/spec.md", "kb:handbook"]);
    });

    fixture.with(cx, |state, _, cx| {
        state.remove_new_mission_attachment("docs/spec.md".to_string(), cx)
    });
    fixture.state.read_with(cx, |state, _| {
        let form = state.workbench.new_mission.as_ref().expect("open");
        assert_eq!(form.attachments, vec!["kb:handbook"]);
    });
}

/// The assistant picker's trigger shows the chosen definition once one is picked, not the
/// placeholder it started with — the bug T-72 reported: the trigger drew "Choose an
/// assistant…" unconditionally, ignoring `NewMissionForm::assistant`.
/// `state::new_mission::assistant_label` is the one function both the trigger and this test call,
/// so they cannot disagree the way the trigger and the form once did.
#[gpui::test]
fn the_trigger_shows_the_picked_assistant(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    fixture.with(cx, |state, window, cx| state.open_new_mission(window, cx));
    fixture.definitions(
        vec![
            a_definition("reviewer", Some(true)),
            a_definition("coach", Some(true)),
        ],
        cx,
    );
    fixture.said();

    fixture.state.read_with(cx, |state, cx| {
        let form = state.workbench.new_mission.as_ref().expect("open");
        let rows = state.new_mission_coordinators(cx);
        assert_eq!(
            ubiq::state::new_mission::coordinator_label(form, &rows, "Choose a coordinator…"),
            "Choose a coordinator…",
            "nothing picked yet"
        );
    });

    fixture.with(cx, |state, _, cx| {
        state.pick_new_mission_coordinator(Coordinator::AgentDefinition("coach".to_string()), cx)
    });

    fixture.state.read_with(cx, |state, cx| {
        let form = state.workbench.new_mission.as_ref().expect("still open");
        let rows = state.new_mission_coordinators(cx);
        assert_eq!(
            ubiq::state::new_mission::coordinator_label(form, &rows, "Choose a coordinator…"),
            "coach",
            "the trigger reflects the pick, not the placeholder"
        );
    });
}

/// A definition the dialog offered is gone by the time the mission exists: the mission still stands,
/// and there is simply nobody left to launch.
#[gpui::test]
fn a_vanished_assistant_still_leaves_the_mission_standing(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    fixture.with(cx, |state, window, cx| state.open_new_mission(window, cx));
    fixture.definitions(vec![a_definition("reviewer", Some(true))], cx);
    fixture.said();

    fixture.with(cx, |state, _, _cx| {
        let form = state.workbench.new_mission.as_mut().expect("open");
        form.title = "Ship v2".to_string();
    });
    fixture.with(cx, |state, _, cx| {
        state.pick_new_mission_coordinator(Coordinator::AgentDefinition("reviewer".to_string()), cx)
    });
    // The definition is forgotten before the id comes back — a race the host can always win.
    fixture.with(cx, |state, _, cx| {
        state.workbench.settings.definitions.clear();
        cx.notify();
    });
    fixture.with(cx, |state, _, cx| state.start_new_mission(cx));
    fixture.said();

    let task_id = TaskId::generate();
    fixture.deliver(
        Message::TaskCreated {
            project_id: fixture.project,
            task: a_task(task_id),
        },
        cx,
    );
    let said = fixture.said();
    assert!(said.iter().any(|m| matches!(
        m,
        Message::SetTaskField {
            field: TaskField::Level(Some(Level::Mission)),
            ..
        }
    )));
    assert!(
        !said
            .iter()
            .any(|m| matches!(m, Message::StartConversation { .. })),
        "nobody to launch"
    );
}
