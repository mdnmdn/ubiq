//! Switching rail modes restores that mode's arrangement.
//!
//! Each mode keeps its own record of which regions were on screen and its own dock blob. Coming
//! back to a mode must put the side panels (the IDE's explorer and chat) back where they were,
//! whatever non-IDE mode the window sat in meanwhile. That is hiding-not-removing: a non-IDE mode
//! leaves the explorer and the chat in the tree, shut, and returning to IDE opens them again.

use std::cell::RefCell;
use std::rc::Rc;

use chrono::Utc;
use gpui::{AppContext as _, Entity, TestAppContext};
use ubiq::app::{AppState, BusHub};
use ubiq::state::RailMode;
use ubiq::state::WindowRegistry;
use ubiq_proto::bus;
use ubiq_proto::ids::ProjectId;
use ubiq_proto::projects::{ProjectHealth, ProjectRecord, ProjectSnapshot};

/// A window on one project, with a bus nobody answers on. Mirrors `panel_reentrancy`'s fixture.
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

    fn switch_to(&self, mode: RailMode, cx: &mut TestAppContext) {
        let state = self.state.clone();
        state.update(cx, |state, cx| state.set_rail_mode(mode, cx));
        cx.run_until_parked();
    }

    fn mode(&self, cx: &mut TestAppContext) -> RailMode {
        self.state
            .read_with(cx, |state, _| state.workbench.rail_mode)
    }

    fn regions_open(&self, cx: &mut TestAppContext) -> (bool, bool, bool) {
        self.state.read_with(cx, |state, cx| state.regions_open(cx))
    }

    /// The full dump of the window's arrangement, as `remember_view` would write it.
    fn dump(&self, cx: &mut TestAppContext) -> String {
        self.state.read_with(cx, |state, cx| {
            serde_json::to_string(&state.dock().read(cx).dump(cx)).unwrap_or_default()
        })
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
            no_local_index: false,
        },
        health: ProjectHealth::Ok,
        open_panes: 0,
        ephemeral: false,
        workarea: "/tmp/ubiq-workarea".to_string(),
    }
}

/// Every leaf panel's permanent name, in the order it appears in a dump.
fn names(blob: &str) -> Vec<String> {
    let mut out = Vec::new();
    collect_names(&serde_json::from_str(blob).unwrap_or_default(), &mut out);
    out
}

fn collect_names(value: &serde_json::Value, into: &mut Vec<String>) {
    match value {
        serde_json::Value::Object(map) => {
            if let Some(name) = map.get("panel_name").and_then(|n| n.as_str()) {
                into.push(name.to_string());
            }
            for child in map.values() {
                collect_names(child, into);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                collect_names(item, into);
            }
        }
        _ => {}
    }
}

/// Leaving IDE for another mode hides the explorer and the chat — they stay in the tree, shut —
/// and coming back restores them. That is the invariant behind the restore, and it must hold for
/// any non-IDE mode: a project mode (Tasks) and a non-project one (Control) both hide, neither
/// removes.
#[gpui::test]
fn returning_from_any_non_ide_mode_restores_the_side_panels(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    assert_eq!(fixture.mode(cx), RailMode::Ide);

    for mode in [RailMode::Tasks, RailMode::Control] {
        // Leave IDE: the side panels go, but nothing is removed from the arrangement.
        let before = fixture.dump(cx);
        let regions_before = fixture.regions_open(cx);
        fixture.switch_to(mode, cx);
        assert_eq!(fixture.mode(cx), mode);
        let in_non_ide = names(&fixture.dump(cx));
        assert!(
            in_non_ide.contains(&"ubiq.explorer".to_string())
                && in_non_ide.contains(&"ubiq.chat".to_string()),
            "{mode:?} hides the side panels in place, it does not remove them: {in_non_ide:?}"
        );

        // Back to IDE: the arrangement is restored whole, side panels and the three regions.
        fixture.switch_to(RailMode::Ide, cx);
        assert_eq!(fixture.mode(cx), RailMode::Ide);
        let after = fixture.dump(cx);
        assert_eq!(
            names(&after),
            names(&before),
            "coming back from {mode:?} restores the arrangement left behind"
        );
        assert_eq!(
            fixture.regions_open(cx),
            regions_before,
            "coming back from {mode:?} restores the regions IDE was left with"
        );
        assert_eq!(
            regions_before,
            (true, false, true),
            "the IDE opens with its side panels and an empty pane region put away"
        );
        assert_eq!(
            fixture.state.read_with(cx, |state, cx| state.project(cx)),
            Some(fixture.project),
            "the window is still pointed at the project it opened on"
        );
    }
}

/// Hiding a mode takes it off the rail; hiding the mode the window is in moves the window on, and
/// the last visible mode cannot be hidden at all.
#[gpui::test]
fn hiding_modes_never_empties_the_rail(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let state = fixture.state.clone();

    state.update(cx, |state, cx| {
        assert!(state.mode_enabled(RailMode::Git, cx));
        state.toggle_mode(RailMode::Git, cx);
        assert!(!state.mode_enabled(RailMode::Git, cx));

        // The window is in IDE: hiding it has to leave the window somewhere else.
        state.toggle_mode(RailMode::Ide, cx);
        assert!(!state.mode_enabled(RailMode::Ide, cx));
        assert!(state.mode_enabled(state.workbench.rail_mode, cx));

        // Everything but the last one goes; the last one stays whatever is asked.
        for mode in RailMode::every() {
            if state.mode_enabled(mode, cx) {
                state.toggle_mode(mode, cx);
            }
        }
        let left: Vec<_> = RailMode::every()
            .filter(|mode| state.mode_enabled(*mode, cx))
            .collect();
        assert_eq!(left.len(), 1, "one mode always survives: {left:?}");
    });
    cx.run_until_parked();
}
