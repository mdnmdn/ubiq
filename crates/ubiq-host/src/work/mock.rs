//! The work the host invents, until there is work to report.
//!
//! Two thirds of this is a fixture and stays one. Sessions and agents are never written down —
//! nothing runs behind them, so every request is answered from here, and the day a session family
//! and a live agent exist this file loses those two functions and nothing else changes. The tasks
//! are different: they are seeded once into a project's `tasks.toml` and are the user's data from
//! then on. Editing a row here changes what a *new* project starts with, never what an existing one
//! holds.
//!
//! That difference is why the ids in this file are constants rather than minted. A task's `session`
//! and a step's `owner` are written down, so a seeded task points at a session id forever. If the
//! mock minted a fresh ULID per boot, every seeded task would name a session that no longer exists
//! and the board's session pills would come up empty on the second run. So the five sessions and
//! the eleven agents carry ULID literals, parsed once, identical on every boot and in every
//! process.
//!
//! The cost is that two projects' mock sessions share ids, and so do their agents. That is
//! acceptable because a mock session is not yet a host object: there is no catalogue of them and no
//! uniqueness to violate, and an id is only ever compared against the other ids in the same
//! project's answer. Task and step ids are minted, because they are written down on the first seed
//! and read back after it, and so never need to survive a boot in this file.

use std::str::FromStr;
use std::sync::LazyLock;

use chrono::Utc;
use ubiq_proto::ids::{SessionId, StepId, TaskId};
use ubiq_proto::work::{
    Activity, AgentId, Priority, Shape, Speaker, Status, Step, StepState, TaskRecord, Turn,
    WorkAgent, WorkSession,
};

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

/// The board's cards, in the order the columns read: three nobody has started, one ready to go,
/// three in flight, one waiting to be looked at, two finished.
///
/// Minted afresh on every call, so seeding two projects gives each its own rows. `now` for both
/// timestamps: a fixture has no history, and inventing one would put ages on the cards that no
/// event ever produced.
pub fn tasks() -> Vec<TaskRecord> {
    let now = Utc::now();
    vec![
        TaskRecord {
            id: TaskId::generate(),
            session: None,
            status: Status::Backlog,
            priority: Priority::High,
            shape: Shape::Coordinated,
            title: "Replace status polling with an event stream".to_string(),
            description: notes(&[
                "## Why",
                "",
                "The status bar asks the host what changed four times a second, and the answer is",
                "almost always **nothing**. The host already knows the moment a pane exits or a",
                "project's health flips \u{2014} it has nowhere to say it.",
                "",
                "- the poll lives in the status bar, one timer, one request",
                "- every reply rebuilds a snapshot the interface is already holding",
                "- a pane that exits between two ticks reads as alive for up to `250ms`",
                "",
                "The timer goes when the family lands. Nothing else in the interface polls.",
            ]),
            steps: unstarted(&[
                "Name the events the host already knows",
                "Add the family to the transport contract",
                "Replace the poll in the status bar",
                "Drop the timer",
            ]),
            created_at: now,
            updated_at: now,
        },
        TaskRecord {
            id: TaskId::generate(),
            session: None,
            status: Status::Backlog,
            priority: Priority::Normal,
            shape: Shape::Direct,
            title: "Keyboard shortcuts for the pane toggles".to_string(),
            description: String::new(),
            steps: unstarted(&[
                "Pick the three chords",
                "Bind them in the window",
                "Say so in the status bar",
            ]),
            created_at: now,
            updated_at: now,
        },
        TaskRecord {
            id: TaskId::generate(),
            session: None,
            status: Status::Backlog,
            priority: Priority::Low,
            shape: Shape::Chain,
            title: "Bundle size budget in CI".to_string(),
            description: String::new(),
            steps: unstarted(&[
                "Measure the release binary",
                "Pick the ceiling",
                "Fail the build over it",
            ]),
            created_at: now,
            updated_at: now,
        },
        TaskRecord {
            id: TaskId::generate(),
            session: Some(session(2)),
            status: Status::Ready,
            priority: Priority::Normal,
            shape: Shape::Direct,
            title: "Persist terminal scrollback per session".to_string(),
            description: String::new(),
            steps: unstarted(&[
                "Decide what a session keeps",
                "Write it down on exit",
                "Read it back on attach",
                "Cap what one pane may hold",
            ]),
            created_at: now,
            updated_at: now,
        },
        TaskRecord {
            id: TaskId::generate(),
            session: Some(session(1)),
            status: Status::InProgress,
            priority: Priority::Normal,
            shape: Shape::Direct,
            title: "Guard the 0\u{d7}0 resize callback".to_string(),
            description: notes(&[
                "## Repro",
                "",
                "Collapse the sidebar, reopen it, and the pane keeps the geometry it had before.",
                "The harness redraws at the old size and the screen tears \u{2014} the classic one.",
                "",
                "The observer re-attaches while the host still measures `0\u{d7}0`, so the first fit",
                "runs against nothing and **never fires again**.",
                "",
                "- ignore the zero-width measurement, fit on the first real one",
                "- the panel tests cover the refit, not the collapse that precedes it",
            ]),
            steps: vec![
                step("Reproduce the dropped fit", StepState::Done, Some(2)),
                step("Guard the observer", StepState::Done, Some(2)),
                step("Run the panel tests", StepState::Working, Some(2)),
            ],
            created_at: now,
            updated_at: now,
        },
        TaskRecord {
            id: TaskId::generate(),
            session: Some(session(2)),
            status: Status::InProgress,
            priority: Priority::Normal,
            shape: Shape::Chain,
            title: "Migrate the session store to persist v2".to_string(),
            description: String::new(),
            steps: vec![
                step("Plan the v1 \u{2192} v2 schema", StepState::Done, Some(3)),
                step("Write the persist adapter", StepState::Working, Some(4)),
                step("Backfill the existing stores", StepState::Idle, Some(4)),
            ],
            created_at: now,
            updated_at: now,
        },
        TaskRecord {
            id: TaskId::generate(),
            session: Some(session(3)),
            status: Status::InProgress,
            priority: Priority::High,
            shape: Shape::Coordinated,
            title: "Cut cold start under 800 ms".to_string(),
            description: notes(&[
                "## The budget",
                "",
                "800 ms cold, from exec to the **first painted frame**. We are at 1.4 s, and the",
                "plugin registry is most of it: every harness definition is parsed before the",
                "window exists.",
                "",
                "- defer the registry behind a lazy init",
                "- hold the number in CI, not in a doc nobody reads",
                "",
                "Measured the same way every time, so two runs are comparable:",
                "",
                "```",
                "hyperfine --warmup 3 'target/release/ubiq --exit-after-first-frame'",
                "```",
            ]),
            steps: vec![
                step("Measure the current boot budget", StepState::Done, Some(6)),
                step("Flamegraph the Tauri boot path", StepState::Done, Some(6)),
                step(
                    "Defer the plugin registry behind lazy init",
                    StepState::Working,
                    Some(7),
                ),
                step("Benchmark before and after", StepState::Failed, Some(8)),
                step(
                    "Decide where the perf notes live",
                    StepState::NeedsYou,
                    Some(9),
                ),
                step("Hold the budget in CI", StepState::Idle, Some(7)),
            ],
            created_at: now,
            updated_at: now,
        },
        TaskRecord {
            id: TaskId::generate(),
            session: Some(session(4)),
            status: Status::InReview,
            priority: Priority::Normal,
            shape: Shape::Direct,
            title: "Normalise Windows path separators".to_string(),
            description: String::new(),
            steps: vec![
                step(
                    "Find every path join in the host",
                    StepState::Done,
                    Some(11),
                ),
                step("Route them through one helper", StepState::Done, Some(11)),
                step("Add the round-trip test", StepState::Done, Some(11)),
            ],
            created_at: now,
            updated_at: now,
        },
        TaskRecord {
            id: TaskId::generate(),
            session: Some(session(5)),
            status: Status::Done,
            priority: Priority::Normal,
            shape: Shape::Direct,
            title: "Draft the 0.3.1 release notes".to_string(),
            description: String::new(),
            steps: vec![
                step("Read the range since 0.3.0", StepState::Done, Some(10)),
                step("Group the changes by area", StepState::Done, Some(10)),
            ],
            created_at: now,
            updated_at: now,
        },
        TaskRecord {
            id: TaskId::generate(),
            session: Some(session(5)),
            status: Status::Done,
            priority: Priority::Low,
            shape: Shape::Direct,
            title: "Kill a project's panes when it closes".to_string(),
            description: String::new(),
            steps: vec![
                step("Kill them on close", StepState::Done, Some(1)),
                step("Write the project's blob first", StepState::Done, Some(1)),
            ],
            created_at: now,
            updated_at: now,
        },
    ]
}

