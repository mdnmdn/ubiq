//! The `ubiq-ask` server: two ways to put a question to the person watching.
//!
//! Every other built-in tool answers from a fact the host already holds. These two do not.
//!
//! **`ask_user_question` parks.** It puts a question on screen and waits until the person answers
//! it, says they would rather talk, or the wait's bound passes — [`crate::ask::Asks`] holds the
//! call meanwhile, and `D138` is why that table exists at all. **It never runs on the listener's
//! thread**: [`super::server::handle`] moves the request onto a thread of its own before calling
//! in here, because every other agent's tool calls come through that one listener and a parked
//! call would hold all of them up.
//!
//! **The bound is short, and giving up is a result rather than a failure** (`D191`, `G360`).
//! Every harness in front of this call keeps a tool timeout of its own and every one of them is
//! shorter than an hour, so a call that waits an hour loses that race and the model reads an
//! opaque harness failure while the user is still deciding. So a silent call waits
//! [`ubiq_proto::ask::ASK_PARK_SECS`] — under the shortest tool timeout this tree knows of — and
//! returns `timedOut` as an ordinary result the model can act on, while the dialog stays on
//! screen and its row is handed to [`crate::armed::Armed`] so the answer still arrives, as the
//! next turn. A call whose client asked for MCP progress notifications gets the patient
//! [`ubiq_proto::ask::ASK_TIMEOUT_SECS`] instead, because its timer is being reset while it waits;
//! that arm lives in [`super::server`] and degrades to this one on any client that does not ask.
//!
//! **`register_question` does not.** It files the same questions in [`crate::armed::Armed`] and
//! returns the minted id on the listener's own thread; the dialog is raised when the agent's turn
//! ends, and the user's answer arrives as the next turn rather than as this call's result. Nothing
//! waits, so no harness's own tool timeout can reach it (`D175`). The cost is that the agent must
//! stop after registering — the tool's description says so, because the dialog is only raised at
//! the turn boundary.
//!
//! **The wire shape is Claude Code's `AskUserQuestion`.** A harness that already knows how to ask
//! a structured question does not have to learn a second form of it, so the arguments below are
//! that tool's, `multiSelect` included — the one camelCase key in Ubiq's own MCP surface, accepted
//! as it is written rather than made a harness's problem. [`ubiq_proto::ask`] holds the rest of
//! the contract and the limits.
//!
//! **A malformed ask is refused before anything is parked.** A question with one option, a header
//! that will not fit a tab, five questions at once — none of those can be drawn, and raising them
//! at a user who cannot make sense of them is worse than telling the model what it got wrong. The
//! refusal is an in-band `isError`, which is the failure a model can read and correct.

use std::sync::mpsc::Receiver;
use std::time::Duration;

use serde::Deserialize;
use serde_json::{Value, json};
use ubiq_proto::ask::{ASK_PARK_SECS, AskAnswer, AskClosed, AskOption, AskOutcome, AskQuestion};
use ubiq_proto::bus::Voice;
use ubiq_proto::ids::AskId;
use ubiq_proto::messages::Message;
use ubiq_proto::work::AgentId;

use super::AskReach;
use super::registry::AgentFacts;
use crate::ask::Ending;

/// The arguments as the harness writes them: Claude Code's own shape, camelCase and all.
#[derive(Deserialize)]
struct Asking {
    questions: Vec<Asked>,
}

/// One question on the wire. `multiSelect` is the harness's spelling; `multi_select` is accepted
/// beside it so a harness using the proto's own naming is not turned away over a key.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Asked {
    question: String,
    header: String,
    options: Vec<AskOption>,
    #[serde(default, alias = "multi_select")]
    multi_select: bool,
}

impl From<Asked> for AskQuestion {
    fn from(asked: Asked) -> Self {
        Self {
            question: asked.question,
            header: asked.header,
            options: asked.options,
            multi_select: asked.multi_select,
        }
    }
}

pub fn call(
    tool: &str,
    arguments: &Value,
    facts: &AgentFacts,
    voice: &Voice,
    reach: &AskReach,
) -> Result<Value, String> {
    match tool {
        // The silent bound. The streaming arm does not come through here: it calls [`raise`] and
        // [`settle`] itself, with the patient bound, because it can say it is still alive.
        "ask_user_question" => ask(
            arguments,
            facts,
            voice,
            reach,
            Duration::from_secs(ASK_PARK_SECS),
        ),
        "register_question" => register(arguments, facts, reach),
        _ => Err(format!(
            "unknown tool: {}/{tool}",
            super::catalogue::UBIQ_ASK
        )),
    }
}

