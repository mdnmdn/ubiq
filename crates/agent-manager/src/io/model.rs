//! The harness-neutral I/O model: the input `am` can feed a running agent,
//! and the events `am` can read back from it — independent of which harness
//! or wire protocol (NDJSON, JSON-RPC, ...) is actually in use.
//!
//! # The vocabulary is ACP's
//!
//! The event names and shapes here are the Agent Client Protocol's
//! `session/update` vocabulary, minus two things: the JSON-RPC envelope, and
//! the session id. `refs/acp-protocol.md` is the wire reference this
//! transcribes, and `_docs/io-modes.md` is the design note.
//!
//! **An event carries no session identity, deliberately.** Whoever holds the
//! table of live bridges supplies it: an embedder keys events by its own id
//! (Ubiq attaches an agent id and puts them on its bus), and an ACP server
//! built on top of this would attach a `sessionId` to the very same event.
//! Identity belongs to the multiplexer, never to the event — which is what
//! makes one bridge usable by several fronts without a second mapping.
//!
//! [`AgentEvent::SessionStarted`]'s `session_id` is not that identity: it is
//! the *harness's own* id for the conversation, the one a resume needs.
//!
//! # Compilation
//!
//! This module is **core** (always compiled, no feature gate): it only needs
//! `serde`/`serde_json`, so a lib-mode embedder built with
//! `--no-default-features` (no `pty`, no `cli`) can still depend on
//! [`AgentInput`], [`AgentEvent`] and the [`IoBridge`] trait.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

// ── content ────────────────────────────────────────────────────────────

/// One piece of content, in ACP's `ContentBlock` shape.
///
/// A harness that only speaks text produces [`Content::Text`] and nothing
/// else; the other variants exist so a bridge that has them does not have to
/// invent a shape for them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Content {
    /// Plain prose — what a harness that only speaks text produces.
    Text {
        /// The prose itself, verbatim; no markup is implied.
        text: String,
    },
    /// An image, embedded inline.
    Image {
        /// base64
        data: String,
        /// The image's MIME type, e.g. `image/png`.
        mime_type: String,
        /// Where the image came from, where the harness says — a display hint, not something a
        /// consumer need fetch.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        uri: Option<String>,
    },
    /// An audio clip, embedded inline. Unlike [`Self::Image`], ACP gives this no `uri` field.
    Audio {
        /// base64
        data: String,
        /// The audio's MIME type, e.g. `audio/wav`.
        mime_type: String,
    },
    /// A reference to a resource the harness has not inlined; a consumer fetches it from `uri`
    /// if it wants the content.
    ResourceLink {
        /// Where to fetch the resource.
        uri: String,
        /// A human-readable label for the resource.
        name: String,
        /// The resource's MIME type, where known.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mime_type: Option<String>,
        /// A longer display title, where it differs from `name`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        /// A human-readable summary of the resource.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        /// The resource's size in bytes, where known.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        size: Option<u64>,
    },
    /// An embedded resource: the content a [`Self::ResourceLink`] would otherwise make a
    /// consumer fetch, inlined instead.
    Resource {
        /// The resource's own content — text or base64 bytes; see [`ResourceContents`].
        resource: ResourceContents,
    },
}

impl Content {
    /// The common case: a plain text block.
    pub fn text(text: impl Into<String>) -> Self {
        Content::Text { text: text.into() }
    }

    /// The text this block carries, if it carries any. A consumer that only
    /// renders prose uses this and ignores everything else.
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Content::Text { text } => Some(text),
            _ => None,
        }
    }
}

/// The body of an embedded resource: text or base64 bytes, discriminated by
/// which field is present, exactly as ACP does it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ResourceContents {
    /// Text content, read straight from `uri`.
    Text {
        /// Where this content lives.
        uri: String,
        /// The content itself.
        text: String,
        /// The content's MIME type, where known.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mime_type: Option<String>,
    },
    /// Binary content, read straight from `uri`.
    Blob {
        /// Where this content lives.
        uri: String,
        /// base64
        blob: String,
        /// The content's MIME type, where known.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mime_type: Option<String>,
    },
}

