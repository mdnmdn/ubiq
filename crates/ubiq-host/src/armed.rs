//! The armed dialogs: questions an agent registered during a turn, to be raised when it ends.
//!
//! **The other ask mode, beside [`crate::ask`].** `ask_user_question` parks the tool call while a
//! person is asked, which is the only shape that can pause an agent mid-sequence — and the only
//! shape a harness's own tool timeout can reach. `register_question` does not wait at all: the
//! call mints an [`AskId`], files it here as *armed* against the calling conversation, and returns
//! on the listener's own thread. Nothing parks, so there is no timer to fight (`D175`).
//!
//! **A registration is for the current turn only.** Everything armed for a conversation is raised
//! as [`ubiq_proto::messages::Message::AskUser`] when that conversation's turn ends, and nothing
//! survives into the next turn: a turn that fails drops what it armed rather than raising a dialog
//! over an error, and a user who types instead of answering closes what was raised.
//!
//! **The answer is the next turn, not a tool result.** There is no channel to release — the call
//! returned long ago — so [`crate::coordinator::Coordinator`] renders the outcome into prose with
//! [`prose`] and prompts the agent with it, exactly as a window's own prompt would. That is the
//! whole difference between the two modes: what the outcome is delivered into.

use std::sync::Mutex;

use ubiq_proto::ask::{AskAnswer, AskQuestion};
use ubiq_proto::ids::AskId;
use ubiq_proto::work::AgentId;

/// One registered dialog, as the table holds it.
struct Row {
    ask_id: AskId,
    /// Which conversation registered it — what every lookup here matches on.
    agent: AgentId,
    /// Kept, unlike [`crate::ask::Asks`]'s: this table outlives the raising, and the prose the
    /// answer becomes is written from the questions it answers.
    questions: Vec<AskQuestion>,
    /// Whether the turn has ended and the dialog is on screen. An unraised row is dropped by a
    /// failed turn; a raised one is waiting on the user.
    raised: bool,
}

/// Every dialog registered and not yet answered.
///
/// Shared behind an `Arc` exactly as [`crate::ask::Asks`] is, and between the same three threads:
/// the MCP listener arms a row, the conversation's pump fires it at the turn boundary, and the
/// coordinator's thread answers it. A `Vec` rather than a map because the order matters — several
/// registrations are raised in registration order.
#[derive(Default)]
pub struct Armed {
    rows: Mutex<Vec<Row>>,
}

impl Armed {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a dialog against a conversation and mint the id the rest of the exchange
    /// references. Returns on the caller's own thread — this is the whole of the tool call.
    pub fn arm(&self, agent: AgentId, questions: Vec<AskQuestion>) -> AskId {
        let ask_id = AskId::generate();
        tracing::debug!(
            agent = %agent,
            ask = %ask_id,
            questions = questions.len(),
            "an agent registered a dialog for the end of its turn",
        );
        self.lock().push(Row {
            ask_id,
            agent,
            questions,
            raised: false,
        });
        ask_id
    }

    /// The turn ended: everything this conversation armed, in registration order, marked raised.
    ///
    /// Called on the pump thread. The caller says each one as `Message::AskUser`, which is the
    /// message the parked mode already raises — the window cannot tell the two modes apart and
    /// does not need to.
    pub fn fire(&self, agent: AgentId) -> Vec<(AskId, Vec<AskQuestion>)> {
        let mut rows = self.lock();
        let mut firing = Vec::new();
        for row in rows.iter_mut() {
            if row.agent == agent && !row.raised {
                row.raised = true;
                firing.push((row.ask_id, row.questions.clone()));
            }
        }
        firing
    }

    /// The turn ended badly: forget what it armed without raising anything. A dialog over an error
    /// asks the user to choose between options the agent can no longer act on.
    ///
    /// Only the rows that were never raised — one already on screen belongs to an earlier turn and
    /// is still answerable.
    pub fn disarm(&self, agent: AgentId) -> usize {
        let mut rows = self.lock();
        let before = rows.len();
        rows.retain(|row| row.agent != agent || row.raised);
        before - rows.len()
    }

    /// Take a raised dialog out to answer it, with the questions the answer is written against.
    /// `None` when this id is not one of ours — which is how the coordinator tells an armed ask
    /// from a parked one.
    pub fn answer(&self, ask_id: AskId) -> Option<Vec<AskQuestion>> {
        let mut rows = self.lock();
        let at = rows
            .iter()
            .position(|row| row.ask_id == ask_id && row.raised)?;
        Some(rows.remove(at).questions)
    }

    /// Forget one row, whatever stage it is at. `false` when nothing here holds that id.
    pub fn close(&self, ask_id: AskId) -> bool {
        let mut rows = self.lock();
        let Some(at) = rows.iter().position(|row| row.ask_id == ask_id) else {
            return false;
        };
        rows.remove(at);
        true
    }

    /// Forget everything one conversation registered, and say which of them the window has on
    /// screen — those are the ones it has to be told about, the same way a parked ask's closing
    /// is told.
    pub fn close_for_agent(&self, agent: AgentId) -> Vec<AskId> {
        let mut rows = self.lock();
        let raised: Vec<AskId> = rows
            .iter()
            .filter(|row| row.agent == agent && row.raised)
            .map(|row| row.ask_id)
            .collect();
        rows.retain(|row| row.agent != agent);
        raised
    }

