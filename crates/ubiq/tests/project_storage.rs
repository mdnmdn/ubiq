//! The project settings dialog's **Project data** row: asking for a storage move, and the two
//! answers it can get (`D173`, `T-249`).
//!
//! **The rule under test is that the row never swaps optimistically.** The host copies a tree
//! across two unrelated roots and refuses the move outright while a pane or a conversation is
//! running in the project, so what the pills draw is the record's own mode until
//! `ProjectStorageMoved` says otherwise — and a refusal puts the row back to that mode rather than
//! guessing from the `storage` on the error, which is the mode that was *not* reached.

use std::time::Duration;

use gpui::{AppContext as _, Entity, TestAppContext, WindowHandle};
use gpui_component::Root;
use ubiq::app::{AppState, BusHub};
use ubiq::state::WindowRegistry;
use ubiq::state::sink::{ColourField, DroneField, ProjectNav};
use ubiq::state::workbench::{ProjectSettings, ProjectSettingsMode};
use ubiq_proto::bus::{FromClient, To};
use ubiq_proto::ids::ProjectId;
use ubiq_proto::messages::Message;
use ubiq_proto::projects::{ProjectHealth, ProjectRecord, ProjectSnapshot, StorageMode};

/// Long enough for a message to cross a channel in the same process.
const PATIENCE: Duration = Duration::from_millis(500);

struct Fixture {
    state: Entity<AppState>,
    _window: WindowHandle<Root>,
    host: ubiq_proto::bus::HostEnd,
    project: ProjectId,
}

impl Fixture {
    /// A window with one project in the catalogue, its settings dialog open on Edit.
    fn open(cx: &mut TestAppContext) -> Self {
        let snapshot = a_project();
        let project = snapshot.record.id;
        let (hub, host) = ubiq_proto::bus::hub();

        cx.update(|cx| {
            gpui_component::init(cx);
            ubiq::theme::set_mode(ubiq::app::boot_theme(), cx);
            BusHub::install(hub, cx);
            WindowRegistry::install(cx);
            cx.global_mut::<WindowRegistry>().apply(snapshot);
        });

        let held: std::rc::Rc<std::cell::RefCell<Option<Entity<AppState>>>> = Default::default();
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

        state.update(cx, |state, _| {
            state.workbench.project_settings = Some(ProjectSettings {
                mode: ProjectSettingsMode::Edit { project },
                colour: ColourField::default(),
                drone: DroneField::default(),
                nav: ProjectNav::default(),
                definitions_use_global: true,
                storage_pending: None,
                storage_dir: None,
                storage_error: None,
            });
        });

        Self {
            state,
            _window: window,
            host,
            project,
        }
    }

    /// Everything this window has said to the host, drained.
    fn said(&self) -> Vec<Message> {
        let mut said = Vec::new();
        while let Ok(event) = self.host.recv_timeout(PATIENCE) {
            if let FromClient::Said { message, .. } = event {
                said.push(message);
            }
        }
        said
    }

    fn dialog<R>(&self, cx: &mut TestAppContext, read: impl Fn(&ProjectSettings) -> R) -> R {
        self.state.read_with(cx, |state, _| {
            read(
                state
                    .workbench
                    .project_settings
                    .as_ref()
                    .expect("the dialog is open"),
            )
        })
    }
}

