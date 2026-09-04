//! Project search's trigger side: what a submitted query puts on the bus, and what the window
//! refuses to draw.
//!
//! The results are the host's; what is Ubiq's here is that a query becomes exactly one
//! `SearchProject` carrying the options as set, that an empty one is not a search, that a second
//! one cancels the first, and that a reply naming a search this window is not holding is dropped.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use chrono::Utc;
use gpui::{AppContext as _, Entity, Focusable as _, TestAppContext, WindowHandle};
use gpui_component::Root;
use gpui_component::input::InputEvent;
use ubiq::app::{AppState, BusHub};
use ubiq::state::WindowRegistry;
use ubiq_proto::bus::{self, FromClient, To};
use ubiq_proto::ids::{ProjectId, SearchId};
use ubiq_proto::messages::Message;
use ubiq_proto::projects::{ProjectHealth, ProjectRecord, ProjectSnapshot};
use ubiq_proto::search::{Batch, FileHit, LineHit, Query};

/// Long enough for a message to cross a channel in the same process.
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

    /// Type into the query field and submit, which is what the field's Enter subscription does.
    fn search(&self, text: &str, cx: &mut TestAppContext) {
        self.window
            .update(cx, |_, window, cx| {
                self.state.update(cx, |state, cx| {
                    let input = state.search.query.clone();
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

fn searches(said: Vec<Message>) -> Vec<(SearchId, Query)> {
    said.into_iter()
        .filter_map(|message| match message {
            Message::SearchProject {
                search_id, query, ..
            } => Some((search_id, query)),
            _ => None,
        })
        .collect()
}

#[gpui::test]
fn submitting_a_query_asks_the_host_to_search(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let _ = fixture.said();

    fixture.search("  needle  ", cx);

    let asked = searches(fixture.said());
    assert_eq!(asked.len(), 1, "one submit is one search");
    assert_eq!(asked[0].1.text, "needle", "the query is trimmed");
    assert!(!asked[0].1.case_sensitive);
    assert!(!asked[0].1.whole_word);
    assert!(!asked[0].1.regex);
    assert_eq!(
        fixture.state.read_with(cx, |state, _| state
            .search
            .active
            .as_ref()
            .map(|a| a.search_id)),
        Some(asked[0].0),
        "the window holds the id it minted"
    );
}

#[gpui::test]
fn an_empty_query_is_not_a_search(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let _ = fixture.said();

    fixture.search("   ", cx);

    assert!(
        searches(fixture.said()).is_empty(),
        "whitespace is not a query"
    );
    assert!(
        fixture
            .state
            .read_with(cx, |state, _| state.search.active.is_none()),
        "and nothing is in flight"
    );
}

#[gpui::test]
fn a_second_search_cancels_the_first(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let _ = fixture.said();

    fixture.search("first", cx);
    let first = searches(fixture.said())[0].0;

    fixture.search("second", cx);
    let said = fixture.said();

    let cancelled = said
        .iter()
        .position(|m| matches!(m, Message::CancelSearch { search_id, .. } if *search_id == first));
    let started = said
        .iter()
        .position(|m| matches!(m, Message::SearchProject { .. }));
    assert!(
        cancelled < started,
        "the old search is cancelled before the new one starts: {said:?}"
    );
}

#[gpui::test]
fn the_options_travel_with_the_next_query(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let _ = fixture.said();

    fixture
        .window
        .update(cx, |_, _, cx| {
            fixture.state.update(cx, |state, cx| {
                state.toggle_search_case(cx);
                state.toggle_search_regex(cx);
            });
        })
        .expect("the window is open");
    fixture.search("ne+dle", cx);

    let query = &searches(fixture.said())[0].1;
    assert!(query.case_sensitive);
    assert!(!query.whole_word);
    assert!(query.regex);
}

#[gpui::test]
fn a_reply_naming_another_search_is_discarded(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let _ = fixture.said();
    fixture.search("needle", cx);

    fixture.host.send(
        To::Everyone,
        Message::SearchMatches {
            project_id: fixture.project,
            search_id: SearchId::generate(),
            batch: Batch::Files(vec![FileHit {
                rel_path: "src/main.rs".to_string(),
                lines: vec![LineHit {
                    line: 1,
                    text: "needle".to_string(),
                    ranges: vec![(0, 6)],
                }],
                truncated: false,
            }]),
        },
    );
    cx.run_until_parked();

    assert!(
        fixture
            .state
            .read_with(cx, |state, _| state.search.results.is_empty()),
        "a batch the window is not holding draws nothing"
    );
}

#[gpui::test]
fn the_header_field_starts_a_search_and_switches_to_the_ide(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let _ = fixture.said();

    fixture
        .window
        .update(cx, |_, window, cx| {
            fixture.state.update(cx, |state, cx| {
                state.set_rail_mode(ubiq::state::RailMode::Control, cx);
                let input = state.command_input.clone();
                input.update(cx, |field, cx| {
                    field.set_value("needle", window, cx);
                    cx.emit(InputEvent::PressEnter {
                        shift: false,
                        secondary: false,
                    });
                });
            });
        })
        .expect("the window is open");
    cx.run_until_parked();

    let asked = searches(fixture.said());
    assert_eq!(asked.len(), 1, "Enter in the header field runs one search");
    assert_eq!(asked[0].1.text, "needle");
    assert_eq!(
        fixture
            .state
            .read_with(cx, |state, _| state.workbench.rail_mode),
        ubiq::state::RailMode::Ide,
        "the ide comes up so the search panel has somewhere to be drawn"
    );
    assert_eq!(
        fixture.state.read_with(cx, |state, cx| state
            .command_input
            .read(cx)
            .value()
            .to_string()),
        "",
        "the header field clears once its search has fired"
    );
    assert_eq!(
        fixture.state.read_with(cx, |state, cx| state
            .search
            .query
            .read(cx)
            .value()
            .to_string()),
        "needle",
        "the search panel's own field carries the query"
    );
}

// ── The panel itself ────────────────────────────────────────────────

/// Every panel name in the dock's saved arrangement, wherever it sits.
fn leaves(value: &serde_json::Value, into: &mut Vec<String>) {
    match value {
        serde_json::Value::Object(map) => {
            if let Some(name) = map.get("panel_name").and_then(|name| name.as_str()) {
                into.push(name.to_string());
            }
            for child in map.values() {
                leaves(child, into);
            }
        }
        serde_json::Value::Array(items) => items.iter().for_each(|item| leaves(item, into)),
        _ => {}
    }
}

impl Fixture {
    fn panel_names(&self, cx: &mut TestAppContext) -> Vec<String> {
        let blob = self.state.read_with(cx, |state, cx| {
            serde_json::to_value(state.dock().read(cx).dump(cx)).expect("the dump serialises")
        });
        let mut names = Vec::new();
        leaves(&blob, &mut names);
        names
    }
}

/// The saved arrangement coming back from the host — what happens on every restart, and on every
/// echo of the blob the window itself just wrote down.
fn restore_arrangement(fixture: &Fixture, blob: serde_json::Value, cx: &mut TestAppContext) {
    let mut modes = std::collections::HashMap::new();
    modes.insert(
        ubiq::state::RailMode::Ide,
        ubiq::state::prefs::ModeLayout {
            show_left: true,
            show_bottom: true,
            show_right: true,
            layout: Some(blob),
        },
    );
    let prefs = ubiq::state::prefs::ViewPrefs {
        modes,
        ..Default::default()
    };
    fixture.host.send(
        To::Everyone,
        Message::Preferences {
            scope: ubiq_proto::projects::Scope::Project(fixture.project),
            value: Some(ubiq::state::prefs::encode(&prefs)),
        },
    );
    cx.run_until_parked();
}

#[gpui::test]
fn revealing_the_search_panel_puts_it_on_screen(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);

    fixture
        .window
        .update(cx, |_, window, cx| {
            fixture
                .state
                .update(cx, |state, cx| state.reveal_search(window, cx));
        })
        .expect("the window is open");
    cx.run_until_parked();

    assert!(
        fixture.panel_names(cx).iter().any(|n| n == "ubiq.search"),
        "the dock holds the search panel: {:?}",
        fixture.panel_names(cx)
    );
    assert!(
        fixture.state.read_with(cx, |state, cx| state
            .dock()
            .read(cx)
            .is_dock_open(gpui_component::dock::DockPlacement::Bottom)),
        "the bottom region is open"
    );
    assert!(
        fixture
            .window
            .update(cx, |_, window, cx| {
                fixture.state.read_with(cx, |state, cx| {
                    state
                        .search
                        .query
                        .read(cx)
                        .focus_handle(cx)
                        .is_focused(window)
                })
            })
            .expect("the window is open"),
        "the query field takes the keyboard, or the reveal looks like nothing happened"
    );
}

#[gpui::test]
fn the_search_panel_survives_its_own_arrangement_coming_back(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);

    fixture
        .window
        .update(cx, |_, window, cx| {
            fixture
                .state
                .update(cx, |state, cx| state.reveal_search(window, cx));
        })
        .expect("the window is open");
    cx.run_until_parked();

    let blob = fixture.state.read_with(cx, |state, cx| {
        serde_json::to_value(state.dock().read(cx).dump(cx)).expect("the dump serialises")
    });
    restore_arrangement(&fixture, blob, cx);

    assert!(
        fixture.panel_names(cx).iter().any(|n| n == "ubiq.search"),
        "the restored arrangement still holds the search panel: {:?}",
        fixture.panel_names(cx)
    );

    // And revealing it again, on the entity the restore built, still puts it on screen.
    fixture
        .window
        .update(cx, |_, window, cx| {
            fixture
                .state
                .update(cx, |state, cx| state.reveal_search(window, cx));
        })
        .expect("the window is open");
    cx.run_until_parked();

    let names = fixture.panel_names(cx);
    assert_eq!(
        names.iter().filter(|n| *n == "ubiq.search").count(),
        1,
        "exactly one search panel, not a second one beside the restored entity: {names:?}"
    );
    assert!(
        fixture.state.read_with(cx, |state, cx| state
            .dock()
            .read(cx)
            .is_dock_open(gpui_component::dock::DockPlacement::Bottom)),
        "the bottom region is open"
    );
}

/// The bottom region already holds the console, and the console's tab is the one displayed.
/// Revealing search has to bring *its* tab to the front, not merely be somewhere in the group.
#[gpui::test]
fn revealing_search_beside_the_console_brings_its_own_tab_forward(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);

    fixture
        .window
        .update(cx, |_, window, cx| {
            fixture.state.update(cx, |state, cx| {
                state.reveal_console(window, cx);
            });
        })
        .expect("the window is open");
    cx.run_until_parked();
    fixture
        .window
        .update(cx, |_, window, cx| {
            fixture.state.update(cx, |state, cx| {
                state.reveal_search(window, cx);
            });
        })
        .expect("the window is open");
    cx.run_until_parked();

    let blob = fixture.state.read_with(cx, |state, cx| {
        serde_json::to_value(state.dock().read(cx).dump(cx)).expect("the dump serialises")
    });
    let bottom = &blob["bottom_dock"]["panel"];
    let kids: Vec<&str> = bottom["children"]
        .as_array()
        .expect("the bottom group has children")
        .iter()
        .map(|kid| kid["panel_name"].as_str().unwrap_or_default())
        .collect();
    let active = bottom["info"]["tabs"]["active_index"]
        .as_u64()
        .expect("the group names its displayed tab") as usize;
    assert_eq!(
        kids.get(active).copied(),
        Some("ubiq.search"),
        "search is the tab on display: {kids:?} active {active}"
    );
}

/// The gesture as the rail actually sees it: search revealed in IDE, the window sent through
/// another mode and back. Coming home must not lose the panel — a mode switch rebuilds the tree.
#[gpui::test]
fn search_comes_back_with_the_ide_mode(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);

    fixture
        .window
        .update(cx, |_, window, cx| {
            fixture
                .state
                .update(cx, |state, cx| state.reveal_search(window, cx));
        })
        .expect("the window is open");
    cx.run_until_parked();

    for mode in [ubiq::state::RailMode::Agents, ubiq::state::RailMode::Ide] {
        fixture
            .state
            .update(cx, |state, cx| state.set_rail_mode(mode, cx));
        cx.run_until_parked();
    }

    assert!(
        fixture.panel_names(cx).iter().any(|n| n == "ubiq.search"),
        "the IDE arrangement kept the search panel: {:?}",
        fixture.panel_names(cx)
    );
}

/// The reveal that lands before the project's saved arrangement does. The blob was written
/// before the panel existed, so it cannot name it — and a restore that drops it is the panel
/// vanishing a frame after the user asked for it.
#[gpui::test]
fn a_reveal_survives_an_arrangement_that_predates_it(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);

    // What the host has stored: the window as it looked before search was ever revealed.
    let stale = fixture.state.read_with(cx, |state, cx| {
        serde_json::to_value(state.dock().read(cx).dump(cx)).expect("the dump serialises")
    });
    let mut named = Vec::new();
    leaves(&stale, &mut named);
    assert!(
        !named.iter().any(|n| n == "ubiq.search"),
        "the stored arrangement predates the reveal: {named:?}"
    );

    fixture
        .window
        .update(cx, |_, window, cx| {
            fixture
                .state
                .update(cx, |state, cx| state.reveal_search(window, cx));
        })
        .expect("the window is open");
    cx.run_until_parked();

    restore_arrangement(&fixture, stale, cx);

    assert!(
        fixture.panel_names(cx).iter().any(|n| n == "ubiq.search"),
        "the reveal outlives the older arrangement: {:?}",
        fixture.panel_names(cx)
    );
    assert!(
        fixture.state.read_with(cx, |state, cx| state
            .dock()
            .read(cx)
            .is_dock_open(gpui_component::dock::DockPlacement::Bottom)),
        "the bottom region is open"
    );
}
