//! The new-pane control's chevron menu: what it offers, and what a row does.
//!
//! The list itself is the host's — which shells a machine has is a fact the interface may not read
//! — so what is Ubiq's here is the seam: that a window asks, that the answer becomes the menu's
//! rows, that a shell row spawns a pane running that shell, and that the row past the last shell
//! reveals the console instead of starting anything.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use chrono::Utc;
use gpui::{AppContext as _, Entity, TestAppContext, WindowHandle};
use gpui_component::Root;
use ubiq::app::{AppState, BusHub};
use ubiq::state::WindowRegistry;
use ubiq::state::dock::Region;
use ubiq::state::{NewPaneRow, WorkbenchState};
use ubiq_proto::bus::{self, FromClient, To};
use ubiq_proto::ids::ProjectId;
use ubiq_proto::messages::{AgentTypeInfo, Message, ShellInfo};
use ubiq_proto::projects::{ProjectHealth, ProjectRecord, ProjectSnapshot};

/// Long enough for a message to cross a channel in the same process.
const PATIENCE: Duration = Duration::from_millis(500);

/// A window on one project, with the host end kept so the test can both answer it and read what
/// the window said.
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

    /// Say what the host says as a window attaches, which is what makes the window ask for the
    /// shell list in the first place.
    fn attach(&self, cx: &mut TestAppContext) {
        self.host.send(
            To::Everyone,
            Message::HostInfo {
                config_root: "/tmp/ubiq-config".to_string(),
                is_default: true,
            },
        );
        cx.run_until_parked();
    }

    /// Answer the shell list, as the host's `shells::available()` would.
    fn answer_shells(&self, shells: Vec<ShellInfo>, cx: &mut TestAppContext) {
        self.host.send(To::Everyone, Message::ShellList { shells });
        cx.run_until_parked();
    }

    /// Answer the agent-type list, as the embedded harness library's own list would.
    fn answer_agent_types(&self, agent_types: Vec<AgentTypeInfo>, cx: &mut TestAppContext) {
        self.host
            .send(To::Everyone, Message::AgentTypes { agent_types });
        cx.run_until_parked();
    }

    /// Open the menu, and pick one of its rows — the two gestures, on the same indexing the menu
    /// is drawn with.
    fn pick(&self, index: usize, cx: &mut TestAppContext) {
        self.window
            .update(cx, |_, window, cx| {
                self.state.update(cx, |state, cx| {
                    state.open_new_pane_menu((12.0, 34.0), cx);
                    state.pick_new_pane_menu(index, window, cx);
                });
            })
            .expect("the window is open");
        cx.run_until_parked();
    }

    /// The arrangement as the dock serialises it, which is where a panel's presence is a fact
    /// rather than a pixel.
    fn arrangement(&self, cx: &mut TestAppContext) -> String {
        self.state.read_with(cx, |state, cx| {
            serde_json::to_string(&state.dock().read(cx).dump(cx)).expect("the dump serialises")
        })
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

fn a_shell(label: &str, program: &str, is_default: bool) -> ShellInfo {
    ShellInfo {
        label: label.to_string(),
        program: program.to_string(),
        is_default,
    }
}

fn an_agent(id: &str, label: &str, available: bool) -> AgentTypeInfo {
    AgentTypeInfo {
        id: id.to_string(),
        label: label.to_string(),
        available,
    }
}

fn a_project() -> ProjectSnapshot {
    ProjectSnapshot {
        record: ProjectRecord {
            id: ProjectId::generate(),
            name: "ubiq".to_string(),
            path: "/tmp/ubiq".to_string(),
            colour: 0,
            created_at: Utc::now(),
            last_opened_at: None,
        },
        health: ProjectHealth::Ok,
        open_panes: 0,
        workarea: "/tmp/ubiq-workarea".to_string(),
    }
}

/// `new_pane_rows` is pure state — no window needed to check the order it puts rows in.
#[test]
fn agent_rows_come_before_shells_with_a_separator_between() {
    let workbench = WorkbenchState {
        agent_types: vec![
            an_agent("claude-code", "Claude Code", true),
            an_agent("codex", "Codex", false),
        ],
        shells: vec![a_shell("zsh", "/bin/zsh", true)],
        ..Default::default()
    };

    let rows = workbench.new_pane_rows(true);
    assert_eq!(
        rows,
        vec![
            NewPaneRow::Agent(0),
            NewPaneRow::Agent(1),
            NewPaneRow::Separator,
            NewPaneRow::Shell(0),
            NewPaneRow::Separator,
            NewPaneRow::Console,
        ],
        "agents lead, then a separator, then the shells, then the console"
    );
}

/// No folder, no pane — the menu with no project open offers nothing to start, agents included.
#[test]
fn no_rows_are_offered_without_a_project() {
    let workbench = WorkbenchState {
        agent_types: vec![an_agent("claude-code", "Claude Code", true)],
        shells: vec![a_shell("zsh", "/bin/zsh", true)],
        ..Default::default()
    };

    assert_eq!(
        workbench.new_pane_rows(false),
        vec![NewPaneRow::Console],
        "a window with no project was offered more than the console"
    );
}

/// A machine with no agent harnesses installed sees exactly the menu it saw before agents existed.
#[test]
fn an_empty_agent_list_degrades_to_todays_menu() {
    let workbench = WorkbenchState {
        shells: vec![a_shell("zsh", "/bin/zsh", true)],
        ..Default::default()
    };

    assert_eq!(
        workbench.new_pane_rows(true),
        vec![
            NewPaneRow::Shell(0),
            NewPaneRow::Separator,
            NewPaneRow::Console
        ],
        "an empty agent list left a stray separator or row ahead of the shells"
    );
}

#[gpui::test]
fn a_window_asks_the_host_what_can_be_started_here(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    fixture.attach(cx);

    let said = fixture.said();
    assert!(
        said.iter()
            .any(|message| matches!(message, Message::ListShells)),
        "the window never asked for the shell list"
    );
    assert!(
        said.iter()
            .any(|message| matches!(message, Message::ListAgentTypes)),
        "the window never asked for the agent-type list"
    );
}

#[gpui::test]
fn the_hosts_answer_is_what_the_menu_offers(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    fixture.answer_shells(
        vec![
            a_shell("zsh", "/bin/zsh", true),
            a_shell("fish", "/opt/homebrew/bin/fish", false),
        ],
        cx,
    );

    let (labels, default) = fixture.state.read_with(cx, |state, _| {
        let shells = &state.workbench.shells;
        (
            shells.iter().map(|s| s.label.clone()).collect::<Vec<_>>(),
            shells.iter().position(|s| s.is_default),
        )
    });
    assert_eq!(labels, vec!["zsh".to_string(), "fish".to_string()]);
    assert_eq!(default, Some(0), "the default keeps the host's own place");

    // A list that arrives again replaces the one before it: a shell that has been uninstalled has
    // to leave the menu.
    fixture.answer_shells(vec![a_shell("sh", "/bin/sh", true)], cx);
    let labels = fixture.state.read_with(cx, |state, _| {
        state
            .workbench
            .shells
            .iter()
            .map(|s| s.label.clone())
            .collect::<Vec<_>>()
    });
    assert_eq!(labels, vec!["sh".to_string()]);
}

#[gpui::test]
fn picking_a_shell_starts_a_pane_running_it(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    fixture.answer_shells(
        vec![
            a_shell("zsh", "/bin/zsh", true),
            a_shell("fish", "/opt/homebrew/bin/fish", false),
        ],
        cx,
    );
    let _ = fixture.said();

    fixture.pick(1, cx);

    let spawned = fixture
        .said()
        .into_iter()
        .find_map(|message| match message {
            Message::SpawnWorkspace {
                agent_type, args, ..
            } => Some((agent_type, args)),
            _ => None,
        })
        .expect("picking a shell asks for a pane");
    assert_eq!(spawned.0, Some("/opt/homebrew/bin/fish".to_string()));
    assert!(spawned.1.is_empty(), "a shell is started with no arguments");
    assert_eq!(
        fixture
            .state
            .read_with(cx, |state, _| state.workbench.open_menu),
        None,
        "a pick closes the menu"
    );
}

#[gpui::test]
fn picking_an_agent_starts_a_pane_running_it(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    fixture.answer_agent_types(
        vec![
            an_agent("claude-code", "Claude Code", true),
            an_agent("codex", "Codex", true),
        ],
        cx,
    );
    let _ = fixture.said();

    fixture.pick(1, cx);

    let spawned = fixture
        .said()
        .into_iter()
        .find_map(|message| match message {
            Message::SpawnWorkspace {
                agent_type, args, ..
            } => Some((agent_type, args)),
            _ => None,
        })
        .expect("picking an agent asks for a pane");
    assert_eq!(
        spawned.0,
        Some("codex".to_string()),
        "the harness's id is what a spawn asks for, never its label"
    );
    assert!(
        spawned.1.is_empty(),
        "an agent is started with no arguments"
    );
    assert_eq!(
        fixture
            .state
            .read_with(cx, |state, _| state.workbench.open_menu),
        None,
        "a pick closes the menu"
    );
}

#[gpui::test]
fn picking_an_unavailable_agent_does_nothing(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    fixture.answer_agent_types(vec![an_agent("codex", "Codex", false)], cx);
    let _ = fixture.said();

    fixture.pick(0, cx);

    assert!(
        !fixture
            .said()
            .iter()
            .any(|message| matches!(message, Message::SpawnWorkspace { .. })),
        "an unavailable harness was started anyway"
    );
}

#[gpui::test]
fn the_last_row_reveals_the_console_rather_than_starting_anything(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    fixture.answer_shells(vec![a_shell("zsh", "/bin/zsh", true)], cx);

    // The pane region starts put away and empty, which is the state the row exists for.
    assert!(
        !fixture
            .state
            .read_with(cx, |state, cx| state.regions_open(cx).1),
        "the bottom region was not put away"
    );
    let _ = fixture.said();

    // Index 1 is the separator between the shells and the console: a row, and no action.
    fixture.pick(1, cx);
    assert!(
        !fixture
            .state
            .read_with(cx, |state, cx| state.regions_open(cx).1),
        "the separator did something"
    );

    fixture.pick(2, cx);

    assert!(
        fixture
            .state
            .read_with(cx, |state, cx| state.regions_open(cx).1),
        "the console row left the bottom region closed"
    );
    assert!(
        !fixture
            .said()
            .iter()
            .any(|message| matches!(message, Message::SpawnWorkspace { .. })),
        "the console row started a pane"
    );
}

#[gpui::test]
fn a_fresh_window_opens_with_an_empty_pane_region_and_no_console(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);

    assert!(
        !fixture.arrangement(cx).contains("ubiq.logs"),
        "the console is in a fresh window's arrangement"
    );
    assert!(
        !fixture
            .state
            .read_with(cx, |state, cx| state.regions_open(cx).1),
        "the pane region opens on a window with nothing in it"
    );
}

