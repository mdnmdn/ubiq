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
use ubiq::state::WindowRegistry;
use ubiq::state::conversation::Run;
use ubiq::ui::conversation::{self, ConversationView};
use ubiq_proto::bus::{self, FromClient, To};
use ubiq_proto::conversation::{
    ConfigCategory, ConfigChoice, ConfigOption, ConfigValue, ConvContent, ConvUpdate, StopReason,
    ToolCallRecord, ToolKind, ToolStatus, UsageRecord,
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
            no_local_index: false,
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

/// Picking a harness asks the host to start a conversation at once, in the same turn — naming is
/// the host's, so there is no prompt in between. The id is what crosses, never the label.
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
        state.pick_new_agent_menu(0, cx);
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
        state.pick_new_agent_menu(1, cx);
    });
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
/// only while a turn runs, Unload only while launched, Resume only while it is not, Delete always.
#[test]
fn the_lifecycle_menu_disables_resume_while_launched_and_unload_once_it_is_not() {
    use ubiq::state::conversation::Conversation;
    use ubiq::ui::conversation::lifecycle_menu_enabled;

    let mut conversation = Conversation::new(
        AgentId::generate(),
        "Claude Code".to_string(),
        String::new(),
    );

    // Freshly launched: running a turn, Unload applies, Resume does not.
    conversation.launched = true;
    conversation.run = Run::Working;
    let [stop, unload, resume, delete] = lifecycle_menu_enabled(&conversation);
    assert!(stop, "a turn is running");
    assert!(unload, "the harness is up");
    assert!(!resume, "already launched");
    assert!(delete, "always enabled");

    // Unloaded: no turn to stop, nothing to unload, Resume is what applies now.
    conversation.launched = false;
    conversation.run = Run::Idle;
    let [stop, unload, resume, delete] = lifecycle_menu_enabled(&conversation);
    assert!(!stop, "nothing is running");
    assert!(!unload, "there is no harness to unload");
    assert!(resume, "not launched");
    assert!(delete, "always enabled");
}

/// The lifecycle glyph's own rule, pulled out the same way the menu's is: derived from
/// `launched`, `run`, `blocks`, `accepts_input` and `config` alone, with nothing new stored on
/// `Conversation` for it.
#[test]
fn the_lifecycle_glyph_reads_launched_run_and_the_transcript() {
    use ubiq::state::conversation::{ConvBlock, Conversation};
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
    c.blocks
        .push(ConvBlock::Agent("said something".to_string()));
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

/// A window whose only content is one conversation, drawn with whichever `header` the test wants —
/// standing in for the agents column (`header: true`) and the chat panel (`header: false`) without
/// dragging in either surface's own dock plumbing.
struct ConversationHarness {
    state: Entity<AppState>,
    agent: AgentId,
    header: bool,
}

impl Render for ConversationHarness {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl gpui::IntoElement {
        let agent = self.agent;
        let view = ConversationView {
            id: SharedString::from("conversation-harness"),
            slot: 0,
            footer: false,
            composer: false,
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
    });
    cx.run_until_parked();
    let mut vcx = VisualTestContext::from_window(without_header.into(), cx);
    assert!(
        vcx.debug_bounds("lifecycle-strip").is_none(),
        "header: false must not draw the strip — the chat panel draws it inline instead"
    );
}
