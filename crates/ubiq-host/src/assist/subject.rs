//! Where every prompt string lives.
//!
//! One function per [`SuggestSubject`], turning the material the coordinator gathered into a
//! [`Request`]. Nothing here reads a repository or a file: the caller brings what it already holds,
//! so a prompt is composed on the coordinator's own terms and never travels the bus.
//!
//! Each function truncates its material against the [`AssistLimits`] it is given, because the
//! half that knows the budget is the half that must cut.

use ubiq_proto::assist::{AssistLimits, ModelRole};
use ubiq_proto::git::{GitEntry, GitMark};

use super::Request;

/// Characters per token, as a rough estimate.
///
/// Deliberately crude: no tokeniser for the backend's own vocabulary is available here, and one
/// would have to be a second one for every future provider. Four characters per token is the
/// common English approximation, and it is used *under*-optimistically — the budget it computes is
/// smaller than the real window, so an estimate that is wrong is wrong in the direction of
/// cutting too much rather than overflowing the session.
const CHARS_PER_TOKEN: usize = 4;

/// Tokens held back for the model's own answer, which a commit subject line barely touches.
const RESPONSE_TOKENS: u32 = 32;

/// What the answer may be at most. One short line, so a runaway generation ends early.
const MAX_RESPONSE_TOKENS: u32 = 64;

/// What a provider check may answer at most. Longer than a subject line on purpose: a check is
/// watched arriving, and a few sentences is enough to see it stream without being enough to cost
/// anything.
const CHECK_RESPONSE_TOKENS: u32 = 160;

const COMMIT_INSTRUCTIONS: &str = "\
You write git commit messages. Given a summary of the changed files in a repository, reply with \
exactly one short imperative commit subject line — under 72 characters, no trailing full stop, no \
prefix, no quotes, no explanation, and nothing after it.";

const COMMIT_LEAD: &str = "Changed files:\n";

/// The prompt for [`ubiq_proto::assist::SuggestSubject::CommitMessage`].
///
/// `changes` is the repository's own list of paths that have something to say — the diff *stat*,
/// not the hunks: a model naming a change needs to know what moved, and every hunk of a large
/// change would not fit a session anyway.
pub fn commit_message(changes: &[GitEntry], limits: &AssistLimits) -> Request {
    let budget = (limits.context_tokens.saturating_sub(RESPONSE_TOKENS) as usize)
        .saturating_mul(CHARS_PER_TOKEN)
        .saturating_sub(COMMIT_INSTRUCTIONS.len() + COMMIT_LEAD.len());

    let mut prompt = String::from(COMMIT_LEAD);
    let mut listed = 0usize;
    for entry in changes {
        let line = format!("{} {}\n", letter(entry), entry.rel_path);
        if prompt.len() - COMMIT_LEAD.len() + line.len() > budget {
            break;
        }
        prompt.push_str(&line);
        listed += 1;
    }
    if listed < changes.len() {
        prompt.push_str(&format!("… and {} more paths\n", changes.len() - listed));
    }

    Request {
        instructions: COMMIT_INSTRUCTIONS.to_string(),
        prompt,
        max_tokens: Some(MAX_RESPONSE_TOKENS),
        // Naming a commit is what the fast model is for; a subject that wanted the other one
        // would say so here, and none does yet.
        role: ModelRole::Fast,
    }
}

const CHECK_INSTRUCTIONS: &str = "\
You are being tested by the application that just configured you. Reply with two or three short \
sentences confirming that you can be reached, and nothing else.";

const CHECK_PROMPT: &str = "\
Confirm you are working. Say which model you are, if you know, and describe in one sentence what \
you are for.";

/// The prompt for [`ubiq_proto::assist::SuggestSubject::ProviderCheck`].
///
/// Deliberately asks for a few sentences rather than a word. This subject exists to show a user
/// that a provider they just configured answers, and an answer arriving a token at a time is the
/// half of that which a single word could not demonstrate — so the response ceiling here is the
/// only one in this module that is not a line.
///
/// It takes no material and so needs no budget: there is nothing to cut, and the limits a
/// backend reports are exactly what this is checking.
pub fn provider_check(role: ModelRole) -> Request {
    Request {
        instructions: CHECK_INSTRUCTIONS.to_string(),
        prompt: CHECK_PROMPT.to_string(),
        max_tokens: Some(CHECK_RESPONSE_TOKENS),
        role,
    }
}

/// What a naming may answer at most. Two short lines, so a model that would rather explain itself
/// is cut off instead of indulged.
const NAMING_RESPONSE_TOKENS: u32 = 48;

