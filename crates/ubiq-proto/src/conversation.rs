//! What a live agent says, on its way to the screen that draws it.
//!
//! # The vocabulary is the Agent Client Protocol's
//!
//! Names and shapes here are ACP's `session/update` vocabulary. Only the
//! vocabulary: the transport stays the bus, because putting JSON-RPC between
//! two halves of one process would undo the embedding for no gain — see `D53`
//! and `D9`. `_docs/references/acp-protocol.md` is the wire reference, and
//! `crates/agent-manager/src/io/model.rs` is the library-side twin these
//! records are mapped from, in `crates/ubiq-host/src/conversation.rs` and
//! nowhere else.
//!
//! Two things follow from borrowing the vocabulary rather than the protocol.
//! An update is a **delta**: a chunk appends, a tool-call patch changes only
//! the fields it names, and nothing here re-sends a whole conversation. And
//! the records are **owned and serialisable by construction**, like every
//! other payload in the contract, so the day the host is a separate process
//! this crosses a socket unchanged.
//!
//! # Multiplexing
//!
//! Nothing here carries an identity. ACP puts a `sessionId` on every
//! session-scoped message and routes N conversations down one connection by
//! it; the bus does the same one layer up with the `agent_id` on every
//! variant of the conversation family. The identity belongs to the message,
//! not to the update, which is what lets the same update be produced once and
//! addressed by whoever is multiplexing.

use serde::{Deserialize, Serialize};

// ── content ────────────────────────────────────────────────────────────

/// One piece of content, in ACP's `ContentBlock` shape.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ConvContent {
    Text(String),
    /// Anything that is not prose. Kept as a described placeholder rather
    /// than as bytes: a transcript says an image arrived, and fetching it is
    /// a separate question nobody has asked yet.
    Other {
        kind: String,
        description: String,
    },
}

impl ConvContent {
    /// The text this block carries, if it carries any.
    pub fn text(&self) -> Option<&str> {
        match self {
            ConvContent::Text(text) => Some(text),
            ConvContent::Other { .. } => None,
        }
    }
}

// ── tool calls ─────────────────────────────────────────────────────────

/// What a tool call is, which is the coloured verb a transcript draws.
/// ACP's ten kinds; a harness the host does not recognise is [`ToolKind::Other`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ToolKind {
    Read,
    Edit,
    Delete,
    Move,
    Search,
    Execute,
    Think,
    Fetch,
    SwitchMode,
    /// Spawning another agent. Its own kind rather than `Think` or `Other`: a delegation starts a
    /// second transcript, and a reader who cannot tell one from a thought cannot tell where the
    /// work went.
    Delegate,
    #[default]
    Other,
}

impl ToolKind {
    /// The word the block's header leads with.
    pub fn label(self) -> &'static str {
        match self {
            ToolKind::Read => "READ",
            ToolKind::Edit => "EDIT",
            ToolKind::Delete => "DELETE",
            ToolKind::Move => "MOVE",
            ToolKind::Search => "SEARCH",
            ToolKind::Execute => "RUN",
            ToolKind::Think => "THINK",
            ToolKind::Fetch => "FETCH",
            ToolKind::SwitchMode => "MODE",
            ToolKind::Delegate => "AGENT",
            ToolKind::Other => "TOOL",
        }
    }
}

/// Where a tool call has got to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ToolStatus {
    #[default]
    Pending,
    InProgress,
    Completed,
    Failed,
}

/// One line of a diff a tool call is showing.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiffKind {
    Added,
    Removed,
    Context,
}

/// One thing a tool call has to show.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ToolContent {
    Text(String),
    /// An edit, with the two texts the transcript diffs. `old_text` absent
    /// means the file is being created.
    Diff {
        path: String,
        old_text: Option<String>,
        new_text: String,
    },
}

/// A file a tool call touched.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolLocation {
    pub path: String,
    /// 1-based, as ACP requires.
    pub line: Option<u32>,
}