// ── tool calls ─────────────────────────────────────────────────────────

/// What a tool call is, for a consumer that draws it. ACP's ten kinds; a
/// bridge maps its harness's tool names onto them and falls back to
/// [`ToolKind::Other`] rather than inventing an eleventh.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolKind {
    /// Reading files or data.
    Read,
    /// Modifying files or content.
    Edit,
    /// Removing files or data.
    Delete,
    /// Moving or renaming files.
    Move,
    /// Searching for information.
    Search,
    /// Running commands or code.
    Execute,
    /// Internal reasoning or planning.
    Think,
    /// Retrieving external data.
    Fetch,
    /// Switching the current session mode.
    SwitchMode,
    /// Anything that doesn't fit the other nine — a bridge's fallback rather than an eleventh
    /// kind it would have to invent.
    #[default]
    Other,
}

/// Where a tool call has got to. `Pending` covers both "its input is still
/// streaming" and "it is waiting for a human".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolStatus {
    /// Not started yet: input is still streaming in, or a permission request is pending.
    #[default]
    Pending,
    /// Currently running.
    InProgress,
    /// Finished without error.
    Completed,
    /// Finished with an error.
    Failed,
}

/// One thing a tool call has to show.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToolContent {
    /// A content block, the same shape as a message chunk's.
    Content {
        /// The block to show.
        content: Content,
    },
    /// An edit, in the shape the transcript's expanded diff block draws.
    /// `old_text` absent means the file is being created.
    Diff {
        /// The file the edit applies to.
        path: String,
        /// The file's content before the edit; absent when the edit creates the file.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        old_text: Option<String>,
        /// The file's content after the edit.
        new_text: String,
    },
    /// A terminal the *client* was asked to create. Nothing produces this
    /// yet; it is here because the vocabulary is documented whole.
    Terminal {
        /// The id of the terminal to embed, from an earlier `terminal/create`.
        terminal_id: String,
    },
}

/// A file a tool call touched, so a consumer can follow along.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolLocation {
    /// The file's path.
    pub path: String,
    /// 1-based, as ACP requires.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
}

/// A tool call as it starts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    /// Unique within the session; later updates and permission requests key on this.
    pub id: String,
    /// A human-readable description of what the tool is doing.
    pub title: String,
    /// What kind of operation this is; drives icon/UI treatment.
    #[serde(default)]
    pub kind: ToolKind,
    /// Where the call has got to.
    #[serde(default)]
    pub status: ToolStatus,
    /// What the call has produced so far, for the consumer to render.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub content: Vec<ToolContent>,
    /// The files this call touches, so a consumer can follow along and scroll to them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub locations: Vec<ToolLocation>,
    /// The raw parameters the harness sent the tool, for a consumer that wants more than the
    /// rendered summary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_input: Option<serde_json::Value>,
}

impl ToolCall {
    /// A call with nothing but the two fields every consumer needs.
    pub fn new(id: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            kind: ToolKind::default(),
            status: ToolStatus::default(),
            content: Vec::new(),
            locations: Vec::new(),
            raw_input: None,
        }
    }
}

/// A patch to a tool call already announced. **An absent field means
/// unchanged**, and `content`/`locations` replace the whole collection rather
/// than appending — both are ACP's rules, and a consumer that applies them
/// the other way silently loses half of an edit.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ToolCallUpdate {
    /// Which call this patches.
    pub id: String,
    /// The new title, if it changed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// The new kind, if it changed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<ToolKind>,
    /// The new status, if it changed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<ToolStatus>,
    /// The call's content, replacing whatever was shown before — not appended.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<Vec<ToolContent>>,
    /// The call's locations, replacing the previous set — not appended.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locations: Option<Vec<ToolLocation>>,
    /// The raw result the tool returned, once it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_output: Option<serde_json::Value>,
}

