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
//!
//! **Everything raised together is answered together, and submitted once** (`G362`). An agent may
//! call `register_question` more than once in a turn, and the turn boundary raises every row it
//! armed. Submitting each answer as it arrives would open a turn on the first while the second
//! dialog was still on screen, and the second answer would then prompt a conversation that is
//! already working. So the rows raised by one [`Armed::fire`] share a [`Batch`]: each is answered
//! on its own, nothing is submitted while any of them is still open, and the last one to settle
//! carries all of them into a single prompt. A row handed over by a parked call ([`Armed::adopt`])
//! is a batch of one — it was raised on its own, mid-turn, and has nobody to wait for.
//!
//! **Every row of a batch has the same three exits**, which is what keeps one from wedging the
//! rest: the user answers it, the user dismisses it ([`Armed::close`], which settles it with
//! nothing to say), or the conversation stops being reachable and the whole batch is forgotten
//! ([`Armed::close_for_agent`]). Nothing here is on a clock — the park bound belongs to
//! [`crate::ask::Asks`] and ends a *call*, never an armed row — so there is no timeout that could
//! take one row of a batch out from under the others.

use std::sync::Mutex;

use ubiq_proto::ask::{AskAnswer, AskOutcome, AskQuestion};
use ubiq_proto::ids::AskId;
use ubiq_proto::work::AgentId;

/// How a row got here, which is the only thing the answer's prose reads off it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arming {
    /// `register_question` filed it, to be raised when the turn ends.
    Registered,
    /// A parked `ask_user_question` gave up on its own tool result and handed the still-open
    /// dialog over (`D191`). The row arrives already raised — the modal has been on screen since
    /// the call parked — and the agent has already been told, in the tool result, that the answer
    /// would arrive this way.
    HandedOver,
}

/// Which raising a row belongs to: the set that is answered together and submitted once (`G362`).
///
/// Minted per raising, not per row. [`Armed::fire`] gives one to every row it puts on screen;
/// [`Armed::arm`] and [`Armed::adopt`] each give the row one of its own, so a row that is never
/// fired — an unraised registration, a handed-over dialog — is a batch of one and settles alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Batch(u64);

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
    arming: Arming,
    /// Which set this row is submitted with.
    batch: Batch,
    /// What the user said, once they have said it. `None` is the whole of "still open": a row
    /// stays in the table after it is settled, holding its answer, until the last of its batch
    /// settles too and they all leave together.
    ///
    /// An empty vector is a real settling — the user dismissed this one, or chose to talk about
    /// it — and contributes nothing to the prompt without holding the rest back.
    said: Option<Vec<AskAnswer>>,
}

/// One dialog going on screen, as [`Armed::fire`] hands it to the caller.
///
/// [`Self::at`] and [`Self::of`] are the batch's own arithmetic, passed straight to the window so
/// the dialog can say which of the turn's questions it is and that nothing is sent until every one
/// of them is answered.
pub struct Raising {
    pub ask_id: AskId,
    pub questions: Vec<AskQuestion>,
    /// Which of the batch this is, counting from one.
    pub at: usize,
    /// How many were raised together.
    pub of: usize,
}

/// A batch that is complete: every dialog raised together has been answered or given up on, and
/// this is everything they said.
pub struct Settled {
    /// How the batch got here. One reading for the whole of it, because the two armings never
    /// mix: a handed-over row is always a batch of one.
    pub arming: Arming,
    /// The rows that had something to say, in the order they were raised. A row the user
    /// dismissed is not here, and a batch every row of which was dismissed leaves this empty.
    pub asks: Vec<(Vec<AskQuestion>, Vec<AskAnswer>)>,
}

/// What settling one row did to its batch.
pub enum Answering {
    /// No row here holds that id and is still open — which is how the coordinator tells an armed
    /// ask from a parked one.
    NotOurs,
    /// Recorded, and the batch is still waiting on this many other dialogs. Nothing is submitted.
    Waiting { left: usize },
    /// That was the last one: here is the whole batch, ready to become one prompt.
    Settled(Settled),
}

