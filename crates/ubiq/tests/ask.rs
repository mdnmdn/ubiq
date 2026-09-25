//! An agent asking the user a structured question, from the host's message to the answer.
//!
//! The claim under test is the one the interface half of the feature is: an ask is a record on the
//! conversation that raised it, and the dialog is only a view over one. Everything else follows
//! from that — the entry is written whether or not the dialog opened, closing keeps the drafts,
//! and confirming is the one thing that sends.
//!
//! The window is the chat tests' own: one project, one host end to speak as, and a conversation
//! the host says is up. "Shown" is arranged by attaching a chat tab to the agent, which is one of
//! the two readings `conversation_shown` has.

use std::cell::RefCell;
use std::rc::Rc;

use chrono::Utc;
use gpui::{AppContext as _, Entity, TestAppContext, WindowHandle};
use gpui_component::Root;
use ubiq::app::{AppState, BusHub, DialogCancel};
use ubiq::state::WindowRegistry;
use ubiq::state::ask::AskStage;
use ubiq_proto::ask::{AskClosed, AskOption, AskOutcome, AskQuestion};
use ubiq_proto::bus::{self, FromClient, To};
use ubiq_proto::ids::{AskId, ProjectId};
use ubiq_proto::messages::Message;
use ubiq_proto::projects::{ProjectHealth, ProjectRecord, ProjectSnapshot};
use ubiq_proto::work::{Activity, AgentId, WorkAgent, WorkSession};

/// How long a drain of the host's inbox waits for one more message before calling it empty.
const PATIENCE: std::time::Duration = std::time::Duration::from_millis(500);

struct Fixture {
    state: Entity<AppState>,
    window: WindowHandle<Root>,
    host: bus::HostEnd,
    project: ProjectId,
    agent: AgentId,
}

impl Fixture {
    /// A window on one project, with one conversation the host has said is up.
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
        let agent = AgentId::generate();
        let fixture = Self {
            state,
            window,
            host,
            project,
            agent,
        };
        fixture.started(an_agent(agent, "Fixer"), cx);
        fixture
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

    /// Put the conversation on screen, by attaching the default chat tab to it — one of the two
    /// readings of "this conversation is being drawn".
    fn show(&self, cx: &mut TestAppContext) {
        let agent = self.agent;
        let tab = self
            .state
            .read_with(cx, |state, cx| state.open_project(cx).unwrap().chats[0].id);
        self.state
            .update(cx, |state, cx| state.attach_chat(tab, Some(agent), cx));
        cx.run_until_parked();
    }

    /// Take the conversation off screen again — both readings at once: the agents screen gave it a
    /// column of its own the moment the host said it was up, so benching it is half of hiding it,
    /// and a chat tab attached to nothing is the other half.
    fn hide(&self, cx: &mut TestAppContext) {
        let agent = self.agent;
        let tab = self
            .state
            .read_with(cx, |state, cx| state.open_project(cx).unwrap().chats[0].id);
        self.state.update(cx, |state, cx| {
            state.attach_chat(tab, None, cx);
            state.bench_agent(agent, cx);
        });
        cx.run_until_parked();
    }

    /// The host raises a question, exactly as a parked tool call does.
    fn asks(&self, ask_id: AskId, questions: Vec<AskQuestion>, cx: &mut TestAppContext) {
        self.host.send(
            To::Everyone,
            Message::AskUser {
                agent_id: self.agent,
                ask_id,
                questions,
            },
        );
        cx.run_until_parked();
    }

    /// Anything that needs a window on the stack — every control the dialog draws does.
    fn with_window(
        &self,
        cx: &mut TestAppContext,
        f: impl FnOnce(&mut AppState, &mut gpui::Window, &mut gpui::Context<AppState>),
    ) {
        let state = self.state.clone();
        self.window
            .update(cx, |_, window, cx| {
                state.update(cx, |state, cx| f(state, window, cx));
            })
            .expect("the window is up");
        cx.run_until_parked();
    }

    /// Everything this window has said to the host, drained.
    fn said(&self) -> Vec<Message> {
        let mut said = Vec::new();
        while let Ok(event) = self.host.recv_timeout(PATIENCE) {
            if let FromClient::Said { message, .. } = event {
                said.push(message);
            }
        }
        said
    }