fn a_project() -> ProjectSnapshot {
    let id = ProjectId::generate();
    ProjectSnapshot {
        record: ProjectRecord {
            id,
            name: "ubiq".to_string(),
            path: "/tmp/ubiq".to_string(),
            colour: 0,
            custom_colour: None,
            storage: StorageMode::UbiqManaged,
            temporary: false,
            created_at: chrono::Utc::now(),
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
        workarea: format!("/tmp/ubiq-test/projects/{id}/ui"),
    }
}

/// The ask goes out as `SetProjectStorage`, the row starts waiting, and a second click while it
/// waits says nothing more — the host is answering the first.
#[gpui::test]
fn the_row_asks_once_and_waits(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let project = fixture.project;

    fixture.state.update(cx, |state, cx| {
        state.set_project_storage(project, StorageMode::ProjectManaged, cx);
        state.set_project_storage(project, StorageMode::UbiqManaged, cx);
    });
    cx.run_until_parked();

    let said: Vec<Message> = fixture
        .said()
        .into_iter()
        .filter(|message| matches!(message, Message::SetProjectStorage { .. }))
        .collect();
    assert_eq!(said.len(), 1, "one ask, not two: {said:?}");
    assert!(matches!(
        said[0],
        Message::SetProjectStorage {
            storage: StorageMode::ProjectManaged,
            ..
        }
    ));

    assert_eq!(
        fixture.dialog(cx, |dialog| dialog.storage_pending),
        Some(StorageMode::ProjectManaged),
        "the row names what it is waiting for"
    );
    // And nothing was drawn as moved: the record still says where the data is.
    assert_eq!(
        fixture.state.read_with(cx, |_, cx| WindowRegistry::read(cx)
            .project(project)
            .expect("the project is in the catalogue")
            .record
            .storage),
        StorageMode::UbiqManaged,
    );
}

/// A refusal puts the row back to the record's own mode and says why in full. The `storage` on the
/// error is the mode that was not reached, so nothing is read off it.
#[gpui::test]
fn a_refusal_restores_the_row_and_is_shown(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let project = fixture.project;

    fixture.state.update(cx, |state, cx| {
        state.set_project_storage(project, StorageMode::ProjectManaged, cx)
    });
    cx.run_until_parked();

    fixture.host.send(
        To::Everyone,
        Message::ProjectStorageError {
            project_id: project,
            storage: StorageMode::ProjectManaged,
            error: "close what is running in this project first".to_string(),
        },
    );
    cx.run_until_parked();

    assert_eq!(
        fixture.dialog(cx, |dialog| dialog.storage_pending),
        None,
        "the row has stopped waiting"
    );
    assert_eq!(
        fixture.dialog(cx, |dialog| dialog.storage_error.clone()),
        Some("close what is running in this project first".to_string()),
        "and says why, in the host's own words"
    );
    assert_eq!(
        fixture.state.read_with(cx, |_, cx| WindowRegistry::read(cx)
            .project(project)
            .expect("the project is in the catalogue")
            .record
            .storage),
        StorageMode::UbiqManaged,
        "the data is where it was",
    );
}

/// The move finished: the row stops waiting, keeps the directory the host named, and takes its new
/// mode from the `ProjectChanged` broadcast beside the answer rather than from the answer itself.
#[gpui::test]
fn a_move_clears_the_pending_state_and_shows_the_directory(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let project = fixture.project;

    fixture.state.update(cx, |state, cx| {
        state.set_project_storage(project, StorageMode::ProjectManaged, cx)
    });
    cx.run_until_parked();

    let mut moved = a_project();
    moved.record.id = project;
    moved.record.storage = StorageMode::ProjectManaged;
    fixture
        .host
        .send(To::Everyone, Message::ProjectChanged { project: moved });
    fixture.host.send(
        To::Everyone,
        Message::ProjectStorageMoved {
            project_id: project,
            storage: StorageMode::ProjectManaged,
            dir: "/tmp/ubiq/.ubiq".to_string(),
        },
    );
    cx.run_until_parked();

    assert_eq!(fixture.dialog(cx, |dialog| dialog.storage_pending), None);
    assert_eq!(
        fixture.dialog(cx, |dialog| dialog.storage_error.clone()),
        None
    );
    assert_eq!(
        fixture.dialog(cx, |dialog| dialog.storage_dir.clone()),
        Some("/tmp/ubiq/.ubiq".to_string()),
        "where the host says the data is now"
    );
    assert_eq!(
        fixture.state.read_with(cx, |_, cx| WindowRegistry::read(cx)
            .project(project)
            .expect("the project is in the catalogue")
            .record
            .storage),
        StorageMode::ProjectManaged,
        "and the record's own mode came from the broadcast",
    );
}
