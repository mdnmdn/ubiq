//! The `ubiq-ask` server: one tool, and the only one that waits for a human.
//!
//! Every other built-in tool answers from a fact the host already holds. This one puts a question
//! on screen and parks until the person watching answers it, says they would rather talk, or an
//! hour passes — [`crate::ask::Asks`] holds the call meanwhile, and `D138` is why that table
//! exists at all. **It never runs on the listener's thread**: [`super::server::handle`] moves the
//! request onto a thread of its own before calling in here, because every other agent's tool calls
//! come through that one listener and a parked call would hold all of them up.
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

use serde::Deserialize;
use serde_json::{Value, json};
use ubiq_proto::ask::{ASK_TIMEOUT_SECS, AskAnswer, AskClosed, AskOption, AskOutcome, AskQuestion};
use ubiq_proto::bus::Voice;
use ubiq_proto::messages::Message;
use ubiq_proto::work::AgentId;

use super::AskReach;
use super::registry::AgentFacts;

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
        "ask_user_question" => ask(arguments, facts, voice, reach),
        _ => Err(format!(
            "unknown tool: {}/{tool}",
            super::catalogue::UBIQ_ASK
        )),
    }
}

/// Raise the question, wait for the answer, and turn whatever ended it into a tool result.
fn ask(
    arguments: &Value,
    facts: &AgentFacts,
    voice: &Voice,
    reach: &AskReach,
) -> Result<Value, String> {
    let asking: Asking = serde_json::from_value(arguments.clone())
        .map_err(|error| format!("ask_user_question could not read its arguments: {error}"))?;
    let questions: Vec<AskQuestion> = asking
        .questions
        .into_iter()
        .map(AskQuestion::from)
        .collect();
    ubiq_proto::ask::check(&questions)
        .map_err(|wrong| format!("this ask cannot be drawn: {wrong}"))?;

    // A pane's key is a `PaneId` and names no conversation, so there is no transcript to raise the
    // question in and nothing to answer it with. Refused rather than parked.
    let agent_id: AgentId = facts.key.parse().map_err(|_| {
        "only a conversation can ask the user a question; this agent is a terminal pane".to_string()
    })?;

    let (ask_id, waiting) = reach.asks.raise(agent_id, &questions);
    // The coordinator alone knows which window owns the conversation, so the ask goes to it as an
    // ordinary bus fact and it does the addressing — the same route a notification takes from a
    // tool call. A conversation with no owner is ended there as `Gone`, which releases this wait
    // rather than leaving it to the timeout.
    voice.say(Message::AskUser {
        agent_id,
        ask_id,
        questions: questions.clone(),
    });

    match reach.asks.wait(ask_id, &waiting) {
        Ok(AskOutcome::Answered(answers)) => Ok(answered(&questions, &answers)),
        Ok(AskOutcome::Chat) => Ok(json!({
            "answered": [],
            "chat": true,
            "summary": "The user would rather discuss this than pick an option. They are saying so in the chat: read what they write next and carry on from it, rather than asking again.",
        })),
        Err(AskClosed::Timeout) => {
            // The table released this row by itself, so nothing else is going to tell the window:
            // without this the dialog stays on screen offering an answer the tool call has
            // already stopped waiting for. Same route out as the question came in by — the
            // coordinator does the addressing.
            voice.say(Message::AskEnded {
                agent_id,
                ask_id,
                why: AskClosed::Timeout,
            });
            Err(format!(
                "nobody answered this question within {} minutes, so it was closed. Carry on without an answer, or say what you are blocked on.",
                ASK_TIMEOUT_SECS / 60
            ))
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
    use crate::ask::Asks;
    use std::sync::Arc;
    use std::time::Duration;
    use ubiq_proto::bus;

    fn facts(key: &str) -> AgentFacts {
        AgentFacts {
            key: key.to_string(),
            name: "claude 1".to_string(),
            harness: "Claude Code".to_string(),
            ..Default::default()
        }
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
        let reach = AskReach {
            asks: Arc::clone(&asks),
        };
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
        let reach = AskReach {
            asks: Arc::clone(&asks),
        };
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
        let reach = AskReach {
            asks: Arc::clone(&asks),
        };
        let voice = host.voice();
        let agent = AgentId::generate().to_string();

        let waiting = Arc::clone(&asks);
        let calling = std::thread::spawn(move || {
            call(
                "ask_user_question",
                &well_formed(),
                &facts(&agent),
                &voice,
                &AskReach { asks: waiting },
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

    /// A wait that gives up takes its own row out of the table, so nothing else is left to tell
    /// the window: without the `AskEnded` this asserts, the dialog stays on screen offering an
    /// answer the tool call has already stopped waiting for.
    #[test]
    fn a_timed_out_ask_tells_the_window_it_is_over() {
        let asks = Arc::new(Asks::with_timeout(Duration::from_millis(20)));
        let (hub, host) = bus::hub();
        let voice = host.voice();
        let agent = AgentId::generate().to_string();

        let refusal = call(
            "ask_user_question",
            &well_formed(),
            &facts(&agent),
            &voice,
            &AskReach {
                asks: Arc::clone(&asks),
            },
        )
        .unwrap_err();
        assert!(refusal.contains("nobody answered"), "{refusal}");
        assert!(asks.is_empty());

        // The question first, then the closing, both on the host's own voice for the coordinator
        // to address.
        let raised = match host.recv().expect("the bus is open") {
            bus::FromClient::Said {
                message: Message::AskUser { ask_id, .. },
                ..
            } => ask_id,
            other => panic!("expected the ask to be raised, got {other:?}"),
        };
        match host.recv().expect("the bus is open") {
            bus::FromClient::Said {
                message: Message::AskEnded { ask_id, why, .. },
                ..
            } => {
                assert_eq!(ask_id, raised);
                assert_eq!(why, AskClosed::Timeout);
            }
            other => panic!("expected the ask to be closed, got {other:?}"),
        }
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
                &AskReach { asks: waiting },
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
                &AskReach { asks: waiting },
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
}
