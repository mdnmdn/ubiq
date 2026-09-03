//! A live agent's stream, from the bus to the record the rest of the window reads.
//!
//! The claim under test is the one that makes the conversation family worth having: what the
//! window already holds is what it draws. An update arrives, the projection folds it in, and the
//! agent's record — its badge, its ring, its token count, its model — is refreshed from that fold
//! rather than from a second question to the host. A round trip per token would be a round trip
//! per token.
//!
//! The other half is the seam the user reaches: picking a harness asks the host to start a
//! conversation, and the composer prompts the agent it is pointed at without writing a line of
//! its own — the line is drawn when the harness echoes it back.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use chrono::Utc;
use gpui::{AppContext as _, Entity, TestAppContext, WindowHandle};
use gpui_component::Root;
use ubiq::app::{AppState, BusHub};
use ubiq::state::WindowRegistry;
use ubiq::state::conversation::Run;
use ubiq_proto::bus::{self, FromClient, To};
use ubiq_proto::conversation::{ConvContent, ConvUpdate, StopReason, UsageRecord};
use ubiq_proto::ids::{ProjectId, SessionId};
use ubiq_proto::messages::{AgentTypeInfo, Message};
use ubiq_proto::projects::{ProjectHealth, ProjectRecord, ProjectSnapshot};
use ubiq_proto::work::{Activity, AgentId, WorkAgent, WorkSession};

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

    /// The host says an agent is up, exactly as `StartConversation` is answered.
    fn started(&self, agent: WorkAgent, cx: &mut TestAppContext) {
        self.started_with_input(agent, true, cx);
    }

    /// The same, saying whether the harness takes a second turn.
    fn started_with_input(&self, agent: WorkAgent, accepts_input: bool, cx: &mut TestAppContext) {
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
                accepts_input,
            },
        );
        cx.run_until_parked();
    }

    fn update(&self, agent_id: AgentId, seq: u64, update: ConvUpdate, cx: &mut TestAppContext) {
        self.host.send(
            To::Everyone,
            Message::ConversationUpdate {
                agent_id,
                seq,
                update: Box::new(update),
            },
        );
        cx.run_until_parked();
    }

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
            created_at: Utc::now(),
            last_opened_at: None,
        },
        health: ProjectHealth::Ok,
        open_panes: 0,
        workarea: "/tmp/ubiq-workarea".to_string(),
    }
}

/// The record the host mints for a live agent: everything a mock has, and nothing said yet.
fn an_agent(id: AgentId) -> WorkAgent {
    WorkAgent {
        id,
        session: SessionId::generate(),
        task: None,
        parent: None,
        name: "Claude Code".to_string(),
        role: "Implementer".to_string(),
        activity: Activity::Ended,
        note: String::new(),
        branch: "main".to_string(),
        tokens: 0.0,
        harness: "Claude Code".to_string(),
        account: "work".to_string(),
        model: String::new(),
        context_pct: 0,
        thread: Vec::new(),
    }
}

fn chunk(text: &str) -> ConvUpdate {
    ConvUpdate::AgentChunk {
        content: ConvContent::Text(text.to_string()),
        message_id: Some("m1".to_string()),
    }
}

/// A conversation joins the work the way any other agent does, so the sidebar and the graph find
/// it with no change of their own.
#[gpui::test]
fn a_started_conversation_reaches_the_projection(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let id = AgentId::generate();
    fixture.started(an_agent(id), cx);

    let (in_work, live) = fixture.state.read_with(cx, |state, cx| {
        (
            state.work(cx).and_then(|work| work.agent(id)).is_some(),
            state.conversation(id, cx).is_some(),
        )
    });
    assert!(in_work, "the agent never reached the work projection");
    assert!(live, "no conversation was opened for it");
}

