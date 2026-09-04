//! Opening a file must not read the window from inside a panel's own update.
//!
//! The dock activates the tab it has just added, which reaches `BasePanel::set_active` **inside
//! that panel's lease**. What the window does about it ends in writing the arrangement down, and
//! writing it down asks every panel in the tree for its name, its visibility and its payload —
//! including the one still leased. Doing that work inline is not a race but a certainty:
//! `cannot read ubiq::ui::dock::WorkbenchPanel while it is already being updated`.
//!
//! So the answer waits a turn. These tests are the guard on that: a window with no graphics device
//! behind it, a file opened through the gesture that opens one, and a frame drawn. Take the
//! deferral out of `ui/dock/mod.rs` and both of them panic in `settle_panels`.

use std::cell::RefCell;
use std::rc::Rc;

use chrono::Utc;
use gpui::{AppContext as _, Entity, TestAppContext};
use ubiq::app::{AppState, BusHub};
use ubiq::state::WindowRegistry;
use ubiq_proto::bus;
use ubiq_proto::ids::ProjectId;
use ubiq_proto::projects::{ProjectHealth, ProjectRecord, ProjectSnapshot};

/// A window on one project, with a bus nobody answers on.
///
/// The host end is kept alive for the test's duration rather than dropped: the window says what it
/// wants of a project the moment it opens one, and a bus with no reader left is a different code
/// path from a bus with a reader that never replies. Nothing has to answer — every assertion here
/// is about what the interface does on its own.
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

        // The window's root is the component library's, exactly as the binary builds it; the state
        // is taken out on the way past, because that is what the test drives.
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

    /// The gesture that opens a file, and the frame that puts its panel in the dock.
    ///
    /// The two are one step here for the same reason they are one step for the user: `select_file`
    /// only queues the panel, and `settle_panels` — which is where the dock activates the new tab,
    /// and where the panic was — is drained during render.
    fn open_file(&self, path: &str, cx: &mut TestAppContext) {
        let path = path.to_string();
        self.state
            .update(cx, |state, cx| state.select_file(path, cx));
        cx.run_until_parked();
    }

    /// The saved arrangement as it stands: every `ubiq.file` leaf in the dock's dump, by tab key.
    ///
    /// This is the round trip the panic was on the far side of — `DockArea::dump` is what reads
    /// every panel in the tree — so a dump that comes back with the file in it is the whole of the
    /// fix being in place.
    fn file_leaves(&self, cx: &mut TestAppContext) -> Vec<String> {
        let blob = self.state.read_with(cx, |state, cx| {
            serde_json::to_value(state.dock().read(cx).dump(cx)).expect("the dump serialises")
        });
        let mut keys = Vec::new();
        collect_file_leaves(&blob, &mut keys);
        keys
    }

    /// Which tab the editor says is active, for the project on screen.
    fn active_file(&self, cx: &mut TestAppContext) -> Option<String> {
        self.state.read_with(cx, |state, cx| {
            state
                .editor(cx)
                .and_then(|editor| editor.active_file())
                .map(|file| file.key())
        })
    }
}

/// Every `ubiq.file` leaf's tab key, wherever it sits in the tree.
fn collect_file_leaves(value: &serde_json::Value, into: &mut Vec<String>) {
    match value {
        serde_json::Value::Object(map) => {
            if map.get("panel_name").and_then(|name| name.as_str()) == Some("ubiq.file")
                && let Some(key) = map
                    .get("info")
                    .and_then(|info| info.get("panel"))
                    .and_then(|payload| payload.get("key"))
                    .and_then(|key| key.as_str())
            {
                into.push(key.to_string());
            }
            for child in map.values() {
                collect_file_leaves(child, into);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                collect_file_leaves(item, into);
            }
        }
        _ => {}
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
        workarea: "/tmp/ubiq-workarea".to_string(),
    }
}

/// Opening a file is the gesture the panic was reachable from, and the only assertion that matters
/// first is that the frame it is drained on completes at all.
#[gpui::test]
fn opening_a_file_does_not_read_the_window_from_inside_its_own_panel(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    fixture.open_file("crates/ubiq/src/app.rs", cx);

    assert_eq!(
        fixture.file_leaves(cx),
        vec!["crates/ubiq/src/app.rs".to_string()],
        "the file's panel is in the dock, and the arrangement round-trips through its dump"
    );
    assert_eq!(
        fixture.active_file(cx).as_deref(),
        Some("crates/ubiq/src/app.rs"),
        "the dock activated the tab it added, and the editor heard it"
    );
    assert!(
        fixture.state.read_with(cx, |state, cx| state
            .file("crates/ubiq/src/app.rs", cx)
            .is_some()),
        "the window holds the tab its panel is a view of"
    );
    assert_eq!(
        fixture.state.read_with(cx, |state, cx| state.project(cx)),
        Some(fixture.project),
        "the window is still pointed at the project it opened on"
    );
}

/// The same path by the other route: a second file joins the group the first is in, and activating
/// it is the dock telling a panel it is displayed while the panel that was displayed steps back.
#[gpui::test]
fn activating_a_second_file_tab_survives_the_same_reentrancy(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    fixture.open_file("crates/ubiq/src/app.rs", cx);
    fixture.open_file("crates/ubiq/src/theme.rs", cx);

    let mut leaves = fixture.file_leaves(cx);
    leaves.sort();
    assert_eq!(
        leaves,
        vec![
            "crates/ubiq/src/app.rs".to_string(),
            "crates/ubiq/src/theme.rs".to_string(),
        ],
        "both files are panels, and both write their tab key into the arrangement"
    );
    assert_eq!(
        fixture.active_file(cx).as_deref(),
        Some("crates/ubiq/src/theme.rs"),
        "the tab that was activated last is the active one"
    );
}
