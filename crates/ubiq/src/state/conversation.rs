//! One live agent's conversation, as the window holds it.
//!
//! The host is the only writer. What arrives is a stream of deltas — a chunk
//! appends to the block it belongs to, a tool-call patch changes only the
//! fields it names — and this is where they are folded into something a
//! transcript can draw. Nothing here invents a line: the composer appends
//! nothing when it sends, and the user's own turn appears when the harness
//! echoes it back.
//!
//! What *is* derived here is presentation: the run pill, the activity badge,
//! the context ring. Those are read off the stream rather than asked for,
//! because a second round trip per token would be a round trip per token.

use std::collections::HashMap;

use ubiq_proto::conversation::{
    ConfigOption, ConvContent, ConvUpdate, PermissionOption, PlanEntry, StopReason, ToolCallPatch,
    ToolCallRecord, UsageRecord,
};
use ubiq_proto::work::{Activity, AgentId};

/// One thing in a transcript, in the order it was said.
#[derive(Clone, Debug, PartialEq)]
pub enum ConvBlock {
    /// What the user said, as the harness received it.
    User(String),
    /// Assistant prose, markdown.
    Agent(String),
    /// Reasoning.
    Thought(String),
    /// A tool call and whether its detail is open.
    Tool { call: ToolCallRecord, open: bool },
}

/// A permission the agent is waiting on.
#[derive(Clone, Debug, PartialEq)]
pub struct Pending {
    pub request_id: String,
    pub tool_call: ToolCallPatch,
    pub options: Vec<PermissionOption>,
}

/// Whether the agent is working or waiting for a turn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Run {
    Idle,
    Working,
    Ended,
}

impl Run {
    pub fn label(self) -> &'static str {
        match self {
            Run::Idle => "Idle",
            Run::Working => "Working",
            Run::Ended => "Ended",
        }
    }
}

/// A live agent's conversation.
#[derive(Clone, Debug)]
pub struct Conversation {
    pub id: AgentId,
    /// The harness's display name, from the record the host minted.
    pub harness: String,
    /// The identity it runs as, empty when it resolved none and fell back to the user's own
    /// home. Fixed for the conversation's life: a turn already taken was taken as somebody.
    pub account: String,
    pub blocks: Vec<ConvBlock>,
    /// What the harness says it is answering with. Empty until it says.
    pub model: Option<String>,
    pub mode: Option<String>,
    /// Context and cost, as of the last thing the harness reported.
    pub usage: Option<UsageRecord>,
    pub run: Run,
    pub stop_reason: Option<StopReason>,
    /// What the harness advertised: the model, the mode, the thinking level.
    /// One list, because upstream has one mechanism for all of them.
    pub config: Vec<ConfigOption>,
    pub plan: Vec<PlanEntry>,
    pub pending: Option<Pending>,
    /// The last thing that went wrong, until the next thing happens.
    pub error: Option<String>,
    /// Whether this harness takes a second turn at all.
    pub accepts_input: bool,
    /// What the composer holds, unsent.
    pub draft: String,

    /// The highest sequence number applied. An update that does not follow it
    /// is a gap, and a gap is worth saying rather than silently drawing.
    seq: u64,
    /// Which block each tool call is, so a patch reaches it in one lookup
    /// rather than a scan of the whole transcript.
    tools: HashMap<String, usize>,
    /// The message currently being appended to, and which block it is. A
    /// change of id starts a new block — that is what a message id is for.
    open: Option<(String, usize)>,
}

impl Conversation {
    pub fn new(id: AgentId, harness: String, account: String) -> Self {
        Self {
            id,
            harness,
            account,
            blocks: Vec::new(),
            model: None,
            mode: None,
            usage: None,
            run: Run::Idle,
            stop_reason: None,
            config: Vec::new(),
            plan: Vec::new(),
            pending: None,
            error: None,
            accepts_input: true,
            draft: String::new(),
            seq: 0,
            tools: HashMap::new(),
            open: None,
        }
    }

