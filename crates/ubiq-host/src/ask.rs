//! The parked calls: every question an agent has asked the user and not yet had an answer to.
//!
//! One tool in Ubiq does not answer itself. `ubiq-ask`'s `ask_user_question` raises a question at
//! the person watching and the harness waits for them, so the call has to be *held* somewhere
//! between the listener taking it and the dialog answering it — a row per call, keyed by the
//! [`AskId`] the listener minted, and this is that table (`D138`).
//!
//! **The table is the whole of the coupling.** The thread serving the tool call parks on a
//! channel; the coordinator's thread — which is where [`ubiq_proto::messages::Message::AnswerAsk`]
//! lands, like every other message from a window — calls [`Asks::answer`] and returns immediately.
//! Neither thread waits on the other, and nothing here reaches back into the coordinator's state:
//! a parked call knows its own id and nothing else about the host.
//!
//! **Every parked call ends, one way or another.** The user answers it, the user says they would
//! rather talk ([`AskOutcome::Chat`]), the conversation goes ([`AskClosed::Gone`]), or
//! [`ASK_TIMEOUT_SECS`] passes and it is released as [`AskClosed::Timeout`]. A tool call that
//! never returns is a wedged agent, so the timeout is not optional: every wait is bounded by it,
//! whatever else does or does not happen.

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::time::Duration;

use ubiq_proto::ask::{ASK_TIMEOUT_SECS, AskClosed, AskOutcome, AskQuestion};
use ubiq_proto::ids::AskId;
use ubiq_proto::work::AgentId;

/// How a parked call ended: what the user said, or why nobody said anything.
pub type Ending = Result<AskOutcome, AskClosed>;

/// One parked call, as the table holds it.
struct Parked {
    /// Which conversation asked — what [`Asks::end_for_agent`] matches on when it goes.
    agent: AgentId,
    /// The waiting thread's end of the call.
    reply: Sender<Ending>,
}

/// Every question waiting on a human.
///
/// Shared behind an `Arc`: the coordinator holds one clone and the MCP listener another, exactly
/// as [`crate::kb::Kb`] and [`crate::help::Help`] are shared, because the two halves of an ask —
/// the call that parked and the answer that releases it — arrive on different threads.
pub struct Asks {
    pending: Mutex<HashMap<AskId, Parked>>,
    timeout: Duration,
}

impl Default for Asks {
    fn default() -> Self {
        Self::new()
    }
}

impl Asks {
    pub fn new() -> Self {
        Self::with_timeout(Duration::from_secs(ASK_TIMEOUT_SECS))
    }

    /// The same, waiting for a duration of the caller's choosing. Tests use it: proving that a
    /// wait gives up is worth a millisecond and is not worth an hour.
    pub fn with_timeout(timeout: Duration) -> Self {
        Self {
            pending: Mutex::new(HashMap::new()),
            timeout,
        }
    }

    /// Park a call: mint the id the rest of the exchange references, and hand back the end the
    /// serving thread waits on.
    ///
    /// The questions are passed in only so the caller need not hold them separately — they travel
    /// on to the window in [`ubiq_proto::messages::Message::AskUser`], and the table keeps no copy
    /// of them: nothing here ever re-reads a question, and a dialog that lost them has lost the
    /// window they were drawn in too.
    pub fn raise(&self, agent: AgentId, questions: &[AskQuestion]) -> (AskId, Receiver<Ending>) {
        let ask_id = AskId::generate();
        let (reply, waiting) = channel();
        self.lock().insert(ask_id, Parked { agent, reply });
        tracing::debug!(
            agent = %agent,
            ask = %ask_id,
            questions = questions.len(),
            "an agent is asking the user something",
        );
        (ask_id, waiting)
    }

    /// Wait for whatever ends this ask, giving up after the timeout.
    ///
    /// Called on a thread of its own, never on the listener's (`D138`). A timeout takes the row
    /// out here rather than leaving it for a dialog that is still open to answer: once this has
    /// returned, the tool call has already been answered and a late answer has nowhere to land —
    /// which is why the caller tells the window about a timeout (`Message::AskEnded`), so the
    /// dialog stops offering one.
    pub fn wait(&self, ask_id: AskId, waiting: &Receiver<Ending>) -> Ending {
        match waiting.recv_timeout(self.timeout) {
            Ok(ending) => ending,
            Err(RecvTimeoutError::Timeout) => {
                self.lock().remove(&ask_id);
                tracing::info!(ask = %ask_id, "an ask timed out with nobody answering");
                Err(AskClosed::Timeout)
            }
            // Defensive only: in the running host the waiting thread holds the `Arc<Asks>` it is
            // waiting on, so the table cannot go out from under it, and every path that takes a
            // row out sends on it first. Read as `Gone` rather than panicking, because a wedged
            // tool call is the worse failure.
            Err(RecvTimeoutError::Disconnected) => Err(AskClosed::Gone),
        }
    }

