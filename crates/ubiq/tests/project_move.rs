//! Moving a project out of the window that holds it, from inside that window's own update.
//!
//! "Open in a new window" is a menu item drawn by the window that holds the project, so the click
//! arrives through a `cx.listener` and the holding window's `AppState` is leased for the whole of
//! it. The new window has to take the live project off that same entity — `hand_off_project`, so
//! the panes and the conversations move rather than being killed — and doing it inline is a
//! certainty, not a race: `cannot update ubiq::app::AppState while it is already being updated`.
//!
//! So `open_project_window` waits a turn. This test is the guard on that: take the defer out of
//! `app/mod.rs` and it panics instead of opening a window.

use std::cell::RefCell;
use std::rc::Rc;

use chrono::Utc;
use gpui::{AppContext as _, Entity, TestAppContext, WindowId};
use ubiq::app::{AppState, BusHub, open_project_window};
use ubiq::state::WindowRegistry;
use ubiq_proto::bus;
use ubiq_proto::ids::ProjectId;
use ubiq_proto::projects::{ProjectHealth, ProjectRecord, ProjectSnapshot};

/// One window on one project, with a bus nobody answers on — the same fixture
/// `panel_reentrancy.rs` opens, and for the same reason: every assertion here is about what the
/// interface does on its own.
struct Fixture {
    state: Entity<AppState>,
    project: ProjectId,
    _host: bus::HostEnd,
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
        cx.add_window(move |window, cx| {
            let state = cx.new(|cx| AppState::for_project(Some(project), 'A', window, cx));
            *taken.borrow_mut() = Some(state.clone());
            gpui_component::Root::new(state, window, cx)
        });
        cx.run_until_parked();

        let state = held
            .borrow_mut()
            .take()
            .expect("the window built its state");
        Self {
            state,
            project,
            _host: host,
        }
    }

    /// Which window the registry says holds the project.
    fn holder(&self, cx: &mut TestAppContext) -> Option<WindowId> {
        let project = self.project;
        cx.update(|cx| WindowRegistry::read(cx).holder(project).map(|slot| slot.id))
    }
}

fn a_project() -> ProjectSnapshot {
    let id = ProjectId::generate();
    ProjectSnapshot {
        record: ProjectRecord {
            id,
            name: "moved".to_string(),
            path: "/tmp/ubiq-test/moved".to_string(),
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
        workarea: format!("/tmp/ubiq-test/projects/{id}/ui"),
    }
}

/// The gesture as the menu makes it: inside the holding window's lease.
#[gpui::test]
fn open_in_a_new_window_from_the_window_that_holds_the_project(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let first = fixture.holder(cx).expect("the window holds its project");

    // Exactly what `ui/project_menu.rs` and `ui/all_projects.rs` do from a `cx.listener`: the
    // state is leased, and the call has to survive that.
    let project = fixture.project;
    fixture.state.update(cx, |_state, cx| {
        open_project_window(Some(project), cx);
    });
    cx.run_until_parked();

    // The project left: another window holds it now, and the one it came from does not.
    let second = fixture.holder(cx).expect("the project is still open");
    assert_ne!(
        second, first,
        "the project should have moved to the new window"
    );
    cx.update(|cx| {
        let registry = WindowRegistry::read(cx);
        assert!(
            !registry.slot(first).is_some_and(|slot| slot.holds(project)),
            "the window it came from should have let go of it"
        );
    });
}
