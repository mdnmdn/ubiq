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
use gpui::{
    AppContext as _, Context, Entity, IntoElement, Render, SharedString, TestAppContext,
    VisualTestContext, Window, WindowHandle,
};
use gpui_component::Root;
use ubiq::app::{AppState, BusHub};
use ubiq::state::agents::{COMPOSER_ROWS_MAX, COMPOSER_ROWS_MIN};
use ubiq::state::conversation::{Conversation, Pending, Run, TranscriptScroll, short_model_label};
use ubiq::state::{NewAgentSurface, WindowRegistry};
use ubiq::ui::conversation::{self, ConversationView};
use ubiq_proto::bus::{self, FromClient, To};
use ubiq_proto::conversation::{
    ConfigCategory, ConfigChoice, ConfigOption, ConfigValue, ConvContent, ConvUpdate,
    PermissionKind, PermissionOption, StopReason, Subagent, ToolCallPatch, ToolCallRecord,
    ToolKind, ToolStatus, UsageRecord,
};
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
                raw: None,
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
            custom_colour: None,
            temporary: false,
            created_at: Utc::now(),
            last_opened_at: None,
            search_excludes: Vec::new(),
            index: None,
        },
        health: ProjectHealth::Ok,
        open_panes: 0,
        ephemeral: false,
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
        summary: None,
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
        subagent: None,
    }
}

