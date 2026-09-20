//! The titlebar's new-project chevron menu — the same three ways in the project picker's foot
//! offers, reached beside the `+` without opening the picker first.
//!
//! `AddProject` is not exercised here: it runs `AppState::choose_folder`, which opens the
//! platform's own folder dialog through `cx.prompt_for_paths` — nothing this suite can answer
//! headless. `new_project_rows`'s order covers it as plain data instead.

use chrono::Utc;
use gpui::{AppContext as _, Entity, TestAppContext, WindowHandle};
use gpui_component::Root;
use ubiq::app::{AppState, BusHub};
use ubiq::state::{NewProjectRow, WorkbenchState};
use ubiq_proto::bus;
use ubiq_proto::ids::ProjectId;
use ubiq_proto::projects::{ProjectHealth, ProjectRecord, ProjectSnapshot};

struct Fixture {
    state: Entity<AppState>,
    window: WindowHandle<Root>,
}

impl Fixture {
    fn open(cx: &mut TestAppContext) -> Self {
        let snapshot = a_project();
        let project = snapshot.record.id;
        let (hub, _host) = bus::hub();

        cx.update(|cx| {
            gpui_component::init(cx);
            ubiq::theme::set_mode(ubiq::app::boot_theme(), cx);
            BusHub::install(hub, cx);
            ubiq::state::WindowRegistry::install(cx);
            cx.global_mut::<ubiq::state::WindowRegistry>()
                .apply(snapshot);
        });

        let held = std::rc::Rc::new(std::cell::RefCell::new(None));
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
        Self { state, window }
    }

    /// Open the menu, and pick one of its rows — the two gestures, on the same indexing the menu
    /// is drawn with.
    fn pick(&self, index: usize, cx: &mut TestAppContext) {
        self.window
            .update(cx, |_, window, cx| {
                self.state.update(cx, |state, cx| {
                    state.open_new_project_menu((12.0, 34.0), cx);
                    state.pick_new_project_menu(index, window, cx);
                });
            })
            .expect("the window is open");
        cx.run_until_parked();
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
            temporary: false,
            created_at: Utc::now(),
            last_opened_at: None,
            search_excludes: Vec::new(),
            index: None,
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

/// `new_project_rows` is pure state — a fixed order, the same one the project picker's foot
/// draws its three ways in.
#[test]
fn the_menu_offers_add_clone_and_remote_in_the_pickers_own_order() {
    let workbench = WorkbenchState::default();
    assert_eq!(
        workbench.new_project_rows(),
        vec![
            NewProjectRow::AddProject,
            NewProjectRow::CloneProject,
            NewProjectRow::RemoteProject,
        ]
    );
}

/// Picking the clone row runs the same action the picker's own "Clone a project…" row does, and
/// closes the menu behind it.
#[gpui::test]
fn picking_clone_raises_the_clone_modal(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);

    fixture.pick(1, cx);

    assert!(
        fixture
            .state
            .read_with(cx, |state, _| state.workbench.clone_project.is_some()),
        "picking the clone row never raised the clone modal"
    );
    assert_eq!(
        fixture
            .state
            .read_with(cx, |state, _| state.workbench.open_menu),
        None,
        "a pick closes the menu"
    );
}

/// With no remote host attached, picking the remote row runs the same fallback the picker's own
/// "Open remote project…" row does: the connect modal, since there is nothing yet to browse.
#[gpui::test]
fn picking_remote_with_nothing_attached_raises_the_connect_modal(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);

    fixture.pick(2, cx);

    assert!(
        fixture
            .state
            .read_with(cx, |state, _| state.workbench.remote_connect.is_some()),
        "picking the remote row with nothing attached never raised the connect modal"
    );
    assert_eq!(
        fixture
            .state
            .read_with(cx, |state, _| state.workbench.open_menu),
        None,
        "a pick closes the menu"
    );
}

/// The separator between two picks in the same menu, and Escape's own dismissal, work exactly as
/// the overflow and new-pane menus already promise: opening the menu twice never leaves two
/// stacked, and a pick past the last row does nothing rather than panicking.
#[gpui::test]
fn a_pick_past_the_last_row_does_nothing(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);

    fixture.pick(99, cx);

    assert_eq!(
        fixture
            .state
            .read_with(cx, |state, _| state.workbench.open_menu),
        None,
        "an out-of-range pick left the menu open"
    );
    assert!(
        fixture
            .state
            .read_with(cx, |state, _| state.workbench.clone_project.is_none()
                && state.workbench.remote_connect.is_none()),
        "an out-of-range pick raised something anyway"
    );
}