impl ToolCallUpdate {
    /// The common case: a call that has finished, one way or the other.
    pub fn finished(id: impl Into<String>, status: ToolStatus) -> Self {
        Self {
            id: id.into(),
            status: Some(status),
            ..Self::default()
        }
    }
}

// ── the plan, commands, config ─────────────────────────────────────────

/// How urgent one plan entry is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanPriority {
    /// Do this first.
    High,
    /// The default priority.
    #[default]
    Medium,
    /// Do this last, or skip it if time runs out.
    Low,
}

/// Where one plan entry has got to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanStatus {
    /// Not started yet.
    #[default]
    Pending,
    /// Underway.
    InProgress,
    /// Done.
    Completed,
}

/// One line of the agent's own todo list. A plan event carries **every**
/// entry each time — it is a replacement, not an append.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanEntry {
    /// The step itself, in the agent's own words.
    pub content: String,
    /// How urgent this step is.
    #[serde(default)]
    pub priority: PlanPriority,
    /// Where this step has got to.
    #[serde(default)]
    pub status: PlanStatus,
}

/// A slash command the session advertises.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandInfo {
    /// The command's name, typed after the slash.
    pub name: String,
    /// A human-readable summary, for a command palette.
    #[serde(default)]
    pub description: String,
    /// What free text after the command name is for, if it takes any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_hint: Option<String>,
}

/// What a config option is *for*, so a consumer can group its pickers. A hint
/// only: it must never change what an id means.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigCategory {
    /// A session mode selector.
    Mode,
    /// A model selector.
    Model,
    /// A model-related parameter: context size, or a speed/quality trade-off.
    ModelConfig,
    /// A reasoning/thinking-level selector.
    ThoughtLevel,
    /// An open enum upstream, so an unknown category is data rather than an
    /// error.
    #[serde(untagged)]
    Other(String),
}

/// One choice in a select-shaped config option.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigChoice {
    /// What a consumer sends back to select this choice.
    pub value: String,
    /// The label to display.
    pub name: String,
    /// A longer explanation, where the harness has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The group this choice is drawn under, where the harness groups them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
}

/// A session-level knob the harness advertises: the model, the mode, the
/// thinking level, or anything else it grew.
///
/// **This is one mechanism, not four.** Upstream deprecated dedicated mode
/// methods and has no model methods at all; every picker is a config option
/// with a [`ConfigCategory`]. So a harness that grows a fifth knob needs no
/// change in a consumer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConfigOption {
    /// Stable identifier a consumer sends back in [`AgentInput::SetConfigOption`].
    pub id: String,
    /// The label to display.
    pub name: String,
    /// A longer explanation, where the harness has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// What this option is for, for grouping — a hint only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<ConfigCategory>,
    /// The option's type and current setting.
    #[serde(flatten)]
    pub value: ConfigValue,
}

/// A config option's type and its current setting.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ConfigValue {
    /// A pick-one option, offered as a list of choices.
    Select {
        /// The currently selected choice's `value`.
        current_value: String,
        /// The choices on offer.
        #[serde(default)]
        options: Vec<ConfigChoice>,
    },
    /// An on/off option.
    Boolean {
        /// Whether it is currently on.
        current_value: bool,
    },
}

/// What a consumer sets a config option to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ConfigSetting {
    /// A select option's new `value`.
    Text(String),
    /// A boolean option's new setting.
    Flag(bool),
}

// ── permissions ────────────────────────────────────────────────────────

/// The four answers a permission dialog can offer. "Always" is what makes the
/// feature bearable, and it is the caller's job to remember it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionKind {
    /// Allow this operation, this time only.
    AllowOnce,
    /// Allow this operation and remember the choice — the caller's job, since the protocol
    /// itself does not remember it.
    AllowAlways,
    /// Reject this operation, this time only.
    RejectOnce,
    /// Reject this operation and remember the choice.
    RejectAlways,
}

