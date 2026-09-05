//! The chat panel's state.
//!
//! **What the panel shows is not here.** A conversation is the host's, projected into
//! [`super::conversation`] and drawn by the one view every surface shares, so this holds only
//! which of them the panel has selected and the panel's own furniture. The fixtures below are the
//! record-shaped mock the panel drew before conversations were real, kept for the surfaces that
//! still read them.

use ubiq_proto::work::AgentId;

/// The context window the token pill is a fraction of.
pub const CONTEXT_TOKENS: f32 = 200_000.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ToolKind {
    Read,
    Edit,
    Bash,
    Grep,
}

impl ToolKind {
    pub fn label(self) -> &'static str {
        match self {
            ToolKind::Read => "READ",
            ToolKind::Edit => "EDIT",
            ToolKind::Bash => "BASH",
            ToolKind::Grep => "GREP",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DiffKind {
    Add,
    Remove,
    Context,
}

impl DiffKind {
    pub fn marker(self) -> &'static str {
        match self {
            DiffKind::Add => "+",
            DiffKind::Remove => "\u{2212}",
            DiffKind::Context => " ",
        }
    }
}

#[derive(Clone, Debug)]
pub struct DiffLine {
    pub kind: DiffKind,
    pub text: String,
}

impl DiffLine {
    pub fn add(text: &str) -> Self {
        Self {
            kind: DiffKind::Add,
            text: text.to_string(),
        }
    }

    pub fn remove(text: &str) -> Self {
        Self {
            kind: DiffKind::Remove,
            text: text.to_string(),
        }
    }

    pub fn context(text: &str) -> Self {
        Self {
            kind: DiffKind::Context,
            text: text.to_string(),
        }
    }
}

/// What an assistant turn's tool block shows. `body` carries plain output rows (READ, BASH, GREP);
/// `diff` carries an edit. A block uses one or the other, never both.
#[derive(Clone, Debug)]
pub struct ToolCall {
    pub kind: ToolKind,
    pub target: String,
    pub meta: String,
    pub expanded: bool,
    pub body: Vec<String>,
    pub diff: Vec<DiffLine>,
}

impl ToolCall {
    /// Whether the header is worth clicking — a block with nothing behind it does not expand.
    pub fn has_body(&self) -> bool {
        !self.body.is_empty() || !self.diff.is_empty()
    }
}

#[derive(Clone, Debug)]
pub enum Block {
    Markdown(String),
    Tool(ToolCall),
}

#[derive(Clone, Debug)]
pub enum ChatMessage {
    User(String),
    Assistant(Vec<Block>),
}

#[derive(Clone, Debug)]
pub struct Chat {
    pub id: usize,
    pub title: String,
    /// Relative time, as the list shows it: "2m", "1h", "yst".
    pub when: String,
    pub messages: Vec<ChatMessage>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RunState {
    Idle,
    Working,
}

impl RunState {
    pub fn label(self) -> &'static str {
        match self {
            RunState::Idle => "Idle",
            RunState::Working => "Working",
        }
    }
}

pub struct ChatState {
    /// Which of the project's conversations the panel is showing.
    ///
    /// A *view* onto a conversation the host owns, never the conversation itself: closing the
    /// panel or selecting another leaves every one of them running. `None` means nothing is
    /// selected yet, which is what an empty project looks like.
    pub selected: Option<AgentId>,
    pub chats: Vec<Chat>,
    pub active: usize,
    pub run: RunState,
    pub tokens: f32,
    pub collapsed: bool,
    pub attachment: bool,
    /// Mirror of the composer's textarea, so rendering never has to read the entity.
    pub draft: String,
    next_id: usize,
}

impl ChatState {
    pub fn new(chats: Vec<Chat>, tokens: f32) -> Self {
        let next_id = chats.iter().map(|c| c.id + 1).max().unwrap_or(1);
        Self {
            selected: None,
            chats,
            active: 0,
            run: RunState::Idle,
            tokens,
            collapsed: true,
            attachment: false,
            draft: String::new(),
            next_id,
        }
    }

    pub fn active_chat(&self) -> Option<&Chat> {
        self.chats.get(self.active)
    }

    pub fn active_chat_mut(&mut self) -> Option<&mut Chat> {
        self.chats.get_mut(self.active)
    }

    /// Percentage of the context window in use, for the token pill's ring.
    pub fn context_pct(&self) -> u8 {
        ((self.tokens / CONTEXT_TOKENS) * 100.0)
            .round()
            .clamp(0.0, 100.0) as u8
    }

    pub fn new_chat(&mut self) {
        let id = self.next_id;
        self.next_id += 1;
        self.chats.insert(
            0,
            Chat {
                id,
                title: "New chat".to_string(),
                when: "now".to_string(),
                messages: Vec::new(),
            },
        );
        self.active = 0;
    }

    /// Toggle one tool block, addressed by its position in the active chat.
    pub fn toggle_tool(&mut self, message: usize, block: usize) {
        if let Some(chat) = self.chats.get_mut(self.active)
            && let Some(ChatMessage::Assistant(blocks)) = chat.messages.get_mut(message)
            && let Some(Block::Tool(tool)) = blocks.get_mut(block)
        {
            tool.expanded = !tool.expanded;
        }
    }

    /// Append the draft as a user turn plus a canned reply. There is no agent behind this yet, so
    /// the reply reports the composer's own selection and stops.
    pub fn send(&mut self) {
        let text = self.draft.trim().to_string();
        if text.is_empty() {
            return;
        }

        let reply =
            "Queued. No harness is attached to this pane yet, so the turn ends here.".to_string();

        // An empty chat takes its title from the first thing said in it.
        let title = first_line(&text);
        if let Some(chat) = self.chats.get_mut(self.active) {
            if chat.messages.is_empty() {
                chat.title = title;
                chat.when = "now".to_string();
            }
            chat.messages.push(ChatMessage::User(text));
            chat.messages
                .push(ChatMessage::Assistant(vec![Block::Markdown(reply)]));
        }

        self.draft.clear();
        self.attachment = false;
        self.run = RunState::Idle;
    }
}

fn first_line(text: &str) -> String {
    let line = text.lines().next().unwrap_or(text).trim();
    if line.chars().count() > 48 {
        let head: String = line.chars().take(47).collect();
        format!("{head}\u{2026}")
    } else {
        line.to_string()
    }
}
