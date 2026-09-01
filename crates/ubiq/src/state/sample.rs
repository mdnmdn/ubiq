//! The fixtures left.
//!
//! Projects, the file tree, a file's bytes and the panes all come from the host now, and the
//! constructors that invented them are gone. What is left is the two screens with no transport
//! family behind them — the chat, whose composer sends to nothing and whose reply is canned, and
//! the agents screen's orchestration graph. A fixture is still the honest way to draw either, and
//! each goes the same way when it gets a family.

use super::agents::{
    Activity, Agent, AgentId, AgentsState, Priority, Session, SessionId, Shape, Speaker, Status,
    Step, StepState, Task, TaskId, Turn,
};
use super::chat::{Block, Chat, ChatMessage, ChatState, DiffLine, ToolCall, ToolKind};

/// The chat panel's conversations.
pub fn chat() -> ChatState {
    ChatState::new(
        vec![
            Chat {
                id: 1,
                title: "Fix terminal resize after panel collapse".to_string(),
                when: "2m".to_string(),
                messages: opening_thread(),
            },
            Chat {
                id: 2,
                title: "Migrate session store to persist v2".to_string(),
                when: "1h".to_string(),
                messages: Vec::new(),
            },
            Chat {
                id: 3,
                title: "Draft 0.3.1 release notes".to_string(),
                when: "yst".to_string(),
                messages: Vec::new(),
            },
        ],
        41_200.0,
    )
}

fn opening_thread() -> Vec<ChatMessage> {
    vec![
        ChatMessage::User(
            "The xterm panel stops resizing after I collapse and reopen the sidebar. Find where \
             the fit call is getting dropped."
                .to_string(),
        ),
        ChatMessage::Assistant(vec![
            Block::Tool(ToolCall {
                kind: ToolKind::Read,
                target: "panels/AgentTerminal.tsx".to_string(),
                meta: "142 ln".to_string(),
                expanded: false,
                body: vec![
                    "40  useEffect(() => {".to_string(),
                    "41    if (!visible) return;".to_string(),
                    "42    const ro = new ResizeObserver(() => fitAddon.fit());".to_string(),
                    "43    ro.observe(hostRef.current!);".to_string(),
                    "44    return () => ro.disconnect();".to_string(),
                    "45  }, [visible]);".to_string(),
                ],
                diff: Vec::new(),
            }),
            Block::Markdown(
                "**Found it.** The host stays mounted with `hidden`, so the observer re-attaches \
                 while the box still measures 0\u{d7}0 \u{2014} `fit()` runs once against nothing \
                 and never fires again."
                    .to_string(),
            ),
            Block::Tool(ToolCall {
                kind: ToolKind::Edit,
                target: "panels/AgentTerminal.tsx".to_string(),
                meta: "+4 \u{2212}1".to_string(),
                expanded: true,
                body: Vec::new(),
                diff: vec![
                    DiffLine::remove("const ro = new ResizeObserver(() => fitAddon.fit());"),
                    DiffLine::add("const ro = new ResizeObserver(([entry]) => {"),
                    DiffLine::add("  if (entry.contentRect.width === 0) return;"),
                    DiffLine::add("  fitAddon.fit();"),
                    DiffLine::add("});"),
                ],
            }),
            Block::Tool(ToolCall {
                kind: ToolKind::Bash,
                target: "pnpm test panels".to_string(),
                meta: "exit 0".to_string(),
                expanded: false,
                body: vec![
                    "PASS  src/panels/AgentTerminal.test.tsx".to_string(),
                    "  \u{2713} refits once the host has width (18 ms)".to_string(),
                    "Tests: 1 passed, 1 total".to_string(),
                ],
                diff: Vec::new(),
            }),
            Block::Markdown(
                "The observer now ignores the zero-width measurement and fits on the first real \
                 one. Want me to add the same guard to `SidebarHost`?"
                    .to_string(),
            ),
        ]),
    ]
}

// ── The agents screen ───────────────────────────────────────────────
//
// The orchestration graph has no transport family either, so it is invented here beside the chat
// and goes the same way when it gets one. Nothing here says where anything is drawn: the fixture
// is definitions only, and `state::layout` arranges them.

/// The graph, its tasks and the sessions they belong to.
pub fn agents() -> AgentsState {
    AgentsState::new(sessions(), agent_cards(), tasks())
}

fn sessions() -> Vec<Session> {
    vec![
        Session {
            id: 1,
            name: "fix/terminal-refit".to_string(),
            branch: "fix/terminal-refit".to_string(),
            worktree: true,
        },
        Session {
            id: 2,
            name: "feat/session-store".to_string(),
            branch: "feat/session-store".to_string(),
            worktree: true,
        },
        Session {
            id: 3,
            name: "spike/cold-start".to_string(),
            branch: "spike/cold-start".to_string(),
            worktree: true,
        },
        Session {
            id: 4,
            name: "fix/win-paths".to_string(),
            branch: "fix/win-paths".to_string(),
            worktree: true,
        },
        Session {
            id: 5,
            name: "main".to_string(),
            branch: "main".to_string(),
            worktree: false,
        },
    ]
}