impl PermissionKind {
    /// Whether choosing this lets the tool run.
    pub fn allows(self) -> bool {
        matches!(self, Self::AllowOnce | Self::AllowAlways)
    }
}

/// One button on a permission dialog.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionOption {
    /// Echoed back in [`PermissionOutcome::Selected`] to say which button was pressed.
    pub option_id: String,
    /// The label to display on the button.
    pub name: String,
    /// A hint for icon/UI treatment — the actual effect is whatever the harness does with
    /// `option_id`.
    pub kind: PermissionKind,
}

/// How a permission request was answered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum PermissionOutcome {
    /// The turn was cancelled before the human answered. Every pending
    /// request must be answered this way when a turn is cancelled.
    Cancelled,
    /// The human picked one of the offered options.
    Selected {
        /// The `option_id` of the [`PermissionOption`] chosen.
        option_id: String,
    },
}

// ── usage, and the end of a turn ───────────────────────────────────────

/// What a turn cost in money.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Cost {
    /// Cumulative for the whole session, not just this turn.
    pub amount: f64,
    /// ISO 4217.
    pub currency: String,
}

/// Why a turn stopped.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    /// The turn ended on its own — the ordinary case.
    #[default]
    EndTurn,
    /// The agent hit its maximum token count.
    MaxTokens,
    /// The agent hit the maximum number of model requests allowed within one turn.
    MaxTurnRequests,
    /// The model declined to continue. Distinct from [`Self::Failed`], which is a broken run
    /// rather than the model refusing.
    Refusal,
    /// The client cancelled the turn.
    Cancelled,
    /// Not ACP's: a harness that failed rather than finished. Kept separate
    /// from `Refusal`, which is the model declining rather than the run
    /// breaking.
    Failed,
}

// ── the events ─────────────────────────────────────────────────────────

