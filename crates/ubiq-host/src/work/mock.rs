//! The work the host invents, until there is work to report.
//!
//! All of this is a fixture and stays one. Sessions and agents are never written down — nothing
//! runs behind them, so every request is answered from here, and the day a session family and a
//! live agent exist this file loses those two functions and nothing else changes. Tasks are not
//! seeded from here: a new project's board starts empty, and the tasks a user writes down are
//! theirs from the first one, kept in `tasks.toml` and never touched by this file.
//!
//! That is why the ids in this file are constants rather than minted. An agent's `session` is
//! answered fresh on every boot, so a mock minting a new ULID each time would still work — but a
//! literal is what lets a hand-read `session` field in a project's own records say which fixture
//! row it means, and it costs nothing to keep. So the five sessions and the eleven agents carry
//! ULID literals, parsed once, identical on every boot and in every process.
//!
//! The cost is that two projects' mock sessions share ids, and so do their agents. That is
//! acceptable because a mock session is not yet a host object: there is no catalogue of them and no
//! uniqueness to violate, and an id is only ever compared against the other ids in the same
//! project's answer.

use std::str::FromStr;
use std::sync::LazyLock;

use ubiq_proto::ids::SessionId;
use ubiq_proto::work::{Activity, AgentId, Speaker, Turn, WorkAgent, WorkSession};

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

/// The mock's eleven agents, on the same scheme.
static AGENTS: LazyLock<[AgentId; 11]> = LazyLock::new(|| {
    [
        id("01M0CK00000000000000AGNT01"),
        id("01M0CK00000000000000AGNT02"),
        id("01M0CK00000000000000AGNT03"),
        id("01M0CK00000000000000AGNT04"),
        id("01M0CK00000000000000AGNT05"),
        id("01M0CK00000000000000AGNT06"),
        id("01M0CK00000000000000AGNT07"),
        id("01M0CK00000000000000AGNT08"),
        id("01M0CK00000000000000AGNT09"),
        id("01M0CK00000000000000AGNT10"),
        id("01M0CK00000000000000AGNT11"),
    ]
});

/// Session *n*, one-based, so the fixture below reads the way it was written: `session(2)` is the
/// second row of [`sessions`].
fn session(n: usize) -> SessionId {
    SESSIONS[n - 1]
}

/// Agent *n*, one-based, matching the order of [`agents`].
fn agent(n: usize) -> AgentId {
    AGENTS[n - 1]
}

/// The sessions the two screens over the work draw: four pieces of work in worktrees of their own,
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

/// The one agent every session's master answers to. Named rather than spelled `1` at four call
/// sites, because "the orchestrator" is the fact and its place in the list is not.
const ORCHESTRATOR: usize = 1;