    /// The badge the sidebar and the column header draw.
    ///
    /// Derived rather than carried: an activity is a reading of the last
    /// thing that happened, and the stream already says what that was.
    pub fn activity(&self) -> Activity {
        if self.pending.is_some() {
            return Activity::NeedsYou;
        }
        match (self.run, self.stop_reason) {
            (Run::Ended, Some(StopReason::Failed)) => Activity::Failed,
            (Run::Ended, _) => Activity::Ended,
            (Run::Idle, _) => Activity::Ended,
            (Run::Working, _) => match self.blocks.last() {
                Some(ConvBlock::Thought(_)) => Activity::Thinking,
                Some(ConvBlock::Tool { .. }) => Activity::Tools,
                _ => Activity::Writing,
            },
        }
    }

    /// The percentage the ring draws, when a window is known.
    pub fn context_pct(&self) -> Option<u8> {
        self.usage.as_ref().and_then(UsageRecord::context_pct)
    }

    /// Tokens in the context, as a count rather than a ratio.
    pub fn tokens(&self) -> u64 {
        self.usage.as_ref().map_or(0, |usage| usage.used)
    }

    /// What the turn cost so far, where the harness reports money.
    pub fn cost_usd(&self) -> Option<f64> {
        self.usage.as_ref().and_then(|usage| usage.cost_usd)
    }

    /// Whether a `seq` follows the last one applied.
    ///
    /// The bus promises order per agent, so a gap means a message was lost
    /// rather than reordered — worth reporting, and worth applying anyway,
    /// since half a transcript beats none.
    pub fn is_next(&self, seq: u64) -> bool {
        seq == self.seq + 1
    }

    /// Fold one delta in.
    pub fn apply(&mut self, seq: u64, update: ConvUpdate) {
        self.seq = self.seq.max(seq);
        self.error = None;

        match update {
            ConvUpdate::Started { model, mode, .. } => {
                self.model = model;
                self.mode = mode;
            }

            ConvUpdate::UserChunk { content, .. } => {
                // A user turn always starts a block: the harness echoes one
                // per prompt, and merging two would merge two questions.
                self.open = None;
                if let Some(text) = text_of(&content) {
                    self.blocks.push(ConvBlock::User(text));
                }
                self.run = Run::Working;
            }
            ConvUpdate::AgentChunk {
                content,
                message_id,
            } => {
                self.run = Run::Working;
                self.append(message_id, content, false);
            }
            ConvUpdate::ThoughtChunk {
                content,
                message_id,
            } => {
                self.run = Run::Working;
                self.append(message_id, content, true);
            }

            ConvUpdate::ToolCall(call) => {
                self.run = Run::Working;
                self.open = None;
                self.tools.insert(call.id.clone(), self.blocks.len());
                self.blocks.push(ConvBlock::Tool { call, open: false });
            }
            ConvUpdate::ToolCallUpdate(patch) => self.patch_tool(patch),

            ConvUpdate::Plan(entries) => self.plan = entries,
            ConvUpdate::ConfigOptions(options) => self.config = options,
            ConvUpdate::ModeChanged { mode_id } => self.mode = Some(mode_id),
            // The title is the conversation's, and the column takes its name
            // from the agent record instead — so there is nowhere to put it
            // that a reader would look. Held rather than drawn.
            ConvUpdate::Title(_) => {}

            ConvUpdate::Usage(usage) => {
                // A model is only named where the harness named it: a usage
                // report for a fallback model must not rename the column.
                if usage.model.is_some() {
                    self.model = usage.model.clone();
                }
                self.usage = Some(usage);
            }

            ConvUpdate::PermissionRequest {
                request_id,
                tool_call,
                options,
            } => {
                self.pending = Some(Pending {
                    request_id,
                    tool_call,
                    options,
                });
            }

            ConvUpdate::TurnEnded { stop_reason, error } => {
                self.open = None;
                self.run = Run::Idle;
                self.stop_reason = Some(stop_reason);
                self.error = error;
            }
        }
    }

