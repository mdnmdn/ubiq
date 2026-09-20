//! A project that runs on a drone: the row it keeps, and the dialog that pins it.
//!
//! **The rule under test is that a duplicate `ProjectId` belongs to the local host.** A drone
//! launched with `--root-id` announces a folder under the id the local catalogue already minted,
//! so the catalogue sees the same project twice — and the local record is the one with the user's
//! name, colour and origin on it. The bus half of that rule is unit-tested beside `Bus` itself;
//! what is here is the projection the picker draws from, and the form that writes the origin.

use std::time::Duration;

use gpui_component::input::InputEvent;
use ubiq::app::{AppState, BusHub};
use ubiq::state::sink::{DroneField, ProjectNav};
use ubiq::state::windows::WindowRegistry;
use ubiq::state::workbench::{ProjectSettings, ProjectSettingsMode};
use ubiq_proto::bus::FromClient;
use ubiq_proto::ids::{ProjectId, SshProfileId};
use ubiq_proto::messages::Message;
use ubiq_proto::projects::{
    DroneChange, DroneOrigin, ProjectHealth, ProjectRecord, ProjectSnapshot,
};
use ubiq_proto::settings::DronePreset;

/// Long enough for a message to cross a channel in the same process.
const PATIENCE: Duration = Duration::from_millis(500);

fn origin(profile: SshProfileId, root: &str) -> DroneOrigin {
    DroneOrigin {
        profile,
        root: root.to_string(),
        preset: DronePreset::Session,
        linger_secs: None,
    }
}

fn snapshot(id: ProjectId, name: &str, runs_on: Option<DroneOrigin>) -> ProjectSnapshot {
    ProjectSnapshot {
        record: ProjectRecord {
            id,
            name: name.to_string(),
            path: "/srv/proj".to_string(),
            colour: 3,
            custom_colour: None,
            temporary: false,
            created_at: chrono::Utc::now(),
            last_opened_at: None,
            search_excludes: Vec::new(),
            index: None,
            tools: Vec::new(),
            managed_repos: Vec::new(),
            lanes: Vec::new(),
            runs_on,
            initials: String::new(),
        },
        health: ProjectHealth::Ok,
        open_panes: 0,
        ephemeral: false,
        workarea: format!("/tmp/ubiq-test/projects/{id}/ui"),
    }
}

/// The drone's catalogue answer names the pinned folder under the local id. It must leave the row
/// exactly as it is: not removed, not duplicated beside itself, and not overwritten with the
/// drone's bare reading of the folder — which knows nothing of the name or the origin.
#[test]
fn a_drones_answer_neither_evicts_nor_duplicates_the_local_row() {
    let profile = SshProfileId::generate();
    let pinned = ProjectId::generate();
    let mut registry = WindowRegistry::default();
    registry.replace_all(vec![snapshot(
        pinned,
        "the pinned one",
        Some(origin(profile, "/srv/proj")),
    )]);

    // What `receive_project` does with a drone's list: `keep` names the rows this answer has no
    // opinion about, and the pinned row is one of them because the local host owns it.
    registry.replace_all_except(
        vec![
            snapshot(pinned, "proj", None),
            snapshot(ProjectId::generate(), "the drone's own", None),
        ],
        &[pinned],
    );

    let rows: Vec<&ProjectSnapshot> = registry.all().collect();
    assert_eq!(rows.len(), 2, "one pinned row, plus the drone's own");
    let kept = registry.project(pinned).expect("the pinned row survived");
    assert_eq!(kept.record.name, "the pinned one");
    assert_eq!(
        kept.record.runs_on.as_ref().map(|o| o.root.as_str()),
        Some("/srv/proj"),
        "the origin is the local record's and the drone says nothing about it"
    );
}