    /// How many dialogs are registered, raised or not. For the tests and for a log line.
    pub fn len(&self) -> usize {
        self.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Poisoned means a thread panicked holding it, and the table is a list of questions rather
    /// than an invariant — [`crate::ask::Asks`] lives by the same rule.
    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<Row>> {
        self.rows
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl std::fmt::Debug for Armed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Armed").field("rows", &self.len()).finish()
    }
}

/// What the user said, as the prompt that opens the next turn.
///
/// **Prose, not the JSON a tool result carries.** The answer arrives as a user message on this
/// mode, so it has to read as one — the transcript shows the dialog and what was picked, and this
/// is what the model is given. An answer naming a question the ask does not have is dropped, on
/// [`super::mcp::ask`]'s own reading: an answer is addressed by position, and a position nothing
/// was asked at says nothing.
pub fn prose(questions: &[AskQuestion], answers: &[AskAnswer]) -> String {
    let mut out = String::from(
        "I answered the question you registered before ending your turn. Carry on from this.\n",
    );
    for answer in answers {
        let Some(question) = questions.get(answer.question) else {
            continue;
        };
        out.push_str(&format!("\n{} — {}\n", question.header, question.question));
        let mut said = answer.chosen.join(", ");
        if let Some(other) = &answer.other {
            match said.is_empty() {
                true => said = other.clone(),
                false => said.push_str(&format!(", {other}")),
            }
        }
        if said.is_empty() {
            said = "nothing".to_string();
        }
        out.push_str(&format!("- Answer: {said}\n"));
        if let Some(notes) = &answer.notes {
            out.push_str(&format!("- Notes: {notes}\n"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ubiq_proto::ask::AskOption;

    fn question(header: &str) -> AskQuestion {
        AskQuestion {
            question: "Which way?".to_string(),
            header: header.to_string(),
            options: vec![
                AskOption {
                    label: "Left".to_string(),
                    description: String::new(),
                    preview: None,
                },
                AskOption {
                    label: "Right".to_string(),
                    description: String::new(),
                    preview: None,
                },
            ],
            multi_select: false,
        }
    }

    #[test]
    fn a_turn_ending_raises_everything_that_conversation_armed_in_order() {
        let armed = Armed::new();
        let agent = AgentId::generate();
        let other = AgentId::generate();
        let first = armed.arm(agent, vec![question("First")]);
        let second = armed.arm(agent, vec![question("Second")]);
        armed.arm(other, vec![question("Theirs")]);

        let fired = armed.fire(agent);
        assert_eq!(
            fired.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
            vec![first, second]
        );
        assert_eq!(fired[0].1[0].header, "First");
        // The other conversation's registration is untouched, and a second turn end raises
        // nothing twice.
        assert!(armed.fire(agent).is_empty());
        assert_eq!(armed.len(), 3);
    }

    #[test]
    fn a_failed_turn_drops_what_it_armed_and_leaves_what_was_already_raised() {
        let armed = Armed::new();
        let agent = AgentId::generate();
        let raised = armed.arm(agent, vec![question("First")]);
        assert_eq!(armed.fire(agent).len(), 1);
        armed.arm(agent, vec![question("Second")]);

        assert_eq!(armed.disarm(agent), 1);
        assert_eq!(armed.len(), 1);
        assert!(armed.answer(raised).is_some());
    }

    #[test]
    fn only_a_raised_dialog_can_be_answered() {
        let armed = Armed::new();
        let agent = AgentId::generate();
        let ask_id = armed.arm(agent, vec![question("First")]);

        // Registered but not yet raised: no dialog exists to have answered it.
        assert!(armed.answer(ask_id).is_none());
        armed.fire(agent);
        assert!(armed.answer(ask_id).is_some());
        // And the row is gone, so a second answer for the same id is not ours either.
        assert!(armed.answer(ask_id).is_none());
        assert!(armed.is_empty());
    }

    #[test]
    fn a_conversation_going_forgets_its_rows_and_names_the_ones_on_screen() {
        let armed = Armed::new();
        let agent = AgentId::generate();
        let other = AgentId::generate();
        let raised = armed.arm(agent, vec![question("First")]);
        armed.fire(agent);
        armed.arm(agent, vec![question("Second")]);
        armed.arm(other, vec![question("Theirs")]);

        assert_eq!(armed.close_for_agent(agent), vec![raised]);
        assert_eq!(armed.len(), 1);
    }

    #[test]
    fn the_prose_reads_as_what_the_user_said() {
        let questions = vec![question("Direction")];
        let answers = vec![AskAnswer {
            question: 0,
            chosen: vec!["Left".to_string()],
            other: None,
            notes: Some("but only for now".to_string()),
        }];
        let text = prose(&questions, &answers);
        assert!(text.contains("Direction — Which way?"), "{text}");
        assert!(text.contains("- Answer: Left"), "{text}");
        assert!(text.contains("- Notes: but only for now"), "{text}");
    }
}