/// The whole point of folding the stream in the window: the badge, the ring, the token count and
/// the model are readings of what already arrived.
#[gpui::test]
fn an_update_refreshes_the_agent_record(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let id = AgentId::generate();
    fixture.started(an_agent(id), cx);

    fixture.update(id, 1, chunk("working on it"), cx);
    fixture.update(
        id,
        2,
        ConvUpdate::Usage(UsageRecord {
            used: 40_000,
            size: 200_000,
            cost_usd: Some(0.5),
            model: Some("claude-opus-5".to_string()),
        }),
        cx,
    );

    let record = fixture.state.read_with(cx, |state, cx| {
        state
            .work(cx)
            .and_then(|work| work.agent(id))
            .cloned()
            .expect("the agent is in the projection")
    });
    assert_eq!(record.activity, Activity::Writing);
    assert_eq!(record.context_pct, 20);
    assert_eq!(record.tokens, 40_000.0);
    assert_eq!(record.model, "claude-opus-5");

    let blocks = fixture.state.read_with(cx, |state, cx| {
        state.conversation(id, cx).unwrap().blocks.len()
    });
    assert_eq!(blocks, 1, "one message, one block");
}

/// The transcript outlives the harness, and the record stops moving with it.
#[gpui::test]
fn an_ended_conversation_is_kept(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let id = AgentId::generate();
    fixture.started(an_agent(id), cx);
    fixture.update(id, 1, chunk("done"), cx);

    fixture.host.send(
        To::Everyone,
        Message::ConversationEnded {
            agent_id: id,
            stop_reason: StopReason::Failed,
        },
    );
    cx.run_until_parked();

    let (run, blocks, activity) = fixture.state.read_with(cx, |state, cx| {
        let conversation = state.conversation(id, cx).expect("the transcript is kept");
        (
            conversation.run,
            conversation.blocks.len(),
            state
                .work(cx)
                .and_then(|work| work.agent(id))
                .unwrap()
                .activity,
        )
    });
    assert_eq!(run, Run::Ended);
    assert_eq!(blocks, 1, "the transcript went with the harness");
    assert_eq!(activity, Activity::Failed);
}

/// A sentence has to land where the user is looking, whether or not a conversation exists to hang
/// it on — a start that failed before one did is exactly the case worth saying.
#[gpui::test]
fn an_error_is_surfaced_with_or_without_a_conversation(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let ghost = AgentId::generate();

    fixture.host.send(
        To::Everyone,
        Message::ConversationError {
            agent_id: ghost,
            error: "claude-code is not on this machine".to_string(),
        },
    );
    cx.run_until_parked();

    assert_eq!(
        fixture
            .state
            .read_with(cx, |state, _| state.workbench.work_error.clone()),
        Some("claude-code is not on this machine".to_string())
    );

    let id = AgentId::generate();
    fixture.started(an_agent(id), cx);
    fixture.host.send(
        To::Everyone,
        Message::ConversationError {
            agent_id: id,
            error: "the stream broke".to_string(),
        },
    );
    cx.run_until_parked();

    assert_eq!(
        fixture.state.read_with(cx, |state, cx| state
            .conversation(id, cx)
            .unwrap()
            .error
            .clone()),
        Some("the stream broke".to_string()),
        "the sentence never reached the transcript it belongs to"
    );
}