/// One thing a running agent said.
///
/// Serialized `#[serde(tag = "type")]` in snake_case, and the names match
/// ACP's `sessionUpdate` discriminants wherever ACP has one, so
/// [`crate::io::to_acp`] is a rename rather than a translation.
///
/// Three variants have no ACP `session/update` equivalent because ACP carries
/// them at the protocol level instead: [`Self::SessionStarted`] (the result of
/// `session/new`), [`Self::PermissionRequest`] (a request back to the client)
/// and [`Self::TurnEnded`] (the `session/prompt` response's stop reason).
/// They are events here because a bridge has one stream, not a JSON-RPC peer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentEvent {
    /// The session exists, and here is what it can do. `session_id` is the
    /// *harness's* id — what a resume needs — not the consumer's.
    SessionStarted {
        /// The harness's own id for the conversation — what a resume needs, not the consumer's
        /// identity for the event stream.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_id: Option<String>,
        /// The model in use, where the harness says up front.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model: Option<String>,
        /// The mode in use, where the harness says up front.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mode: Option<String>,
        /// The tool names this run can call.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        tools: Vec<String>,
        /// The subagent types this run can spawn.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        agents: Vec<String>,
    },

    /// Something the *user* said, echoed by the harness. A consumer renders
    /// this rather than echoing its own composer, so the transcript is what
    /// the harness actually received.
    UserMessageChunk {
        /// The piece of content this chunk carries.
        content: Content,
        /// Chunks sharing this id belong to one logical message.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message_id: Option<String>,
    },
    /// Assistant prose. Chunks sharing a `message_id` are one message; a
    /// change of id starts a new one.
    AgentMessageChunk {
        /// The piece of content this chunk carries.
        content: Content,
        /// Chunks sharing this id belong to one logical message.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message_id: Option<String>,
    },
    /// Reasoning, drawn as a thinking block.
    AgentThoughtChunk {
        /// The piece of content this chunk carries.
        content: Content,
        /// Chunks sharing this id belong to one logical message.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message_id: Option<String>,
    },

    /// A tool call starts.
    ToolCall {
        /// The call itself.
        #[serde(flatten)]
        call: ToolCall,
    },
    /// A tool call progresses or ends. Absent fields are unchanged.
    ToolCallUpdate {
        /// The patch to apply.
        #[serde(flatten)]
        update: ToolCallUpdate,
    },

    /// The agent's todo list, complete each time.
    Plan {
        /// Every entry, in order — this replaces the whole list, not just the changed lines.
        entries: Vec<PlanEntry>,
    },
    /// Every slash command the session has.
    AvailableCommandsUpdate {
        /// The complete list, replacing whatever was advertised before.
        commands: Vec<CommandInfo>,
    },
    /// The mode changed, harness-side.
    CurrentModeUpdate {
        /// The mode's new id.
        current_mode_id: String,
    },
    /// Every config option, complete each time — setting one can change
    /// another, so a partial set would leave a stale picker on screen.
    ConfigOptionUpdate {
        /// The complete set, replacing whatever was advertised before.
        options: Vec<ConfigOption>,
    },
    /// The conversation's own title and modification time.
    SessionInfoUpdate {
        /// The conversation's title, where the harness names it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        /// When the conversation was last touched, ISO 8601.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        updated_at: Option<String>,
    },

    /// Context and money. `used`/`size` are tokens, and the ratio is the
    /// context ring — which is why no consumer needs a context-window
    /// constant of its own.
    UsageUpdate {
        /// Tokens currently occupying the context window.
        used: u64,
        /// The context window's total size, in tokens.
        size: u64,
        /// What the session has cost so far, cumulative — not just this turn.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cost: Option<Cost>,
        /// Which model these numbers are for, where the harness says.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model: Option<String>,
    },

    /// The agent is asking a human. Answered with
    /// [`AgentInput::AnswerPermission`] naming one of the options.
    PermissionRequest {
        /// Echoed back in [`AgentInput::AnswerPermission`] so the harness knows which request
        /// this answers.
        request_id: String,
        /// The call awaiting approval — often just an id, since every [`ToolCallUpdate`] field
        /// but `id` is optional.
        tool_call: ToolCallUpdate,
        /// The buttons to offer.
        options: Vec<PermissionOption>,
    },

    /// The turn is over.
    TurnEnded {
        /// Why the turn stopped.
        #[serde(default)]
        stop_reason: StopReason,
        /// Detail for [`StopReason::Failed`], or any other reason worth explaining.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },

    /// Harness diagnostics, passed through.
    Log {
        /// The diagnostic's severity, harness-defined (e.g. `"info"`, `"warn"`).
        level: String,
        /// The diagnostic text.
        message: String,
    },
}

// ── input ──────────────────────────────────────────────────────────────

/// One unit of input `am` can feed a running agent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentInput {
    /// A turn.
    Prompt {
        /// The content blocks making up the prompt.
        content: Vec<Content>,
    },
    /// Interrupt the turn in flight. Every pending permission request must
    /// then be answered [`PermissionOutcome::Cancelled`].
    Cancel,
    /// Answer a [`AgentEvent::PermissionRequest`]. `updated_input` may rewrite
    /// the tool's input before it runs, which is how a caller forces (say) a
    /// background command into the foreground.
    AnswerPermission {
        /// Which [`AgentEvent::PermissionRequest`] this answers.
        request_id: String,
        /// The human's choice.
        outcome: PermissionOutcome,
        /// A replacement for the tool's input, if the caller wants to change what runs before it
        /// does.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        updated_input: Option<serde_json::Value>,
    },
    /// Change a model, a mode, a thinking level — whatever the harness
    /// advertised under that id.
    SetConfigOption {
        /// Which [`ConfigOption::id`] to change.
        config_id: String,
        /// The new setting.
        value: ConfigSetting,
    },
}