/// The graph's cards: a project manager, and the ten agents under it.
///
/// **The orchestrator is the parent of every session's master agent.** It is the one card with no
/// parent, each session's root answers to it, and everyone else answers to the root of their own
/// session — so the spawn tree runs project, session, work, and the graph draws a connector from the
/// orchestrator down into each session below it.
///
/// A parent outside a card's own container is drawn across the boundary and does not stack it:
/// `state::layout`'s `inside` counts only a parent in the same container, so a session's master
/// stays on the top row of its task rather than being pushed down a level by a parent that is not
/// in the box. `WorkProjection::now` reads a coordinator the same way, so a coordinated task still
/// speaks through its own lead.
pub fn agents() -> Vec<WorkAgent> {
    vec![
        card(
            1,
            5,
            None,
            "Orchestrator",
            "Project manager",
            Activity::NeedsYou,
            "Waiting for your next instruction. Three tasks in flight.",
            "main",
            18_900.0,
        ),
        // Task 5 in the fixture's numbering: the terminal refit.
        card(
            2,
            1,
            Some(ORCHESTRATOR),
            "Fixer",
            "Implementer",
            Activity::Tools,
            "Running `cargo test panels` after the ResizeObserver guard.",
            "fix/terminal-refit",
            42_100.0,
        ),
        // Task 6: the session store migration, which agents 3 and 4 hand between them.
        card(
            3,
            2,
            Some(ORCHESTRATOR),
            "Spec",
            "Analyst",
            Activity::Ended,
            "Handed over a migration plan for the v1 \u{2192} v2 store schema.",
            "feat/session-store",
            31_700.0,
        ),
        card(
            4,
            2,
            Some(3),
            "Builder",
            "Implementer",
            Activity::Writing,
            "Writing the persist adapter and the v1 \u{2192} v2 migration.",
            "feat/session-store",
            58_300.0,
        ),
        // Task 7: the cold-start spike, one lead over four workers.
        card(
            5,
            3,
            Some(ORCHESTRATOR),
            "Perf lead",
            "Activity coordinator",
            Activity::Thinking,
            "Rebalancing the workers across the startup phases.",
            "spike/cold-start",
            26_400.0,
        ),
        card(
            6,
            3,
            Some(5),
            "Profiler",
            "Investigator",
            Activity::Tools,
            "Tracing the Tauri boot with `cargo flamegraph`.",
            "spike/cold-start",
            19_200.0,
        ),
        card(
            7,
            3,
            Some(5),
            "Rust dev",
            "Implementer",
            Activity::Writing,
            "Deferring the plugin registry behind a lazy init.",
            "spike/cold-start",
            37_000.0,
        ),
        card(
            8,
            3,
            Some(5),
            "Bench",
            "Verifier",
            Activity::Failed,
            "Harness exited 137 \u{2014} the bench run was killed under memory pressure.",
            "spike/cold-start",
            11_500.0,
        ),
        card(
            9,
            3,
            Some(5),
            "Scribe",
            "Documentation",
            Activity::NeedsYou,
            "Needs your call: publish the perf notes to the KB or keep them local?",
            "spike/cold-start",
            8_000.0,
        ),
        // Task 9: the release notes.
        card(
            10,
            5,
            Some(1),
            "Chronicler",
            "Documentation",
            Activity::Ended,
            "Grouped every change since 0.3.0 by the area it touched.",
            "main",
            14_600.0,
        ),
        // Task 8: the Windows paths, in review.
        card(
            11,
            4,
            Some(ORCHESTRATOR),
            "Porter",
            "Implementer",
            Activity::Ended,
            "Every path join in the host goes through one helper now.",
            "fix/win-paths",
            22_800.0,
        ),
    ]
}

/// One card. `id`, `session` and `parent` are the fixture's numbers, resolved through the constants
/// here so every call site stays a row of plain values.
///
/// `task` is not among them: a task's id belongs to whatever the project's own `tasks.toml` holds,
/// which this file never sees. An agent with no task draws as ungrouped, which is honest; giving
/// it one is `link`'s job in `work/mod.rs`, done against whatever the user has actually written
/// down.
#[allow(clippy::too_many_arguments)]
fn card(
    id: usize,
    session_n: usize,
    parent: Option<usize>,
    name: &str,
    role: &str,
    activity: Activity,
    note: &str,
    branch: &str,
    tokens: f32,
) -> WorkAgent {
    WorkAgent {
        id: agent(id),
        session: session(session_n),
        task: None,
        parent: parent.map(agent),
        name: name.to_string(),
        summary: None,
        role: role.to_string(),
        activity,
        note: note.to_string(),
        branch: branch.to_string(),
        tokens,
        harness: "Claude Code".to_string(),
        account: "work".to_string(),
        model: "Opus 4.6".to_string(),
        context_pct: ((tokens / 200_000.0) * 100.0).round() as u8,
        persistent: false,
        accept_all: false,
        debug_dump: None,
        // One line each: the last thing the agent said, which is also what its card prints.
        thread: vec![Turn {
            from: Speaker::Agent,
            text: note.to_string(),
        }],
    }
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

        let agents: HashSet<_> = AGENTS.iter().copied().collect();
        assert_eq!(agents.len(), AGENTS.len());
    }

    /// An agent's parent and session are drawn from the same two sets.
    #[test]
    fn mock_agents_reference_sessions_and_parents_that_exist() {
        let cards = agents();
        let sessions: HashSet<_> = sessions().into_iter().map(|s| s.id).collect();
        let ids: HashSet<_> = cards.iter().map(|a| a.id).collect();

        for card in &cards {
            assert!(sessions.contains(&card.session), "{}", card.name);
            if let Some(parent) = card.parent {
                assert!(ids.contains(&parent), "{}", card.name);
            }
        }
    }
}
