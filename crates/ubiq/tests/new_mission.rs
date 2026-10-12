//! The new-mission dialog's wiring: opening it asks for the definition list, Start is refused
//! without a name, and Start says one `CreateMission` carrying the whole of what was decided — the
//! brief, the initial plan and the driver — then closes the dialog. The host does the rest
//! (`crates/ubiq-host/tests/mission.rs`).

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use chrono::Utc;
use gpui::{AppContext as _, Entity, TestAppContext, WindowHandle};
use gpui_component::Root;
use ubiq::app::{AppState, BusHub};
use ubiq::state::WindowRegistry;
use ubiq_proto::bus::{self, FromClient};
use ubiq_proto::ids::ProjectId;
use ubiq_proto::messages::Message;
use ubiq_proto::projects::{ProjectHealth, ProjectRecord, ProjectSnapshot};

const PATIENCE: Duration = Duration::from_millis(300);

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

    /// Everything the window has said to the host so far.
    fn said(&self) -> Vec<Message> {
        let mut said = Vec::new();
        while let Ok(event) = self.host.recv_timeout(PATIENCE) {
            if let FromClient::Said { message, .. } = event {
                said.push(message);
            }
        }
        said
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
            definitions_use_global: true,
            definitions_allowed: Vec::new(),
        },
        health: ProjectHealth::Ok,
        open_panes: 0,
        ephemeral: false,
        workarea: "/tmp/ubiq-workarea".to_string(),
    }
}

use ubiq::state::new_mission::PlanSource;
use ubiq_proto::ids::TaskId;
use ubiq_proto::mission::{Driver, InitialPlan, MissionSettings, NewMission};

#[gpui::test]
fn opening_the_dialog_asks_for_the_definitions(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    fixture.said();
    fixture.with(cx, |state, window, cx| state.open_new_mission(window, cx));

    assert!(
        fixture
            .said()
            .iter()
            .any(|m| matches!(m, Message::ListAgentDefinitions))
    );
    fixture.state.read_with(cx, |state, _| {
        assert!(state.workbench.new_mission.is_some());
    });
}

#[gpui::test]
fn start_is_refused_without_a_name(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    fixture.with(cx, |state, window, cx| state.open_new_mission(window, cx));
    fixture.said();

    fixture.with(cx, |state, _, cx| state.start_new_mission(cx));
    assert!(fixture.said().is_empty(), "nothing is sent");
    fixture.state.read_with(cx, |state, _| {
        assert!(state.workbench.new_mission.is_some(), "the dialog stays up");
    });
}

#[gpui::test]
fn start_says_one_create_mission_with_the_initial_plan_and_the_driver(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    fixture.with(cx, |state, window, cx| state.open_new_mission(window, cx));
    fixture.said();

    let adopted = TaskId::generate();
    fixture.with(cx, |state, _, cx| {
        let form = state.workbench.new_mission.as_mut().expect("open");
        form.title = "Ship v2".to_string();
        form.description = "Get it out the door.".to_string();
        form.plan_text = "# Plan\n\nDo it.".to_string();
        state.add_new_mission_attachments(vec!["docs/spec.md".to_string()], cx);
        state.toggle_new_mission_include(adopted, cx);
        state.set_new_mission_plan_source(PlanSource::Paste, cx);
        state.pick_new_mission_planner(Some("architect".to_string()), cx);
        state.set_new_mission_driver(Driver::Manual, cx);
        state.nudge_new_mission_parallel(true, cx);
        state.toggle_new_mission_approve(cx);
    });
    fixture.with(cx, |state, _, cx| state.start_new_mission(cx));

    let said = fixture.said();
    assert_eq!(said.len(), 1, "one message, and the host does the rest");
    let Message::CreateMission {
        project_id,
        mission,
    } = &said[0]
    else {
        panic!("not a CreateMission: {:?}", said[0]);
    };
    assert_eq!(*project_id, fixture.project);
    assert_eq!(
        **mission,
        NewMission {
            title: "Ship v2".to_string(),
            description: "Get it out the door.".to_string(),
            attachments: vec!["docs/spec.md".to_string()],
            include: vec![adopted],
            initial_plan: InitialPlan::Paste {
                body: "# Plan\n\nDo it.".to_string()
            },
            settings: MissionSettings {
                driver: Driver::Manual,
                planner: Some("architect".to_string()),
                worker: None,
                parallel: 2,
                approve: true,
                planner_agent: None,
            },
        }
    );
    fixture.state.read_with(cx, |state, _| {
        assert!(state.workbench.new_mission.is_none(), "the dialog closes");
    });
}

#[gpui::test]
fn a_plan_file_picked_from_the_project_is_sent_as_a_file(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    fixture.with(cx, |state, window, cx| state.open_new_mission(window, cx));
    fixture.said();

    fixture.with(cx, |state, _, cx| {
        state.workbench.new_mission.as_mut().expect("open").title = "From a file".to_string();
        state.set_new_mission_plan_file("wip/plan.md".to_string(), cx);
        state.start_new_mission(cx);
    });
    let said = fixture.said();
    let Message::CreateMission { mission, .. } = &said[0] else {
        panic!("not a CreateMission");
    };
    assert_eq!(
        mission.initial_plan,
        InitialPlan::File {
            target: "wip/plan.md".to_string()
        }
    );
}
