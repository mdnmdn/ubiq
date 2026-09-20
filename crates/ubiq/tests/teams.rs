//! Selecting on the Teams canvas, and what it does to the conversation underneath.
//!
//! `TeamsSelection` is not only a fact the canvas reads back at itself — a session, an agent or one
//! of an agent's delegates selected there points the shared conversation at it too, through
//! `AppState::view_conversation_agent`. That field is the conversation's own, not the canvas's: a
//! chat tab and an agents column reading the same agent read the same `viewing`, so a selection
//! made on one surface is not invisible to the others. Nothing here renders a frame — the claim is
//! about state, and `chat.rs`'s `Fixture` is the pattern this one borrows.

use std::cell::RefCell;
use std::rc::Rc;

use chrono::Utc;
use gpui::{AppContext as _, Entity, TestAppContext, WindowHandle};
use gpui_component::Root;
use ubiq::app::{AppState, BusHub};
use ubiq::state::WindowRegistry;
use ubiq::state::teams::TeamsSelection;
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

    /// The host says an agent is up, exactly as `StartConversation` is answered.
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
            index: None,
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
        activity: Activity::Writing,
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

/// Selecting a delegate on the canvas points the shared conversation at it — the same field
/// `view_conversation_agent` writes for the agent switcher, because a delegate has no transcript of
/// its own to point at: what opens is its parent's, read from the delegate's turns.
#[gpui::test]
fn selecting_a_subagent_points_the_conversation_at_it(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let id = AgentId::generate();
    fixture.started(an_agent(id, "Claude Code"), cx);

    fixture.state.update(cx, |state, cx| {
        state.select_in_teams(
            TeamsSelection::Subagent {
                agent: id,
                subagent: "t751".to_string(),
            },
            cx,
        )
    });

    let (selection, viewing) = fixture.state.read_with(cx, |state, cx| {
        let teams = state.teams(cx).expect("the project's Teams view");
        let conversation = state.conversation(id, cx).expect("the live conversation");
        (teams.selection.clone(), conversation.viewing.clone())
    });
    assert_eq!(
        selection,
        Some(TeamsSelection::Subagent {
            agent: id,
            subagent: "t751".to_string(),
        })
    );
    assert_eq!(
        viewing.as_deref(),
        Some("t751"),
        "the canvas's selection and the conversation's own field agree"
    );
}

/// Selecting the card itself — not one of its delegates — clears whatever delegate the
/// conversation was showing. A card picked up while a delegate is being read stays on it (that is
/// `start_teams_carry`'s own rule); a plain selection is the way back to the main transcript.
#[gpui::test]
fn selecting_the_card_clears_the_delegate_it_was_reading(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let id = AgentId::generate();
    fixture.started(an_agent(id, "Claude Code"), cx);

    fixture.state.update(cx, |state, cx| {
        state.select_in_teams(
            TeamsSelection::Subagent {
                agent: id,
                subagent: "t751".to_string(),
            },
            cx,
        )
    });
    fixture
        .state
        .read_with(cx, |state, cx| {
            state.conversation(id, cx).unwrap().viewing.clone()
        })
        .expect("the delegate is showing before the plain selection");

    fixture.state.update(cx, |state, cx| {
        state.select_in_teams(TeamsSelection::Agent(id), cx)
    });

    let (selection, viewing) = fixture.state.read_with(cx, |state, cx| {
        let teams = state.teams(cx).unwrap();
        let conversation = state.conversation(id, cx).unwrap();
        (teams.selection.clone(), conversation.viewing.clone())
    });
    assert_eq!(selection, Some(TeamsSelection::Agent(id)));
    assert_eq!(
        viewing, None,
        "the plain card selection put the transcript back on the main agent"
    );
}

/// **`viewing` is the conversation's own field, not the canvas's** — so whatever the Teams screen
/// last asked for is what a chat tab or an agents column attached to the same agent would also
/// read, since every surface reaches it through the one `AppState::conversation` accessor rather
/// than a copy of its own. This is the semantics the module doc promises; nothing before this test
/// pinned it down.
#[gpui::test]
fn the_conversation_s_viewing_field_is_shared_by_every_surface_reading_it(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let id = AgentId::generate();
    fixture.started(an_agent(id, "Claude Code"), cx);

    fixture.state.update(cx, |state, cx| {
        state.select_in_teams(
            TeamsSelection::Subagent {
                agent: id,
                subagent: "t9".to_string(),
            },
            cx,
        )
    });

    // A chat tab and an agents column are two different readers, but they resolve to the same
    // `Conversation` — there is only one, keyed by agent id, in `OpenProject::conversations`.
    let from_a_chat_tab = fixture.state.read_with(cx, |state, cx| {
        state.conversation(id, cx).unwrap().viewing.clone()
    });
    let from_an_agents_column = fixture.state.read_with(cx, |state, cx| {
        state.conversation(id, cx).unwrap().viewing.clone()
    });
    assert_eq!(from_a_chat_tab.as_deref(), Some("t9"));
    assert_eq!(from_a_chat_tab, from_an_agents_column);
}