const NAMING_INSTRUCTIONS: &str = "\
You name conversations between a developer and a coding assistant. Given the opening exchange of \
one, reply with exactly two lines and nothing else. The first line is a title of at most six \
words, capitalised as a heading, with no trailing full stop, no quotes and no prefix. The second \
line says what the conversation is about in exactly five words, with no trailing full stop.";

const NAMING_ASKED: &str = "Asked:\n";

const NAMING_ANSWERED: &str = "\n\nAnswered:\n";

/// The prompt that names a conversation.
///
/// The one prompt in this module with no [`ubiq_proto::assist::SuggestSubject`] behind it: naming
/// a conversation is not something a window asks for, it is something the host notices it can do
/// once an agent has answered its first prompt. The wording still belongs here, with every other
/// wording, so that a naming is tested the same way a commit message is.
///
/// `asked` is the opening turn and `answered` is the agent's first reply, which together are the
/// only part of a conversation that is about what the user came to do — a later turn is about
/// where the work got to, and a title that followed it would keep moving.
pub fn conversation_title(asked: &str, answered: &str, limits: &AssistLimits) -> Request {
    let budget = (limits.context_tokens.saturating_sub(NAMING_RESPONSE_TOKENS) as usize)
        .saturating_mul(CHARS_PER_TOKEN)
        .saturating_sub(NAMING_INSTRUCTIONS.len() + NAMING_ASKED.len() + NAMING_ANSWERED.len());

    // Two arbitrary lengths against one budget, so each half gets half of it: a long opening
    // prompt cannot crowd out the reply that says what was done about it, and a long reply
    // cannot bury the request that a title is mostly named after.
    let half = budget / 2;
    let mut prompt = String::from(NAMING_ASKED);
    prompt.push_str(clip(asked.trim(), half));
    prompt.push_str(NAMING_ANSWERED);
    prompt.push_str(clip(answered.trim(), budget.saturating_sub(half)));

    Request {
        instructions: NAMING_INSTRUCTIONS.to_string(),
        prompt,
        max_tokens: Some(NAMING_RESPONSE_TOKENS),
        // Naming is the fast model's work, the same as naming a commit.
        role: ModelRole::Fast,
    }
}

/// The two lines a naming answered, split into a title and its summary.
///
/// The wording that asks for the format is in this module, so the reading of it belongs here too.
/// A model that ignored the format and wrote one line has still given a title, and a title with
/// no summary is worth using — the summary is a tooltip, and a tooltip is allowed to be absent.
/// `None` is only for an answer with no usable line in it at all.
pub fn naming(answer: &str) -> Option<(String, Option<String>)> {
    let mut lines = answer
        .lines()
        .map(undecorate)
        .filter(|line| !line.is_empty());
    let title = lines.next()?;
    Some((title, lines.next()))
}

/// One answered line without the decoration a model adds despite being asked not to — a `Title:`
/// label, a bullet, or quotes around the whole of it.
fn undecorate(line: &str) -> String {
    let line = line.trim().trim_start_matches(['-', '*', '#']).trim();
    let line = match line.split_once(':') {
        Some((label, rest))
            if matches!(
                label.trim().to_ascii_lowercase().as_str(),
                "title" | "summary"
            ) =>
        {
            rest
        }
        _ => line,
    };
    line.trim()
        .trim_matches(['"', '\'', '`'])
        .trim()
        .to_string()
}

/// The first `budget` characters of `text`, cut on a character boundary.
///
/// Characters rather than bytes because the budget is an estimate of tokens and `&text[..n]` on a
/// byte that is not a boundary is a panic — a conversation carries whatever the user typed, so
/// this is the one subject whose material is not ASCII by construction.
fn clip(text: &str, budget: usize) -> &str {
    match text.char_indices().nth(budget) {
        Some((at, _)) => &text[..at],
        None => text,
    }
}

