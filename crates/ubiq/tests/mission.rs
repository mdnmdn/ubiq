//! The mission side panel, drawn from a snapshot: a `MissionList` the host sent and a panel the
//! window revealed. The rows are `ui::mission::panel`'s own and not reachable from here, so what
//! this asserts is that the whole panel draws over every kind of *Needs you* item without
//! tripping, and that the window holds the snapshot the panel reads.

use std::cell::RefCell;
use std::rc::Rc;

use chrono::Utc;
use gpui::{AppContext as _, Entity, TestAppContext, WindowHandle};
use gpui_component::Root;
use ubiq::app::{AppState, BusHub};
use ubiq::state::WindowRegistry;
use ubiq_proto::bus::{self, To};
use ubiq_proto::ids::ProjectId;
use ubiq_proto::messages::Message;
use ubiq_proto::projects::{ProjectHealth, ProjectRecord, ProjectSnapshot};

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

use gpui::VisualTestContext;
use ubiq_proto::ids::TaskId;
use ubiq_proto::messages::Message as M;
use ubiq_proto::mission::{MissionSettings, MissionSnapshot, NeedsYou, Phase};
use ubiq_proto::work::{Level, Status, TaskRecord};

fn snapshot(project: ProjectId, task: TaskId, worker_task: TaskId) -> MissionSnapshot {
    MissionSnapshot {
        project_id: project,
        task_id: task,
        phase: Phase::Running,
        settings: MissionSettings::default(),
        documents: vec![],
        open_threads: 0,
        needs_you: vec![
            NeedsYou::AwaitingApproval { task: worker_task },
            NeedsYou::WorkerGone { task: worker_task },
            NeedsYou::HumanTask { task: worker_task },
            NeedsYou::ReviewFailedTwice { task: worker_task },
            NeedsYou::ReadyForReview {
                left_in_review: vec![worker_task],
            },
        ],
        running: vec![],
        next: vec![],
        agents: vec![],
        replan_at: None,
    }
}

#[gpui::test]
fn the_panel_draws_every_needs_you_item_of_a_snapshot(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let mut parent = TaskRecord::new("Ship v2".into(), None, Utc::now());
    parent.level = Some(Level::Mission);
    parent.status = Status::InProgress;
    let mut work = TaskRecord::new("the work".into(), None, Utc::now());
    work.parent = Some(parent.id);
    work.status = Status::InReview;
    let (mission, worker_task) = (parent.id, work.id);

    fixture.deliver(
        M::WorkList {
            project_id: fixture.project,
            sessions: vec![],
            agents: vec![],
            tasks: vec![parent, work],
        },
        cx,
    );
    fixture.deliver(
        M::MissionList {
            project_id: fixture.project,
            missions: vec![snapshot(fixture.project, mission, worker_task)],
        },
        cx,
    );
    fixture.with(cx, |state, _, cx| state.open_mission_panel(mission, cx));

    fixture.state.read_with(cx, |state, cx| {
        let held = state
            .mission(mission, cx)
            .expect("the window holds the snapshot");
        assert_eq!(held.needs_you.len(), 5);
        assert_eq!(held.phase, Phase::Running);
    });

    // Drawn: the panel is revealed on the next frame and builds a row for every item.
    let mut vcx = VisualTestContext::from_window(fixture.window.into(), cx);
    vcx.update(|window, cx| {
        window.refresh();
        let _ = cx;
    });
    vcx.run_until_parked();
}