/// The drone goes: the row stays and says which drone is not there. A user whose laptop slept
/// still has their project.
#[test]
fn a_dropped_drone_leaves_the_row_unreadable_rather_than_gone() {
    let profile = SshProfileId::generate();
    let pinned = ProjectId::generate();
    let mut registry = WindowRegistry::default();
    registry.replace_all(vec![snapshot(
        pinned,
        "the pinned one",
        Some(origin(profile, "/srv/proj")),
    )]);

    registry.mark_unreadable(pinned, "the drone build box is not attached".to_string());

    let row = registry.project(pinned).expect("the row is still there");
    assert_eq!(
        row.health,
        ProjectHealth::Unreadable("the drone build box is not attached".to_string())
    );
    assert!(row.record.runs_on.is_some(), "and it is still pinned");
}

/// The three states an `UpdateProject` can carry about a drone, off the form that writes them.
#[test]
fn the_form_says_set_local_or_nothing_at_all() {
    let profile = SshProfileId::generate();
    let current = origin(profile, "/srv/proj");

    // Nothing said: the form agrees with the record.
    let unchanged = DroneField::from_origin(Some(&current));
    assert_eq!(unchanged.change("/srv/proj", Some(&current)), None);

    // Set: a folder that moved.
    assert_eq!(
        unchanged.change("/srv/elsewhere", Some(&current)),
        Some(DroneChange::Set(origin(profile, "/srv/elsewhere")))
    );

    // Local: brought home.
    let home = DroneField::default();
    assert_eq!(
        home.change("/srv/proj", Some(&current)),
        Some(DroneChange::Local)
    );
    // And a project that was already local stays silent rather than saying so again.
    assert_eq!(home.change("", None), None);

    // On a drone, but no profile picked yet — a half-filled form is not an origin.
    let half = DroneField {
        on_drone: true,
        profile: None,
        preset: DronePreset::Managed,
    };
    assert_eq!(half.change("/srv/proj", None), None);
}

/// The Remote arm of the nav, and the enablement rule it had to learn. The knowledge base joined
/// it on the same footing, which is why the count is here rather than the arm's position.
#[gpui::test]
fn the_remote_nav_needs_a_record_to_attach_to(cx: &mut gpui::TestAppContext) {
    use gpui::AppContext as _;

    assert_eq!(ProjectNav::all().len(), 7);
    assert_eq!(ProjectNav::Remote.label(), "Remote");
    assert_eq!(ProjectNav::Kb.label(), "Knowledge base");

    let (hub, _host) = ubiq_proto::bus::hub();
    cx.update(|cx| {
        gpui_component::init(cx);
        ubiq::theme::set_mode(ubiq::app::boot_theme(), cx);
        BusHub::install(hub, cx);
        WindowRegistry::install(cx);
        ubiq::app::install_key_bindings(cx);
    });

    let held: std::rc::Rc<std::cell::RefCell<Option<gpui::Entity<AppState>>>> = Default::default();
    let taken = held.clone();
    let _handle = cx.add_window(move |window, cx| {
        let state = cx.new(|cx| AppState::for_project(None, 'A', window, cx));
        *taken.borrow_mut() = Some(state.clone());
        gpui_component::Root::new(state, window, cx)
    });
    cx.run_until_parked();
    let state = held
        .borrow_mut()
        .take()
        .expect("the window built its state");

    state.update(cx, |state, cx| {
        // The sink page is a fixture: every arm is reachable there.
        state.set_sink_project_nav(ProjectNav::Remote, cx);
        assert_eq!(state.sink.project.nav, ProjectNav::Remote);

        // A folder not in the catalogue yet has no record to pin, so Remote is refused exactly
        // as Tools is.
        state.workbench.project_settings = Some(ProjectSettings {
            mode: ProjectSettingsMode::Create {
                path: "/tmp/x".to_string(),
            },
            colour: Default::default(),
            drone: DroneField::default(),
            nav: ProjectNav::General,
        });
        state.set_sink_project_nav(ProjectNav::Remote, cx);
        assert_eq!(
            state.workbench.project_settings.as_ref().unwrap().nav,
            ProjectNav::General
        );

        // Editing an existing project, it opens.
        state.workbench.project_settings = Some(ProjectSettings {
            mode: ProjectSettingsMode::Edit {
                project: ProjectId::generate(),
            },
            colour: Default::default(),
            drone: DroneField::default(),
            nav: ProjectNav::General,
        });
        state.set_sink_project_nav(ProjectNav::Remote, cx);
        assert_eq!(
            state.workbench.project_settings.as_ref().unwrap().nav,
            ProjectNav::Remote
        );
    });
}

