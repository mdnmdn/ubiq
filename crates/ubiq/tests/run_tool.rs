//! The titlebar's run control — the play triangle and the chevron beside it — and the Restart a
//! stopped tool pane's tab offers.
//!
//! What is Ubiq's here is the seam, the same one `new_pane.rs` covers for the shells: that the
//! rows are the host's answer, that a pick sends `RunTool`, that starting one brings the pane
//! region on screen — moving the window to the IDE when the mode it is in has none — and that a
//! stopped tool pane can be run again with the two values the first run was addressed with.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use chrono::Utc;
use gpui::{AppContext as _, Entity, TestAppContext, WindowHandle};
use gpui_component::Root;
use ubiq::app::{AppState, BusHub};
use ubiq::state::dock::PanelKind;
use ubiq::state::{RailMode, WindowRegistry, WorkbenchState};
use ubiq_proto::bus::{self, FromClient, To};
use ubiq_proto::ids::{PaneId, ProjectId, ToolId};
use ubiq_proto::messages::Message;
use ubiq_proto::projects::{ProjectHealth, ProjectRecord, ProjectSnapshot, Scope};
use ubiq_proto::tools::{ListedTool, ToolDef};

/// Long enough for a message to cross a channel in the same process.
const PATIENCE: Duration = Duration::from_millis(500);

struct Fixture {
    state: Entity<AppState>,
    window: WindowHandle<Root>,
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
        }
    }

    /// Answer the tool list, as the host's own `ListTools` reply would.
    fn answer_tools(&self, tools: Vec<ListedTool>, cx: &mut TestAppContext) {
        self.host.send(
            To::Everyone,
            Message::ToolsListed {
                system: tools,
                project: Vec::new(),
            },
        );
        cx.run_until_parked();
    }

    /// Open the run menu and pick one of its rows — the two gestures, on the menu's own indexing.
    fn pick(&self, index: usize, cx: &mut TestAppContext) {
        self.window
            .update(cx, |_, window, cx| {
                self.state.update(cx, |state, cx| {
                    state.open_run_tool_menu((12.0, 34.0), cx);
                    state.pick_run_tool_menu(index, window, cx);
                });
            })
            .expect("the window is open");
        cx.run_until_parked();
    }

    fn bottom_open(&self, cx: &mut TestAppContext) -> bool {
        self.state
            .read_with(cx, |state, cx| state.regions_open(cx).1)
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

fn a_tool(name: &str, applicable: bool) -> ListedTool {
    ListedTool {
        scope: Scope::Interface,
        tool: ToolDef {
            id: ToolId::generate(),
            name: name.to_string(),
            command: "cargo".to_string(),
            args: "build".to_string(),
            env: Default::default(),
            platforms: Vec::new(),
            wait_on_exit: true,
            wait_on_error: false,
            single_instance: false,
            starting_folder: None,
        },
        applicable,
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
        },
        health: ProjectHealth::Ok,
        open_panes: 0,
        ephemeral: false,
        workarea: "/tmp/ubiq-workarea".to_string(),
    }
}

/// The rows are the applicable tools and nothing else — a macOS-only row on Windows is not
/// offered, and the host is what stamped that.
#[test]
fn the_menu_offers_the_applicable_tools_only() {
    let workbench = WorkbenchState {
        tools: vec![
            a_tool("Build", true),
            a_tool("Notarise", false),
            a_tool("Watch", true),
        ],
        ..Default::default()
    };

    assert_eq!(workbench.run_tool_rows(), vec![0, 2]);
}

/// Control and the kitchen sink are not views onto a project's folder, so a pane started from
/// either has nowhere to be seen — and starting one moves the window to the IDE.
#[test]
fn only_the_project_modes_hold_a_pane_region() {
    for mode in RailMode::every() {
        assert_eq!(
            mode.has_pane_region(),
            !matches!(mode, RailMode::Control | RailMode::Sink),
            "{mode:?} answered the wrong thing about its pane region"
        );
    }
}

/// Picking a row asks the host to run that tool, addressed by the scope and the id the host's own
/// listing carried.
#[gpui::test]
fn picking_a_tool_asks_the_host_to_run_it(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let tools = vec![a_tool("Build", true), a_tool("Watch", true)];
    let wanted = tools[1].tool.id;
    fixture.answer_tools(tools, cx);
    let _ = fixture.said();

    fixture.pick(1, cx);

    let run = fixture
        .said()
        .into_iter()
        .find_map(|message| match message {
            Message::RunTool { scope, id, .. } => Some((scope, id)),
            _ => None,
        })
        .expect("picking a tool asks for a run");
    assert_eq!(run, (Scope::Interface, wanted));
    assert_eq!(
        fixture
            .state
            .read_with(cx, |state, _| state.workbench.open_menu),
        None,
        "a pick closes the menu"
    );
}

