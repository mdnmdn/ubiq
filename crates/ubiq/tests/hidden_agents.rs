//! The hidden-agents chevron (`T-266`, `T-275`): what it lists when a live agent is shown by no
//! surface the *current rail mode* draws.
//!
//! `ui::menus::hidden_agents` used to be read off `AppState::offscreen_panes`, the **pane**
//! model — right for a harness running in a real pseudo-terminal pane, wrong for an agent an
//! agents-column tab or a chat tab draws, because neither ever puts a panel over a pane for it.
//! The bug was the menu saying "No hidden agents" while such an agent sat live and invisible. The
//! fix reads the **agent** model instead — `work.agents` narrowed to `AgentsView::live`.
//!
//! **"Shown" is mode-relative, not global.** The columns are the centre only in `RailMode::AGENTS`
//! (`ui/rail.rs`), so a live agent sitting in one is shown while the window is on that screen and
//! hidden — "to attach" — everywhere else, IDE mode included. A chat tab's own attachment is drawn
//! in every mode except Agents, Control and the sink (`state::dock::PanelKind::Chat`), so it counts
//! as shown there and nowhere else. Hidden is live minus whichever of those two the current mode
//! actually draws.

use std::cell::RefCell;
use std::rc::Rc;

use chrono::Utc;
use gpui::{AppContext as _, Entity, TestAppContext, WindowHandle};
use gpui_component::Root;
use ubiq::app::{AppState, BusHub};
use ubiq::ext::ids;
use ubiq::state::{RailMode, WindowRegistry};
use ubiq::ui::menus;
use ubiq_proto::bus::{self, To};
use ubiq_proto::ids::ProjectId;
use ubiq_proto::messages::Message;
use ubiq_proto::projects::{ProjectHealth, ProjectRecord, ProjectSnapshot};
use ubiq_proto::work::{Activity, AgentId, WorkAgent, WorkSession};

struct Fixture {
    state: Entity<AppState>,
    _window: WindowHandle<Root>,
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
            _window: window,
            host,
            project,
        }
    }

    /// The host says an agent is up, exactly as `StartConversation` is answered. With no pending
    /// attach in flight this is also what reveals it — through `AppState::reveal_agent_for_mode`,
    /// the `ConversationStarted` arm's `(None, false, false)` case — onto whatever surface the
    /// window's current rail mode can actually show: the agents columns in `RailMode::AGENTS`, an
    /// attached chat panel in the dock everywhere else. See `chat.rs`'s own tests for the
    /// chat-panel half.
    fn started(&self, agent: WorkAgent, cx: &mut TestAppContext) {
        let session = WorkSession {
            id: agent.session,
            name: "the project".to_string(),
            branch: String::new(),
            worktree: false,
        };
        self.host.send(
            To::Everyone,
            Message::ConversationStarted {
                project_id: self.project,
                agent: Box::new(agent),
                session,
                accepts_input: true,
            },
        );
        cx.run_until_parked();
    }

    /// The menu's rows, read off the one list the overlay draws and a pick resolves against.
    fn rows(&self, cx: &mut TestAppContext) -> Vec<String> {
        self.state.read_with(cx, |state, cx| {
            menus::entries(ids::MENU_HIDDEN_AGENTS, state, cx)
                .iter()
                .map(|entry| entry.label.to_string())
                .collect()
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
            definitions_use_global: true,
            definitions_allowed: Vec::new(),
        },
        health: ProjectHealth::Ok,
        open_panes: 0,
        ephemeral: false,
        workarea: "/tmp/ubiq-workarea".to_string(),
    }
}

fn an_agent(id: AgentId, name: &str) -> WorkAgent {
    WorkAgent {
        id,
        session: ubiq_proto::ids::SessionId::generate(),
        task: None,
        parent: None,
        name: name.to_string(),
        summary: None,
        role: "Implementer".to_string(),
        activity: Activity::Thinking,
        note: String::new(),
        branch: "main".to_string(),
        tokens: 0.0,
        harness: "Claude Code".to_string(),
        account: "work".to_string(),
        model: String::new(),
        context_pct: 0,
        persistent: false,
        accept_all: false,
        debug_dump: None,
        run_dir: None,
        config_dir: None,
        thread: Vec::new(),
    }
}

/// A window that has never seen an agent says so plainly, the `NO_HIDDEN_AGENTS_ROW` case.
#[gpui::test]
fn nothing_running_says_no_hidden_agents(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    assert_eq!(
        fixture.rows(cx),
        vec![menus::NO_HIDDEN_AGENTS_ROW.to_string()]
    );
}