/// Which spawned subagent a line came from.
///
/// **An id and a kind are two different questions.** "Which subagent" is [`Self::id`] — Claude
/// Code's `parent_tool_use_id`, which is the id of the `Task` tool call that spawned the agent, so
/// three agents spawned in one turn are three ids however alike they are. "What kind of subagent"
/// is [`Self::kind`] — `"general-purpose"`, which every one of those three shares. A transcript
/// that shows one agent at a time keys off the id; anything that asks what a *type* of agent costs
/// keys off the kind, which is why [`UsageRecord::subagent`] deliberately carries only the latter.
///
/// [`Self::model`] and [`Self::thinking`] answer "what is it running as", and are filled **only
/// where the stream says so** — a delegate drawn as inheriting the parent's model or effort when
/// the harness never stated it is a guess wearing a fact's clothes, so the field stays `None` and
/// a reader draws nothing.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Subagent {
    /// The spawning tool call's id — the instance, and the join key back to that call.
    pub id: String,
    /// The type the harness named, where it named one.
    pub kind: Option<String>,
    /// The model this delegate is answering with, where the harness named it. The same for every
    /// line of one delegate: the bridge remembers what the spawn resolved to rather than reporting
    /// whatever the block in hand happened to carry.
    #[serde(default)]
    pub model: Option<String>,
    /// The thinking or reasoning effort this delegate runs at, where the harness states it.
    /// **No harness states it per delegate today** — Claude Code's spawn names a description, a
    /// prompt and a subagent type, and its launch result adds only the resolved model — so this is
    /// `None` everywhere until one does.
    #[serde(default)]
    pub thinking: Option<String>,
}

/// A tool call as it starts.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolCallRecord {
    pub id: String,
    pub title: String,
    pub kind: ToolKind,
    pub status: ToolStatus,
    pub content: Vec<ToolContent>,
    pub locations: Vec<ToolLocation>,
    /// Which spawned subagent made the call; `None` is the conversation itself.
    #[serde(default)]
    pub subagent: Option<Subagent>,
}

/// A patch to a tool call already announced.
///
/// **An absent field means unchanged**, and `content`/`locations` replace the
/// whole collection rather than appending — both are ACP's rules, and a
/// consumer that applies them the other way silently loses half of an edit.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ToolCallPatch {
    pub id: String,
    pub title: Option<String>,
    pub kind: Option<ToolKind>,
    pub status: Option<ToolStatus>,
    pub content: Option<Vec<ToolContent>>,
    pub locations: Option<Vec<ToolLocation>>,
}

// ── the plan, config, permissions ──────────────────────────────────────

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlanPriority {
    High,
    #[default]
    Medium,
    Low,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlanStatus {
    #[default]
    Pending,
    InProgress,
    Completed,
}

/// One line of the agent's own todo list. A plan update carries **every**
/// entry, so applying one replaces the list rather than extending it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanEntry {
    pub content: String,
    pub priority: PlanPriority,
    pub status: PlanStatus,
}

/// What a config option is for, so the composer knows which picker to draw it
/// in. A hint only: it never changes what an id means.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConfigCategory {
    Mode,
    Model,
    ModelConfig,
    ThoughtLevel,
    /// An open enum upstream, so a category this build has not heard of is
    /// data rather than an error.
    Other(String),
}

/// One choice in a select-shaped config option.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigChoice {
    pub value: String,
    pub name: String,
    pub description: Option<String>,
    pub group: Option<String>,
}

/// A session-level knob the harness advertises.
///
/// **The model, the mode and the thinking level are all this one shape** —
/// upstream deprecated dedicated mode methods and never had model methods, so
/// a picker is generated from a list rather than enumerated in code. A
/// harness that grows a fourth knob needs no change in the interface.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ConfigOption {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub category: Option<ConfigCategory>,
    pub value: ConfigValue,
}

/// A config option's type and its current setting.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ConfigValue {
    Select {
        current: String,
        choices: Vec<ConfigChoice>,
    },
    Flag {
        current: bool,
    },
}

/// The four answers a permission dialog can offer.
///
/// **A display hint, and only that.** What an option actually does is whatever the harness decided
/// when it minted the option; [`PermissionOption::option_id`] is the opaque token that carries it,
/// and it is echoed back unchanged. These four values are what lets a surface tell an allow from a
/// reject without reading an id it has no business reading.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PermissionKind {
    AllowOnce,
    AllowAlways,
    RejectOnce,
    RejectAlways,
}

