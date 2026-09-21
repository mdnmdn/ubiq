//! A question an agent asks the user, and the answer that goes back to it.
//!
//! This is the one tool call in Ubiq that does not answer itself. Every other MCP tool reads or
//! writes something the host already holds and returns on the listener's own thread; this one
//! parks, a human is asked, and the harness waits — so the exchange needs a vocabulary on the
//! wire, and this module is it.
//!
//! **The shape is Claude Code's `AskUserQuestion`, deliberately.** A harness that already knows
//! how to ask a structured question should not have to learn a second form of the same question,
//! so the fields, the counts and the limits below are that tool's — one to four questions, two to
//! four options each, a header short enough to read as a chip, previews on single-select only.
//! `_docs/inbox/feedback-tools.md` is the schema this was taken from.
//!
//! **The answer is not only a set of picks.** A user who would rather talk than choose says so —
//! [`AskOutcome::Chat`] — and the tool call ends without an answer to any question. That is a real
//! outcome, not a failure: the agent is told the user wants to discuss it, and the next thing it
//! reads is the user's own words in the transcript.
//!
//! **Labels, not indices, travel back.** An answer names the option it picked by its label, so a
//! harness reading the result never has to hold the question list to make sense of it, and a
//! result that outlives the ask still says what was chosen. The free-text answer rides beside the
//! picks rather than as one of them: "Other" is always offered and is never one of the options the
//! agent wrote.

use serde::{Deserialize, Serialize};

/// The most questions one ask may carry. More than this and the dialog reads as a form.
pub const MAX_QUESTIONS: usize = 4;
/// The fewest options a question may offer. One option is not a question.
pub const MIN_OPTIONS: usize = 2;
/// The most options a question may offer.
pub const MAX_OPTIONS: usize = 4;
/// The longest a header may be. It is drawn as a tab label, and a tab strip of four sentences is
/// not a tab strip.
pub const HEADER_MAX: usize = 12;

/// One question, with everything the dialog needs to draw it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AskQuestion {
    /// The question in full, as it is asked. Ends with a question mark, by the tool's own rule.
    pub question: String,
    /// The short label the tab wears — at most [`HEADER_MAX`] characters.
    pub header: String,
    /// What may be picked, between [`MIN_OPTIONS`] and [`MAX_OPTIONS`] of them.
    pub options: Vec<AskOption>,
    /// Whether more than one option may be picked at once.
    #[serde(default)]
    pub multi_select: bool,
}

/// One answer a question offers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AskOption {
    /// What the control says — a few words.
    pub label: String,
    /// What picking it means, drawn under the label.
    #[serde(default)]
    pub description: String,
    /// Something to show beside the choice: a snippet, a sketch, a diff. Single-select only —
    /// a preview belongs to *the* choice, and several at once has nowhere to be drawn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview: Option<String>,
}

/// What the user said about one question.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AskAnswer {
    /// Which question this answers, by position in the ask's own list.
    pub question: usize,
    /// The labels picked, in the order they were offered. Empty where the user only wrote.
    #[serde(default)]
    pub chosen: Vec<String>,
    /// What the user wrote under "Other", which every question offers and no agent writes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub other: Option<String>,
    /// Whatever else the user wanted the agent to know about this question.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

/// How an ask ended.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AskOutcome {
    /// The user confirmed: one entry per question they answered.
    Answered(Vec<AskAnswer>),
    /// The user would rather talk about it. No question is answered and the dialog is dismissed;
    /// the agent is told so, and reads the rest in the transcript.
    Chat,
}

/// Why an ask stopped waiting without the user ending it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AskClosed {
    /// Nobody answered within [`ASK_TIMEOUT_SECS`]; the tool call has already been answered as
    /// unanswered and the dialog can no longer send anything.
    Timeout,
    /// The conversation that raised it is gone — ended, unloaded, or its harness died.
    Gone,
}

/// How long an ask waits for a human before it gives up. Long, because a question worth asking is
/// worth walking away from and coming back to; bounded, because the harness is blocked meanwhile
/// and a tool call that never returns is a wedged agent.
pub const ASK_TIMEOUT_SECS: u64 = 3600;

impl AskQuestion {
    /// Whether this question is one the dialog can draw and the user can answer, and what is wrong
    /// with it if not. The host checks every question before it parks a call: a malformed ask is
    /// answered as an error on the spot rather than raised at a user who cannot make sense of it.
    pub fn check(&self) -> Result<(), String> {
        if self.question.trim().is_empty() {
            return Err("a question with no text".to_string());
        }
        if self.header.trim().is_empty() {
            return Err("a question with no header".to_string());
        }
        if self.header.chars().count() > HEADER_MAX {
            return Err(format!(
                "header `{}` is longer than {HEADER_MAX} characters",
                self.header
            ));
        }
        if self.options.len() < MIN_OPTIONS || self.options.len() > MAX_OPTIONS {
            return Err(format!(
                "question `{}` offers {} options; {MIN_OPTIONS} to {MAX_OPTIONS} are allowed",
                self.header,
                self.options.len()
            ));
        }
        if self
            .options
            .iter()
            .any(|option| option.label.trim().is_empty())
        {
            return Err(format!(
                "question `{}` has an unlabelled option",
                self.header
            ));
        }
        if self.multi_select && self.options.iter().any(|option| option.preview.is_some()) {
            return Err(format!(
                "question `{}` is multi-select and carries a preview",
                self.header
            ));
        }
        Ok(())
    }
}

/// Whether a whole ask is drawable, and what is wrong with it if not.
pub fn check(questions: &[AskQuestion]) -> Result<(), String> {
    if questions.is_empty() {
        return Err("an ask with no questions".to_string());
    }
    if questions.len() > MAX_QUESTIONS {
        return Err(format!(
            "{} questions; at most {MAX_QUESTIONS} are allowed",
            questions.len()
        ));
    }
    questions.iter().try_for_each(AskQuestion::check)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn option(label: &str) -> AskOption {
        AskOption {
            label: label.to_string(),
            description: String::new(),
            preview: None,
        }
    }

    fn question() -> AskQuestion {
        AskQuestion {
            question: "Which way?".to_string(),
            header: "Direction".to_string(),
            options: vec![option("Left"), option("Right")],
            multi_select: false,
        }
    }

    #[test]
    fn a_well_formed_ask_passes() {
        assert!(check(&[question()]).is_ok());
    }

    #[test]
    fn an_ask_with_no_questions_is_refused() {
        assert!(check(&[]).is_err());
    }

    #[test]
    fn too_many_questions_are_refused() {
        let many = vec![question(), question(), question(), question(), question()];
        assert!(check(&many).is_err());
    }

    #[test]
    fn a_long_header_is_refused() {
        let mut one = question();
        one.header = "a header far too long".to_string();
        assert!(one.check().is_err());
    }

    #[test]
    fn one_option_is_not_a_question() {
        let mut one = question();
        one.options.truncate(1);
        assert!(one.check().is_err());
    }

    #[test]
    fn a_preview_on_a_multi_select_is_refused() {
        let mut one = question();
        one.multi_select = true;
        one.options[0].preview = Some("fn main() {}".to_string());
        assert!(one.check().is_err());
    }
}