/// Picking a harness raises the naming prompt, and confirming it asks the host to start one. The
/// id is what crosses, never the label.
#[gpui::test]
fn picking_a_harness_starts_a_conversation(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    fixture.host.send(
        To::Everyone,
        Message::AgentTypes {
            agent_types: vec![
                AgentTypeInfo {
                    id: "claude-code".to_string(),
                    label: "Claude Code".to_string(),
                    available: true,
                },
                AgentTypeInfo {
                    id: "codex".to_string(),
                    label: "Codex".to_string(),
                    available: false,
                },
            ],
        },
    );
    cx.run_until_parked();
    let _ = fixture.said();

    fixture.state.update(cx, |state, cx| {
        state.open_new_agent_menu((10.0, 20.0), cx);
    });
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture
            .state
            .update(cx, |state, cx| state.pick_new_agent_menu(0, window, cx));
    })
    .unwrap();
    fixture.state.update(cx, |state, cx| {
        state.start_named_agent(cx);
    });
    cx.run_until_parked();

    let started = fixture
        .said()
        .into_iter()
        .find_map(|message| match message {
            Message::StartConversation {
                project_id,
                agent_type,
                ..
            } => Some((project_id, agent_type)),
            _ => None,
        })
        .expect("picking a harness asks for a conversation");
    assert_eq!(started.0, fixture.project);
    assert_eq!(started.1, "claude-code");

    // A harness the host could not find is drawn disabled and takes no click.
    fixture.state.update(cx, |state, cx| {
        state.open_new_agent_menu((10.0, 20.0), cx);
    });
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture
            .state
            .update(cx, |state, cx| state.pick_new_agent_menu(1, window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    assert!(
        !fixture
            .said()
            .iter()
            .any(|message| matches!(message, Message::StartConversation { .. })),
        "an unavailable harness was started anyway"
    );
}

/// The composer sends and appends nothing: the line is drawn when the harness echoes it back.
#[gpui::test]
fn the_composer_prompts_without_writing_a_line(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let id = AgentId::generate();
    fixture.started(an_agent(id), cx);
    let _ = fixture.said();

    fixture
        .window
        .update(cx, |_, window, cx| {
            fixture.state.update(cx, |state, cx| {
                state.column_inputs[0].update(cx, |input, cx| {
                    input.set_value("look at the parser", window, cx);
                });
                state.prompt_agent(id, 0, window, cx);
            });
        })
        .expect("the window is open");
    cx.run_until_parked();

    let text = fixture
        .said()
        .into_iter()
        .find_map(|message| match message {
            Message::PromptAgent { agent_id, text } if agent_id == id => Some(text),
            _ => None,
        })
        .expect("the composer never sent a turn");
    assert_eq!(text, "look at the parser");

    assert!(
        fixture.state.read_with(cx, |state, cx| state
            .conversation(id, cx)
            .unwrap()
            .blocks
            .is_empty()),
        "the composer wrote its own half of the conversation"
    );
}

/// A harness that takes nothing after its first turn says so when it starts,
/// so the composer refuses rather than sending into a void and learning from
/// the error that comes back.
#[gpui::test]
fn a_one_shot_harness_is_known_before_a_turn_is_typed(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let id = AgentId::generate();
    fixture.started_with_input(an_agent(id), false, cx);

    assert!(
        !fixture.state.read_with(cx, |state, cx| state
            .conversation(id, cx)
            .unwrap()
            .accepts_input),
        "the capability did not travel with the agent"
    );

    fixture
        .window
        .update(cx, |_, window, cx| {
            fixture.state.update(cx, |state, cx| {
                state.prompt_agent(id, 0, window, cx);
            });
        })
        .expect("the window is open");
    cx.run_until_parked();

    assert!(
        !fixture
            .said()
            .iter()
            .any(|message| matches!(message, Message::PromptAgent { .. })),
        "a turn was sent to a harness that cannot take one"
    );
}

/// The failure this guards against looked like nothing happening at all: the host started the
/// harness, the agent reached the projection, and the screen stayed empty — because the sidebar
/// lists agents *under* a session and the window's own session is not one the work invented.
#[gpui::test]
fn a_started_agent_is_listed_under_its_session_and_put_on_the_field(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let id = AgentId::generate();
    let agent = an_agent(id);
    let session = agent.session;
    fixture.started(agent, cx);

    let (listed, on_the_field) = fixture.state.read_with(cx, |state, cx| {
        (
            state
                .work(cx)
                .is_some_and(|work| work.sessions.iter().any(|s| s.id == session)),
            state
                .agents(cx)
                .is_some_and(|agents| agents.columns.iter().any(|c| c.tabs.contains(&id))),
        )
    });
    assert!(
        listed,
        "the agent's session is not in the list that draws it"
    );
    assert!(
        on_the_field,
        "an agent the user asked for was left on the bench"
    );
}