impl PermissionKind {
    /// Whether this reads as going ahead rather than refusing.
    pub fn allows(self) -> bool {
        matches!(self, Self::AllowOnce | Self::AllowAlways)
    }

    /// Whether the harness offered to remember the answer. **The harness remembers it, not
    /// Ubiq**: there is no client-side allowlist here, and an "always" answer is one more
    /// `option_id` echoed back like any other.
    pub fn remembers(self) -> bool {
        matches!(self, Self::AllowAlways | Self::RejectAlways)
    }
}

/// One button on a permission dialog.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionOption {
    pub option_id: String,
    pub name: String,
    pub kind: PermissionKind,
}

// ── usage, and the end of a turn ───────────────────────────────────────

/// What one usage report says was spent — a flow, summed over a conversation.
///
/// The mirror of `agent_manager::io::Spend`, and split the same way: cache
/// *read* is context re-used and cache *creation* is context newly written,
/// so folding them together makes any "how much was cached" reading sit at
/// ~100% forever. `output` excludes `thinking` where the harness separates
/// them, so [`TokenSpend::total`] counts a reasoning token once.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenSpend {
    pub input: u64,
    pub output: u64,
    pub thinking: u64,
    pub cache_read: u64,
    pub cache_creation: u64,
}

impl TokenSpend {
    /// Every token this report billed, however it was spent.
    pub fn total(&self) -> u64 {
        self.input
            .saturating_add(self.output)
            .saturating_add(self.thinking)
            .saturating_add(self.cache_read)
            .saturating_add(self.cache_creation)
    }

    /// The part of it that was context re-used. Cache *creation* is not in
    /// it: that is context paid for once, not context saved.
    pub fn cached(&self) -> u64 {
        self.cache_read
    }
}

/// Context and money for one model.
///
/// **Three separate things, and conflating them is what this shape exists to
/// prevent.** `used` over `size` is *occupancy* — the tokens sitting in the
/// context window right now, a level that falls on compaction as legitimately
/// as it rises — and it **is** the context ring. `spend` and `cost_usd` are
/// *flows*: what one report billed, meant to be summed. `subagent` is
/// *origin*: which spawned agent spent it, `None` being the conversation
/// itself. Nothing in the interface holds a context-window constant: the
/// window is per model, and the harness is the only thing that knows which
/// model answered.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UsageRecord {
    pub used: u64,
    pub size: u64,
    /// What *this* report cost, not the session's running total — a bridge
    /// that only ever hears a cumulative figure reports the delta.
    pub cost_usd: Option<f64>,
    pub model: Option<String>,
    /// What this report billed, where the harness reported spend it has not
    /// already reported. Never occupancy.
    #[serde(default)]
    pub spend: Option<TokenSpend>,
    /// The subagent **type** that spent it, and deliberately not the instance the transcript
    /// switches on: a bucket per instance would mint a fresh dimension value on every spawn,
    /// forever, and "what did general-purpose subagents cost me" is the question this answers.
    /// [`Subagent`] is where the two part company — do not unify them.
    #[serde(default)]
    pub subagent: Option<String>,
}

impl UsageRecord {
    /// The percentage the ring draws, or `None` when the window is unknown.
    pub fn context_pct(&self) -> Option<u8> {
        (self.size > 0).then(|| {
            ((self.used as f64 / self.size as f64) * 100.0)
                .round()
                .clamp(0.0, 100.0) as u8
        })
    }
}

/// How much of the user's rate-limit window is spent, and when it resets.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RateLimitRecord {
    pub five_hour_pct: Option<u8>,
    pub five_hour_resets_at: Option<i64>,
    pub seven_day_pct: Option<u8>,
    pub seven_day_resets_at: Option<i64>,
    pub status: String,
    /// Whether spending past the plan is accepted — `"rejected"` where the
    /// account has no credit line. `None` where the harness does not say.
    #[serde(default)]
    pub overage_status: Option<String>,
    /// Why overage is off, where the harness gives a reason.
    #[serde(default)]
    pub overage_reason: Option<String>,
}

