//! The interface-owned settings blob: the interface versions it, so a schema it does not know is
//! discarded rather than half-applied.
//!
//! The Host layer is a different animal — the host owns its schema, not this file — so its round
//! trip is asserted over the bus rather than over `decode`/`encode`: a toggle writes `SetSettings`
//! on the Host layer, and the checkbox shows whatever the host answers, not a local default.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use chrono::Utc;
use gpui::{AppContext as _, Entity, TestAppContext, WindowHandle};
use gpui_component::input::InputEvent;
use ubiq::app::{AppState, BusHub};
use ubiq::state::WindowRegistry;
use ubiq::state::editor::{OpenFile, ViewLayout, ViewerKind};
use ubiq::state::settings::{self, MarkdownOpen, UiSettings};
use ubiq_proto::bus::{self, FromClient, To};
use ubiq_proto::ids::ProjectId;
use ubiq_proto::messages::Message;
use ubiq_proto::projects::{ProjectHealth, ProjectRecord, ProjectSnapshot};
use ubiq_proto::settings::{HOST_SETTINGS_SCHEMA, HostSettings, SettingsLayer};

#[test]
fn a_blob_survives_the_round_trip() {
    let settings = UiSettings {
        schema: settings::SCHEMA,
        explorer_preview: false,
        rail_projects: false,
        markdown_open: MarkdownOpen::Source,
        vim_mode: true,
    };
    let back = settings::decode(&settings::encode(&settings)).expect("decodes");
    assert_eq!(back, settings);
}

#[test]
fn missing_fields_open_on_defaults() {
    let blob = r#"{"schema":1}"#;
    let back = settings::decode(blob).expect("decodes");
    assert!(back.explorer_preview);
    assert!(back.rail_projects);
    assert_eq!(back.markdown_open, MarkdownOpen::Preview);
    // A blob written before vim mode existed opens with it off, rather than being discarded.
    assert!(!back.vim_mode);
}

#[test]
fn a_newer_schema_is_discarded() {
    let blob = r#"{"schema":99,"explorer_preview":false}"#;
    assert!(settings::decode(blob).is_none());
}

#[test]
fn unreadable_is_discarded() {
    assert!(settings::decode("not json").is_none());
}

#[test]
fn markdown_open_picks_the_layout_a_new_tab_starts_in() {
    let preview = OpenFile::opening(
        "README.md",
        ubiq::state::editor::Subject::File,
        ViewLayout::Preview,
    );
    assert_eq!(preview.viewer, ViewerKind::Markdown);
    assert_eq!(preview.layout, ViewLayout::Preview);

    let source = OpenFile::opening(
        "README.md",
        ubiq::state::editor::Subject::File,
        ViewLayout::Source,
    );
    assert_eq!(source.layout, ViewLayout::Source);

    // A mermaid file still opens in preview: the setting is markdown's.
    let mermaid = OpenFile::opening(
        "flow.mmd",
        ubiq::state::editor::Subject::File,
        ViewLayout::Source,
    );
    assert_eq!(mermaid.layout, ViewLayout::Preview);

    // Plain text has no preview.
    let rust = OpenFile::opening(
        "main.rs",
        ubiq::state::editor::Subject::File,
        ViewLayout::Preview,
    );
    assert_eq!(rust.layout, ViewLayout::Source);
}

/// Long enough for a message to cross a channel in the same process.
const PATIENCE: Duration = Duration::from_millis(500);

