//! The fixtures left.
//!
//! Projects, the file tree, a file's bytes and the panes all come from the host now, and the
//! constructors that invented them are gone. What is left is the two screens with no transport
//! family behind them — the chat, whose composer sends to nothing and whose reply is canned, and
//! the agents screen's orchestration graph. A fixture is still the honest way to draw either, and
//! each goes the same way when it gets a family.

use super::agents::{
    Activity, Agent, AgentId, AgentsState, Session, SessionId, Shape, Speaker, Step, Task, TaskId,
    Turn,
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
// and goes the same way when it gets one. Positions are graph coordinates at 100% zoom; the
// containers round the cards are computed from them, so moving a card moves its container too.

/// The graph, its tasks and the sessions they belong to.
pub fn agents() -> AgentsState {
    AgentsState::new(sessions(), agent_cards(), tasks())
}

fn sessions() -> Vec<Session> {
    vec![
        Session {
            id: 1,
            name: "agent-manager".to_string(),
            branch: "main".to_string(),
        },
        Session {
            id: 2,
            name: "release 0.3.1".to_string(),
            branch: "release/0.3.1".to_string(),
        },
    ]
}

fn tasks() -> Vec<Task> {
    vec![
        Task {
            id: 1,
            session: 1,
            shape: Shape::Direct,
            title: "Guard the 0\u{d7}0 resize callback".to_string(),
            steps: vec![
                Step {
                    title: "Reproduce the dropped fit".to_string(),
                    done: true,
                    owner: Some(2),
                },
                Step {
                    title: "Guard the observer".to_string(),
                    done: true,
                    owner: Some(2),
                },
                Step {
                    title: "Run the panel tests".to_string(),
                    done: false,
                    owner: Some(2),
                },
            ],
        },
        Task {
            id: 2,
            session: 1,
            shape: Shape::Chain,
            title: "Migrate the session store".to_string(),
            steps: vec![
                Step {
                    title: "Plan the v1 \u{2192} v2 schema".to_string(),
                    done: true,
                    owner: Some(3),
                },
                Step {
                    title: "Write the persist adapter".to_string(),
                    done: false,
                    owner: Some(4),
                },
                Step {
                    title: "Backfill the existing stores".to_string(),
                    done: false,
                    owner: Some(4),
                },
            ],
        },
        Task {
            id: 3,
            session: 1,
            shape: Shape::Coordinated,
            title: "Cut cold start under 800 ms".to_string(),
            steps: vec![
                Step {
                    title: "Split the budget across phases".to_string(),
                    done: true,
                    owner: Some(5),
                },
                Step {
                    title: "Trace the boot with cargo flamegraph".to_string(),
                    done: false,
                    owner: Some(6),
                },
                Step {
                    title: "Defer the plugin registry".to_string(),
                    done: false,
                    owner: Some(7),
                },
                Step {
                    title: "Bench the result".to_string(),
                    done: false,
                    owner: Some(8),
                },
                Step {
                    title: "Publish the perf notes".to_string(),
                    done: false,
                    owner: Some(9),
                },
            ],
        },
        Task {
            id: 4,
            session: 2,
            shape: Shape::Direct,
            title: "Draft the release notes".to_string(),
            steps: vec![
                Step {
                    title: "Read the range since 0.3.0".to_string(),
                    done: true,
                    owner: Some(10),
                },
                Step {
                    title: "Group the changes by area".to_string(),
                    done: false,
                    owner: Some(10),
                },
            ],
        },
    ]
}

fn agent_cards() -> Vec<Agent> {
    vec![
        card(
            1,
            1,
            None,
            None,
            "Orchestrator",
            "Project manager",
            Activity::NeedsYou,
            "Waiting for your next instruction. Three tasks in flight.",
            "main",
            18_900.0,
            (430.0, 30.0),
        ),
        card(
            2,
            1,
            Some(1),
            Some(1),
            "Fixer",
            "Implementer",
            Activity::Tools,
            "Running `cargo test panels` after the ResizeObserver guard.",
            "fix/terminal-refit",
            42_100.0,
            (30.0, 260.0),
        ),
        card(
            3,
            1,
            Some(2),
            Some(1),
            "Spec",
            "Analyst",
            Activity::Ended,
            "Handed over a migration plan for the v1 \u{2192} v2 store schema.",
            "feat/session-store",
            31_700.0,
            (350.0, 260.0),
        ),
        card(
            4,
            1,
            Some(2),
            Some(3),
            "Builder",
            "Implementer",
            Activity::Writing,
            "Writing the persist adapter and the v1 \u{2192} v2 migration.",
            "feat/session-store",
            58_300.0,
            (660.0, 260.0),
        ),
        card(
            5,
            1,
            Some(3),
            Some(1),
            "Perf lead",
            "Activity coordinator",
            Activity::Thinking,
            "Splitting the budget across startup phases and rebalancing its workers.",
            "spike/cold-start",
            26_400.0,
            (190.0, 500.0),
        ),
        card(
            6,
            1,
            Some(3),
            Some(5),
            "Profiler",
            "Investigator",
            Activity::Tools,
            "Tracing the Tauri boot with `cargo flamegraph`.",
            "spike/cold-start",
            19_200.0,
            (30.0, 700.0),
        ),
        card(
            7,
            1,
            Some(3),
            Some(5),
            "Rust dev",
            "Implementer",
            Activity::Writing,
            "Deferring the plugin registry behind a lazy init.",
            "spike/cold-start",
            37_000.0,
            (340.0, 700.0),
        ),
        card(
            8,
            1,
            Some(3),
            Some(5),
            "Bench",
            "Verifier",
            Activity::Failed,
            "Harness exited 137 \u{2014} the bench run was killed under memory pressure.",
            "spike/cold-start",
            11_500.0,
            (30.0, 860.0),
        ),
        card(
            9,
            1,
            Some(3),
            Some(5),
            "Scribe",
            "Documentation",
            Activity::NeedsYou,
            "Needs your call: publish the perf notes to the KB or keep them local?",
            "spike/cold-start",
            8_000.0,
            (340.0, 860.0),
        ),
        card(
            10,
            2,
            Some(4),
            None,
            "Chronicler",
            "Documentation",
            Activity::Writing,
            "Grouping every change since 0.3.0 by the area it touched.",
            "release/0.3.1",
            14_600.0,
            (120.0, 120.0),
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
    at: (f32, f32),
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
        at,
        // One line each: the last thing the agent said, which is also what its card prints.
        thread: vec![Turn {
            from: Speaker::Agent,
            text: note.to_string(),
        }],
    }
}
