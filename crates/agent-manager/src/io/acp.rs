//! Best-effort projection of the neutral [`crate::io::AgentEvent`] model onto
//! the Agent Client Protocol (ACP) `session/update` notification shape.
//!
//! **Fidelity caveat:** this is a *stateless, one-event-in → one-value-out*
//! mapping over the subset of [`AgentEvent`] variants that translate cleanly
//! to an ACP `sessionUpdate`-discriminated object. It does not emit the full
//! ACP `session/update` JSON-RPC envelope (method name, `sessionId`, request
//! framing), does not bracket a turn with start/end markers, and does not
//! thread any session id through — callers that need a real ACP session
//! stream will need a fuller, stateful adapter (future work). Events with no
//! reasonable ACP representation map to `None` and are skipped by the
//! caller.

use serde_json::{json, Value};

use crate::io::AgentEvent;

/// Map one [`AgentEvent`] to the `update` payload of an ACP `session/update`
/// notification, or `None` if this event has no ACP representation.
///
/// Field-name choices:
/// - [`AgentEvent::ToolCall::id`] is optional upstream; when absent we fall
///   back to `name` for `toolCallId` (still a stable-ish identifier, better
///   than emitting `null`).
/// - [`AgentEvent::ToolResult::id`] is optional upstream too; when absent we
///   emit `null` for `toolCallId` since there's no name to fall back to.
pub fn to_acp(event: &AgentEvent) -> Option<Value> {
    match event {
        AgentEvent::AssistantText { text } => Some(json!({
            "sessionUpdate": "agent_message_chunk",
            "content": {"type": "text", "text": text},
        })),
        AgentEvent::Thinking { text } => Some(json!({
            "sessionUpdate": "agent_thought_chunk",
            "content": {"type": "text", "text": text},
        })),
        AgentEvent::ToolCall { id, name, input } => Some(json!({
            "sessionUpdate": "tool_call",
            "toolCallId": id.clone().unwrap_or_else(|| name.clone()),
            "title": name,
            "status": "pending",
            "rawInput": input,
        })),
        AgentEvent::ToolResult { id, content } => Some(json!({
            "sessionUpdate": "tool_call_update",
            "toolCallId": id,
            "status": "completed",
            "rawOutput": content,
        })),
        AgentEvent::SessionStarted { .. }
        | AgentEvent::ApprovalRequest { .. }
        | AgentEvent::Usage { .. }
        | AgentEvent::Result { .. }
        | AgentEvent::Log { .. } => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assistant_text_maps_to_agent_message_chunk() {
        let ev = AgentEvent::AssistantText {
            text: "hi".to_string(),
        };
        let got = to_acp(&ev).unwrap();
        assert_eq!(
            got,
            json!({
                "sessionUpdate": "agent_message_chunk",
                "content": {"type": "text", "text": "hi"},
            })
        );
    }

    #[test]
    fn thinking_maps_to_agent_thought_chunk() {
        let ev = AgentEvent::Thinking {
            text: "hmm".to_string(),
        };
        let got = to_acp(&ev).unwrap();
        assert_eq!(
            got,
            json!({
                "sessionUpdate": "agent_thought_chunk",
                "content": {"type": "text", "text": "hmm"},
            })
        );
    }

    #[test]
    fn tool_call_with_id_maps_to_tool_call() {
        let ev = AgentEvent::ToolCall {
            id: Some("call-1".to_string()),
            name: "bash".to_string(),
            input: json!({"cmd": "ls"}),
        };
        let got = to_acp(&ev).unwrap();
        assert_eq!(
            got,
            json!({
                "sessionUpdate": "tool_call",
                "toolCallId": "call-1",
                "title": "bash",
                "status": "pending",
                "rawInput": {"cmd": "ls"},
            })
        );
    }

    #[test]
    fn tool_call_without_id_falls_back_to_name() {
        let ev = AgentEvent::ToolCall {
            id: None,
            name: "bash".to_string(),
            input: json!({}),
        };
        let got = to_acp(&ev).unwrap();
        assert_eq!(got["toolCallId"], json!("bash"));
    }

    #[test]
    fn tool_result_maps_to_tool_call_update() {
        let ev = AgentEvent::ToolResult {
            id: Some("call-1".to_string()),
            content: json!({"stdout": "ok"}),
        };
        let got = to_acp(&ev).unwrap();
        assert_eq!(
            got,
            json!({
                "sessionUpdate": "tool_call_update",
                "toolCallId": "call-1",
                "status": "completed",
                "rawOutput": {"stdout": "ok"},
            })
        );
    }

    #[test]
    fn unmapped_variants_are_none() {
        assert_eq!(to_acp(&AgentEvent::SessionStarted { session_id: None }), None);
        assert_eq!(
            to_acp(&AgentEvent::ApprovalRequest {
                request_id: "r1".to_string(),
                tool_name: "bash".to_string(),
                input: json!({}),
            }),
            None
        );
        assert_eq!(
            to_acp(&AgentEvent::Usage {
                input_tokens: Some(1),
                output_tokens: Some(2),
            }),
            None
        );
        assert_eq!(
            to_acp(&AgentEvent::Result {
                success: true,
                error: None,
            }),
            None
        );
        assert_eq!(
            to_acp(&AgentEvent::Log {
                level: "info".to_string(),
                message: "hi".to_string(),
            }),
            None
        );
    }
}
