//! An agent's structured question to the user, as the window holds it.
//!
//! **One record per [`AskId`], kept on the conversation that raised it.** Several agents across
//! several projects may be waiting on a human at once, and an ask is a thing that agent asked —
//! so it is filed beside `Conversation::pending`, for that field's reason: a side channel joined
//! to the transcript by id rather than a block in it. A single form on the window would make the
//! second ask overwrite the first, and the second agent is still blocked.
//!
//! **The dialog is a view, not the ask.** [`AskDialog`] names which record is on screen and which
//! tab of it; everything the user has filled in lives on the record. That is what lets the dialog
//! be closed without confirming and reopened with the drafts intact — closing takes the view away
//! and touches nothing that was typed.
//!
//! **"Other" is an option position, not a field.** It is drawn as one more choice at index
//! `options.len()`, so picking it obeys the same single- or multi-select rule as everything above
//! it; what was typed under it rides back in [`AskAnswer::other`] rather than as a label, because
//! no agent wrote it.

use ubiq_proto::ask::{AskAnswer, AskClosed, AskQuestion};
use ubiq_proto::ids::AskId;
use ubiq_proto::work::AgentId;

/// What the user has put into one question so far.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AskDraft {
    /// Which options are picked, by position. `options.len()` is "Other".
    pub chosen: Vec<usize>,
    /// What was typed under "Other".
    pub other: String,
    /// Whatever else the user wanted to say about this question.
    pub notes: String,
}

/// Where an ask has got to. Only [`AskStage::Waiting`] can still be answered; every other reading
/// makes the dialog a record of what was said.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AskStage {
    /// Nobody has answered yet, and the harness is parked on it.
    Waiting,
    /// Confirmed, with what went back.
    Answered(Vec<AskAnswer>),
    /// The user would rather talk about it.
    Chatted,
    /// The host stopped waiting — it timed out, or the conversation went away.
    Ended(AskClosed),
}

/// One ask, its questions, what has been filled in, and how it ended.
#[derive(Clone, Debug, PartialEq)]
pub struct AskRecord {
    pub ask_id: AskId,
    pub questions: Vec<AskQuestion>,
    /// One per question, in step with [`Self::questions`].
    pub drafts: Vec<AskDraft>,
    pub stage: AskStage,
    /// How many blocks the conversation held when this ask was filed — `conversation.blocks.len()`
    /// at that moment, not this ask's own position in [`Conversation::asks`].
    ///
    /// The transcript row this ask draws is placed just before the first block-anchored row whose
    /// block index is `>= at_block`, which is what puts it where it actually arrived rather than
    /// always at the tail: a block appended later has a higher index, so the row stays put as the
    /// transcript grows underneath it.
    pub at_block: usize,
    /// Whose transcript the entry is drawn in: the id of the spawned subagent that asked, or
    /// `None` for the conversation's own turns.
    ///
    /// **One ask, one transcript.** A conversation and every subagent it spawned share one
    /// `AgentId` — a delegate's tool calls come down the same stream and reach the host's MCP
    /// listener under the same key — so an ask with no owner here would be drawn in the main
    /// agent's transcript *and* in every delegate's, which is one question wearing several
    /// faces. Read off who was speaking when the ask landed; see
    /// [`Conversation::asking_subagent`](crate::state::conversation::Conversation::asking_subagent).
    pub subagent: Option<String>,
}

impl AskRecord {
    pub fn new(
        ask_id: AskId,
        questions: Vec<AskQuestion>,
        at_block: usize,
        subagent: Option<String>,
    ) -> Self {
        let drafts = vec![AskDraft::default(); questions.len()];
        Self {
            ask_id,
            questions,
            drafts,
            stage: AskStage::Waiting,
            at_block,
            subagent,
        }
    }

    /// Whether anything the user does still reaches the agent. A dialog reopened after this is a
    /// transcript of the exchange, and draws no control that would send.
    pub fn live(&self) -> bool {
        self.stage == AskStage::Waiting
    }

    /// The answers that went back, where any did.
    pub fn answers(&self) -> Option<&[AskAnswer]> {
        match &self.stage {
            AskStage::Answered(answers) => Some(answers),
            _ => None,
        }
    }

    /// Which position "Other" occupies for one question: one past the options the agent wrote.
    pub fn other_at(&self, question: usize) -> usize {
        self.questions.get(question).map_or(0, |q| q.options.len())
    }

    /// Pick or unpick one option. Multi-select toggles; single-select replaces, and picking the
    /// one already picked unpicks it — a question the user changed their mind about having
    /// answered at all is not a question they can only answer.
    pub fn toggle(&mut self, question: usize, option: usize) {
        let multi = self.questions.get(question).is_some_and(|q| q.multi_select);
        let Some(draft) = self.drafts.get_mut(question) else {
            return;
        };
        match draft.chosen.iter().position(|at| *at == option) {
            Some(at) => {
                draft.chosen.remove(at);
            }
            None => {
                if !multi {
                    draft.chosen.clear();
                }
                draft.chosen.push(option);
                draft.chosen.sort_unstable();
            }
        }
    }

    pub fn picked(&self, question: usize, option: usize) -> bool {
        self.drafts
            .get(question)
            .is_some_and(|draft| draft.chosen.contains(&option))
    }

    /// Whether one question counts as answered: something is picked, and "Other" where it was
    /// picked has something written under it — an empty "Other" is a question with nothing said
    /// about it wearing an answer's clothes.
    ///
    /// The single reading of that rule. [`Self::ready`] and the footer's "still to answer" note
    /// both ask it, so the note can never claim everything is answered under a dim Confirm.
    pub fn answered(&self, question: usize) -> bool {
        let other = self.other_at(question);
        self.drafts.get(question).is_some_and(|draft| {
            !draft.chosen.is_empty()
                && (!draft.chosen.contains(&other) || !draft.other.trim().is_empty())
        })
    }

    /// Whether Confirm does anything: every question is answered, by [`Self::answered`]'s reading.
    pub fn ready(&self) -> bool {
        self.live()
            && !self.questions.is_empty()
            && (0..self.questions.len()).all(|at| self.answered(at))
    }

    /// What goes back on the wire. Labels rather than positions, in the order the agent offered
    /// them, and the two free-text fields only where something was written.
    pub fn answer(&self) -> Vec<AskAnswer> {
        self.questions
            .iter()
            .enumerate()
            .map(|(at, question)| {
                let draft = self.drafts.get(at).cloned().unwrap_or_default();
                let chosen = draft
                    .chosen
                    .iter()
                    .filter_map(|pick| question.options.get(*pick))
                    .map(|option| option.label.clone())
                    .collect();
                let wrote = draft.chosen.contains(&question.options.len());
                AskAnswer {
                    question: at,
                    chosen,
                    other: (wrote && !draft.other.trim().is_empty())
                        .then(|| draft.other.trim().to_string()),
                    notes: (!draft.notes.trim().is_empty()).then(|| draft.notes.trim().to_string()),
                }
            })
            .collect()
    }
}

/// Which ask is on screen, and which of its questions. The drafts are the record's; this is only
/// the view over one of them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AskDialog {
    pub agent_id: AgentId,
    pub ask_id: AskId,
    /// Which question's tab is up.
    pub tab: usize,
    /// The option the keyboard is on, for the current tab — `options.len()` is "Other". Reset to
    /// `0` on every tab switch, since a cursor left where the previous question's list ended would
    /// read as a pick on a question it never visited.
    pub cursor: usize,
}