impl AgentInput {
    /// The common case: a plain text prompt.
    pub fn prompt(text: impl Into<String>) -> Self {
        AgentInput::Prompt {
            content: vec![Content::text(text)],
        }
    }

    /// The prompt's text, joined — what a bridge whose harness takes a bare
    /// string sends.
    pub fn prompt_text(&self) -> Option<String> {
        match self {
            AgentInput::Prompt { content } => Some(
                content
                    .iter()
                    .filter_map(Content::as_text)
                    .collect::<Vec<_>>()
                    .join(""),
            ),
            _ => None,
        }
    }
}

// ── the bridge ─────────────────────────────────────────────────────────

/// A handle that feeds a running agent from a thread that does not own its
/// bridge.
///
/// This exists because [`IoBridge::next_event`] blocks and both its methods
/// take `&mut self`: a pump thread that owns the bridge to read it can never
/// also be handed a prompt. A bridge whose input side is independently
/// shareable answers [`IoBridge::input`] with one of these; a one-shot
/// harness answers `None`, and that `None` is the capability signal — a
/// composer learns a harness takes no second prompt from it rather than by
/// sending into a void.
pub trait AgentInputSink: Send + Sync {
    /// Feed the agent one unit of input, from any thread.
    fn send(&self, input: AgentInput) -> crate::Result<()>;
}

/// A live, harness-specific bridge between `am` and one running agent
/// process, translating [`AgentInput`]/[`AgentEvent`] to and from that
/// harness's actual wire protocol (NDJSON, JSON-RPC, ...).
///
/// `Send`, because [`Self::next_event`] blocks: an embedder that has anything
/// else to do puts the bridge on a thread of its own and reads it there.
pub trait IoBridge: Send {
    /// Feed the agent one unit of input.
    fn send(&mut self, input: AgentInput) -> crate::Result<()>;

    /// Pull the next normalized event, or `None` at end of stream.
    ///
    /// **Blocks** until there is one.
    fn next_event(&mut self) -> crate::Result<Option<AgentEvent>>;

    /// A handle that can feed this agent from another thread, if the bridge
    /// has one. See [`AgentInputSink`].
    fn input(&self) -> Option<Arc<dyn AgentInputSink>> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_message_chunk_round_trips_tagged_json() {
        let ev = AgentEvent::AgentMessageChunk {
            content: Content::text("hi"),
            message_id: Some("m1".to_string()),
        };
        let json = serde_json::to_string(&ev).unwrap();
        assert!(
            json.contains("\"type\":\"agent_message_chunk\""),
            "json was: {json}"
        );
        assert!(json.contains("\"text\":\"hi\""));
        let back: AgentEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(back, ev);
    }

    #[test]
    fn session_started_round_trips_tagged_json() {
        let ev = AgentEvent::SessionStarted {
            session_id: Some("abc-123".to_string()),
            model: Some("claude-opus-5".to_string()),
            mode: None,
            tools: vec!["Read".to_string()],
            agents: Vec::new(),
        };
        let json = serde_json::to_string(&ev).unwrap();
        assert!(
            json.contains("\"type\":\"session_started\""),
            "json was: {json}"
        );
        let back: AgentEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(back, ev);
    }

    /// The tool call's fields sit beside the tag rather than under a wrapper,
    /// which is what makes the ACP projection a rename.
    #[test]
    fn tool_call_flattens_beside_the_tag() {
        let mut call = ToolCall::new("t1", "Read src/main.rs");
        call.kind = ToolKind::Read;
        let ev = AgentEvent::ToolCall { call };
        let json = serde_json::to_string(&ev).unwrap();
        assert!(json.contains("\"type\":\"tool_call\""), "json was: {json}");
        assert!(json.contains("\"id\":\"t1\""), "json was: {json}");
        assert!(json.contains("\"kind\":\"read\""), "json was: {json}");
        let back: AgentEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(back, ev);
    }

