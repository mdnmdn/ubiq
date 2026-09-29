//! The hidden-agents chevron (`T-266`, `T-275`): what it lists when a live agent is shown by no
//! surface at all.
//!
//! `ui::menus::hidden_agents` used to be read off `AppState::offscreen_panes`, the **pane**
//! model — right for a harness running in a real pseudo-terminal pane, wrong for an agent an
//! agents-column tab or a chat tab draws, because neither ever puts a panel over a pane for it.
//! The bug was the menu saying "No hidden agents" while such an agent sat live and invisible. The
//! fix reads the **agent** model instead — `work.agents` narrowed to `AgentsView::live`, hidden
//! being live minus what every agents-column tab and every chat tab's `attached` shows.

use std::cell::RefCell;
use std::rc::Rc;

use chrono::Utc;
use gpui::{AppContext as _, Entity, TestAppContext, WindowHandle};
use gpui_component::Root;
use ubiq::app::{AppState, BusHub};
use ubiq::ext::ids;
use ubiq::state::WindowRegistry;
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
    /// attach in flight this is also what reveals the agent onto the agents screen's field — see
    /// `receive_conversation`'s `(None, false, false)` arm — so a fresh agent starts out shown,
    /// not hidden.
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
#[gpui::test]
fn a_live_agent_shown_nowhere_is_listed_and_picking_it_reveals_it(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
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
/// in — is visible, so the menu leaves it out.
#[gpui::test]
fn a_displayed_agent_is_not_listed(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
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
        "an agent a column already shows must not also be in the hidden-agents menu"
    );
}