/// A select-shaped `ConfigOption`, for the three ids the host mints: `model`, `thinking`, `mode`.
fn config_option(
    id: &str,
    category: ConfigCategory,
    current: &str,
    choices: &[(&str, &str)],
) -> ConfigOption {
    ConfigOption {
        id: id.to_string(),
        name: id.to_string(),
        description: None,
        category: Some(category),
        value: ConfigValue::Select {
            current: current.to_string(),
            choices: choices
                .iter()
                .map(|(value, name)| ConfigChoice {
                    value: value.to_string(),
                    name: name.to_string(),
                    description: None,
                    group: None,
                })
                .collect(),
        },
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
            spend: None,
            subagent: None,
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

/// Unlike `ConversationEnded`, the conversation is back to its pre-launch state: the pickers
/// return, and the transcript above stays exactly as it was for a resume to run under.
#[gpui::test]
fn an_unloaded_conversation_goes_back_to_idle_and_keeps_its_transcript(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let id = AgentId::generate();
    fixture.started(an_agent(id), cx);
    fixture.update(id, 1, chunk("done"), cx);

    fixture
        .host
        .send(To::Everyone, Message::ConversationUnloaded { agent_id: id });
    cx.run_until_parked();

    let (run, launched, blocks) = fixture.state.read_with(cx, |state, cx| {
        let conversation = state.conversation(id, cx).expect("the transcript is kept");
        (
            conversation.run,
            conversation.launched,
            conversation.blocks.len(),
        )
    });
    assert_eq!(run, Run::Idle);
    assert!(!launched, "the next turn starts a new harness");
    assert_eq!(blocks, 1, "unload does not touch the transcript");
}

/// The name Ubiq wrote for itself lands on the record every surface reads, and the summary lands
/// beside it as the hover — one message, both facts, and no place in the transcript's sequence.
#[gpui::test]
fn a_naming_renames_the_record_and_carries_its_summary(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let id = AgentId::generate();
    fixture.started(an_agent(id), cx);

    let before = fixture.state.read_with(cx, |state, cx| {
        state
            .work(cx)
            .and_then(|work| work.agent(id))
            .cloned()
            .expect("the agent is in the projection")
    });
    assert_eq!(
        before.summary, None,
        "a conversation nobody has named has nothing to say on hover"
    );

    fixture.host.send(
        To::Everyone,
        Message::ConversationNamed {
            agent_id: id,
            title: "Sidebar Fold Control".to_string(),
            summary: Some("adding a collapsible sidebar".to_string()),
        },
    );
    cx.run_until_parked();

    let record = fixture.state.read_with(cx, |state, cx| {
        state
            .work(cx)
            .and_then(|work| work.agent(id))
            .cloned()
            .expect("the agent is still in the projection")
    });
    assert_eq!(record.name, "Sidebar Fold Control");
    assert_eq!(
        record.summary.as_deref(),
        Some("adding a collapsible sidebar")
    );

    // A naming is not a transcript delta: it carries no `seq`, so it must not have consumed one
    // or the next real update would read as a gap.
    let (seq_is_untouched, blocks) = fixture.state.read_with(cx, |state, cx| {
        let conversation = state
            .conversation(id, cx)
            .expect("the conversation is here");
        (conversation.is_next(1), conversation.blocks.len())
    });
    assert!(
        seq_is_untouched,
        "the naming took a sequence number that belongs to the harness"
    );
    assert_eq!(blocks, 0, "a naming is not something anybody said");
}

/// A model that answered a title and nothing after it has still named the conversation, and a
/// re-naming that answers no summary clears the one before it rather than leaving it to describe
/// a conversation as it was.
#[gpui::test]
fn a_naming_without_a_summary_clears_the_one_before_it(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let id = AgentId::generate();
    fixture.started(an_agent(id), cx);

    for (title, summary) in [
        ("First Reading", Some("what it looked like first")),
        ("Second Reading", None),
    ] {
        fixture.host.send(
            To::Everyone,
            Message::ConversationNamed {
                agent_id: id,
                title: title.to_string(),
                summary: summary.map(str::to_string),
            },
        );
        cx.run_until_parked();
    }

    let record = fixture.state.read_with(cx, |state, cx| {
        state
            .work(cx)
            .and_then(|work| work.agent(id))
            .cloned()
            .expect("the agent is in the projection")
    });
    assert_eq!(record.name, "Second Reading");
    assert_eq!(record.summary, None, "a stale reading outlived its naming");
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

/// The `+` no longer starts anything by itself. Its first row raises the New agent form — which
/// is what asks the harness, the identity, the model, the level and the mode — and its second
/// turns the same menu into the list of conversations already running.
#[gpui::test]
fn the_plus_menu_offers_the_form_and_the_attach_list(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    fixture.host.send(
        To::Everyone,
        Message::AgentTypes {
            agent_types: vec![AgentTypeInfo {
                id: "claude-code".to_string(),
                label: "Claude Code".to_string(),
                command: "claude".to_string(),
                available: true,
                chat: true,
                modes: Vec::new(),
                unattended_mode: None,
            }],
        },
    );
    cx.run_until_parked();
    let _ = fixture.said();

    // Row 1 keeps the menu open on its second stage rather than starting anything.
    fixture
        .window
        .update(cx, |_, window, cx| {
            fixture.state.update(cx, |state, cx| {
                state.open_new_agent_menu((10.0, 20.0), NewAgentSurface::Agents, cx);
                state.pick_new_agent_menu(1, window, cx);
            })
        })
        .expect("the window is open");
    cx.run_until_parked();
    assert!(
        fixture
            .state
            .read_with(cx, |state, _| state.workbench.new_agent_menu)
            .is_some_and(|menu| menu.attach),
        "the second row opens the attach stage of the same menu"
    );
    assert!(
        !fixture
            .said()
            .iter()
            .any(|message| matches!(message, Message::StartConversation { .. })),
        "nothing is started until the form is answered"
    );

    // Row 0 shuts the menu and raises the form, and still starts nothing on its own.
    fixture
        .window
        .update(cx, |_, window, cx| {
            fixture.state.update(cx, |state, cx| {
                state.open_new_agent_menu((10.0, 20.0), NewAgentSurface::Agents, cx);
                state.pick_new_agent_menu(0, window, cx);
            })
        })
        .expect("the window is open");
    cx.run_until_parked();
    assert!(
        fixture
            .state
            .read_with(cx, |state, _| state.workbench.new_agent_menu.is_none()),
        "the menu is put away once the form is up"
    );
    assert!(
        fixture
            .state
            .read_with(cx, |state, _| state.new_agent_form().is_some()),
        "the first row raises the New agent form"
    );
    assert!(
        !fixture
            .said()
            .iter()
            .any(|message| matches!(message, Message::StartConversation { .. })),
        "the form asks before anything is launched"
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

/// The pre-launch model picker filters by `AppState::picker_search`, and a pick names the
/// filtered row's value — never the unfiltered list's row at that same index. Five choices, a
/// query that matches two of them, and the two are not the list's first two: if the picker ever
/// forgot to filter before indexing, this would send the wrong model.
#[gpui::test]
fn the_model_picker_filters_and_picks_the_filtered_row(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let id = AgentId::generate();
    fixture.started(an_agent(id), cx);

    let choices = vec![
        ConfigChoice {
            value: "opus5".to_string(),
            name: "Claude Opus 5".to_string(),
            description: None,
            group: None,
        },
        ConfigChoice {
            value: "sonnet5".to_string(),
            name: "Claude Sonnet 5".to_string(),
            description: None,
            group: None,
        },
        ConfigChoice {
            value: "haiku5".to_string(),
            name: "Claude Haiku 5".to_string(),
            description: None,
            group: None,
        },
        ConfigChoice {
            value: "gpt5-codex".to_string(),
            name: "GPT-5 Codex".to_string(),
            description: None,
            group: None,
        },
        ConfigChoice {
            value: "gpt5-mini".to_string(),
            name: "GPT-5 mini".to_string(),
            description: None,
            group: None,
        },
    ];
    fixture.update(
        id,
        1,
        ConvUpdate::ConfigOptions(vec![ConfigOption {
            id: "model".to_string(),
            name: "Model".to_string(),
            description: None,
            category: Some(ConfigCategory::Model),
            value: ConfigValue::Select {
                current: "opus5".to_string(),
                choices: choices.clone(),
            },
        }]),
        cx,
    );

    fixture
        .window
        .update(cx, |_, window, cx| {
            fixture.state.update(cx, |state, cx| {
                state.toggle_agent_config_menu(id, "model".to_string(), window, cx);
                let search = state.picker_search.clone();
                search.update(cx, |input, cx| {
                    input.set_value("gpt", window, cx);
                });
            });
        })
        .expect("the window is open");
    cx.run_until_parked();

    let (row_count, filtered_first) = fixture.state.read_with(cx, |state, cx| {
        let conversation = state
            .conversation(id, cx)
            .expect("the agent has a conversation");
        let search = state.picker_search.read(cx).value().to_string();
        let row = ubiq::ui::conversation::config_choices(conversation, "model", &search)
            .expect("the harness has already advertised its models");
        (row.names.len(), row.values[0].clone())
    });
    assert_eq!(row_count, 2, "gpt matches two of the five choices");
    assert_eq!(filtered_first, "gpt5-codex");
    assert_ne!(
        filtered_first, choices[0].value,
        "the filtered list's row 0 must not be the unfiltered list's row 0"
    );

    fixture
        .window
        .update(cx, |_, window, cx| {
            fixture.state.update(cx, |state, cx| {
                state.pick_agent_config(
                    id,
                    "model".to_string(),
                    filtered_first.clone(),
                    window,
                    cx,
                );
            });
        })
        .expect("the window is open");
    cx.run_until_parked();

    let sent = fixture
        .said()
        .into_iter()
        .find_map(|message| match message {
            Message::SetAgentConfig {
                agent_id,
                config_id,
                value,
            } if agent_id == id && config_id == "model" => Some(value),
            _ => None,
        })
        .expect("picking a filtered row asks the host to set that model");
    assert_eq!(sent, "gpt5-codex");
}

/// The host mints up to three config options at once. Every one of them offered means every one
/// of them is a real picker — `config_choices` returns `Some` for each id, not just the model.
#[gpui::test]
fn config_options_carrying_all_three_draws_three_pickers(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let id = AgentId::generate();
    fixture.started(an_agent(id), cx);

    fixture.update(
        id,
        1,
        ConvUpdate::ConfigOptions(vec![
            config_option(
                "model",
                ConfigCategory::Model,
                "opus5",
                &[("opus5", "Claude Opus 5"), ("sonnet5", "Claude Sonnet 5")],
            ),
            config_option(
                "thinking",
                ConfigCategory::ThoughtLevel,
                "low",
                &[("low", "Low"), ("high", "High")],
            ),
            config_option(
                "mode",
                ConfigCategory::Mode,
                "",
                &[("plan", "Plan"), ("edit", "Edit")],
            ),
        ]),
        cx,
    );

    fixture.state.read_with(cx, |state, cx| {
        let conversation = state
            .conversation(id, cx)
            .expect("the agent has a conversation");
        for config_id in ["model", "thinking", "mode"] {
            assert!(
                ubiq::ui::conversation::config_choices(conversation, config_id, "").is_some(),
                "{config_id} should draw its own picker"
            );
        }
    });
}

/// A `ConfigOptions` naming only a model draws exactly one picker — no thinking or mode row
/// invented for a harness that never offered them.
#[gpui::test]
fn a_config_options_with_only_a_model_draws_one_picker(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let id = AgentId::generate();
    fixture.started(an_agent(id), cx);

    fixture.update(
        id,
        1,
        ConvUpdate::ConfigOptions(vec![config_option(
            "model",
            ConfigCategory::Model,
            "opus5",
            &[("opus5", "Claude Opus 5")],
        )]),
        cx,
    );

    fixture.state.read_with(cx, |state, cx| {
        let conversation = state
            .conversation(id, cx)
            .expect("the agent has a conversation");
        assert!(ubiq::ui::conversation::config_choices(conversation, "model", "").is_some());
        assert!(ubiq::ui::conversation::config_choices(conversation, "thinking", "").is_none());
        assert!(ubiq::ui::conversation::config_choices(conversation, "mode", "").is_none());
    });
}

/// Picking a thinking level sends the same `SetAgentConfig` a model pick would, keyed on
/// `"thinking"` instead of `"model"` — one mechanism, three ids.
#[gpui::test]
fn picking_a_thinking_level_puts_set_agent_config_on_the_bus(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let id = AgentId::generate();
    fixture.started(an_agent(id), cx);

    fixture.update(
        id,
        1,
        ConvUpdate::ConfigOptions(vec![config_option(
            "thinking",
            ConfigCategory::ThoughtLevel,
            "low",
            &[("low", "Low"), ("high", "High")],
        )]),
        cx,
    );

    fixture
        .window
        .update(cx, |_, window, cx| {
            fixture.state.update(cx, |state, cx| {
                state.pick_agent_config(id, "thinking".to_string(), "high".to_string(), window, cx);
            });
        })
        .expect("the window is open");
    cx.run_until_parked();

    let sent = fixture
        .said()
        .into_iter()
        .find_map(|message| match message {
            Message::SetAgentConfig {
                agent_id,
                config_id,
                value,
            } if agent_id == id && config_id == "thinking" => Some(value),
            _ => None,
        })
        .expect("picking a thinking level asks the host to set it");
    assert_eq!(sent, "high");
}

/// The regression this package exists to prevent: a model pick re-sends `ConfigOptions` with
/// `thinking` recomputed for the new model, and a level the old model accepted may not exist
/// under the new one. A `chosen` entry the fresh options no longer back must not survive.
#[gpui::test]
fn a_thought_level_missing_from_the_new_options_is_forgotten(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let id = AgentId::generate();
    fixture.started(an_agent(id), cx);

    fixture.update(
        id,
        1,
        ConvUpdate::ConfigOptions(vec![
            config_option(
                "model",
                ConfigCategory::Model,
                "opus5",
                &[("opus5", "Claude Opus 5"), ("haiku5", "Claude Haiku 5")],
            ),
            config_option(
                "thinking",
                ConfigCategory::ThoughtLevel,
                "low",
                &[("low", "Low"), ("high", "High"), ("ultra", "Ultra")],
            ),
        ]),
        cx,
    );

    fixture
        .window
        .update(cx, |_, window, cx| {
            fixture.state.update(cx, |state, cx| {
                state.pick_agent_config(
                    id,
                    "thinking".to_string(),
                    "ultra".to_string(),
                    window,
                    cx,
                );
            });
        })
        .expect("the window is open");
    cx.run_until_parked();

    fixture.state.read_with(cx, |state, cx| {
        let conversation = state
            .conversation(id, cx)
            .expect("the agent has a conversation");
        assert_eq!(
            conversation.chosen.get("thinking").map(String::as_str),
            Some("ultra"),
            "the pick was held before the model changed"
        );
    });

    // The user switches models. Haiku has no "ultra" level — the host re-sends `ConfigOptions`
    // with `thinking` recomputed for it.
    fixture.update(
        id,
        2,
        ConvUpdate::ConfigOptions(vec![
            config_option(
                "model",
                ConfigCategory::Model,
                "haiku5",
                &[("opus5", "Claude Opus 5"), ("haiku5", "Claude Haiku 5")],
            ),
            config_option(
                "thinking",
                ConfigCategory::ThoughtLevel,
                "low",
                &[("low", "Low"), ("high", "High")],
            ),
        ]),
        cx,
    );

    fixture.state.read_with(cx, |state, cx| {
        let conversation = state
            .conversation(id, cx)
            .expect("the agent has a conversation");
        assert_eq!(
            conversation.chosen.get("thinking"),
            None,
            "a thinking level the new model does not offer must not survive"
        );
    });
}

/// The three-dots menu's own rule, pulled out where it can be checked without rendering it: Stop
/// only while a turn runs, Abort and Unload only while launched, Resume only while it is not,
/// Delete always.
#[test]
fn the_lifecycle_menu_disables_resume_while_launched_and_unload_once_it_is_not() {
    use ubiq::ui::conversation::lifecycle_menu_enabled;

    let mut conversation = Conversation::new(
        AgentId::generate(),
        "Claude Code".to_string(),
        String::new(),
    );

    // Freshly launched: running a turn, Unload applies, Resume does not.
    conversation.launched = true;
    conversation.run = Run::Working;
    let [stop, abort, unload, resume, delete] = lifecycle_menu_enabled(&conversation);
    assert!(stop, "a turn is running");
    assert!(abort, "there is a process to kill");
    assert!(unload, "the harness is up");
    assert!(!resume, "already launched");
    assert!(delete, "always enabled");

    // Unloaded: no turn to stop, nothing to unload, Resume is what applies now.
    conversation.launched = false;
    conversation.run = Run::Idle;
    let [stop, abort, unload, resume, delete] = lifecycle_menu_enabled(&conversation);
    assert!(!stop, "nothing is running");
    assert!(!abort, "there is no process left to kill");
    assert!(!unload, "there is no harness to unload");
    assert!(resume, "not launched");
    assert!(delete, "always enabled");
}

/// The lifecycle glyph's own rule, pulled out the same way the menu's is: derived from
/// `launched`, `run`, `blocks`, `accepts_input` and `config` alone, with nothing new stored on
/// `Conversation` for it.
#[test]
fn the_lifecycle_glyph_reads_launched_run_and_the_transcript() {
    use ubiq::state::conversation::ConvBlock;
    use ubiq::ui::conversation::{Lifecycle, lifecycle};

    let fresh = || {
        Conversation::new(
            AgentId::generate(),
            "Claude Code".to_string(),
            String::new(),
        )
    };

    // Never launched, and nothing has answered `ListAgentTypes`'s config yet.
    let c = fresh();
    assert_eq!(lifecycle(&c), Lifecycle::Starting);

    // Never launched, but the harness's config has arrived — the pickers are answerable.
    let mut c = fresh();
    c.config = vec![config_option(
        "model",
        ConfigCategory::Model,
        "opus5",
        &[("opus5", "Claude Opus 5")],
    )];
    assert_eq!(lifecycle(&c), Lifecycle::Ready);

    // A turn in flight carries its own activity rather than one flattened "working".
    let mut c = fresh();
    c.launched = true;
    c.run = Run::Working;
    c.blocks.push(ConvBlock::Tool {
        call: ToolCallRecord {
            id: "t1".to_string(),
            title: "tool".to_string(),
            kind: ToolKind::default(),
            status: ToolStatus::default(),
            content: Vec::new(),
            locations: Vec::new(),
            subagent: None,
        },
        open: false,
    });
    assert_eq!(lifecycle(&c), Lifecycle::Working(Activity::Tools));

    // Loaded and between turns.
    let mut c = fresh();
    c.launched = true;
    c.run = Run::Idle;
    assert_eq!(lifecycle(&c), Lifecycle::Idle);

    // Unloaded and never-launched both read `launched == false` — the transcript is what tells
    // them apart. A blank one reads as `Starting` above; one with something said reads as
    // `Unloaded`, whatever its stale `config` still holds from before the harness went.
    let mut c = fresh();
    c.config = vec![config_option(
        "model",
        ConfigCategory::Model,
        "opus5",
        &[("opus5", "Claude Opus 5")],
    )];
    c.blocks.push(ConvBlock::Agent {
        body: "said something".to_string(),
        subagent: None,
    });
    assert_eq!(lifecycle(&c), Lifecycle::Unloaded);

    // A conversation the harness will take no more turns on reads `Ended`, whether it ran one to
    // completion or its harness never takes a second turn at all.
    let mut c = fresh();
    c.launched = true;
    c.run = Run::Ended;
    assert_eq!(lifecycle(&c), Lifecycle::Ended);

    let mut c = fresh();
    c.accepts_input = false;
    assert_eq!(lifecycle(&c), Lifecycle::Ended);
}

/// A bare conversation, the way the state layer's own tests build one: nothing said, nothing
/// launched, no harness behind it.
fn a_conversation() -> Conversation {
    Conversation::new(
        AgentId::generate(),
        "Claude Code".to_string(),
        String::new(),
    )
}

/// A tool call as the harness announces one. `subagent` is who raised it — `None` for the
/// conversation's own turn, and the delegate's instance id for one of its calls.
fn a_tool_call(id: &str, title: &str, subagent: Option<&str>) -> ConvUpdate {
    ConvUpdate::ToolCall(ToolCallRecord {
        id: id.to_string(),
        title: title.to_string(),
        kind: ToolKind::Execute,
        status: ToolStatus::InProgress,
        content: Vec::new(),
        locations: Vec::new(),
        subagent: subagent.map(|id| Subagent {
            id: id.to_string(),
            kind: Some("general-purpose".to_string()),
            ..Default::default()
        }),
    })
}

/// One line spoken by a delegate — the only evidence the window has that the agent exists, and
/// so what puts it in the switcher at all.
fn a_delegate_line(id: &str) -> ConvUpdate {
    ConvUpdate::AgentChunk {
        content: ConvContent::Text("On it.".to_string()),
        message_id: Some(format!("m-{id}")),
        subagent: Some(Subagent {
            id: id.to_string(),
            kind: Some("general-purpose".to_string()),
            ..Default::default()
        }),
    }
}

fn a_permission_option(option_id: &str, name: &str, kind: PermissionKind) -> PermissionOption {
    PermissionOption {
        option_id: option_id.to_string(),
        name: name.to_string(),
        kind,
    }
}

/// The three-option request a harness that offers to remember sends: yes, yes-always, no.
/// `call_id` is the call the request is about, which is the whole of how it is routed.
fn a_permission_request(request_id: &str, call_id: &str) -> ConvUpdate {
    ConvUpdate::PermissionRequest {
        request_id: request_id.to_string(),
        tool_call: ToolCallPatch {
            id: call_id.to_string(),
            ..ToolCallPatch::default()
        },
        options: vec![
            a_permission_option("allow-once", "Yes", PermissionKind::AllowOnce),
            a_permission_option("allow-always", "Yes, always", PermissionKind::AllowAlways),
            a_permission_option("reject-once", "No", PermissionKind::RejectOnce),
        ],
    }
}

/// Where the "needs you" strip sends a reader: whose transcript, and which block of it. Both
/// halves are one reading of the call the request names — a strip that switched to one agent and
/// scrolled to a block of another's would land the reader where the prompt is not.
#[test]
fn a_request_routes_to_the_delegate_that_raised_it_and_the_block_its_call_is_drawn_as() {
    let mut c = a_conversation();
    // 0: the Task call that spawned the delegate, which is the main agent's own call.
    c.apply(1, a_tool_call("t751", "Formal greeting agent", None));
    // 1: what the delegate said, which is what puts it in the transcript at all.
    c.apply(2, a_delegate_line("t751"));
    // 2: a call the delegate raised. 3: one the conversation raised itself.
    c.apply(3, a_tool_call("sub-call", "Bash ls", Some("t751")));
    c.apply(4, a_tool_call("own-call", "Bash git status", None));
    c.apply(5, a_permission_request("r-sub", "sub-call"));
    c.apply(6, a_permission_request("r-own", "own-call"));
    c.apply(7, a_permission_request("r-ghost", "never-announced"));

    let route = |request_id: &str| {
        let held = c
            .pending
            .iter()
            .find(|held| held.request_id == request_id)
            .expect("the request is held");
        c.pending_route(held)
    };

    assert_eq!(
        route("r-sub"),
        (Some("t751".to_string()), Some(2)),
        "a request about a delegate's call is the delegate's, at the block its call is drawn as"
    );
    assert_eq!(
        route("r-own"),
        (None, Some(3)),
        "a request about the conversation's own call belongs to no delegate"
    );
    assert_eq!(
        route("r-ghost"),
        (None, None),
        "a call the transcript never saw has no block — and this is the reading that sends the \
         strip to the self-contained prompt at the tail rather than nowhere"
    );
}

/// A request naming a call this transcript has never seen reads as the main agent's rather than
/// being filed under a guess — there is nothing to file it under.
#[test]
fn a_request_for_a_call_the_transcript_never_saw_reads_as_the_main_agents() {
    let mut c = a_conversation();
    c.apply(1, a_permission_request("r-ghost", "never-announced"));

    let held = &c.pending[0];
    assert_eq!(
        c.pending_subagent(held),
        None,
        "no block, no speaker — and the main agent's own turn is the honest fallback"
    );
    assert_eq!(
        c.pending_route(held),
        (None, None),
        "and no block to be taken to either"
    );
    assert_eq!(
        c.pending_count(None),
        1,
        "counted against the conversation itself, so nothing goes unanswered"
    );
}

/// What each row of the switcher is counting. Two delegates each blocked on their own request are
/// blocked on one each: a count that read the whole list would put every delegate's question on
/// every delegate's row.
#[test]
fn two_delegates_blocked_on_their_own_requests_count_one_each() {
    let mut c = a_conversation();
    c.apply(1, a_tool_call("t751", "Formal greeting agent", None));
    c.apply(2, a_delegate_line("t751"));
    c.apply(3, a_tool_call("t754", "Pirate-style greeting agent", None));
    c.apply(4, a_delegate_line("t754"));
    c.apply(5, a_tool_call("call-a", "Bash ls", Some("t751")));
    c.apply(6, a_tool_call("call-b", "Bash pwd", Some("t754")));
    c.apply(7, a_permission_request("r-a", "call-a"));
    c.apply(8, a_permission_request("r-b", "call-b"));

    let who = |request_id: &str| {
        let held = c
            .pending
            .iter()
            .find(|held| held.request_id == request_id)
            .expect("the request is held");
        c.pending_subagent(held).map(str::to_string)
    };

    assert_eq!(who("r-a").as_deref(), Some("t751"));
    assert_eq!(who("r-b").as_deref(), Some("t754"));
    assert_eq!(
        c.pending_count(Some("t751")),
        1,
        "its own question, not its sibling's as well"
    );
    assert_eq!(c.pending_count(Some("t754")), 1);
    assert_eq!(
        c.pending_count(None),
        0,
        "the conversation itself is blocked on neither"
    );
}

/// The count reaches the row: `waiting` is what puts `need you` on a delegate in place of what it
/// would otherwise say it was doing, and a delegate with nothing outstanding says nothing.
#[test]
fn a_delegates_tab_reports_the_requests_it_is_blocked_on() {
    let mut c = a_conversation();
    c.apply(1, a_tool_call("t751", "Formal greeting agent", None));
    c.apply(2, a_delegate_line("t751"));
    c.apply(3, a_tool_call("t754", "Pirate-style greeting agent", None));
    c.apply(4, a_delegate_line("t754"));
    c.apply(5, a_tool_call("call-a", "Bash ls", Some("t751")));
    c.apply(6, a_permission_request("r-a", "call-a"));

    assert_eq!(
        c.subagents()
            .iter()
            .map(|tab| (tab.id.clone(), tab.waiting))
            .collect::<Vec<_>>(),
        vec![("t751".to_string(), 1), ("t754".to_string(), 0)],
        "one row per instance, each counting only its own question"
    );
}

/// The "all" of yes / no / all: the one option that both goes ahead and is remembered. It is not
/// the plain allow — answering with that would draw a control that lies about lasting — and where
/// the harness offered no such reading there is nothing to draw.
#[test]
fn the_always_option_is_the_one_that_both_allows_and_remembers() {
    let three = Pending {
        request_id: "r1".to_string(),
        tool_call: ToolCallPatch::default(),
        options: vec![
            a_permission_option("allow-once", "Yes", PermissionKind::AllowOnce),
            a_permission_option("allow-always", "Yes, always", PermissionKind::AllowAlways),
            a_permission_option("reject-once", "No", PermissionKind::RejectOnce),
        ],
    };
    assert_eq!(
        three.always_option().map(|option| option.kind),
        Some(PermissionKind::AllowAlways),
        "the one that allows and remembers, picked by hint rather than by reading an id"
    );
    assert_eq!(
        three
            .always_option()
            .map(|option| option.option_id.as_str()),
        Some("allow-always"),
        "and answered with the harness's own opaque id"
    );
    assert_eq!(
        three
            .option_for(true)
            .map(|option| option.option_id.as_str()),
        Some("allow-once"),
        "the plain allow is still what the keyboard answers with"
    );

    let two = Pending {
        request_id: "r2".to_string(),
        tool_call: ToolCallPatch::default(),
        options: vec![
            a_permission_option("allow-once", "Yes", PermissionKind::AllowOnce),
            a_permission_option("reject-once", "No", PermissionKind::RejectOnce),
        ],
    };
    assert_eq!(
        two.always_option(),
        None,
        "no lasting reading offered, so no All button — one drawn here would not last"
    );
    assert_eq!(
        two.option_for(true).map(|option| option.option_id.as_str()),
        Some("allow-once"),
        "yes and no are still answerable"
    );
}

/// A question outranks the turn it is blocking. `Working` is what a turn in flight reads as, but a
/// turn in flight *and* blocked on a human is not working — it is the one state that needs the
/// reader to do something, and the dot in the title exists to carry exactly that across a window
/// nobody is looking at.
#[test]
fn a_pending_request_reads_waiting_and_outranks_the_turn_it_blocks() {
    use ubiq::ui::conversation::{Lifecycle, lifecycle};

    let mut c = a_conversation();
    c.launched = true;
    c.apply(1, chunk("working on it"));
    assert_eq!(
        lifecycle(&c),
        Lifecycle::Working(Activity::Writing),
        "a turn in flight and nothing blocking it"
    );

    c.apply(2, a_permission_request("r1", "never-announced"));
    assert_eq!(c.run, Run::Working, "the turn has not ended, only stalled");
    assert_eq!(
        lifecycle(&c),
        Lifecycle::Waiting,
        "the question outranks the turn it is blocking"
    );
    assert_ne!(
        lifecycle(&c),
        Lifecycle::Working(Activity::NeedsYou),
        "a blocked turn is never drawn as a kind of working"
    );
    assert_eq!(Lifecycle::Waiting.label(), "Needs you");

    c.answered("r1");
    assert_eq!(
        lifecycle(&c),
        Lifecycle::Working(Activity::Writing),
        "answered, and back to whatever the turn was doing"
    );

    // Ended still outranks everything: a harness taking no more turns is not waiting on anybody,
    // whatever a race left in `pending`.
    c.apply(3, a_permission_request("r2", "never-announced"));
    c.run = Run::Ended;
    assert_eq!(lifecycle(&c), Lifecycle::Ended);
}

/// The dot's four readings, pinned because a dot that changes meaning silently is exactly the bug
/// this rule exists to prevent: amber needs you, blue is moving, green is fine, grey is not
/// running. And `Working` no longer varies with the activity — the colour answers "is it moving",
/// and the tooltip is where "doing what" is said.
#[test]
fn the_lifecycle_dot_has_four_readings_and_working_is_one_of_them() {
    use ubiq::theme;
    use ubiq::ui::conversation::{Lifecycle, lifecycle_colour};

    assert_eq!(lifecycle_colour(Lifecycle::Waiting), theme::warning());

    for activity in [
        Activity::Thinking,
        Activity::Writing,
        Activity::Tools,
        Activity::NeedsYou,
    ] {
        assert_eq!(
            lifecycle_colour(Lifecycle::Working(activity)),
            theme::info(),
            "every kind of working is the one working colour"
        );
    }

    assert_eq!(lifecycle_colour(Lifecycle::Ready), theme::success());
    assert_eq!(lifecycle_colour(Lifecycle::Idle), theme::success());

    assert_eq!(lifecycle_colour(Lifecycle::Starting), theme::text_faint());
    assert_eq!(lifecycle_colour(Lifecycle::Unloaded), theme::text_faint());
    assert_eq!(lifecycle_colour(Lifecycle::Ended), theme::text_faint());

    assert_ne!(
        lifecycle_colour(Lifecycle::Waiting),
        lifecycle_colour(Lifecycle::Working(Activity::Thinking)),
        "the two states the reader has to tell apart at a glance are not the same colour"
    );
}

/// A request to be taken to a block is answered once, and not by the frame that cannot answer it.
///
/// The frame that switched transcripts is measuring the transcript that has gone, so it holds the
/// request back and the next frame — the first whose measurements are of the transcript the block
/// is in — answers it. Then it is gone: a jump answered every frame afterwards would pin the
/// reader to that block for good.
///
/// Only the bookkeeping is reachable here. `away_from_tail`, `near_viewport` and where `sync`
/// actually leaves the handle are readings of the last frame's layout, and a `ScrollHandle` that
/// has never been painted has no bounds and no maximum offset — a test asserting against those
/// would be asserting against zeroes, which is why there is none.
#[test]
fn a_scroll_request_is_held_until_the_transcript_has_settled_and_then_answered_once() {
    let scroll = TranscriptScroll::default();
    let key = (AgentId::generate(), None);

    assert!(!scroll.request_held(), "nothing asked for yet");
    scroll.request(7);
    assert!(scroll.request_held(), "asked for, and not yet answered");

    // The frame that arrived at this transcript: its measurements are of the one before it.
    scroll.sync(key.clone(), 1);
    assert_eq!(
        scroll.take_request(),
        None,
        "the frame that switched transcripts cannot answer a jump"
    );
    assert!(
        scroll.request_held(),
        "so it stays held rather than being dropped — and the frame asks to be drawn again"
    );

    // The next frame is drawing the same transcript, so it is the one that can.
    scroll.sync(key, 1);
    assert_eq!(
        scroll.take_request(),
        Some(7),
        "answered by the frame that can"
    );
    assert!(!scroll.request_held(), "and cleared by being answered");
    assert_eq!(
        scroll.take_request(),
        None,
        "answered exactly once, not re-answered every frame afterwards"
    );
}

/// Windowing is an optimisation with a floor: below it every block is built every frame, which is
/// both cheaper than the bookkeeping and exact on the first frame. And it needs a painted
/// viewport to measure against, so a slot that has never drawn windows nothing whatever its
/// length — the first frame builds the lot and the second one narrows it.
#[test]
fn a_transcript_windows_only_when_it_is_long_and_the_slot_has_painted() {
    let scroll = TranscriptScroll::default();

    assert!(!scroll.windows(0), "nothing to window");
    assert!(
        !scroll.windows(40),
        "at the floor, not above it — every block is built"
    );
    assert!(
        !scroll.windows(41),
        "long enough, but a slot that has never painted has no viewport to measure against"
    );
    assert_eq!(
        scroll.child_bounds(0),
        None,
        "and no child was painted to measure either"
    );
}

/// A window whose only content is one conversation, drawn with whichever `header` the test wants —
/// standing in for the agents column (`header: true`) and the chat panel (`header: false`) without
/// dragging in either surface's own dock plumbing.
struct ConversationHarness {
    state: Entity<AppState>,
    agent: AgentId,
    header: bool,
    /// The agent switcher lives in the bottom block, which only exists when a footer or a
    /// composer does — so a test that wants to see it asks for one.
    footer: bool,
    /// The composer, which the panel must never move: a test that checks it stays put asks for it.
    composer: bool,
}

impl Render for ConversationHarness {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl gpui::IntoElement {
        let agent = self.agent;
        let view = ConversationView {
            id: SharedString::from("conversation-harness"),
            slot: 0,
            footer: self.footer,
            composer: self.composer,
            header: self.header,
        };
        self.state
            .update(cx, |state, cx| match state.conversation(agent, cx) {
                Some(live) => conversation::render(state, live, view, window, cx),
                None => gpui::div().into_any_element(),
            })
    }
}

/// The lifecycle strip — the status glyph and the three-dots menu — is the shared conversation
/// view's own to draw or withhold. `header: true` is the agents column, unchanged; `header: false`
/// is the chat panel, which draws the same two controls itself, inline in its own toolbar, so the
/// shared view must not draw them a second time.
#[gpui::test]
fn header_true_draws_the_lifecycle_strip_and_false_does_not(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let id = AgentId::generate();
    fixture.started(an_agent(id), cx);

    let with_header = cx.add_window(|_, _cx| ConversationHarness {
        state: fixture.state.clone(),
        agent: id,
        header: true,
        footer: false,
        composer: false,
    });
    cx.run_until_parked();
    let mut vcx = VisualTestContext::from_window(with_header.into(), cx);
    assert!(
        vcx.debug_bounds("lifecycle-strip").is_some(),
        "header: true should draw the strip, as the agents column always has"
    );

    let without_header = cx.add_window(|_, _cx| ConversationHarness {
        state: fixture.state.clone(),
        agent: id,
        header: false,
        footer: false,
        composer: false,
    });
    cx.run_until_parked();
    let mut vcx = VisualTestContext::from_window(without_header.into(), cx);
    assert!(
        vcx.debug_bounds("lifecycle-strip").is_none(),
        "header: false must not draw the strip — the chat panel draws it inline instead"
    );
}

/// The switcher is drawn only where there is something to switch to. A conversation that never
/// spawned a subagent looks exactly as it did before — no empty strip.
#[gpui::test]
fn the_agent_switcher_appears_only_once_a_subagent_has(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let id = AgentId::generate();
    fixture.started(an_agent(id), cx);
    fixture.update(id, 1, chunk("on it"), cx);

    let window = cx.add_window(|_, _cx| ConversationHarness {
        state: fixture.state.clone(),
        agent: id,
        header: true,
        footer: true,
        composer: true,
    });
    cx.run_until_parked();
    let mut vcx = VisualTestContext::from_window(window.into(), cx);
    assert!(
        vcx.debug_bounds("agent-switcher").is_none(),
        "no subagent, no row"
    );

    // Turn 2 of the real capture: one `Task` call, and the agent it spawned speaking under that
    // call's id.
    fixture.update(
        id,
        2,
        ConvUpdate::ToolCall(ToolCallRecord {
            id: "t751".to_string(),
            title: "Formal greeting agent".to_string(),
            kind: ToolKind::Other,
            status: ToolStatus::InProgress,
            content: Vec::new(),
            locations: Vec::new(),
            subagent: None,
        }),
        cx,
    );
    fixture.update(
        id,
        3,
        ConvUpdate::AgentChunk {
            content: ConvContent::Text("Good day.".to_string()),
            message_id: Some("m9".to_string()),
            subagent: Some(Subagent {
                id: "t751".to_string(),
                kind: Some("general-purpose".to_string()),
                ..Default::default()
            }),
        },
        cx,
    );
    vcx.run_until_parked();
    assert!(
        vcx.debug_bounds("agent-switcher").is_some(),
        "a spawned subagent puts the row above the control area"
    );
    assert!(
        vcx.debug_bounds("agent-switcher-panel").is_none(),
        "collapsed by default — one row saying how many, and nothing else"
    );

    // Opening draws the list upward, over the transcript: one row per agent, the main agent's
    // among them, and the composer exactly where it was.
    let composer_before = vcx
        .debug_bounds("composer-field")
        .expect("the composer is drawn");
    fixture.state.update(&mut vcx, |state, cx| {
        state.toggle_conversation_subagents(id, cx)
    });
    vcx.run_until_parked();
    assert!(
        vcx.debug_bounds("agent-switcher-panel").is_some(),
        "clicking the header expands the list"
    );
    assert!(
        vcx.debug_bounds("agent-row-main").is_some(),
        "the main agent is always in the list — it is the way back"
    );
    assert!(
        vcx.debug_bounds("agent-row-t751").is_some(),
        "one row per spawned agent"
    );
    assert_eq!(
        vcx.debug_bounds("composer-field"),
        Some(composer_before),
        "the list grows over the transcript, so the composer does not move under the cursor"
    );

    fixture.state.update(&mut vcx, |state, cx| {
        state.toggle_conversation_subagents(id, cx)
    });
    vcx.run_until_parked();
    assert!(
        vcx.debug_bounds("agent-switcher-panel").is_none(),
        "clicking the header again collapses it"
    );

    // What the delegate runs as: the panel's row says it on hover, the reading strip says it above
    // the transcript, and both read the one `SubagentTab` field through `short_model_label` — so
    // one conversation never spells the same model two ways.
    fixture.update(
        id,
        4,
        ConvUpdate::AgentChunk {
            content: ConvContent::Text("And again.".to_string()),
            message_id: Some("m10".to_string()),
            subagent: Some(Subagent {
                id: "t751".to_string(),
                kind: Some("general-purpose".to_string()),
                model: Some("claude-haiku-4-5-20251001".to_string()),
                thinking: None,
            }),
        },
        cx,
    );
    vcx.run_until_parked();

    let (tip, short) = fixture.state.read_with(&vcx, |state, cx| {
        let live = state.conversation(id, cx).expect("the conversation");
        let tab = live.subagents().into_iter().next().expect("the delegate");
        let short = short_model_label(&live.harness, tab.model.as_deref().unwrap());
        (conversation::subagent_tip(live, &tab), short)
    });
    assert_eq!(
        tip, "Formal greeting agent — general-purpose · haiku",
        "the row's hover names the kind and the model, and no thinking level nobody stated"
    );
    assert!(
        tip.contains(&short),
        "the strip shortens the same field the same way"
    );

    fixture.state.update(&mut vcx, |state, cx| {
        state.view_conversation_agent(id, Some("t751".to_string()), cx)
    });
    vcx.run_until_parked();
    assert!(
        vcx.debug_bounds("reading-strip").is_some(),
        "reading a delegate names it, and what it is running as, above the transcript"
    );

    // And switching is the conversation's own state, so two conversations on screen are read
    // independently.
    fixture.state.update(&mut vcx, |state, cx| {
        state.view_conversation_agent(id, Some("t751".to_string()), cx)
    });
    let (viewing, visible) = fixture.state.read_with(&vcx, |state, cx| {
        let live = state.conversation(id, cx).expect("the conversation");
        (
            live.viewing_subagent().map(str::to_string),
            live.visible_blocks().len(),
        )
    });
    assert_eq!(viewing.as_deref(), Some("t751"));
    assert_eq!(
        visible, 2,
        "only what the subagent itself said, both lines of it"
    );
}

/// Up in an empty composer brings the last turn back; up in one being written leaves it alone.
#[gpui::test]
fn up_in_an_empty_composer_recalls_the_last_turn(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let id = AgentId::generate();
    fixture.started(an_agent(id), cx);
    fixture.update(
        id,
        1,
        ConvUpdate::UserChunk {
            content: ConvContent::Text("look at the parser".to_string()),
            message_id: Some("u1".to_string()),
        },
        cx,
    );

    let recalled = fixture
        .window
        .update(cx, |_, window, cx| {
            fixture
                .state
                .update(cx, |state, cx| state.recall_last_message(0, window, cx))
        })
        .expect("the window is open");
    assert!(recalled, "the last turn never came back");
    assert_eq!(
        fixture
            .state
            .read_with(cx, |state, cx| state.column_inputs[0]
                .read(cx)
                .value()
                .to_string()),
        "look at the parser"
    );

    // A draft in the field is work in progress, and the key that would overwrite it does nothing.
    let recalled = fixture
        .window
        .update(cx, |_, window, cx| {
            fixture.state.update(cx, |state, cx| {
                state.column_inputs[0].update(cx, |input, cx| {
                    input.set_value("half a thought", window, cx);
                });
                state.recall_last_message(0, window, cx)
            })
        })
        .expect("the window is open");
    assert!(!recalled, "a draft was overwritten by the recall");
}

/// One `READ`, at whichever status the run has reached.
fn a_read(id: &str, status: ToolStatus) -> ConvUpdate {
    ConvUpdate::ToolCall(ToolCallRecord {
        id: id.to_string(),
        title: format!("read {id}"),
        kind: ToolKind::Read,
        status,
        content: Vec::new(),
        locations: Vec::new(),
        subagent: None,
    })
}

/// A run of same-kind calls is folded, and the fold holds the last card out only while that call
/// is still going: a call in flight is the one part of the run a reader is following, and a
/// finished run has nothing left to follow, so the row becomes the whole of it.
#[gpui::test]
fn a_finished_run_of_calls_folds_whole_and_a_running_one_keeps_its_last_card(
    cx: &mut TestAppContext,
) {
    let fixture = Fixture::open(cx);
    let id = AgentId::generate();
    fixture.started(an_agent(id), cx);
    fixture.update(id, 1, a_read("t1", ToolStatus::Completed), cx);
    fixture.update(id, 2, a_read("t2", ToolStatus::Completed), cx);
    fixture.update(id, 3, a_read("t3", ToolStatus::InProgress), cx);

    // The card selectors are block indices, so the transcript's shape is pinned before it is read.
    assert_eq!(
        fixture.state.read_with(cx, |state, cx| state
            .conversation(id, cx)
            .expect("the conversation")
            .blocks
            .len()),
        3,
        "three calls, three blocks, indices 0 through 2"
    );

    let window = cx.add_window(|_, _cx| ConversationHarness {
        state: fixture.state.clone(),
        agent: id,
        header: false,
        footer: false,
        composer: false,
    });
    cx.run_until_parked();
    let mut vcx = VisualTestContext::from_window(window.into(), cx);

    assert!(
        vcx.debug_bounds("tool-group-t1-2-earlier-calls").is_some(),
        "the two calls behind the one in flight are the row, and the row says so"
    );
    assert!(
        vcx.debug_bounds("tool-card-2").is_some(),
        "the call still running stays on screen"
    );
    assert!(
        vcx.debug_bounds("tool-card-0").is_none() && vcx.debug_bounds("tool-card-1").is_none(),
        "the calls before it are under the fold"
    );

    // The harness stamps the last call done — the same patch the bridge sends, not a poke at the
    // record — and the run has no moving part left.
    fixture.update(
        id,
        4,
        ConvUpdate::ToolCallUpdate(ToolCallPatch {
            id: "t3".to_string(),
            status: Some(ToolStatus::Completed),
            ..ToolCallPatch::default()
        }),
        cx,
    );
    vcx.run_until_parked();

    assert!(
        vcx.debug_bounds("tool-group-t1-3-calls").is_some(),
        "a finished run is the one row, and it stands for all three — no `earlier` left to be \
         earlier than"
    );
    assert!(
        vcx.debug_bounds("tool-card-2").is_none(),
        "the last card joins the fold the moment it finishes"
    );
    assert!(
        vcx.debug_bounds("tool-group-t1-2-earlier-calls").is_none(),
        "and the row that stood for part of the run is gone with it"
    );
}

/// Two is not a run. Folding it would replace a card with a row of the same height and one more
/// thing to learn, so both cards are drawn whatever their status.
#[gpui::test]
fn two_same_kind_calls_are_left_as_they_are(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let id = AgentId::generate();
    fixture.started(an_agent(id), cx);
    fixture.update(id, 1, a_read("t1", ToolStatus::Completed), cx);
    fixture.update(id, 2, a_read("t2", ToolStatus::Completed), cx);

    let window = cx.add_window(|_, _cx| ConversationHarness {
        state: fixture.state.clone(),
        agent: id,
        header: false,
        footer: false,
        composer: false,
    });
    cx.run_until_parked();
    let mut vcx = VisualTestContext::from_window(window.into(), cx);

    assert!(
        vcx.debug_bounds("tool-group-t1-2-calls").is_none(),
        "nothing was folded"
    );
    assert!(
        vcx.debug_bounds("tool-card-0").is_some() && vcx.debug_bounds("tool-card-1").is_some(),
        "both calls are drawn as themselves"
    );
}

/// Which folded runs are open is the conversation's own state, so a run shown stays shown while
/// the transcript grows around it, and a second click puts it back.
#[gpui::test]
fn a_folded_run_opens_and_shuts_on_its_first_call_id(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let id = AgentId::generate();
    fixture.started(an_agent(id), cx);
    fixture.update(id, 1, a_read("t1", ToolStatus::Completed), cx);
    fixture.update(id, 2, a_read("t2", ToolStatus::Completed), cx);
    fixture.update(id, 3, a_read("t3", ToolStatus::Completed), cx);

    let open = |cx: &mut TestAppContext| {
        fixture.state.read_with(cx, |state, cx| {
            state
                .conversation(id, cx)
                .expect("the conversation")
                .open_groups
                .contains("t1")
        })
    };
    assert!(!open(cx), "a run arrives folded");

    fixture.state.update(cx, |state, cx| {
        state.toggle_conversation_tool_group(id, "t1".to_string(), cx)
    });
    assert!(open(cx), "the chevron shows the calls the row stands for");

    fixture.state.update(cx, |state, cx| {
        state.toggle_conversation_tool_group(id, "t1".to_string(), cx)
    });
    assert!(!open(cx), "and puts them back");
}

// ── how tall a composer is ────────────────────────────────────────

/// The grip on the composer's top edge sizes the field: up is more writing space, down is less,
/// and every step is measured from where the drag went down rather than from the last frame — a
/// drag that outruns the pointer must not drift.
#[gpui::test]
fn dragging_the_composer_grip_sizes_the_field_from_where_it_went_down(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let rows = |cx: &mut TestAppContext| {
        fixture
            .state
            .read_with(cx, |state, _| state.composer_rows[0])
    };

    assert_eq!(
        rows(cx),
        None,
        "an untouched composer grows with what is typed"
    );

    // A pointer crossing the view with no drag live is somebody else's gesture.
    fixture
        .state
        .update(cx, |state, cx| state.drag_composer_resize(100.0, cx));
    assert_eq!(rows(cx), None, "a drag nobody started resized a field");

    // An empty field nobody has sized draws one row, and that is the height the drag counts from:
    // a grip that jumped to some other size on the first pixel would be a size nobody asked for.
    fixture
        .state
        .update(cx, |state, cx| state.start_composer_resize(0, 500.0, cx));
    fixture
        .state
        .update(cx, |state, cx| state.drag_composer_resize(460.0, cx));
    assert_eq!(
        rows(cx),
        Some(COMPOSER_ROWS_MIN + 2),
        "two rows' worth of pointer is two more rows of writing space"
    );

    // The same position again is the same answer: the height is a function of where the pointer
    // is, not of how many moves it took to get there.
    fixture
        .state
        .update(cx, |state, cx| state.drag_composer_resize(460.0, cx));
    assert_eq!(
        rows(cx),
        Some(COMPOSER_ROWS_MIN + 2),
        "the drag accumulated instead of measuring from where it began"
    );

    fixture
        .state
        .update(cx, |state, cx| state.end_composer_resize(cx));
    assert!(
        fixture
            .state
            .read_with(cx, |state, _| state.composer_drag.is_none()),
        "the release left a drag live for the next pointer that crosses the view"
    );
    fixture
        .state
        .update(cx, |state, cx| state.drag_composer_resize(100.0, cx));
    assert_eq!(
        rows(cx),
        Some(COMPOSER_ROWS_MIN + 2),
        "a move after the release still resized the field"
    );

    // And downward, from a field with rows to lose — the same arithmetic, counted from the size
    // this one already stands at rather than from the one row an empty field draws.
    fixture
        .state
        .update(cx, |state, cx| state.set_composer_rows(0, Some(8), cx));
    fixture
        .state
        .update(cx, |state, cx| state.start_composer_resize(0, 500.0, cx));
    fixture
        .state
        .update(cx, |state, cx| state.drag_composer_resize(560.0, cx));
    assert_eq!(rows(cx), Some(5), "three rows down is three rows shorter");
}

/// A drag past either end stops at it. The floor is a field with a row in it; the ceiling is what
/// leaves a transcript on screen.
#[gpui::test]
fn a_drag_past_either_end_stops_at_the_bounds(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let rows = |cx: &mut TestAppContext| {
        fixture
            .state
            .read_with(cx, |state, _| state.composer_rows[0])
    };

    fixture
        .state
        .update(cx, |state, cx| state.start_composer_resize(0, 500.0, cx));
    fixture
        .state
        .update(cx, |state, cx| state.drag_composer_resize(-5_000.0, cx));
    assert_eq!(rows(cx), Some(COMPOSER_ROWS_MAX));

    fixture
        .state
        .update(cx, |state, cx| state.drag_composer_resize(5_000.0, cx));
    assert_eq!(rows(cx), Some(COMPOSER_ROWS_MIN));
}

/// A double-click on the grip hands the field back to growing with what is typed — a size dragged
/// by hand needs a way back that is not dragging it to exactly where it was. What it goes back to
/// is the pool's own behaviour, so the next drag counts from the row an empty field draws rather
/// than from the size it was reset out of.
#[gpui::test]
fn resetting_a_composer_returns_it_to_growing_with_what_is_typed(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let rows = |cx: &mut TestAppContext| {
        fixture
            .state
            .read_with(cx, |state, _| state.composer_rows[0])
    };

    fixture
        .state
        .update(cx, |state, cx| state.start_composer_resize(0, 500.0, cx));
    fixture
        .state
        .update(cx, |state, cx| state.drag_composer_resize(400.0, cx));
    fixture
        .state
        .update(cx, |state, cx| state.end_composer_resize(cx));
    assert_eq!(rows(cx), Some(COMPOSER_ROWS_MIN + 5));

    fixture
        .state
        .update(cx, |state, cx| state.set_composer_rows(0, None, cx));
    assert_eq!(rows(cx), None, "the field is the pool's own again");

    fixture
        .state
        .update(cx, |state, cx| state.start_composer_resize(0, 500.0, cx));
    fixture
        .state
        .update(cx, |state, cx| state.drag_composer_resize(480.0, cx));
    assert_eq!(
        rows(cx),
        Some(COMPOSER_ROWS_MIN + 1),
        "a reset field was dragged from the size it was reset out of"
    );
}