    /// The ask records filed on the conversation.
    fn entries(&self, cx: &mut TestAppContext) -> Vec<(AskId, AskStage)> {
        self.state.read_with(cx, |state, cx| {
            state
                .open_project(cx)
                .unwrap()
                .conversations
                .get(&self.agent)
                .map(|conversation| {
                    conversation
                        .asks
                        .iter()
                        .map(|ask| (ask.ask_id, ask.stage.clone()))
                        .collect()
                })
                .unwrap_or_default()
        })
    }

    fn dialog_is_up(&self, cx: &mut TestAppContext) -> bool {
        self.state
            .read_with(cx, |state, _| state.workbench.ask.is_some())
    }
}

/// Nothing in this window holds that agent — the project was closed between the tool call and the
/// push — so the window says so rather than leaving the harness parked until the host's timeout.
#[gpui::test]
fn an_ask_for_an_agent_this_window_does_not_hold_says_it_is_gone(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    fixture.show(cx);
    let _ = fixture.said();

    let stranger = AgentId::generate();
    let ask_id = AskId::generate();
    fixture.host.send(
        To::Everyone,
        Message::AskUser {
            agent_id: stranger,
            ask_id,
            questions: vec![a_question("Direction", false)],
        },
    );
    cx.run_until_parked();

    assert!(!fixture.dialog_is_up(cx), "there is nothing to draw it in");
    assert!(fixture.entries(cx).is_empty(), "and nothing to file it on");
    let ended = fixture
        .said()
        .into_iter()
        .find_map(|message| match message {
            Message::AskEnded { agent_id, why, .. } => Some((agent_id, why)),
            _ => None,
        })
        .expect("the window told the host nobody will answer");
    assert_eq!(ended, (stranger, AskClosed::Gone));
}

/// The conversation is deleted under an open dialog. The record went with it, so the dialog goes
/// too — a modal layer with nothing in it would block every later ask and could not be dismissed.
#[gpui::test]
fn deleting_the_conversation_takes_the_dialog_with_it(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    fixture.show(cx);

    let ask_id = AskId::generate();
    fixture.asks(ask_id, vec![a_question("Direction", false)], cx);
    assert!(fixture.dialog_is_up(cx));
    let _ = fixture.said();

    fixture.host.send(
        To::Everyone,
        Message::ConversationDeleted {
            agent_id: fixture.agent,
        },
    );
    cx.run_until_parked();

    assert!(
        !fixture.dialog_is_up(cx),
        "the dialog went with the record it was drawing"
    );
    assert!(
        fixture
            .state
            .read_with(cx, |state, _| state.top_layer())
            .is_none(),
        "and the modal stack is clear"
    );

    // Which is what lets the next one open: a layer left behind would downgrade it to a bell and
    // make `show_ask` refuse.
    let other = AgentId::generate();
    fixture.started(an_agent(other, "Other"), cx);
    let next = AskId::generate();
    fixture.host.send(
        To::Everyone,
        Message::AskUser {
            agent_id: other,
            ask_id: next,
            questions: vec![a_question("Direction", false)],
        },
    );
    cx.run_until_parked();
    fixture.with_window(cx, move |state, window, cx| {
        state.show_ask(other, next, window, cx)
    });
    assert!(fixture.dialog_is_up(cx), "the next ask opened a dialog");
}

/// The footer's note and Confirm read the same question: "Other" picked with nothing under it is
/// not an answer, and the note must not claim the agent has everything it is waiting for.
#[gpui::test]
fn an_empty_other_is_unanswered_to_the_footer_as_well_as_to_confirm(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    fixture.show(cx);

    let ask_id = AskId::generate();
    fixture.asks(ask_id, vec![a_question("Direction", false)], cx);
    fixture.with_window(cx, |state, _, cx| state.toggle_ask_option(2, cx));

    let (answered, ready) = fixture.state.read_with(cx, |state, cx| {
        let record = state.ask_record(fixture.agent, ask_id, cx).unwrap();
        (record.answered(0), record.ready())
    });
    assert!(!answered, "an empty Other answers nothing");
    assert!(!ready, "which is why Confirm is dim");

    // And words under it settle both at once.
    fixture.with_window(cx, |state, _, cx| {
        state.retype_ask_other("straight on".to_string(), cx)
    });
    let (answered, ready) = fixture.state.read_with(cx, |state, cx| {
        let record = state.ask_record(fixture.agent, ask_id, cx).unwrap();
        (record.answered(0), record.ready())
    });
    assert!(answered && ready, "one reading, both surfaces");
}

