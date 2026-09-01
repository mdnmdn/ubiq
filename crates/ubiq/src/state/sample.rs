//! The fixture left.
//!
//! Projects, the file tree, a file's bytes, the panes and now the work all come from the host, and
//! the constructors that invented them are gone. What is left is the one screen with no transport
//! family behind it — the chat, whose composer sends to nothing and whose reply is canned. A
//! fixture is still the honest way to draw it, and it goes the same way when it gets a family.

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
