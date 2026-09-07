//! Where every prompt string lives.
//!
//! One function per [`SuggestSubject`], turning the material the coordinator gathered into a
//! [`Request`]. Nothing here reads a repository or a file: the caller brings what it already holds,
//! so a prompt is composed on the coordinator's own terms and never travels the bus.
//!
//! Each function truncates its material against the [`AssistLimits`] it is given, because the
//! half that knows the budget is the half that must cut.

use ubiq_proto::assist::AssistLimits;
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