/// A ticket's notes, one source line per rendered line.
///
/// The markdown stays readable in this file, and the renderer gets exactly the bytes an editor
/// would have handed it. The host does not parse any of it.
fn notes(lines: &[&str]) -> String {
    lines.join("\n")
}

/// The steps of a task nobody has picked up: named, unowned, and not started.
fn unstarted(titles: &[&str]) -> Vec<Step> {
    titles
        .iter()
        .map(|title| step(title, StepState::Idle, None))
        .collect()
}

/// One step. `owner` is the fixture's agent number rather than an id, so the numbering above reads
/// as it always did and the constants are resolved in one place.
fn step(title: &str, state: StepState, owner: Option<usize>) -> Step {
    Step {
        id: StepId::generate(),
        title: title.to_string(),
        state,
        owner: owner.map(agent),
    }
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
/// `task` is not among them: the task ids are minted by [`tasks`] and written down, so by the time
/// a second boot asks for the agents the records they belong to came from the store and no number
/// in this file names one. An agent with no task draws as ungrouped, which is honest; attaching
/// them to the records it just read is the caller's to do, by title, if it wants the containers.
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
        role: role.to_string(),
        activity,
        note: note.to_string(),
        branch: branch.to_string(),
        tokens,
        harness: "Claude Code".to_string(),
        model: "Opus 4.6".to_string(),
        context_pct: ((tokens / 200_000.0) * 100.0).round() as u8,
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

    /// The cross-references the seed writes down resolve against the sessions and agents the mock
    /// answers with. This is the whole reason the ids are constants.
    #[test]
    fn mock_tasks_reference_sessions_and_agents_that_exist() {
        let sessions: HashSet<_> = sessions().into_iter().map(|s| s.id).collect();
        let agents: HashSet<_> = agents().into_iter().map(|a| a.id).collect();

        for task in tasks() {
            if let Some(id) = task.session {
                assert!(sessions.contains(&id), "{} names no session", task.title);
            }
            for step in &task.steps {
                if let Some(owner) = step.owner {
                    assert!(agents.contains(&owner), "{} names no agent", step.title);
                }
            }
        }
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