    /// The harness has gone.
    pub fn ended(&mut self, stop_reason: StopReason) {
        self.open = None;
        self.pending = None;
        self.run = Run::Ended;
        self.stop_reason = Some(stop_reason);
    }

    /// Toggle a tool block's detail.
    pub fn toggle_tool(&mut self, id: &str) {
        if let Some(ConvBlock::Tool { open, .. }) =
            self.tools.get(id).map(|ix| &mut self.blocks[*ix])
        {
            *open = !*open;
        }
    }

    /// Append a chunk to the message it belongs to, starting a new block when
    /// the message id changes — which is what a message id is for.
    ///
    /// A chunk with no id at all can only extend the block immediately before
    /// it, and only if that block is the same kind: a harness that numbers
    /// nothing still streams in order.
    fn append(&mut self, message_id: Option<String>, content: ConvContent, thought: bool) {
        let Some(text) = text_of(&content) else {
            return;
        };

        let same_kind = |block: &ConvBlock| {
            matches!(
                (block, thought),
                (ConvBlock::Agent(_), false) | (ConvBlock::Thought(_), true)
            )
        };

        if let Some((open_id, ix)) = &self.open
            && message_id.as_ref().is_none_or(|id| id == open_id)
            && self.blocks.get(*ix).is_some_and(same_kind)
        {
            match &mut self.blocks[*ix] {
                ConvBlock::Agent(body) | ConvBlock::Thought(body) => body.push_str(&text),
                _ => unreachable!("same_kind just matched one of these two"),
            }
            return;
        }

        let ix = self.blocks.len();
        self.blocks.push(if thought {
            ConvBlock::Thought(text)
        } else {
            ConvBlock::Agent(text)
        });
        self.open = Some((message_id.unwrap_or_default(), ix));
    }

    /// Apply a tool-call patch: **absent means unchanged**, and content and
    /// locations replace rather than append.
    fn patch_tool(&mut self, patch: ToolCallPatch) {
        let Some(ix) = self.tools.get(&patch.id).copied() else {
            // A result for a call nobody announced. Dropping it is right:
            // drawing a completed call with no beginning would invent one.
            return;
        };
        let Some(ConvBlock::Tool { call, .. }) = self.blocks.get_mut(ix) else {
            return;
        };

        if let Some(title) = patch.title {
            call.title = title;
        }
        if let Some(kind) = patch.kind {
            call.kind = kind;
        }
        if let Some(status) = patch.status {
            call.status = status;
        }
        if let Some(content) = patch.content {
            call.content = content;
        }
        if let Some(locations) = patch.locations {
            call.locations = locations;
        }
    }
}

