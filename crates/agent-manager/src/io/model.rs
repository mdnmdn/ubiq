//! The harness-neutral I/O model: the input `am` can feed a running agent,
//! and the events `am` can read back from it — independent of which harness
//! or wire protocol (NDJSON, JSON-RPC, ...) is actually in use.
//!
//! This module is **core** (always compiled, no feature gate): it only needs
//! `serde`/`serde_json`, both of which are always available, so a lib-mode
//! embedder built with `--no-default-features` (no `pty`, no `cli`) can still
//! depend on [`AgentInput`], [`AgentEvent`], and the [`IoBridge`] trait. Only
//! the *concrete* bridges that speak a harness's real protocol (landing in
//! later steps) and the raw-tty [`super::passthrough`] module need extra
//! deps, and stay feature-gated.
//!
//! See `_docs/target/io-modes.md` §"The `IoBridge` trait" for the design
//! this transcribes.

use serde::{Deserialize, Serialize};

/// One unit of input `am` can feed a running agent.
///
/// Serialized with `#[serde(tag = "type")]` so NDJSON produced from this type
/// is self-describing, e.g. `{"type":"prompt","text":"..."}`.
///
/// Note: [`AgentInput::Prompt`] is a *struct* variant (`{ text: String }`)
/// rather than a tuple newtype (`Prompt(String)`) — serde's internally
/// tagged representation cannot merge a bare scalar (like a `String`) with
/// the tag field, only a map/struct, so every variant here carries named
/// fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentInput {
    /// Send a user prompt / message.
    Prompt {
        /// The prompt text.
        text: String,
    },
    /// Answer a pending tool-approval request.
    ApproveTool {
        /// The id of the approval request being answered (echoed from the
        /// matching [`AgentEvent::ApprovalRequest`]).
        request_id: String,
        /// Allow or deny the tool call.
        decision: ApprovalDecision,
        /// Optionally replace the tool's input before it runs (e.g. an
        /// edited command), if the harness supports it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        updated_input: Option<serde_json::Value>,
    },
    /// Interrupt/cancel the agent's current turn.
    Interrupt,
}

/// The answer to an [`AgentEvent::ApprovalRequest`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDecision {
    /// Allow the tool call to proceed.
    Allow,
    /// Deny the tool call.
    Deny,
}

/// One normalized event read back from a running agent.
///
/// Serialized with `#[serde(tag = "type")]` so NDJSON produced from this type
/// is self-describing, e.g. `{"type":"assistant_text","text":"..."}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentEvent {
    /// The agent session has started.
    SessionStarted {
        /// Harness-assigned session id, if any.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_id: Option<String>,
    },
    /// A chunk (or the whole) of the assistant's text reply.
    AssistantText {
        /// The text.
        text: String,
    },
    /// A chunk of the agent's "thinking"/reasoning trace, if the harness
    /// exposes one.
    Thinking {
        /// The text.
        text: String,
    },
    /// The agent invoked a tool.
    ToolCall {
        /// Harness-assigned call id, if any.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        /// Tool name.
        name: String,
        /// Tool input, as the harness reported it.
        input: serde_json::Value,
    },
    /// A tool call's result.
    ToolResult {
        /// The call id this result answers, if known.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        /// The result content.
        content: serde_json::Value,
    },
    /// The agent is asking for approval to run a tool.
    ApprovalRequest {
        /// Id to echo back in [`AgentInput::ApproveTool`].
        request_id: String,
        /// Tool name awaiting approval.
        tool_name: String,
        /// Tool input awaiting approval.
        input: serde_json::Value,
    },
    /// Token usage, if the harness reports it.
    Usage {
        /// Input tokens.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        input_tokens: Option<u64>,
        /// Output tokens.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        output_tokens: Option<u64>,
    },
    /// The run finished.
    Result {
        /// Whether the run succeeded.
        success: bool,
        /// Error message, if it didn't.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    /// A log line the harness emitted that doesn't fit another variant.
    Log {
        /// Severity, harness-defined (e.g. `"info"`, `"warn"`, `"error"`).
        level: String,
        /// The message.
        message: String,
    },
}

/// A live, harness-specific bridge between `am` and one running agent
/// process, translating [`AgentInput`]/[`AgentEvent`] to/from that harness's
/// actual wire protocol (NDJSON, JSON-RPC, ...).
///
/// Concrete implementations land per-harness in later steps (C2/C3/C4);
/// until then, [`crate::harness::Harness::structured_bridge`] defaults to an
/// error.
pub trait IoBridge {
    /// Feed the agent one unit of input (a prompt, a tool-approval answer).
    fn send(&mut self, input: AgentInput) -> crate::Result<()>;
    /// Pull the next normalized event, or `None` at end of stream.
    fn next_event(&mut self) -> crate::Result<Option<AgentEvent>>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_event_assistant_text_round_trips_tagged_json() {
        let ev = AgentEvent::AssistantText {
            text: "hi".to_string(),
        };
        let json = serde_json::to_string(&ev).unwrap();
        assert!(
            json.contains("\"type\":\"assistant_text\""),
            "json was: {json}"
        );
        assert!(json.contains("\"text\":\"hi\""));
        let back: AgentEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(back, ev);
    }

    #[test]
    fn agent_event_session_started_round_trips_tagged_json() {
        let ev = AgentEvent::SessionStarted {
            session_id: Some("abc-123".to_string()),
        };
        let json = serde_json::to_string(&ev).unwrap();
        assert!(
            json.contains("\"type\":\"session_started\""),
            "json was: {json}"
        );
        let back: AgentEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(back, ev);
    }

    #[test]
    fn agent_event_result_round_trips_tagged_json() {
        let ev = AgentEvent::Result {
            success: false,
            error: Some("boom".to_string()),
        };
        let json = serde_json::to_string(&ev).unwrap();
        assert!(json.contains("\"type\":\"result\""), "json was: {json}");
        assert!(json.contains("\"success\":false"));
        let back: AgentEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(back, ev);
    }

    #[test]
    fn agent_input_prompt_round_trips_tagged_json() {
        let input = AgentInput::Prompt {
            text: "do the thing".to_string(),
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"type\":\"prompt\""), "json was: {json}");
        assert!(json.contains("\"text\":\"do the thing\""));
        let back: AgentInput = serde_json::from_str(&json).unwrap();
        assert_eq!(back, input);
    }

    #[test]
    fn agent_input_interrupt_round_trips_tagged_json() {
        let input = AgentInput::Interrupt;
        let json = serde_json::to_string(&input).unwrap();
        assert_eq!(json, "{\"type\":\"interrupt\"}");
        let back: AgentInput = serde_json::from_str(&json).unwrap();
        assert_eq!(back, input);
    }
}