/// The board's cards, in the order the columns read: three nobody has started, one ready to go,
/// three in flight, one waiting to be looked at, two finished.
fn tasks() -> Vec<Task> {
    vec![
        Task {
            id: 1,
            session: None,
            status: Status::Backlog,
            priority: Priority::High,
            shape: Shape::Coordinated,
            title: "Replace status polling with an event stream".to_string(),
            steps: unstarted(&[
                "Name the events the host already knows",
                "Add the family to the transport contract",
                "Replace the poll in the status bar",
                "Drop the timer",
            ]),
        },
        Task {
            id: 2,
            session: None,
            status: Status::Backlog,
            priority: Priority::Normal,
            shape: Shape::Direct,
            title: "Keyboard shortcuts for the pane toggles".to_string(),
            steps: unstarted(&[
                "Pick the three chords",
                "Bind them in the window",
                "Say so in the status bar",
            ]),
        },
        Task {
            id: 3,
            session: None,
            status: Status::Backlog,
            priority: Priority::Low,
            shape: Shape::Chain,
            title: "Bundle size budget in CI".to_string(),
            steps: unstarted(&[
                "Measure the release binary",
                "Pick the ceiling",
                "Fail the build over it",
            ]),
        },
        Task {
            id: 4,
            session: Some(2),
            status: Status::Ready,
            priority: Priority::Normal,
            shape: Shape::Direct,
            title: "Persist terminal scrollback per session".to_string(),
            steps: unstarted(&[
                "Decide what a session keeps",
                "Write it down on exit",
                "Read it back on attach",
                "Cap what one pane may hold",
            ]),
        },
        Task {
            id: 5,
            session: Some(1),
            status: Status::InProgress,
            priority: Priority::Normal,
            shape: Shape::Direct,
            title: "Guard the 0\u{d7}0 resize callback".to_string(),
            steps: vec![
                step("Reproduce the dropped fit", StepState::Done, Some(2)),
                step("Guard the observer", StepState::Done, Some(2)),
                step("Run the panel tests", StepState::Working, Some(2)),
            ],
        },
        Task {
            id: 6,
            session: Some(2),
            status: Status::InProgress,
            priority: Priority::Normal,
            shape: Shape::Chain,
            title: "Migrate the session store to persist v2".to_string(),
            steps: vec![
                step("Plan the v1 \u{2192} v2 schema", StepState::Done, Some(3)),
                step("Write the persist adapter", StepState::Working, Some(4)),
                step("Backfill the existing stores", StepState::Idle, Some(4)),
            ],
        },
        Task {
            id: 7,
            session: Some(3),
            status: Status::InProgress,
            priority: Priority::High,
            shape: Shape::Coordinated,
            title: "Cut cold start under 800 ms".to_string(),
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
        },
        Task {
            id: 8,
            session: Some(4),
            status: Status::InReview,
            priority: Priority::Normal,
            shape: Shape::Direct,
            title: "Normalise Windows path separators".to_string(),
            steps: vec![
                step(
                    "Find every path join in the host",
                    StepState::Done,
                    Some(11),
                ),
                step("Route them through one helper", StepState::Done, Some(11)),
                step("Add the round-trip test", StepState::Done, Some(11)),
            ],
        },
        Task {
            id: 9,
            session: Some(5),
            status: Status::Done,
            priority: Priority::Normal,
            shape: Shape::Direct,
            title: "Draft the 0.3.1 release notes".to_string(),
            steps: vec![
                step("Read the range since 0.3.0", StepState::Done, Some(10)),
                step("Group the changes by area", StepState::Done, Some(10)),
            ],
        },
        Task {
            id: 10,
            session: Some(5),
            status: Status::Done,
            priority: Priority::Low,
            shape: Shape::Direct,
            title: "Kill a project's panes when it closes".to_string(),
            steps: vec![
                step("Kill them on close", StepState::Done, Some(1)),
                step("Write the project's blob first", StepState::Done, Some(1)),
            ],
        },
    ]
}

/// The steps of a task nobody has picked up: named, unowned, and not started.
fn unstarted(titles: &[&str]) -> Vec<Step> {
    titles
        .iter()
        .map(|title| step(title, StepState::Idle, None))
        .collect()
}

fn step(title: &str, state: StepState, owner: Option<AgentId>) -> Step {
    Step {
        title: title.to_string(),
        state,
        owner,
    }
}

fn agent_cards() -> Vec<Agent> {
    vec![
        card(
            1,
            5,
            None,
            None,
            "Orchestrator",
            "Project manager",
            Activity::NeedsYou,
            "Waiting for your next instruction. Three tasks in flight.",
            "main",
            18_900.0,
        ),
        card(
            2,
            1,
            Some(5),
            None,
            "Fixer",
            "Implementer",
            Activity::Tools,
            "Running `cargo test panels` after the ResizeObserver guard.",
            "fix/terminal-refit",
            42_100.0,
        ),
        card(
            3,
            2,
            Some(6),
            None,
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
            Some(6),
            Some(3),
            "Builder",
            "Implementer",
            Activity::Writing,
            "Writing the persist adapter and the v1 \u{2192} v2 migration.",
            "feat/session-store",
            58_300.0,
        ),
        card(
            5,
            3,
            Some(7),
            None,
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
            Some(7),
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
            Some(7),
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
            Some(7),
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
            Some(7),
            Some(5),
            "Scribe",
            "Documentation",
            Activity::NeedsYou,
            "Needs your call: publish the perf notes to the KB or keep them local?",
            "spike/cold-start",
            8_000.0,
        ),
        card(
            10,
            5,
            Some(9),
            Some(1),
            "Chronicler",
            "Documentation",
            Activity::Ended,
            "Grouped every change since 0.3.0 by the area it touched.",
            "main",
            14_600.0,
        ),
        card(
            11,
            4,
            Some(8),
            None,
            "Porter",
            "Implementer",
            Activity::Ended,
            "Every path join in the host goes through one helper now.",
            "fix/win-paths",
            22_800.0,
        ),
    ]
}

#[allow(clippy::too_many_arguments)]
fn card(
    id: AgentId,
    session: SessionId,
    task: Option<TaskId>,
    parent: Option<AgentId>,
    name: &str,
    role: &str,
    activity: Activity,
    note: &str,
    branch: &str,
    tokens: f32,
) -> Agent {
    Agent {
        id,
        session,
        task,
        parent,
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