/// The one-letter status a `git status --short` reader already knows.
fn letter(entry: &GitEntry) -> char {
    match entry.mark() {
        Some(GitMark::Conflict) => 'U',
        Some(GitMark::Untracked) => 'A',
        Some(GitMark::Staged) => 'S',
        Some(GitMark::Modified) => 'M',
        Some(GitMark::Ignored) | None => '?',
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ubiq_proto::git::GitPathChange;

    fn changed(rel_path: &str) -> GitEntry {
        GitEntry {
            rel_path: rel_path.to_string(),
            index: None,
            worktree: Some(GitPathChange::Modified),
            conflicted: false,
            ignored: false,
        }
    }

    fn limits(context_tokens: u32) -> AssistLimits {
        AssistLimits {
            label: "Fake model".into(),
            context_tokens,
        }
    }

    #[test]
    fn a_small_change_is_listed_whole() {
        let changes = [changed("src/a.rs"), changed("src/b.rs")];
        let request = commit_message(&changes, &limits(4096));
        assert!(request.prompt.contains("M src/a.rs"));
        assert!(request.prompt.contains("M src/b.rs"));
        assert!(!request.prompt.contains("more paths"));
    }

    #[test]
    fn a_check_asks_the_role_it_was_given_and_carries_no_material() {
        let request = provider_check(ModelRole::Smart);
        assert_eq!(request.role, ModelRole::Smart);
        assert_eq!(request.max_tokens, Some(CHECK_RESPONSE_TOKENS));
        assert!(!request.prompt.is_empty());
    }

    #[test]
    fn a_naming_carries_both_halves_of_the_opening_exchange() {
        let request = conversation_title(
            "make the sidebar collapse",
            "I have added a fold control to the sidebar header.",
            &limits(4096),
        );
        assert!(request.prompt.contains("make the sidebar collapse"));
        assert!(request.prompt.contains("fold control"));
        assert_eq!(request.max_tokens, Some(NAMING_RESPONSE_TOKENS));
        // Naming is the cheap model's work, whatever the smart one is set to.
        assert_eq!(request.role, ModelRole::Fast);
    }

    #[test]
    fn a_naming_cuts_each_half_against_half_the_budget() {
        // A first prompt long enough to have swallowed the whole window on its own. The reply
        // has to survive it, or a title would be named after the question and never the answer.
        let asked = "why ".repeat(4000);
        let answered = "because the loader ran twice. ".repeat(400);
        let request = conversation_title(&asked, &answered, &limits(200));
        assert!(
            request.prompt.len() < 200 * CHARS_PER_TOKEN,
            "the prompt overflowed its own estimate: {} chars",
            request.prompt.len()
        );
        assert!(
            request.prompt.contains("because the loader ran twice"),
            "the reply was crowded out by the question: {}",
            request.prompt
        );
    }

    #[test]
    fn a_naming_reads_two_lines_and_survives_one() {
        assert_eq!(
            naming("Sidebar Fold Control\nAdding a collapsible sidebar"),
            Some((
                "Sidebar Fold Control".to_string(),
                Some("Adding a collapsible sidebar".to_string())
            ))
        );
        // A model that answered a title and stopped has still given a usable name, and a tooltip
        // is allowed to be absent.
        assert_eq!(
            naming("Sidebar Fold Control"),
            Some(("Sidebar Fold Control".to_string(), None))
        );
        assert_eq!(naming("   \n\n "), None);
    }

    #[test]
    fn a_naming_strips_the_decoration_it_asked_a_model_not_to_add() {
        // Every one of these is a real thing a model does despite the instructions, and each
        // would otherwise become part of the name on a tab.
        assert_eq!(
            naming("Title: \"Sidebar Fold\"\n- Summary: adding a collapsible sidebar"),
            Some((
                "Sidebar Fold".to_string(),
                Some("adding a collapsible sidebar".to_string())
            ))
        );
        // A colon inside an ordinary title is not a label and must survive.
        assert_eq!(
            naming("Fix: the loader ran twice"),
            Some(("Fix: the loader ran twice".to_string(), None))
        );
    }

    #[test]
    fn a_naming_cuts_material_on_a_character_boundary() {
        // A conversation carries whatever the user typed, so this is the one subject whose
        // material is not ASCII by construction — a byte-wise cut here would be a panic.
        let asked = "\u{e9}\u{e9}\u{e9}".repeat(2000);
        let request = conversation_title(&asked, "d\u{fc}rfte gehen", &limits(150));
        assert!(!request.prompt.is_empty());
    }

    #[test]
    fn material_over_the_budget_is_actually_cut() {
        let changes: Vec<GitEntry> = (0..500)
            .map(|n| changed(&format!("crates/somewhere/deep/module_{n}.rs")))
            .collect();
        // Just enough window for the instructions and a handful of lines.
        let request = commit_message(&changes, &limits(120));
        assert!(request.prompt.contains("more paths"));
        assert!(
            request.prompt.len() < 120 * CHARS_PER_TOKEN,
            "the prompt overflowed its own estimate: {} chars",
            request.prompt.len()
        );
        assert!(!request.prompt.contains("module_499.rs"));
    }
}