/// The dialog's rail-initials override: prefilled from the record's own on open, clamped to two
/// characters as it is typed rather than only refused on Save, and sent as `SetProjectInitials`
/// alongside the rename's own `UpdateProject` — never before, because a project being created has
/// no record yet for an override to belong to.
#[gpui::test]
fn the_initials_field_prefills_clamps_and_saves(cx: &mut gpui::TestAppContext) {
    use gpui::AppContext as _;

    let project = ProjectId::generate();
    let mut record = snapshot(project, "ubiq studio", None);
    record.record.initials = "u".to_string();

    let (hub, host) = ubiq_proto::bus::hub();
    cx.update(|cx| {
        gpui_component::init(cx);
        ubiq::theme::set_mode(ubiq::app::boot_theme(), cx);
        BusHub::install(hub, cx);
        WindowRegistry::install(cx);
        cx.global_mut::<WindowRegistry>().apply(record);
        ubiq::app::install_key_bindings(cx);
    });

    let held: std::rc::Rc<std::cell::RefCell<Option<gpui::Entity<AppState>>>> = Default::default();
    let taken = held.clone();
    let window = cx.add_window(move |window, cx| {
        let state = cx.new(|cx| AppState::for_project(Some(project), 'A', window, cx));
        *taken.borrow_mut() = Some(state.clone());
        gpui_component::Root::new(state, window, cx)
    });
    cx.run_until_parked();
    let state = held
        .borrow_mut()
        .take()
        .expect("the window built its state");

    // Opening the dialog fills the field from the record's own override on the next frame.
    window
        .update(cx, |_, _window, cx| {
            state.update(cx, |state, cx| state.open_edit_project(cx));
        })
        .expect("the window is open");
    cx.run_until_parked();
    state.update(cx, |state, cx| {
        assert_eq!(state.project_initials_input.read(cx).value(), "u");
    });

    // Typed past two characters, it is clamped as it is typed, not only refused on Save.
    // `set_value` itself is the silent, programmatic setter used above for the prefill — it
    // deliberately does not raise `InputEvent::Change` — so a typed keystroke is modelled the same
    // way the field's own `PressEnter` gesture is modelled elsewhere: set the text, then raise the
    // event the real input widget raises for a keystroke.
    window
        .update(cx, |_, window, cx| {
            state.update(cx, |state, cx| {
                let input = state.project_initials_input.clone();
                input.update(cx, |input, cx| {
                    input.set_value("xyz", window, cx);
                    cx.emit(InputEvent::Change);
                });
            });
        })
        .expect("the window is open");
    cx.run_until_parked();
    state.update(cx, |state, cx| {
        assert_eq!(state.project_initials_input.read(cx).value(), "xy");
    });

    // Save sends the clamped override alongside the rename's own `UpdateProject`.
    window
        .update(cx, |_, _window, cx| {
            state.update(cx, |state, cx| state.commit_project_settings(cx));
        })
        .expect("the window is open");
    cx.run_until_parked();

    let mut saw_initials = false;
    while let Ok(event) = host.recv_timeout(PATIENCE) {
        if let FromClient::Said { message, .. } = event
            && let Message::SetProjectInitials {
                project_id,
                initials,
            } = message
        {
            assert_eq!(project_id, project);
            assert_eq!(initials, "xy");
            saw_initials = true;
        }
    }
    assert!(saw_initials, "commit sent the clamped override");
}