/// Why a turn stopped.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum StopReason {
    #[default]
    EndTurn,
    MaxTokens,
    MaxTurnRequests,
    Refusal,
    Cancelled,
    /// The run broke rather than the model declining.
    Failed,
}

// ── the update ─────────────────────────────────────────────────────────

/// One thing a live agent said.
///
/// Three of these have no ACP `session/update` equivalent because ACP carries
/// them at the protocol level instead: [`Self::Started`] is the result of
/// `session/new`, [`Self::PermissionRequest`] is a request back to the
/// client, and [`Self::TurnEnded`] is the `session/prompt` response. They are
/// updates here because the bus is one stream, not a JSON-RPC peer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ConvUpdate {
    /// The harness is up, and this is what it can do. `session_id` is the
    /// *harness's* own id, the one a resume needs — not the agent's.
    Started {
        session_id: Option<String>,
        model: Option<String>,
        mode: Option<String>,
        tools: Vec<String>,
        agents: Vec<String>,
    },

    /// Something the user said, as the harness received it. The composer
    /// appends nothing of its own: what is drawn is what arrived.
    UserChunk {
        content: ConvContent,
        message_id: Option<String>,
    },
    /// Assistant prose. Chunks sharing a `message_id` are one message; a
    /// change of id starts a new one.
    AgentChunk {
        content: ConvContent,
        message_id: Option<String>,
        /// Which spawned subagent said it; `None` is the conversation itself.
        #[serde(default)]
        subagent: Option<Subagent>,
    },
    /// Reasoning, drawn as a thinking block.
    ThoughtChunk {
        content: ConvContent,
        message_id: Option<String>,
        /// Which spawned subagent thought it; `None` is the conversation itself.
        #[serde(default)]
        subagent: Option<Subagent>,
    },

    ToolCall(ToolCallRecord),
    /// Absent fields are unchanged.
    ToolCallUpdate(ToolCallPatch),

    /// The agent's todo list, complete each time.
    Plan(Vec<PlanEntry>),
    /// Every config option, complete each time — setting one can change
    /// another, so a partial set would leave a stale picker on screen.
    ConfigOptions(Vec<ConfigOption>),
    /// The harness changed mode by itself.
    ModeChanged {
        mode_id: String,
    },
    /// The conversation's own title, where the harness names it.
    Title(String),

    Usage(UsageRecord),
    RateLimit(RateLimitRecord),

    /// The agent is asking a human. Answered with `AnswerPermission` naming
    /// one of the options.
    PermissionRequest {
        request_id: String,
        tool_call: ToolCallPatch,
        options: Vec<PermissionOption>,
    },

    /// The turn is over. The agent is still alive; `ConversationEnded` is
    /// what says otherwise.
    TurnEnded {
        stop_reason: StopReason,
        error: Option<String>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_update_round_trips() {
        let update = ConvUpdate::AgentChunk {
            content: ConvContent::Text("hello".to_string()),
            message_id: Some("m1".to_string()),
            subagent: None,
        };
        let json = serde_json::to_string(&update).unwrap();
        let back: ConvUpdate = serde_json::from_str(&json).unwrap();
        assert_eq!(back, update);
    }

    /// A patch that names only a status must not claim to clear a title.
    #[test]
    fn a_patch_defaults_every_field_it_does_not_name() {
        let patch = ToolCallPatch {
            id: "t1".to_string(),
            status: Some(ToolStatus::Completed),
            ..ToolCallPatch::default()
        };
        assert_eq!(patch.title, None);
        assert_eq!(patch.content, None);
    }

    #[test]
    fn the_ring_is_used_over_size() {
        let usage = UsageRecord {
            used: 41_200,
            size: 200_000,
            cost_usd: None,
            model: None,
            spend: None,
            subagent: None,
        };
        assert_eq!(usage.context_pct(), Some(21));
    }

    /// A window nobody reported draws no ring, rather than a ring computed
    /// from a constant that would be wrong for half the models in use.
    #[test]
    fn an_unknown_window_has_no_percentage() {
        let usage = UsageRecord {
            used: 41_200,
            size: 0,
            cost_usd: None,
            model: None,
            spend: None,
            subagent: None,
        };
        assert_eq!(usage.context_pct(), None);
    }
}