fn text_of(content: &ConvContent) -> Option<String> {
    match content {
        ConvContent::Text(text) => Some(text.clone()),
        // Something the transcript cannot draw. Saying it arrived beats
        // dropping it silently, and beats pretending it was prose.
        ConvContent::Other { kind, description } => Some(format!("_[{kind}: {description}]_")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ubiq_proto::conversation::{ToolKind, ToolStatus};

    fn conversation() -> Conversation {
        Conversation::new(
            AgentId::generate(),
            "Claude Code".to_string(),
            "work".to_string(),
        )
    }

    fn chunk(text: &str, id: Option<&str>) -> ConvUpdate {
        ConvUpdate::AgentChunk {
            content: ConvContent::Text(text.to_string()),
            message_id: id.map(str::to_string),
        }
    }

    /// Chunks sharing a message id are one message. This is the whole reason
    /// a token stream does not produce a block per token.
    #[test]
    fn chunks_of_one_message_become_one_block() {
        let mut c = conversation();
        c.apply(1, chunk("Hel", Some("m1")));
        c.apply(2, chunk("lo", Some("m1")));

        assert_eq!(c.blocks, vec![ConvBlock::Agent("Hello".to_string())]);
    }

    #[test]
    fn a_new_message_id_starts_a_new_block() {
        let mut c = conversation();
        c.apply(1, chunk("first", Some("m1")));
        c.apply(2, chunk("second", Some("m2")));

        assert_eq!(
            c.blocks,
            vec![
                ConvBlock::Agent("first".to_string()),
                ConvBlock::Agent("second".to_string()),
            ]
        );
    }

    /// Prose and reasoning are different blocks even inside one message.
    #[test]
    fn a_thought_does_not_join_the_prose_before_it() {
        let mut c = conversation();
        c.apply(1, chunk("answering", Some("m1")));
        c.apply(
            2,
            ConvUpdate::ThoughtChunk {
                content: ConvContent::Text("pondering".to_string()),
                message_id: Some("m1".to_string()),
            },
        );

        assert_eq!(c.blocks.len(), 2);
        assert_eq!(c.blocks[1], ConvBlock::Thought("pondering".to_string()));
    }

    #[test]
    fn a_patch_changes_only_what_it_names() {
        let mut c = conversation();
        c.apply(
            1,
            ConvUpdate::ToolCall(ToolCallRecord {
                id: "t1".to_string(),
                title: "Bash ls".to_string(),
                kind: ToolKind::Execute,
                status: ToolStatus::InProgress,
                content: Vec::new(),
                locations: Vec::new(),
            }),
        );
        c.apply(
            2,
            ConvUpdate::ToolCallUpdate(ToolCallPatch {
                id: "t1".to_string(),
                status: Some(ToolStatus::Completed),
                ..ToolCallPatch::default()
            }),
        );

        let ConvBlock::Tool { call, .. } = &c.blocks[0] else {
            panic!("expected a tool block");
        };
        assert_eq!(call.status, ToolStatus::Completed);
        assert_eq!(call.title, "Bash ls", "a patch that named no title kept it");
        assert_eq!(call.kind, ToolKind::Execute);
    }

    /// Drawing a completed call with no beginning would invent one.
    #[test]
    fn a_patch_for_a_call_nobody_announced_is_dropped() {
        let mut c = conversation();
        c.apply(
            1,
            ConvUpdate::ToolCallUpdate(ToolCallPatch {
                id: "ghost".to_string(),
                status: Some(ToolStatus::Completed),
                ..ToolCallPatch::default()
            }),
        );
        assert!(c.blocks.is_empty());
    }

    #[test]
    fn the_ring_comes_from_the_harness_rather_than_a_constant() {
        let mut c = conversation();
        assert_eq!(c.context_pct(), None, "nothing reported yet");

        c.apply(
            1,
            ConvUpdate::Usage(UsageRecord {
                used: 100_000,
                size: 1_000_000,
                cost_usd: Some(0.25),
                model: Some("claude-opus-5".to_string()),
            }),
        );

        assert_eq!(c.context_pct(), Some(10));
        assert_eq!(c.tokens(), 100_000);
        assert_eq!(c.model.as_deref(), Some("claude-opus-5"));
    }

    #[test]
    fn a_waiting_permission_outranks_whatever_it_was_doing() {
        let mut c = conversation();
        c.apply(1, chunk("working", Some("m1")));
        assert_eq!(c.activity(), Activity::Writing);

        c.apply(
            2,
            ConvUpdate::PermissionRequest {
                request_id: "r1".to_string(),
                tool_call: ToolCallPatch::default(),
                options: Vec::new(),
            },
        );
        assert_eq!(c.activity(), Activity::NeedsYou);
    }

    #[test]
    fn a_failed_end_shows_as_failed_rather_than_ended() {
        let mut c = conversation();
        c.ended(StopReason::Failed);
        assert_eq!(c.activity(), Activity::Failed);
        assert_eq!(c.run, Run::Ended);
    }

    /// The bus promises order per agent, so a gap is a lost message.
    #[test]
    fn a_gap_in_the_sequence_is_visible() {
        let mut c = conversation();
        assert!(c.is_next(1));
        c.apply(1, chunk("a", Some("m1")));
        assert!(c.is_next(2));
        assert!(!c.is_next(4));
    }
}