/// The table's contents behind its one lock: the rows, and the counter [`Batch`] ids come from.
#[derive(Default)]
struct Table {
    rows: Vec<Row>,
    next_batch: u64,
}

impl Table {
    fn batch(&mut self) -> Batch {
        self.next_batch += 1;
        Batch(self.next_batch)
    }
}

/// Every dialog registered and not yet answered.
///
/// Shared behind an `Arc` exactly as [`crate::ask::Asks`] is, and between the same three threads:
/// the MCP listener arms a row, the conversation's pump fires it at the turn boundary, and the
/// coordinator's thread answers it. A `Vec` rather than a map because the order matters — several
/// registrations are raised in registration order.
#[derive(Default)]
pub struct Armed {
    table: Mutex<Table>,
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
        let mut table = self.lock();
        // A batch of its own for now. `fire` re-stamps every unraised row of this conversation
        // with one shared batch the moment the turn ends, which is where the grouping is really
        // decided; this only keeps the field honest for a row that never gets that far.
        let batch = table.batch();
        table.rows.push(Row {
            ask_id,
            agent,
            questions,
            raised: false,
            arming: Arming::Registered,
            batch,
            said: None,
        });
        ask_id
    }

    /// Take over a dialog a parked call gave up on, keeping its [`AskId`] (`D191`).
    ///
    /// **Already raised, because it already is.** The modal went on screen the moment the call
    /// parked and nothing has taken it down — no `AskEnded` is sent on this path — so the row
    /// joins the table in the state `fire` would have left it in, and the user's answer takes the
    /// armed mode's route: prose, submitted as the next turn. That is the whole of what happens
    /// to a late answer; without it the answer would reach a call that has already returned and
    /// be dropped.
    ///
    /// Being raised also means [`Armed::disarm`] leaves it alone, which is right: the turn it was
    /// asked in may well fail, and the question is still the user's to answer.
    ///
    /// **A batch of one**, and it has to be: the dialog went on screen mid-turn, on its own, and
    /// the user has been looking at it since. Making it wait for whatever the turn goes on to
    /// register would hold an answer the agent was already told to expect (`G362`).
    pub fn adopt(&self, agent: AgentId, ask_id: AskId, questions: Vec<AskQuestion>) {
        tracing::debug!(
            agent = %agent,
            ask = %ask_id,
            "a parked ask gave up its tool result and handed the dialog over",
        );
        let mut table = self.lock();
        let batch = table.batch();
        table.rows.push(Row {
            ask_id,
            agent,
            questions,
            raised: true,
            arming: Arming::HandedOver,
            batch,
            said: None,
        });
    }

    /// The turn ended: everything this conversation armed, in registration order, marked raised.
    ///
    /// Called on the pump thread. The caller says each one as `Message::AskUser`, which is the
    /// message the parked mode already raises — the window cannot tell the two modes apart and
    /// does not need to.
    ///
    /// **This raising is one batch** (`G362`). Everything that goes up here shares a [`Batch`], so
    /// each dialog is answered on its own and the whole set is submitted once, when the last of
    /// them settles. Each [`Raising`] carries its place in that set for the dialog to say.
    pub fn fire(&self, agent: AgentId) -> Vec<Raising> {
        let mut table = self.lock();
        let batch = table.batch();
        let mut firing: Vec<Raising> = Vec::new();
        for row in table.rows.iter_mut() {
            if row.agent == agent && !row.raised {
                row.raised = true;
                row.batch = batch;
                firing.push(Raising {
                    ask_id: row.ask_id,
                    questions: row.questions.clone(),
                    at: firing.len() + 1,
                    of: 0,
                });
            }
        }
        let of = firing.len();
        for raising in firing.iter_mut() {
            raising.of = of;
        }
        firing
    }

    /// The turn ended badly: forget what it armed without raising anything. A dialog over an error
    /// asks the user to choose between options the agent can no longer act on.
    ///
    /// Only the rows that were never raised — one already on screen belongs to an earlier turn and
    /// is still answerable.
    /// A raised row is never unraised, so this can only ever take whole unraised batches — there
    /// is no way for it to break a batch in half and leave the rest waiting on a row that has
    /// gone.
    pub fn disarm(&self, agent: AgentId) -> usize {
        let mut table = self.lock();
        let before = table.rows.len();
        table.rows.retain(|row| row.agent != agent || row.raised);
        before - table.rows.len()
    }

    /// Record what the user said about one raised dialog, and say what that did to its batch.
    ///
    /// **Nothing is submitted until the batch is whole** (`G362`). [`Answering::Waiting`] means
    /// this answer is held and the other dialogs are still on screen; [`Answering::Settled`] means
    /// this was the last of them and carries everything they said, in the order they were raised.
    /// [`Answering::NotOurs`] is the id nobody here is holding, which is the coordinator's cue to
    /// try the parked table instead.
    ///
    /// [`AskOutcome::Chat`] settles the row with nothing to say: the user would rather talk, so
    /// this question contributes no prose — and does not hold the rest of the batch back either.
    pub fn answer(&self, ask_id: AskId, outcome: &AskOutcome) -> Answering {
        let said = match outcome {
            AskOutcome::Answered(answers) => answers.clone(),
            AskOutcome::Chat => Vec::new(),
        };
        self.settle(ask_id, true, said)
    }

    /// Give one row up without an answer: the dialog is gone from the screen and nobody is going
    /// to answer it.
    ///
    /// **It settles the row rather than forgetting it**, which is what keeps a dismissed dialog
    /// from wedging the batch it was raised with: the row contributes nothing, and if it was the
    /// last one still open the rest is submitted on the spot. [`Answering::NotOurs`] when nothing
    /// here holds that id.
    pub fn close(&self, ask_id: AskId) -> Answering {
        self.settle(ask_id, false, Vec::new())
    }

    /// The one place a row settles. `raised_only` is the answer path's extra gate: a registration
    /// that has not been raised has no dialog for the user to have answered, so an answer naming
    /// it is not ours.
    fn settle(&self, ask_id: AskId, raised_only: bool, said: Vec<AskAnswer>) -> Answering {
        let mut table = self.lock();
        let Some(at) = table.rows.iter().position(|row| {
            row.ask_id == ask_id && row.said.is_none() && (!raised_only || row.raised)
        }) else {
            return Answering::NotOurs;
        };
        table.rows[at].said = Some(said);
        let batch = table.rows[at].batch;
        let arming = table.rows[at].arming;

        let left = table
            .rows
            .iter()
            .filter(|row| row.batch == batch && row.said.is_none())
            .count();
        if left > 0 {
            tracing::debug!(
                ask = %ask_id,
                left,
                "a dialog was settled; its batch is still waiting on the rest",
            );
            return Answering::Waiting { left };
        }

        // The last of them. The whole batch leaves the table together, in the order it was raised,
        // which is the order `rows` already holds it in.
        let mut asks = Vec::new();
        let mut keeping = Vec::with_capacity(table.rows.len());
        for row in std::mem::take(&mut table.rows) {
            if row.batch != batch {
                keeping.push(row);
                continue;
            }
            match row.said {
                Some(said) if !said.is_empty() => asks.push((row.questions, said)),
                _ => {}
            }
        }
        table.rows = keeping;
        Answering::Settled(Settled { arming, asks })
    }

    /// Forget everything one conversation registered, and say which of them the window has on
    /// screen — those are the ones it has to be told about, the same way a parked ask's closing
    /// is told.
    /// **Whatever any of them said goes with them.** A batch half-answered when the conversation
    /// stops being reachable — or when the user typed instead of answering — submits nothing:
    /// a partial set is exactly what `G362` exists to prevent, and the rest of the questions are
    /// not going to be answered now.
    ///
    /// Only the rows still open are named back: a row the user already settled has a dialog that
    /// is no longer offering an answer, and telling the window it ended would rewrite what it
    /// already shows as said.
    pub fn close_for_agent(&self, agent: AgentId) -> Vec<AskId> {
        let mut table = self.lock();
        let open: Vec<AskId> = table
            .rows
            .iter()
            .filter(|row| row.agent == agent && row.raised && row.said.is_none())
            .map(|row| row.ask_id)
            .collect();
        table.rows.retain(|row| row.agent != agent);
        open
    }

    /// How many dialogs are registered, raised or not, settled or not. For the tests and for a
    /// log line.
    pub fn len(&self) -> usize {
        self.lock().rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Poisoned means a thread panicked holding it, and the table is a list of questions rather
    /// than an invariant — [`crate::ask::Asks`] lives by the same rule.
    fn lock(&self) -> std::sync::MutexGuard<'_, Table> {
        self.table
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
/// The opening line says which mode this answer came from, because the two leave the agent in
/// different places: a registration was the last thing it did, whereas a handed-over dialog
/// belongs to a tool call that already returned "nobody answered in time" and was carried on from.
///
/// **One prompt for the whole batch** (`G362`). Several dialogs raised at one turn boundary are
/// answered separately and arrive here together, so the opening line is plural where they are and
/// every answer follows it in the order the questions were raised. `None` where the batch said
/// nothing at all — every dialog dismissed, or chatted away — because a prompt with no answers in
/// it is a turn opened over nothing.
pub fn prose(settled: &Settled) -> Option<String> {
    if settled.asks.is_empty() {
        return None;
    }
    let several = settled.asks.len() > 1;
    let mut out = String::from(match (settled.arming, several) {
        (Arming::Registered, false) => {
            "I answered the question you registered before ending your turn. Carry on from this.\n"
        }
        (Arming::Registered, true) => {
            "I answered every question you registered before ending your turn. They are all here, \
             in the order you asked them. Carry on from this.\n"
        }
        (Arming::HandedOver, _) => {
            "I have now answered the question you asked earlier — the one whose tool call returned \
             without an answer because I did not get to it in time. Carry on from this.\n"
        }
    });
    for (questions, answers) in &settled.asks {
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
    }
    Some(out)
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

    /// One answer, as a window sends it.
    fn said(label: &str) -> AskOutcome {
        AskOutcome::Answered(vec![AskAnswer {
            question: 0,
            chosen: vec![label.to_string()],
            other: None,
            notes: None,
        }])
    }

    fn settled(answering: Answering) -> Settled {
        match answering {
            Answering::Settled(settled) => settled,
            Answering::Waiting { left } => panic!("still waiting on {left}"),
            Answering::NotOurs => panic!("not ours"),
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
            fired.iter().map(|r| r.ask_id).collect::<Vec<_>>(),
            vec![first, second]
        );
        assert_eq!(fired[0].questions[0].header, "First");
        // And each one knows its place in the set, which is what the dialog says.
        assert_eq!(
            fired.iter().map(|r| (r.at, r.of)).collect::<Vec<_>>(),
            vec![(1, 2), (2, 2)]
        );
        // The other conversation's registration is untouched, and a second turn end raises
        // nothing twice.
        assert!(armed.fire(agent).is_empty());
        assert_eq!(armed.len(), 3);
    }

    /// The whole of `G362`: two dialogs raised together are answered separately, nothing is
    /// submitted while either is still open, and the last one to settle carries both.
    #[test]
    fn a_batch_submits_once_everything_in_it_has_been_answered() {
        let armed = Armed::new();
        let agent = AgentId::generate();
        let first = armed.arm(agent, vec![question("First")]);
        let second = armed.arm(agent, vec![question("Second")]);
        assert_eq!(armed.fire(agent).len(), 2);

        // The first answer is held, not submitted.
        match armed.answer(first, &said("Left")) {
            Answering::Waiting { left } => assert_eq!(left, 1),
            _ => panic!("the second dialog is still on screen"),
        }
        // And the row is still in the table, holding what was said.
        assert_eq!(armed.len(), 2);

        let whole = settled(armed.answer(second, &said("Right")));
        assert_eq!(whole.asks.len(), 2);
        assert_eq!(whole.asks[0].0[0].header, "First");
        assert_eq!(whole.asks[1].0[0].header, "Second");
        assert!(armed.is_empty());
    }

    /// A dialog the user dismisses rather than answers must not wedge the rest: it settles with
    /// nothing to say and the batch goes on the moment the last one is in.
    #[test]
    fn dismissing_one_dialog_of_a_batch_does_not_hold_the_others() {
        let armed = Armed::new();
        let agent = AgentId::generate();
        let first = armed.arm(agent, vec![question("First")]);
        let second = armed.arm(agent, vec![question("Second")]);
        armed.fire(agent);

        match armed.close(first) {
            Answering::Waiting { left } => assert_eq!(left, 1),
            _ => panic!("the second dialog is still on screen"),
        }
        let whole = settled(armed.answer(second, &said("Right")));
        assert_eq!(whole.asks.len(), 1, "only the one that was answered");
        assert_eq!(whole.asks[0].0[0].header, "Second");
        assert!(armed.is_empty());

        // The same the other way round: the dismissal arriving last still completes the batch.
        let third = armed.arm(agent, vec![question("Third")]);
        let fourth = armed.arm(agent, vec![question("Fourth")]);
        armed.fire(agent);
        assert!(matches!(
            armed.answer(third, &said("Left")),
            Answering::Waiting { left: 1 }
        ));
        let whole = settled(armed.close(fourth));
        assert_eq!(whole.asks.len(), 1);
        assert_eq!(whole.asks[0].0[0].header, "Third");
    }

    /// Every dialog of a batch given up on settles it with nothing, and `prose` refuses to open a
    /// turn over that.
    #[test]
    fn a_batch_nobody_answered_submits_nothing() {
        let armed = Armed::new();
        let agent = AgentId::generate();
        let first = armed.arm(agent, vec![question("First")]);
        let second = armed.arm(agent, vec![question("Second")]);
        armed.fire(agent);

        assert!(matches!(armed.close(first), Answering::Waiting { left: 1 }));
        let whole = settled(armed.answer(second, &AskOutcome::Chat));
        assert!(whole.asks.is_empty());
        assert!(prose(&whole).is_none());
        assert!(armed.is_empty());
    }

    /// The turn after the one that armed a batch arms its own, and the two never mix: a second
    /// `fire` mints a second batch.
    #[test]
    fn a_later_turns_registrations_are_a_batch_of_their_own() {
        let armed = Armed::new();
        let agent = AgentId::generate();
        let first = armed.arm(agent, vec![question("First")]);
        armed.fire(agent);
        let second = armed.arm(agent, vec![question("Second")]);
        armed.fire(agent);

        // Neither waits for the other.
        assert_eq!(settled(armed.answer(first, &said("Left"))).asks.len(), 1);
        assert_eq!(settled(armed.answer(second, &said("Right"))).asks.len(), 1);
        assert!(armed.is_empty());
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
        assert_eq!(settled(armed.answer(raised, &said("Left"))).asks.len(), 1);
    }

    #[test]
    fn only_a_raised_dialog_can_be_answered() {
        let armed = Armed::new();
        let agent = AgentId::generate();
        let ask_id = armed.arm(agent, vec![question("First")]);

        // Registered but not yet raised: no dialog exists to have answered it.
        assert!(matches!(
            armed.answer(ask_id, &said("Left")),
            Answering::NotOurs
        ));
        armed.fire(agent);
        assert_eq!(settled(armed.answer(ask_id, &said("Left"))).asks.len(), 1);
        // And the row is gone, so a second answer for the same id is not ours either.
        assert!(matches!(
            armed.answer(ask_id, &said("Right")),
            Answering::NotOurs
        ));
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

    /// A batch half-answered when the conversation goes submits nothing, and only the dialog
    /// still open is named back — telling the window about the one already settled would rewrite
    /// what it shows as said.
    #[test]
    fn a_conversation_going_mid_batch_submits_nothing_and_names_only_what_is_open() {
        let armed = Armed::new();
        let agent = AgentId::generate();
        let first = armed.arm(agent, vec![question("First")]);
        let second = armed.arm(agent, vec![question("Second")]);
        armed.fire(agent);
        assert!(matches!(
            armed.answer(first, &said("Left")),
            Answering::Waiting { left: 1 }
        ));

        assert_eq!(armed.close_for_agent(agent), vec![second]);
        assert!(armed.is_empty());
    }

    /// A dialog handed over by a parked call that gave up joins already raised, so it is
    /// answerable at once and a failing turn does not drop it (`D191`).
    #[test]
    fn a_handed_over_dialog_is_answerable_at_once_and_survives_a_failed_turn() {
        let armed = Armed::new();
        let agent = AgentId::generate();
        let ask_id = AskId::generate();
        armed.adopt(agent, ask_id, vec![question("Direction")]);

        assert_eq!(armed.len(), 1);
        // Nothing to fire: it is already on screen.
        assert!(armed.fire(agent).is_empty());
        assert_eq!(armed.disarm(agent), 0);

        let whole = settled(armed.answer(ask_id, &said("Left")));
        assert_eq!(whole.arming, Arming::HandedOver);
        assert_eq!(whole.asks[0].0[0].header, "Direction");
        assert!(armed.is_empty());
    }

    /// A handed-over dialog is its own batch: it does not wait on whatever the turn goes on to
    /// register, and a registration does not wait on it.
    #[test]
    fn a_handed_over_dialog_waits_for_nothing_else() {
        let armed = Armed::new();
        let agent = AgentId::generate();
        let handed = AskId::generate();
        armed.adopt(agent, handed, vec![question("Direction")]);
        let registered = armed.arm(agent, vec![question("Later")]);
        assert_eq!(armed.fire(agent).len(), 1, "only the registration");

        assert_eq!(settled(armed.answer(handed, &said("Left"))).asks.len(), 1);
        assert_eq!(
            settled(armed.answer(registered, &said("Right"))).asks.len(),
            1
        );
        assert!(armed.is_empty());
    }

    fn one(header: &str, label: &str) -> (Vec<AskQuestion>, Vec<AskAnswer>) {
        (
            vec![question(header)],
            vec![AskAnswer {
                question: 0,
                chosen: vec![label.to_string()],
                other: None,
                notes: None,
            }],
        )
    }

    #[test]
    fn the_prose_says_which_mode_the_answer_came_from() {
        let asks = vec![one("Direction", "Left")];
        let registered = prose(&Settled {
            arming: Arming::Registered,
            asks: asks.clone(),
        })
        .expect("something was said");
        assert!(registered.contains("you registered"), "{registered}");
        let handed = prose(&Settled {
            arming: Arming::HandedOver,
            asks,
        })
        .expect("something was said");
        assert!(handed.contains("without an answer"), "{handed}");
    }

    /// A whole batch reads as one message, in the order the dialogs were raised, under an opening
    /// line that says there were several (`G362`).
    #[test]
    fn a_batch_reads_as_one_message_in_the_order_it_was_raised() {
        let text = prose(&Settled {
            arming: Arming::Registered,
            asks: vec![one("First", "Left"), one("Second", "Right")],
        })
        .expect("something was said");
        assert!(text.contains("every question you registered"), "{text}");
        let first = text.find("First").expect("the first question");
        let second = text.find("Second").expect("the second question");
        assert!(first < second, "{text}");
    }

    #[test]
    fn the_prose_reads_as_what_the_user_said() {
        let asks = vec![(
            vec![question("Direction")],
            vec![AskAnswer {
                question: 0,
                chosen: vec!["Left".to_string()],
                other: None,
                notes: Some("but only for now".to_string()),
            }],
        )];
        let text = prose(&Settled {
            arming: Arming::Registered,
            asks,
        })
        .expect("something was said");
        assert!(text.contains("Direction — Which way?"), "{text}");
        assert!(text.contains("- Answer: Left"), "{text}");
        assert!(text.contains("- Notes: but only for now"), "{text}");
    }
}