#[gpui::test]
fn opening_the_empty_pane_region_starts_a_pane(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let _ = fixture.said();

    fixture
        .window
        .update(cx, |_, window, cx| {
            fixture.state.update(cx, |state, cx| {
                state.toggle_region(Region::Bottom, window, cx);
            });
        })
        .expect("the window is open");
    cx.run_until_parked();

    let spawned = fixture
        .said()
        .into_iter()
        .find_map(|message| match message {
            Message::SpawnWorkspace {
                project_id,
                agent_type,
                ..
            } => Some((project_id, agent_type)),
            _ => None,
        })
        .expect("opening the region asks for a pane");
    assert_eq!(spawned.0, fixture.project);
    assert_eq!(
        spawned.1, None,
        "the switch starts the platform's default shell, like a bare click on +"
    );
}

#[gpui::test]
fn the_console_row_puts_the_console_in_the_arrangement(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    // With no shell list answered yet the console is the only row there is.
    fixture.pick(0, cx);

    assert!(
        fixture.arrangement(cx).contains("ubiq.logs"),
        "the console row left it out of the arrangement"
    );
    assert!(
        fixture
            .state
            .read_with(cx, |state, cx| state.regions_open(cx).1),
        "the console arrived in a region nobody can see"
    );
}

#[gpui::test]
fn the_pane_regions_own_groups_are_what_the_control_is_drawn_on(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);

    // The skin is handed a group and knows nothing about placement, so this is the answer that
    // keeps the control on the strip of a pane region holding nothing.
    let (bottom, centre) = fixture.state.read_with(cx, |state, cx| {
        let dock = state.dock().read(cx);
        let node = |region| {
            dock.layout(ubiq::ui::dock::placement_of(region))
                .map(|tree| tree.root().id())
        };
        (node(Region::Bottom), node(Region::Centre))
    });
    let bottom = bottom.expect("the pane region is installed, empty");
    let centre = centre.expect("the centre is always there");

    assert!(
        fixture
            .state
            .read_with(cx, |state, cx| state.is_pane_region(bottom, cx))
    );
    assert!(
        !fixture
            .state
            .read_with(cx, |state, cx| state.is_pane_region(centre, cx)),
        "the centre is not where a pane region's control belongs"
    );
}

/// A pane's tab, which is the one part of it that is a rule rather than a redraw. Asserted without
/// a window: building a real pane starts its emulator's reader thread, which the test scheduler
/// refuses, and the naming does not need one.
#[test]
fn a_panes_tab_is_its_program_and_a_number() {
    let mut taken: Vec<String> = Vec::new();
    for expected in ["zsh 1", "zsh 2", "zsh 3"] {
        let title = ubiq::app::pane_title("/bin/zsh", &taken);
        assert_eq!(title, expected);
        taken.push(title);
    }

    // Each program is numbered in its own sequence, and the path it was started by is not the name.
    assert_eq!(
        ubiq::app::pane_title("/opt/homebrew/bin/fish", &taken),
        "fish 1"
    );

    // A number goes back in the pool when its pane closes, rather than counting upwards for ever.
    taken.retain(|title| title != "zsh 2");
    assert_eq!(ubiq::app::pane_title("/bin/zsh", &taken), "zsh 2");

    // A program with no path, and one with a trailing name only, are named the same way.
    assert_eq!(ubiq::app::pane_title("bash", &[]), "bash 1");
}