/// A window on one project, with the host end kept so a test can both answer it and read what the
/// window said. Mirrors `new_pane`'s fixture.
struct Fixture {
    state: Entity<AppState>,
    window: WindowHandle<gpui_component::Root>,
    host: bus::HostEnd,
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
            gpui_component::Root::new(state, window, cx)
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
        }
    }

    /// Everything the window has said so far, in order.
    fn said(&self) -> Vec<Message> {
        let mut said = Vec::new();
        while let Ok(event) = self.host.recv_timeout(PATIENCE) {
            if let FromClient::Said { message, .. } = event {
                said.push(message);
            }
        }
        said
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

/// The overlay's first write to the Host layer: flipping the checkbox does not touch `UiSettings`
/// at all, because the host is forbidden to parse that blob (`D46`). It writes `SetSettings` on
/// the Host layer instead, carrying the flag it just flipped.
#[gpui::test]
fn flipping_the_isolation_toggle_writes_the_host_layer(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    // Drain the startup `GetSettings` asks before watching for the toggle's own write.
    let _ = fixture.said();

    assert!(
        fixture
            .state
            .read_with(cx, |state, _| state.workbench.settings.host.isolate_agents),
        "isolation defaults to on"
    );

    fixture
        .state
        .update(cx, |state, cx| state.toggle_isolate_agents(cx));
    cx.run_until_parked();

    let written = fixture
        .said()
        .into_iter()
        .find_map(|message| match message {
            Message::SetSettings {
                layer: SettingsLayer::Host,
                value,
            } => Some(value),
            _ => None,
        })
        .expect("the toggle wrote the host layer");
    let sent: HostSettings = serde_json::from_str(&written).expect("a readable blob");
    assert!(
        !sent.isolate_agents,
        "the toggle flipped the flag it sent, not just the flag it holds"
    );
    assert_eq!(
        sent.schema, HOST_SETTINGS_SCHEMA,
        "the write is stamped with this build's host schema"
    );

    assert!(
        !fixture
            .state
            .read_with(cx, |state, _| state.workbench.settings.host.isolate_agents),
        "the toggle's own state matches what it just sent"
    );
}

/// The checkbox shows what the host actually has stored, not this build's default: a `Message::
/// Settings` for the Host layer decodes into the same state the toggle reads.
#[gpui::test]
fn the_hosts_answer_is_what_the_checkbox_shows(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let _ = fixture.said();

    let stored = HostSettings {
        schema: HOST_SETTINGS_SCHEMA,
        isolate_agents: false,
        ..HostSettings::default()
    };
    fixture.host.send(
        To::Everyone,
        Message::Settings {
            layer: SettingsLayer::Host,
            value: Some(serde_json::to_string(&stored).expect("HostSettings serialises")),
        },
    );
    cx.run_until_parked();

    assert!(
        !fixture
            .state
            .read_with(cx, |state, _| state.workbench.settings.host.isolate_agents),
        "the host's answer is what the checkbox shows, not the default it opened on"
    );
}

/// The two search lists are text fields holding a comma-separated line. They commit on Enter —
/// and on blur — rather than on a keystroke, because every commit makes the host write a file.
#[gpui::test]
fn committing_the_search_lists_writes_the_parsed_lists(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let _ = fixture.said();

    fixture.type_into(
        |state| state.search_excludes_input.clone(),
        "vendor, dist ,,",
        cx,
    );
    fixture.type_into(
        |state| state.search_fallbacks_input.clone(),
        " grep ,ag,",
        cx,
    );

    let sent = fixture
        .said()
        .into_iter()
        .filter_map(|message| match message {
            Message::SetSettings {
                layer: SettingsLayer::Host,
                value,
            } => serde_json::from_str::<HostSettings>(&value).ok(),
            _ => None,
        })
        .next_back()
        .expect("both commits wrote the host layer");

    assert_eq!(sent.search_excludes, vec!["vendor", "dist"]);
    assert_eq!(sent.search_fallbacks, vec!["grep", "ag"]);
    assert_eq!(sent.schema, HOST_SETTINGS_SCHEMA);

    assert_eq!(
        fixture
            .state
            .read_with(cx, |state, _| state.workbench.settings.host.clone())
            .search_excludes,
        vec!["vendor", "dist"],
        "the window holds what it sent"
    );
}

impl Fixture {
    /// Type into one of the window's fields and commit it, which is what Enter does.
    fn type_into(
        &self,
        pick: impl Fn(&AppState) -> Entity<gpui_component::input::InputState>,
        text: &str,
        cx: &mut TestAppContext,
    ) {
        self.window
            .update(cx, |_, window, cx| {
                self.state.update(cx, |state, cx| {
                    let input = pick(state);
                    input.update(cx, |field, cx| {
                        field.set_value(text, window, cx);
                        cx.emit(InputEvent::PressEnter {
                            shift: false,
                            secondary: false,
                        });
                    });
                });
            })
            .expect("the window is open");
        cx.run_until_parked();
    }
}