/// The questions and who is asking them, checked — everything both tools need before either does
/// anything of its own.
///
/// A malformed ask is refused here, before a call parks or a dialog is armed: the model is told
/// what is wrong and can correct it. A pane's key is a `PaneId` and names no conversation, so
/// there is no transcript to raise the question in and nothing to answer it with; that is refused
/// here too.
fn reading(
    tool: &str,
    arguments: &Value,
    facts: &AgentFacts,
) -> Result<(AgentId, Vec<AskQuestion>), String> {
    let asking: Asking = serde_json::from_value(arguments.clone())
        .map_err(|error| format!("{tool} could not read its arguments: {error}"))?;
    let questions: Vec<AskQuestion> = asking
        .questions
        .into_iter()
        .map(AskQuestion::from)
        .collect();
    ubiq_proto::ask::check(&questions)
        .map_err(|wrong| format!("this ask cannot be drawn: {wrong}"))?;
    let agent_id: AgentId = facts.key.parse().map_err(|_| {
        "only a conversation can ask the user a question; this agent is a terminal pane".to_string()
    })?;
    Ok((agent_id, questions))
}

/// Register a dialog for the end of this turn and return at once.
///
/// **The whole of the call.** No thread is spawned and nothing waits: the row goes into
/// [`crate::armed::Armed`], the conversation's pump raises it as `Message::AskUser` when the turn
/// ends, and what the user says comes back as the next turn's prompt. The handle returned exists
/// so the model can name its own registration; it is not an id it has to remember, because a
/// registration lives for one turn.
fn register(arguments: &Value, facts: &AgentFacts, reach: &AskReach) -> Result<Value, String> {
    let (agent_id, questions) = reading("register_question", arguments, facts)?;
    let ask_id = reach.armed.arm(agent_id, questions);
    Ok(json!({
        "registered": ask_id.to_string(),
        "summary": "Registered. End your turn now — say what you are waiting on and stop. The dialog is raised the moment this turn ends, and the user's answer arrives as your next turn.",
    }))
}

/// Raise the question, wait for the answer, and turn whatever ended it into a tool result.
///
/// The whole of the blocking arm: [`raise`] puts the dialog up, the table is waited on for
/// `bound`, and [`settle`] turns whatever ended it into a result. The streaming arm in
/// [`super::server`] is the same three steps with its own clock between the first and the last.
fn ask(
    arguments: &Value,
    facts: &AgentFacts,
    voice: &Voice,
    reach: &AskReach,
    bound: Duration,
) -> Result<Value, String> {
    let raised = raise(arguments, facts, voice, reach)?;
    let ending = reach.asks.wait_for(raised.ask_id, &raised.waiting, bound);
    settle(raised, ending, reach, bound)
}

/// A question that is on screen and a call that is waiting on it.
///
/// Returned by [`raise`] so a caller can run its own clock over the wait — the streaming arm does,
/// because it has progress notifications to write while it waits.
pub struct Raised {
    pub agent_id: AgentId,
    pub ask_id: AskId,
    questions: Vec<AskQuestion>,
    pub waiting: Receiver<Ending>,
}

/// Check the ask, park the call, and put the dialog on screen. Everything up to the waiting.
pub fn raise(
    arguments: &Value,
    facts: &AgentFacts,
    voice: &Voice,
    reach: &AskReach,
) -> Result<Raised, String> {
    let (agent_id, questions) = reading("ask_user_question", arguments, facts)?;

    let (ask_id, waiting) = reach.asks.raise(agent_id, &questions);
    // The coordinator alone knows which window owns the conversation, so the ask goes to it as an
    // ordinary bus fact and it does the addressing — the same route a notification takes from a
    // tool call. A conversation with no owner is ended there as `Gone`, which releases this wait
    // rather than leaving it to the bound.
    // No batch: a parked ask is raised on its own, mid-turn, and waits for nothing else. The
    // set that is answered together is the one a turn boundary raises (`G362`).
    voice.say(Message::AskUser {
        agent_id,
        ask_id,
        questions: questions.clone(),
        batch_at: 0,
        batch_of: 0,
    });

    Ok(Raised {
        agent_id,
        ask_id,
        questions,
        waiting,
    })
}