    /// Release a parked call with what the user said. `false` when this ask is not being held —
    /// a second answer for the same id, or one that raced the timeout — which the caller drops
    /// quietly.
    pub fn answer(&self, ask_id: AskId, outcome: AskOutcome) -> bool {
        let Some(parked) = self.lock().remove(&ask_id) else {
            tracing::debug!(ask = %ask_id, "an answer named an ask nobody is holding");
            return false;
        };
        // A closed receiver means the waiting thread already gave up; the row is gone either way.
        let _ = parked.reply.send(Ok(outcome));
        true
    }

    /// Release a parked call without an answer. `false` when it is not being held.
    pub fn end(&self, ask_id: AskId, why: AskClosed) -> bool {
        let Some(parked) = self.lock().remove(&ask_id) else {
            return false;
        };
        let _ = parked.reply.send(Err(why));
        true
    }

    /// Release every call one conversation parked, and say which they were.
    ///
    /// The coordinator calls this wherever a conversation stops being something an answer could
    /// reach — ended, unloaded, or its harness died — and tells the owning window about each id
    /// so a dialog still on screen stops offering an answer that can no longer land.
    pub fn end_for_agent(&self, agent: AgentId, why: AskClosed) -> Vec<AskId> {
        let mut pending = self.lock();
        let ids: Vec<AskId> = pending
            .iter()
            .filter(|(_, parked)| parked.agent == agent)
            .map(|(ask_id, _)| *ask_id)
            .collect();
        for ask_id in &ids {
            if let Some(parked) = pending.remove(ask_id) {
                let _ = parked.reply.send(Err(why));
            }
        }
        ids
    }

    /// How many calls are parked. For the tests and for a log line.
    pub fn len(&self) -> usize {
        self.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// A poisoned lock means a serving thread panicked holding it. The table is a set of waiting
    /// calls, not an invariant: taking it back releases them, and refusing to ever answer another
    /// ask would be the worse failure — [`crate::mcp::Registry`] lives by the same rule.
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<AskId, Parked>> {
        self.pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl std::fmt::Debug for Asks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Asks")
            .field("pending", &self.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ubiq_proto::ask::{AskAnswer, AskOption};

    fn question() -> AskQuestion {
        AskQuestion {
            question: "Which way?".to_string(),
            header: "Direction".to_string(),
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

    fn answered(label: &str) -> AskOutcome {
        AskOutcome::Answered(vec![AskAnswer {
            question: 0,
            chosen: vec![label.to_string()],
            other: None,
            notes: None,
        }])
    }

    #[test]
    fn an_answer_releases_the_parked_call() {
        let asks = Asks::new();
        let agent = AgentId::generate();
        let (ask_id, waiting) = asks.raise(agent, &[question()]);
        assert_eq!(asks.len(), 1);

        assert!(asks.answer(ask_id, answered("Left")));
        assert_eq!(asks.wait(ask_id, &waiting), Ok(answered("Left")));
        assert!(asks.is_empty());
    }

    #[test]
    fn a_second_answer_for_the_same_ask_is_ignored() {
        let asks = Asks::new();
        let (ask_id, waiting) = asks.raise(AgentId::generate(), &[question()]);

        assert!(asks.answer(ask_id, answered("Left")));
        assert!(!asks.answer(ask_id, answered("Right")));
        assert_eq!(asks.wait(ask_id, &waiting), Ok(answered("Left")));
        // And the second one never reached the waiting end either.
        assert_eq!(
            waiting.recv_timeout(Duration::from_millis(10)),
            Err(RecvTimeoutError::Disconnected)
        );
    }

    #[test]
    fn a_wait_that_nobody_answers_times_out_and_forgets_the_row() {
        let asks = Asks::with_timeout(Duration::from_millis(20));
        let (ask_id, waiting) = asks.raise(AgentId::generate(), &[question()]);

        assert_eq!(asks.wait(ask_id, &waiting), Err(AskClosed::Timeout));
        assert!(asks.is_empty());
        // The dialog answering after that changes nothing: the call is already answered.
        assert!(!asks.answer(ask_id, answered("Left")));
    }

    #[test]
    fn a_conversation_going_releases_everything_it_parked() {
        let asks = Asks::new();
        let agent = AgentId::generate();
        let other = AgentId::generate();
        let (mine, waiting) = asks.raise(agent, &[question()]);
        let (theirs, still_waiting) = asks.raise(other, &[question()]);

        assert_eq!(asks.end_for_agent(agent, AskClosed::Gone), vec![mine]);
        assert_eq!(asks.wait(mine, &waiting), Err(AskClosed::Gone));
        // The other conversation's ask is untouched.
        assert_eq!(asks.len(), 1);
        assert!(asks.answer(theirs, answered("Right")));
        assert_eq!(asks.wait(theirs, &still_waiting), Ok(answered("Right")));
    }

    /// Not a production path — the waiting thread holds the `Arc<Asks>`, so the table cannot be
    /// dropped under it. This pins the defensive arm's behaviour: a dead channel releases the
    /// wait instead of wedging it.
    #[test]
    fn a_dropped_table_releases_a_waiting_call_rather_than_wedging_it() {
        let asks = Asks::new();
        let (ask_id, waiting) = asks.raise(AgentId::generate(), &[question()]);
        drop(asks);
        let asks = Asks::with_timeout(Duration::from_millis(20));
        assert_eq!(asks.wait(ask_id, &waiting), Err(AskClosed::Gone));
    }
}