/// The play triangle runs the first tool the project offers, without opening anything.
#[gpui::test]
fn the_play_control_runs_the_first_tool(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let tools = vec![a_tool("Build", true), a_tool("Watch", true)];
    let first = tools[0].tool.id;
    fixture.answer_tools(tools, cx);
    let _ = fixture.said();

    fixture
        .window
        .update(cx, |_, _window, cx| {
            fixture
                .state
                .update(cx, |state, cx| state.run_first_tool(cx));
        })
        .expect("the window is open");
    cx.run_until_parked();

    let run = fixture
        .said()
        .into_iter()
        .find_map(|message| match message {
            Message::RunTool { id, .. } => Some(id),
            _ => None,
        })
        .expect("the play control asks for a run");
    assert_eq!(run, first);
}

/// A project with no tool has nothing for the play triangle to run, and it asks for nothing
/// rather than starting whatever happens to be first in an empty list.
#[gpui::test]
fn the_play_control_runs_nothing_when_there_is_nothing(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let _ = fixture.said();

    fixture
        .window
        .update(cx, |_, _window, cx| {
            fixture
                .state
                .update(cx, |state, cx| state.run_first_tool(cx));
        })
        .expect("the window is open");
    cx.run_until_parked();

    assert!(
        !fixture
            .said()
            .iter()
            .any(|message| matches!(message, Message::RunTool { .. })),
        "a run was asked for with no tool to run"
    );
}

/// Starting a runner brings the pane region on screen. No region opens by default (`D94`), so the
/// bottom is shut until something asks for it — and a run is exactly that ask.
#[gpui::test]
fn starting_a_runner_opens_the_bottom_region(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    fixture.answer_tools(vec![a_tool("Build", true)], cx);
    assert!(!fixture.bottom_open(cx), "the bottom starts shut");

    fixture.pick(0, cx);

    assert!(
        fixture.bottom_open(cx),
        "starting a runner left the pane region put away"
    );
}

/// And from a mode with no pane region at all, it moves the window to the IDE first.
#[gpui::test]
fn starting_a_runner_from_control_moves_to_the_ide(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    fixture.answer_tools(vec![a_tool("Build", true)], cx);
    fixture.state.update(cx, |state, cx| {
        state.set_rail_mode(RailMode::Control, cx);
    });
    cx.run_until_parked();

    fixture.pick(0, cx);

    assert_eq!(
        fixture
            .state
            .read_with(cx, |state, _| state.workbench.rail_mode),
        RailMode::Ide,
        "a run from Control left the window where a pane cannot be seen"
    );
    assert!(fixture.bottom_open(cx), "the pane region is on screen too");
}

/// The Restart row: offered on a stopped tool pane, and on nothing else.
///
/// A pure check on `ui::tab_menu::rows`, which is the list both the frame that draws the menu and
/// `AppState::pick_tab_menu` read — a row that shifted here would be a row picked by the wrong
/// index there. Whether one pane is restartable is `AppState::tab_restartable`, and a window that
/// really holds a pane needs a terminal emulator on a thread of its own, which the test scheduler
/// refuses; so the two halves are checked apart.
#[test]
fn restart_is_a_row_only_when_there_is_something_to_restart() {
    let terminal = PanelKind::Terminal(PaneId::generate());

    assert_eq!(
        ubiq::ui::tab_menu::rows(&terminal, false, true),
        vec!["Rename\u{2026}", "Restart", "Hide", "Close", "Pin"],
        "Restart sits above the two endings, because it is the opposite of them"
    );
    assert_eq!(
        ubiq::ui::tab_menu::rows(&terminal, false, false),
        vec!["Rename\u{2026}", "Hide", "Close", "Pin"],
        "a pane with nothing to restart was offered the row anyway"
    );
    // Pinning takes Hide away and leaves Restart, which is about the process rather than the tab.
    assert_eq!(
        ubiq::ui::tab_menu::rows(&terminal, true, true),
        vec!["Rename\u{2026}", "Restart", "Close", "Unpin"],
    );
}