/// Turn whatever ended the wait into the tool result the model reads.
///
/// **A wait that gave up is not a failure.** It comes back as an ordinary result saying nobody
/// answered in time, not an `isError`: the model has something to do with that — carry on, or
/// stop and say what it is waiting on — whereas an error is what the harness's own timeout would
/// have produced, which is the failure this whole bound exists to get in front of (`G360`).
///
/// **And the question outlives the call.** The dialog is not taken down: the row is handed to
/// [`crate::armed::Armed`] under the same id, so when the user does answer, the answer arrives as
/// the agent's next turn through the arm-and-fire path rather than landing nowhere (`D191`). No
/// `AskEnded` goes out on this path — sending one would close the modal under the user mid-answer.
pub fn settle(
    raised: Raised,
    ending: Ending,
    reach: &AskReach,
    bound: Duration,
) -> Result<Value, String> {
    let Raised {
        agent_id,
        ask_id,
        questions,
        ..
    } = raised;
    match ending {
        Ok(AskOutcome::Answered(answers)) => Ok(answered(&questions, &answers)),
        Ok(AskOutcome::Chat) => Ok(json!({
            "answered": [],
            "chat": true,
            "summary": "The user would rather discuss this than pick an option. They are saying so in the chat: read what they write next and carry on from it, rather than asking again.",
        })),
        Err(AskClosed::Timeout) => {
            reach.armed.adopt(agent_id, ask_id, questions);
            Ok(json!({
                "answered": [],
                "chat": false,
                "timedOut": true,
                "summary": format!(
                    "Nobody answered within {} seconds, so this call is returning without an answer. \
                     The question is still on screen and the user can still answer it — if they do, \
                     their answer reaches you as your next turn. Carry on without it if you can; \
                     otherwise end your turn and say you are waiting on this question.",
                    bound.as_secs(),
                ),
            }))
        }
        Err(AskClosed::Gone) => Err(
            "this question was closed because the conversation that asked it has gone.".to_string(),
        ),
    }
}