/// A live agent shown in no column and no chat tab is exactly what this menu exists for — the bug
/// under `T-275` was this case reading as "No hidden agents". Picking its row reveals it.
///
/// Run in Agents mode, so a reveal — which only ever changes column membership — lands somewhere
/// the current mode actually draws; see `a_column_only_agent_is_hidden_outside_agents_mode` for
/// the mode-relative half of the rule.
#[gpui::test]
fn a_live_agent_shown_nowhere_is_listed_and_picking_it_reveals_it(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    fixture.state.update(cx, |state, cx| {
        state.set_rail_mode(RailMode::AGENTS, cx);
    });
    let id = AgentId::generate();
    fixture.started(an_agent(id, "Fix the parser"), cx);

    // Freshly started, it was revealed onto the field — bench it, the same thing a `Hide` on an
    // agents-column tab does, to land in the state the bug report is about: live, and drawn by no
    // surface at all.
    fixture.state.update(cx, |state, cx| {
        state.bench_agent(id, cx);
    });
    cx.run_until_parked();

    assert!(
        !fixture
            .state
            .read_with(cx, |state, cx| state.agents(cx).unwrap().on_screen(id)),
        "benching should have taken the agent off every column"
    );

    let expected_label = fixture
        .state
        .read_with(cx, |state, cx| {
            state
                .work(cx)
                .unwrap()
                .agents
                .iter()
                .find(|a| a.id == id)
                .map(|a| state.agent_title(a))
        })
        .expect("the agent is in the projection")
        .to_string();

    assert_eq!(
        fixture.rows(cx),
        vec![expected_label],
        "a live agent shown nowhere must be the only row"
    );

    fixture
        ._window
        .update(cx, |_, window, cx| {
            fixture.state.update(cx, |state, cx| {
                state.open_hidden_agents_menu((0.0, 0.0), cx);
                state.pick_hidden_agents_menu(0, window, cx);
            });
        })
        .expect("the window is open");
    cx.run_until_parked();

    assert!(
        fixture
            .state
            .read_with(cx, |state, cx| state.agents(cx).unwrap().on_screen(id)),
        "picking the row should have revealed the agent back onto a column"
    );
    assert_eq!(
        fixture.rows(cx),
        vec![menus::NO_HIDDEN_AGENTS_ROW.to_string()],
        "the agent just revealed should no longer be hidden"
    );
}

/// The counterpart: an agent revealed onto the agents screen — the default a fresh start lands
/// in — is visible **while Agents mode is the one on screen**, so the menu leaves it out there.
#[gpui::test]
fn a_displayed_agent_is_not_listed_in_agents_mode(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    fixture.state.update(cx, |state, cx| {
        state.set_rail_mode(RailMode::AGENTS, cx);
    });
    let id = AgentId::generate();
    fixture.started(an_agent(id, "Ship the release"), cx);

    assert!(
        fixture
            .state
            .read_with(cx, |state, cx| state.agents(cx).unwrap().on_screen(id)),
        "a fresh start should have landed the agent on a column"
    );

    assert_eq!(
        fixture.rows(cx),
        vec![menus::NO_HIDDEN_AGENTS_ROW.to_string()],
        "an agent a column already shows, in the mode that draws columns, must not also be in \
         the hidden-agents menu"
    );
}

/// The mode-relative half of the rule (`T-266`/`T-275` follow-up): the agents columns are the
/// centre only in `RailMode::AGENTS` (`ui/rail.rs`). An agent that sits in a column and nothing
/// else is shown while Agents mode is the one on screen, and hidden — "to attach" — the moment
/// the window leaves it: leaving a mode does not itself change column membership, only whether
/// anything draws it. (A *fresh* start no longer lands on a column outside Agents mode at all —
/// see `chat.rs`'s own tests for what `reveal_agent_for_mode` does with one instead — but an
/// agent already benched onto a column this way stays there when the mode changes under it, the
/// same as any other view.)
#[gpui::test]
fn a_column_only_agent_is_hidden_outside_agents_mode(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    fixture.state.update(cx, |state, cx| {
        state.set_rail_mode(RailMode::AGENTS, cx);
    });

    let id = AgentId::generate();
    fixture.started(an_agent(id, "Ship the release"), cx);

    assert!(
        fixture
            .state
            .read_with(cx, |state, cx| state.agents(cx).unwrap().on_screen(id)),
        "started in Agents mode, the agent lands on a column"
    );

    fixture.state.update(cx, |state, cx| {
        state.set_rail_mode(RailMode::IDE, cx);
    });
    cx.run_until_parked();

    assert!(
        fixture
            .state
            .read_with(cx, |state, cx| state.agents(cx).unwrap().on_screen(id)),
        "leaving Agents mode does not itself change column membership"
    );

    let expected_label = fixture
        .state
        .read_with(cx, |state, cx| {
            state
                .work(cx)
                .unwrap()
                .agents
                .iter()
                .find(|a| a.id == id)
                .map(|a| state.agent_title(a))
        })
        .expect("the agent is in the projection")
        .to_string();

    assert_eq!(
        fixture.rows(cx),
        vec![expected_label],
        "IDE mode draws no columns, so a column-only agent reads as hidden the moment the window \
         leaves Agents mode"
    );
}

/// The other surface, read the same way: a chat tab's attachment is drawn in IDE mode (every mode
/// except Agents, Control and the sink — `state::dock::PanelKind::Chat`), so an agent attached
/// there stays shown even though its column membership is invisible in this mode.
#[gpui::test]
fn a_chat_attached_agent_stays_shown_in_ide_mode(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let id = AgentId::generate();
    fixture.started(an_agent(id, "Ship the release"), cx);

    let tab = fixture
        .state
        .read_with(cx, |state, cx| state.open_project(cx).unwrap().chats[0].id);
    fixture
        .state
        .update(cx, |state, cx| state.attach_chat(tab, Some(id), cx));
    cx.run_until_parked();

    assert_eq!(
        fixture.rows(cx),
        vec![menus::NO_HIDDEN_AGENTS_ROW.to_string()],
        "a chat tab attached to it draws in IDE mode, so the agent is shown there"
    );
}
