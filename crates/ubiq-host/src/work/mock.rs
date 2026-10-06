//! The work the host invents, until there is work to report.
//!
//! All of this is a fixture and stays one. Sessions are never written down — nothing runs behind
//! them, so every request is answered from here, and the day a session family exists this file
//! goes and nothing else changes. Agents are not invented here any more: the project's agents are
//! the live ones, and nothing else (T-330). Tasks are not seeded from here: a new project's board starts empty, and the tasks a user writes down are
//! theirs from the first one, kept in `tasks.toml` and never touched by this file.
//!
//! That is why the ids in this file are constants rather than minted. A mock minting a new ULID
//! each boot would still work — but a literal is what lets a hand-read `session` field in a
//! project's own records say which fixture row it means, and it costs nothing to keep. So the five
//! sessions carry ULID literals, parsed once, identical on every boot and in every process.
//!
//! The cost is that two projects' mock sessions share ids. That is
//! acceptable because a mock session is not yet a host object: there is no catalogue of them and no
//! uniqueness to violate, and an id is only ever compared against the other ids in the same
//! project's answer.

use std::str::FromStr;
use std::sync::LazyLock;

use ubiq_proto::ids::SessionId;
use ubiq_proto::work::WorkSession;

/// Parse one of the literals below.
///
/// A malformed literal is a bug in this file that no caller can do anything about, and the
/// alternative to panicking is a nil id that looks real. The unit test at the bottom means the
/// panic is reached in CI rather than on a user's first boot.
fn id<T: FromStr>(literal: &str) -> T
where
    T::Err: std::fmt::Debug,
{
    T::from_str(literal).expect("a mock id literal is a valid ULID")
}

/// The mock's five sessions, by the id each one keeps across every boot.
///
/// Readable literals rather than random ones, so a `session` field in a hand-read `tasks.toml`
/// says which fixture row it means. `Ulid::from_str` is not `const`, hence the lock.
static SESSIONS: LazyLock<[SessionId; 5]> = LazyLock::new(|| {
    [
        id("01M0CK00000000000000SESS01"),
        id("01M0CK00000000000000SESS02"),
        id("01M0CK00000000000000SESS03"),
        id("01M0CK00000000000000SESS04"),
        id("01M0CK00000000000000SESS05"),
    ]
});

/// Session *n*, one-based, so the fixture below reads the way it was written: `session(2)` is the
/// second row of [`sessions`].
fn session(n: usize) -> SessionId {
    SESSIONS[n - 1]
}

/// The sessions the work is filed under: four pieces of work in worktrees of their own,
/// and the project's own folder.
pub fn sessions() -> Vec<WorkSession> {
    vec![
        WorkSession {
            id: session(1),
            name: "fix/terminal-refit".to_string(),
            branch: "fix/terminal-refit".to_string(),
            worktree: true,
        },
        WorkSession {
            id: session(2),
            name: "feat/session-store".to_string(),
            branch: "feat/session-store".to_string(),
            worktree: true,
        },
        WorkSession {
            id: session(3),
            name: "spike/cold-start".to_string(),
            branch: "spike/cold-start".to_string(),
            worktree: true,
        },
        WorkSession {
            id: session(4),
            name: "fix/win-paths".to_string(),
            branch: "fix/win-paths".to_string(),
            worktree: true,
        },
        WorkSession {
            id: session(5),
            name: "main".to_string(),
            branch: "main".to_string(),
            worktree: false,
        },
    ]
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    /// Every literal parses and no two are the same. Both failures are typos in this file, and
    /// neither shows up as anything but an empty pill three screens away.
    #[test]
    fn every_mock_id_literal_parses_and_is_distinct() {
        let sessions: HashSet<_> = SESSIONS.iter().copied().collect();
        assert_eq!(sessions.len(), SESSIONS.len());
    }
}