/// What the user picked, question by question, plus a line per question a model can read without
/// walking the JSON.
///
/// An answer naming a question this ask does not have is dropped: the answer is addressed by
/// position, and a position nothing was asked at says nothing. Labels rather than indices go back,
/// on [`ubiq_proto::ask`]'s own standing.
fn answered(questions: &[AskQuestion], answers: &[AskAnswer]) -> Value {
    let mut entries: Vec<Value> = Vec::new();
    let mut lines: Vec<String> = Vec::new();

    for answer in answers {
        let Some(question) = questions.get(answer.question) else {
            continue;
        };
        let mut entry = json!({
            "header": question.header,
            "question": question.question,
            "chosen": answer.chosen,
        });
        if let Some(other) = &answer.other {
            entry["other"] = json!(other);
        }
        if let Some(notes) = &answer.notes {
            entry["notes"] = json!(notes);
        }
        entries.push(entry);

        let mut said = answer.chosen.join(", ");
        if let Some(other) = &answer.other {
            if said.is_empty() {
                said = other.clone();
            } else {
                said.push_str(&format!(", {other}"));
            }
        }
        if said.is_empty() {
            said = "nothing".to_string();
        }
        if let Some(notes) = &answer.notes {
            said.push_str(&format!(" ({notes})"));
        }
        lines.push(format!("{}: {said}", question.header));
    }

    json!({
        "answered": entries,
        "chat": false,
        "summary": lines.join("\n"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::armed::Armed;
    use crate::ask::Asks;
    use std::sync::Arc;
    use std::time::Duration;
    use ubiq_proto::bus;

    /// The parked table under test, with an armed table nothing in these reaches.
    fn reaching(asks: Arc<Asks>) -> AskReach {
        AskReach {
            asks,
            armed: Arc::new(Armed::new()),
        }
    }

    fn facts(key: &str) -> AgentFacts {
        AgentFacts {
            key: key.to_string(),
            name: "claude 1".to_string(),
            harness: "Claude Code".to_string(),
            ..Default::default()
        }
    }

    fn picked() -> AskOutcome {
        AskOutcome::Answered(vec![AskAnswer {
            question: 0,
            chosen: vec!["Left".to_string()],
            other: None,
            notes: None,
        }])
    }

    fn well_formed() -> Value {
        json!({"questions": [{
            "question": "Which way?",
            "header": "Direction",
            "options": [{"label": "Left"}, {"label": "Right", "description": "the other one"}],
            "multiSelect": false,
        }]})
    }

    /// A refusal must not leave a row behind: the model is told what is wrong and the user is
    /// never shown a dialog that cannot be drawn.
    #[test]
    fn a_malformed_ask_is_refused_without_parking() {
        let asks = Arc::new(Asks::with_timeout(Duration::from_millis(20)));
        let (hub, host) = bus::hub();
        let reach = reaching(Arc::clone(&asks));
        let voice = host.voice();
        let agent = AgentId::generate().to_string();

        let one_option = json!({"questions": [{
            "question": "Which way?",
            "header": "Direction",
            "options": [{"label": "Left"}],
        }]});
        let refusal = call(
            "ask_user_question",
            &one_option,
            &facts(&agent),
            &voice,
            &reach,
        )
        .unwrap_err();
        assert!(refusal.contains("cannot be drawn"), "{refusal}");
        assert!(asks.is_empty());

        // The same for arguments that are not an ask at all.
        assert!(
            call(
                "ask_user_question",
                &json!({"question": "Which way?"}),
                &facts(&agent),
                &voice,
                &reach
            )
            .is_err()
        );
        assert!(asks.is_empty());
        drop(hub);
    }

    /// A pane has no transcript to ask in, so the call is refused rather than parked forever.
    #[test]
    fn an_agent_that_is_not_a_conversation_is_refused() {
        let asks = Arc::new(Asks::with_timeout(Duration::from_millis(20)));
        let (hub, host) = bus::hub();
        let reach = reaching(Arc::clone(&asks));
        let refused = call(
            "ask_user_question",
            &well_formed(),
            &facts("not-a-ulid"),
            &host.voice(),
            &reach,
        )
        .unwrap_err();
        assert!(refused.contains("only a conversation"), "{refused}");
        assert!(asks.is_empty());
        drop(hub);
    }

    /// The whole round trip on two threads, as it runs: the call parks, something answers, and
    /// the result carries the labels and a summary.
    #[test]
    fn an_answer_comes_back_as_labels_and_a_summary() {
        let asks = Arc::new(Asks::new());
        let (hub, host) = bus::hub();
        let reach = reaching(Arc::clone(&asks));
        let voice = host.voice();
        let agent = AgentId::generate().to_string();

        let waiting = Arc::clone(&asks);
        let calling = std::thread::spawn(move || {
            call(
                "ask_user_question",
                &well_formed(),
                &facts(&agent),
                &voice,
                &reaching(waiting),
            )
        });

        // The ask reaches the host as `AskUser`, which is what the coordinator addresses.
        let ask_id = loop {
            match host.recv().expect("the bus is open") {
                bus::FromClient::Said {
                    message: Message::AskUser { ask_id, .. },
                    ..
                } => break ask_id,
                _ => continue,
            }
        };
        assert!(asks.answer(
            ask_id,
            AskOutcome::Answered(vec![AskAnswer {
                question: 0,
                chosen: vec!["Left".to_string()],
                other: None,
                notes: Some("but only for now".to_string()),
            }])
        ));

        let result = calling
            .join()
            .expect("the calling thread")
            .expect("an answer");
        assert_eq!(result["answered"][0]["header"], "Direction");
        assert_eq!(result["answered"][0]["chosen"][0], "Left");
        assert_eq!(result["summary"], "Direction: Left (but only for now)");
        assert!(asks.is_empty());
        drop(reach);
        drop(hub);
    }

    /// The bounded park's whole point: a wait that gives up answers the model with something it
    /// can act on rather than failing at it, and the question survives the call (`D191`).
    #[test]
    fn a_park_that_gives_up_answers_the_model_and_keeps_the_question() {
        let asks = Arc::new(Asks::new());
        let armed = Arc::new(Armed::new());
        let (hub, host) = bus::hub();
        let voice = host.voice();
        let agent_id = AgentId::generate();
        let reach = AskReach {
            asks: Arc::clone(&asks),
            armed: Arc::clone(&armed),
        };

        let result = ask(
            &well_formed(),
            &facts(&agent_id.to_string()),
            &voice,
            &reach,
            Duration::from_millis(20),
        )
        .expect("giving up is a result, not an error");
        assert_eq!(result["timedOut"], true);
        assert_eq!(result["answered"].as_array().unwrap().len(), 0);
        let summary = result["summary"].as_str().unwrap();
        assert!(summary.contains("still on screen"), "{summary}");
        assert!(summary.contains("next turn"), "{summary}");

        // The parked row is gone, and the dialog it raised is now the armed table's — already
        // raised, so the user can still answer it.
        assert!(asks.is_empty());
        assert_eq!(armed.len(), 1);

        // The question went out and nothing closed it: an `AskEnded` here would take the modal
        // down under a user who is still reading it.
        let raised = match host.recv().expect("the bus is open") {
            bus::FromClient::Said {
                message: Message::AskUser { ask_id, .. },
                ..
            } => ask_id,
            other => panic!("expected the ask to be raised, got {other:?}"),
        };
        assert!(
            host.recv_timeout(Duration::from_millis(20)).is_err(),
            "giving up on the call must not close the dialog",
        );

        // And the late answer lands: the armed table holds it under the same id, which is what
        // the coordinator's `AnswerAsk` arm looks in first.
        // A handed-over dialog is a batch of one, so answering it settles the whole of it at once.
        let settled = match armed.answer(
            raised,
            &AskOutcome::Answered(vec![AskAnswer {
                question: 0,
                chosen: vec!["Left".to_string()],
                other: None,
                notes: None,
            }]),
        ) {
            crate::armed::Answering::Settled(settled) => settled,
            _ => panic!("the handed-over dialog waits for nothing else"),
        };
        assert_eq!(settled.arming, crate::armed::Arming::HandedOver);
        let text = crate::armed::prose(&settled).expect("something was said");
        assert!(text.contains("without an answer"), "{text}");
        assert!(text.contains("- Answer: Left"), "{text}");
        drop(hub);
    }

    /// A user who answers on the last millisecond gets the tool result they earned: the give-up
    /// finds the row already taken and reads the answer off the channel instead.
    #[test]
    fn an_answer_that_races_the_bound_still_becomes_the_tool_result() {
        let asks = Arc::new(Asks::new());
        let armed = Arc::new(Armed::new());
        let (hub, host) = bus::hub();
        let voice = host.voice();
        let agent_id = AgentId::generate();
        let reach = AskReach {
            asks: Arc::clone(&asks),
            armed: Arc::clone(&armed),
        };

        let raised = raise(
            &well_formed(),
            &facts(&agent_id.to_string()),
            &voice,
            &reach,
        )
        .expect("a well-formed ask");
        // The answer lands before the give-up runs, which is the race: the row is gone and the
        // answer is on the channel, and `give_up` is what has to notice.
        assert!(asks.answer(raised.ask_id, picked()));
        let ending = asks.give_up(raised.ask_id, &raised.waiting);
        let result = settle(raised, ending, &reach, Duration::from_millis(20))
            .expect("the answer, not the give-up");
        assert_eq!(result["answered"][0]["chosen"][0], "Left");
        assert!(result.get("timedOut").is_none());
        // Nothing was handed over: the call answered itself.
        assert!(armed.is_empty());
        drop(hub);
    }

    /// What an inbound `Message::AskEnded` does: the coordinator calls `Asks::end`, and the
    /// parked call comes back at once with the closed-conversation error rather than waiting out
    /// the hour this table is set to.
    #[test]
    fn an_ask_ended_from_outside_frees_the_parked_call() {
        let asks = Arc::new(Asks::new());
        let (hub, host) = bus::hub();
        let voice = host.voice();
        let agent = AgentId::generate().to_string();

        let waiting = Arc::clone(&asks);
        let calling = std::thread::spawn(move || {
            call(
                "ask_user_question",
                &well_formed(),
                &facts(&agent),
                &voice,
                &reaching(waiting),
            )
        });

        let ask_id = loop {
            match host.recv().expect("the bus is open") {
                bus::FromClient::Said {
                    message: Message::AskUser { ask_id, .. },
                    ..
                } => break ask_id,
                _ => continue,
            }
        };
        assert!(asks.end(ask_id, AskClosed::Gone));

        let refusal = calling
            .join()
            .expect("the calling thread")
            .expect_err("a closed ask is an error the model can read");
        assert!(refusal.contains("has gone"), "{refusal}");
        assert!(asks.is_empty());
        drop(hub);
    }

    /// The same from the unload side: the coordinator's `close_asks` ends every ask the
    /// conversation parked, and the harness gets its error instead of a thread parked for an hour
    /// on a conversation that is already dead.
    #[test]
    fn unloading_a_conversation_frees_the_calls_it_parked() {
        let asks = Arc::new(Asks::new());
        let (hub, host) = bus::hub();
        let voice = host.voice();
        let agent_id = AgentId::generate();
        let agent = agent_id.to_string();

        let waiting = Arc::clone(&asks);
        let calling = std::thread::spawn(move || {
            call(
                "ask_user_question",
                &well_formed(),
                &facts(&agent),
                &voice,
                &reaching(waiting),
            )
        });

        let ask_id = loop {
            match host.recv().expect("the bus is open") {
                bus::FromClient::Said {
                    message: Message::AskUser { ask_id, .. },
                    ..
                } => break ask_id,
                _ => continue,
            }
        };
        assert_eq!(asks.end_for_agent(agent_id, AskClosed::Gone), vec![ask_id]);

        let refusal = calling
            .join()
            .expect("the calling thread")
            .expect_err("an unloaded conversation cannot answer");
        assert!(refusal.contains("has gone"), "{refusal}");
        assert!(asks.is_empty());
        drop(hub);
    }

    /// The other mode, on the listener's own thread: the call returns the id it minted, the row is
    /// armed against the caller alone, and nothing is raised and nothing parked until the turn
    /// ends.
    #[test]
    fn registering_arms_a_dialog_and_returns_at_once() {
        let asks = Arc::new(Asks::new());
        let armed = Arc::new(Armed::new());
        let (hub, host) = bus::hub();
        let reach = AskReach {
            asks: Arc::clone(&asks),
            armed: Arc::clone(&armed),
        };
        let agent_id = AgentId::generate();

        let result = call(
            "register_question",
            &well_formed(),
            &facts(&agent_id.to_string()),
            &host.voice(),
            &reach,
        )
        .expect("a registration answers itself");
        assert!(result["registered"].is_string());
        assert!(
            result["summary"]
                .as_str()
                .unwrap()
                .contains("End your turn")
        );
        assert_eq!(armed.len(), 1);
        assert!(asks.is_empty());

        // Nothing went out: the dialog is raised at the turn boundary, by the pump, not here.
        assert!(
            host.recv_timeout(Duration::from_millis(20)).is_err(),
            "a registration says nothing on the bus",
        );

        // And it is the caller's alone — another conversation's turn ending raises nothing.
        assert!(armed.fire(AgentId::generate()).is_empty());
        assert_eq!(armed.fire(agent_id).len(), 1);
        drop(hub);
    }

    /// The same refusals, on the same reading: a registration that cannot be drawn never becomes a
    /// row, and a pane has no transcript to draw one in.
    #[test]
    fn a_registration_that_cannot_be_drawn_is_refused() {
        let armed = Arc::new(Armed::new());
        let (hub, host) = bus::hub();
        let reach = AskReach {
            asks: Arc::new(Asks::new()),
            armed: Arc::clone(&armed),
        };
        let agent = AgentId::generate().to_string();

        let one_option = json!({"questions": [{
            "question": "Which way?",
            "header": "Direction",
            "options": [{"label": "Left"}],
        }]});
        let refusal = call(
            "register_question",
            &one_option,
            &facts(&agent),
            &host.voice(),
            &reach,
        )
        .unwrap_err();
        assert!(refusal.contains("cannot be drawn"), "{refusal}");

        let refused = call(
            "register_question",
            &well_formed(),
            &facts("not-a-ulid"),
            &host.voice(),
            &reach,
        )
        .unwrap_err();
        assert!(refused.contains("only a conversation"), "{refused}");
        assert!(armed.is_empty());
        drop(hub);
    }
}