fn option(label: &str) -> AskOption {
    AskOption {
        label: label.to_string(),
        description: format!("what {label} means"),
        preview: None,
    }
}

fn a_question(header: &str, multi: bool) -> AskQuestion {
    AskQuestion {
        question: format!("Which way for {header}?"),
        header: header.to_string(),
        options: vec![option("Left"), option("Right")],
        multi_select: multi,
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

/// The conversation is on screen and nothing else is up, so the question is asked where the user
/// is already looking — and the entry is written all the same, because the dialog is a view and
/// the record is the ask.
#[gpui::test]
fn an_ask_for_a_shown_conversation_opens_the_dialog(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    fixture.show(cx);
    let _ = fixture.said();

    let ask_id = AskId::generate();
    fixture.asks(ask_id, vec![a_question("Direction", false)], cx);

    assert!(fixture.dialog_is_up(cx), "the dialog opened in place");
    let entries = fixture.entries(cx);
    assert_eq!(entries.len(), 1, "the entry is written either way");
    assert_eq!(entries[0], (ask_id, AskStage::Waiting));
    assert!(
        fixture.said().is_empty(),
        "raising a question sends nothing back on its own"
    );
}

/// Nobody is looking at that conversation, so the bell is what says so — and the entry is what
/// makes the question findable afterwards.
#[gpui::test]
fn an_ask_for_a_conversation_nobody_is_reading_raises_a_notification(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    fixture.hide(cx);
    let _ = fixture.said();

    let ask_id = AskId::generate();
    fixture.asks(ask_id, vec![a_question("Direction", false)], cx);

    assert!(
        !fixture.dialog_is_up(cx),
        "an unread conversation's question does not take the screen"
    );
    assert_eq!(fixture.entries(cx), vec![(ask_id, AskStage::Waiting)]);

    let raised: Vec<_> = fixture
        .said()
        .into_iter()
        .filter_map(|message| match message {
            Message::RaiseNotification { request } => Some(request),
            _ => None,
        })
        .collect();
    assert_eq!(raised.len(), 1, "exactly one bell for one question");
    assert_eq!(raised[0].category.as_deref(), Some("ask"));
    assert_eq!(raised[0].actor.as_deref(), Some("Fixer"));
    assert!(
        raised[0].text.contains("Direction"),
        "the bell names the headers: {}",
        raised[0].text
    );
}

/// A dialog already up counts as "the agent is not visible": whatever the user is doing, taking it
/// away to ask them something else is the interruption the rule exists to prevent.
#[gpui::test]
fn a_second_ask_never_replaces_the_dialog_already_up(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    fixture.show(cx);

    let first = AskId::generate();
    fixture.asks(first, vec![a_question("First", false)], cx);
    assert!(fixture.dialog_is_up(cx));
    let _ = fixture.said();

    let second = AskId::generate();
    fixture.asks(second, vec![a_question("Second", false)], cx);

    let up = fixture
        .state
        .read_with(cx, |state, _| state.workbench.ask.unwrap().ask_id);
    assert_eq!(up, first, "the first question keeps the screen");
    assert_eq!(
        fixture.entries(cx).len(),
        2,
        "two asks, two records — never one form"
    );
    assert!(
        fixture
            .said()
            .iter()
            .any(|message| matches!(message, Message::RaiseNotification { .. })),
        "the second one rang the bell instead"
    );
}

/// Escape puts the dialog away and sends nothing; what was filled in is the record's, so the
/// transcript entry's button reopens exactly what was there.
#[gpui::test]
fn closing_the_dialog_keeps_the_draft_and_reopening_restores_it(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    fixture.show(cx);

    let ask_id = AskId::generate();
    fixture.asks(ask_id, vec![a_question("Direction", false)], cx);
    let _ = fixture.said();

    // Pick "Right", then walk away from the question.
    fixture.with_window(cx, |state, _, cx| state.toggle_ask_option(1, cx));
    fixture.with_window(cx, |state, window, cx| {
        state.cancel_dialog(&DialogCancel, window, cx)
    });
    assert!(!fixture.dialog_is_up(cx), "Escape took the dialog away");
    assert!(
        fixture.said().is_empty(),
        "dismissing is not answering — nothing left the window"
    );

    let agent = fixture.agent;
    fixture.with_window(cx, move |state, window, cx| {
        state.show_ask(agent, ask_id, window, cx)
    });
    assert!(fixture.dialog_is_up(cx), "the entry's button reopened it");
    let chosen = fixture.state.read_with(cx, |state, cx| {
        state.open_ask(cx).unwrap().1.drafts[0].chosen.clone()
    });
    assert_eq!(chosen, vec![1], "the pick survived the dialog");
}

/// Confirm is what sends, and it sends labels rather than positions — with the free text riding
/// beside the picks rather than as one of them.
#[gpui::test]
fn confirming_sends_the_picks_and_makes_the_entry_read_only(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    fixture.show(cx);

    let ask_id = AskId::generate();
    fixture.asks(
        ask_id,
        vec![a_question("Direction", true), a_question("Speed", false)],
        cx,
    );
    let _ = fixture.said();

    // Both options of the multi-select question, then Other on the second — which needs words
    // under it before Confirm does anything.
    fixture.with_window(cx, |state, _, cx| state.toggle_ask_option(0, cx));
    fixture.with_window(cx, |state, _, cx| state.toggle_ask_option(1, cx));
    fixture.with_window(cx, |state, window, cx| state.set_ask_tab(1, window, cx));
    fixture.with_window(cx, |state, _, cx| state.toggle_ask_option(2, cx));

    fixture.with_window(cx, |state, window, cx| state.confirm_ask(window, cx));
    assert!(
        fixture.dialog_is_up(cx),
        "an empty Other leaves Confirm doing nothing"
    );

    fixture.with_window(cx, |state, _, cx| {
        state.retype_ask_other("straight on".to_string(), cx)
    });
    fixture.with_window(cx, |state, _, cx| {
        state.renote_ask("as fast as is safe".to_string(), cx)
    });
    fixture.with_window(cx, |state, window, cx| state.confirm_ask(window, cx));

    assert!(!fixture.dialog_is_up(cx), "confirming closes it");
    let answers = match fixture
        .said()
        .into_iter()
        .find_map(|message| match message {
            Message::AnswerAsk { outcome, .. } => Some(outcome),
            _ => None,
        }) {
        Some(AskOutcome::Answered(answers)) => answers,
        other => panic!("expected an answer, got {other:?}"),
    };
    assert_eq!(answers.len(), 2);
    assert_eq!(answers[0].chosen, vec!["Left", "Right"]);
    assert_eq!(answers[0].other, None);
    assert!(answers[1].chosen.is_empty(), "only Other was picked");
    assert_eq!(answers[1].other.as_deref(), Some("straight on"));
    assert_eq!(answers[1].notes.as_deref(), Some("as fast as is safe"));

    // And the record says so, which is what makes every later opening a transcript.
    let stage = fixture.entries(cx).remove(0).1;
    assert!(matches!(stage, AskStage::Answered(_)));
    let live = fixture.state.read_with(cx, |state, cx| {
        state.ask_record(fixture.agent, ask_id, cx).unwrap().live()
    });
    assert!(!live, "an answered ask can no longer be answered");
}

/// "Chat about this" is a real outcome, not a cancel: the tool call ends and the agent is told the
/// user would rather talk.
#[gpui::test]
fn chatting_about_it_ends_the_ask_with_no_answer(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    fixture.show(cx);

    let ask_id = AskId::generate();
    fixture.asks(ask_id, vec![a_question("Direction", false)], cx);
    let _ = fixture.said();

    fixture.with_window(cx, |state, window, cx| state.chat_about_ask(window, cx));

    assert!(!fixture.dialog_is_up(cx));
    let outcome = fixture
        .said()
        .into_iter()
        .find_map(|message| match message {
            Message::AnswerAsk { outcome, .. } => Some(outcome),
            _ => None,
        })
        .expect("the outcome went back");
    assert_eq!(outcome, AskOutcome::Chat);
    assert_eq!(fixture.entries(cx), vec![(ask_id, AskStage::Chatted)]);

    // And nothing can be sent twice: the record has left `Waiting`.
    fixture.with_window(cx, |state, window, cx| state.chat_about_ask(window, cx));
    assert!(
        fixture.said().is_empty(),
        "an ask that has ended sends nothing more"
    );
}

/// The host stopped waiting. The entry stays and becomes a record of what was asked, which is what
/// a reader coming back to it deserves instead of a control that would send into nothing.
#[gpui::test]
fn an_ask_the_host_ended_becomes_read_only(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    fixture.show(cx);

    let ask_id = AskId::generate();
    fixture.asks(ask_id, vec![a_question("Direction", false)], cx);
    fixture.with_window(cx, |state, _, cx| state.toggle_ask_option(0, cx));
    let _ = fixture.said();

    fixture.host.send(
        To::Everyone,
        Message::AskEnded {
            agent_id: fixture.agent,
            ask_id,
            why: AskClosed::Timeout,
        },
    );
    cx.run_until_parked();

    assert_eq!(
        fixture.entries(cx),
        vec![(ask_id, AskStage::Ended(AskClosed::Timeout))]
    );
    assert!(
        fixture.dialog_is_up(cx),
        "the dialog stays up, now a record of what happened"
    );

    // Every control is inert from here, and Confirm least of all sends.
    fixture.with_window(cx, |state, _, cx| state.toggle_ask_option(1, cx));
    fixture.with_window(cx, |state, window, cx| state.confirm_ask(window, cx));
    let chosen = fixture.state.read_with(cx, |state, cx| {
        state.open_ask(cx).unwrap().1.drafts[0].chosen.clone()
    });
    assert_eq!(chosen, vec![0], "what was picked before it ended is frozen");
    assert!(
        fixture.said().is_empty(),
        "an ask the host ended sends nothing"
    );
}

// ── whose transcript the entry belongs to ───────────────────────────

/// One update from the conversation itself, or from something it spawned.
fn a_chunk(
    fixture: &Fixture,
    seq: u64,
    text: &str,
    subagent: Option<&str>,
    cx: &mut TestAppContext,
) {
    fixture.host.send(
        To::Everyone,
        Message::ConversationUpdate {
            agent_id: fixture.agent,
            seq,
            update: Box::new(ubiq_proto::conversation::ConvUpdate::AgentChunk {
                content: ubiq_proto::conversation::ConvContent::Text(text.to_string()),
                message_id: Some(format!("m{seq}")),
                subagent: subagent.map(|id| ubiq_proto::conversation::Subagent {
                    id: id.to_string(),
                    kind: None,
                    model: None,
                    thinking: None,
                }),
            }),
            raw: None,
        },
    );
    cx.run_until_parked();
}

/// Which transcript the record says it belongs in.
fn filed_against(fixture: &Fixture, cx: &mut TestAppContext) -> Vec<Option<String>> {
    fixture.state.read_with(cx, |state, cx| {
        state
            .open_project(cx)
            .unwrap()
            .conversations
            .get(&fixture.agent)
            .map(|conversation| {
                conversation
                    .asks
                    .iter()
                    .map(|ask| ask.subagent.clone())
                    .collect()
            })
            .unwrap_or_default()
    })
}

/// **The ask belongs to the agent that asked it, and to no other.** A conversation and every
/// subagent it spawns share one `AgentId`, so an ask with nobody named on it is drawn in the main
/// transcript and in every delegate's at once — the same question wearing several faces.
#[gpui::test]
fn an_ask_is_filed_against_the_agent_that_raised_it(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    fixture.show(cx);

    // The conversation's own turn asks: nobody is named, which is the main transcript.
    a_chunk(&fixture, 1, "thinking about it", None, cx);
    fixture.asks(AskId::generate(), vec![a_question("Direction", false)], cx);
    assert_eq!(filed_against(&fixture, cx), vec![None]);

    // A delegate is speaking when the next one lands, so that one is the delegate's.
    fixture.with_window(cx, |state, window, cx| state.close_ask(window, cx));
    a_chunk(&fixture, 2, "the delegate reports", Some("t1"), cx);
    fixture.asks(AskId::generate(), vec![a_question("Approach", false)], cx);
    assert_eq!(
        filed_against(&fixture, cx),
        vec![None, Some("t1".to_string())],
    );
}