    /// An absent field in a patch is "unchanged", so it must not serialize as
    /// a null a consumer could read as "cleared".
    #[test]
    fn tool_call_update_omits_what_it_does_not_change() {
        let ev = AgentEvent::ToolCallUpdate {
            update: ToolCallUpdate::finished("t1", ToolStatus::Completed),
        };
        let json = serde_json::to_string(&ev).unwrap();
        assert!(!json.contains("null"), "json was: {json}");
        assert!(!json.contains("\"title\""), "json was: {json}");
        let back: AgentEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(back, ev);
    }

    #[test]
    fn usage_update_round_trips() {
        let ev = AgentEvent::UsageUpdate {
            used: 41_200,
            size: 1_000_000,
            cost: Some(Cost {
                amount: 0.42,
                currency: "USD".to_string(),
            }),
            model: Some("claude-opus-5".to_string()),
        };
        let json = serde_json::to_string(&ev).unwrap();
        assert!(
            json.contains("\"type\":\"usage_update\""),
            "json was: {json}"
        );
        let back: AgentEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(back, ev);
    }

    #[test]
    fn turn_ended_defaults_its_stop_reason() {
        let ev: AgentEvent = serde_json::from_str(r#"{"type":"turn_ended"}"#).unwrap();
        assert_eq!(
            ev,
            AgentEvent::TurnEnded {
                stop_reason: StopReason::EndTurn,
                error: None,
            }
        );
    }

    #[test]
    fn agent_input_prompt_round_trips_tagged_json() {
        let input = AgentInput::prompt("do the thing");
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"type\":\"prompt\""), "json was: {json}");
        assert!(json.contains("\"text\":\"do the thing\""));
        let back: AgentInput = serde_json::from_str(&json).unwrap();
        assert_eq!(back, input);
        assert_eq!(input.prompt_text().as_deref(), Some("do the thing"));
    }

    #[test]
    fn agent_input_cancel_round_trips_tagged_json() {
        let input = AgentInput::Cancel;
        let json = serde_json::to_string(&input).unwrap();
        assert_eq!(json, "{\"type\":\"cancel\"}");
        let back: AgentInput = serde_json::from_str(&json).unwrap();
        assert_eq!(back, input);
    }

    #[test]
    fn permission_outcome_is_tagged_on_outcome() {
        let outcome = PermissionOutcome::Selected {
            option_id: "allow".to_string(),
        };
        let json = serde_json::to_string(&outcome).unwrap();
        assert!(
            json.contains("\"outcome\":\"selected\""),
            "json was: {json}"
        );
        let back: PermissionOutcome = serde_json::from_str(&json).unwrap();
        assert_eq!(back, outcome);
    }

    /// A category upstream has not defined yet is data, not a parse failure.
    #[test]
    fn config_category_keeps_an_unknown_value() {
        let parsed: ConfigCategory = serde_json::from_str("\"verbosity\"").unwrap();
        assert_eq!(parsed, ConfigCategory::Other("verbosity".to_string()));
        let known: ConfigCategory = serde_json::from_str("\"model\"").unwrap();
        assert_eq!(known, ConfigCategory::Model);
    }

    #[test]
    fn config_option_flattens_its_value() {
        let option = ConfigOption {
            id: "model".to_string(),
            name: "Model".to_string(),
            description: None,
            category: Some(ConfigCategory::Model),
            value: ConfigValue::Select {
                current_value: "opus".to_string(),
                options: vec![ConfigChoice {
                    value: "opus".to_string(),
                    name: "Opus".to_string(),
                    description: None,
                    group: None,
                }],
            },
        };
        let json = serde_json::to_string(&option).unwrap();
        assert!(json.contains("\"type\":\"select\""), "json was: {json}");
        assert!(
            json.contains("\"current_value\":\"opus\""),
            "json was: {json}"
        );
        let back: ConfigOption = serde_json::from_str(&json).unwrap();
        assert_eq!(back, option);
    }
}
