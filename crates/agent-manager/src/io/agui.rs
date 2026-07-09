//! Best-effort projection of the neutral [`crate::io::AgentEvent`] model onto
//! the AG-UI event schema.
//!
//! **Fidelity caveat:** this is a *stateless, one-event-in → one-value-out*
//! mapping over the subset of [`AgentEvent`] variants that translate cleanly
//! to a single AG-UI `type`-discriminated event. It does not emit the full
//! AG-UI run lifecycle (e.g. `TEXT_MESSAGE_START`/`TEXT_MESSAGE_END` framing
//! around message deltas, `TOOL_CALL_END`, `RUN_STARTED`'s `threadId`) and
//! does not thread any run/thread id through beyond what a single event
//! carries — a fuller, stateful adapter that tracks message/tool-call
//! lifecycles is future work. Events with no reasonable AG-UI representation
//! map to `None` and are skipped by the caller.

use serde_json::{json, Value};

use crate::io::AgentEvent;

/// Map one [`AgentEvent`] to a single AG-UI event object, or `None` if this
/// event has no AG-UI representation.
///
/// Field-name choices:
/// - [`AgentEvent::SessionStarted::session_id`] is optional upstream; it
///   becomes AG-UI's `runId`, emitted as JSON `null` when absent.
/// - [`AgentEvent::ToolCall::id`] is optional upstream; when absent we fall
///   back to `name` for `toolCallId`.
/// - [`AgentEvent::ToolResult::id`] is optional upstream too; when absent we
///   emit `null` for `toolCallId` since there's no name to fall back to.
pub fn to_agui(event: &AgentEvent) -> Option<Value> {
    match event {
        AgentEvent::SessionStarted { session_id } => Some(json!({
            "type": "RUN_STARTED",
            "runId": session_id,
        })),
        AgentEvent::AssistantText { text } => Some(json!({
            "type": "TEXT_MESSAGE_CONTENT",
            "delta": text,
        })),
        AgentEvent::Thinking { text } => Some(json!({
            "type": "THINKING_TEXT_MESSAGE_CONTENT",
            "delta": text,
        })),
        AgentEvent::ToolCall { id, name, input } => Some(json!({
            "type": "TOOL_CALL_START",
            "toolCallId": id.clone().unwrap_or_else(|| name.clone()),
            "toolCallName": name,
            "rawArgs": input,
        })),
        AgentEvent::ToolResult { id, content } => Some(json!({
            "type": "TOOL_CALL_RESULT",
            "toolCallId": id,
            "content": content,
        })),
        AgentEvent::Result { success, error } => {
            if *success {
                Some(json!({"type": "RUN_FINISHED"}))
            } else {
                Some(json!({"type": "RUN_ERROR", "message": error}))
            }
        }
        AgentEvent::ApprovalRequest { .. } | AgentEvent::Usage { .. } | AgentEvent::Log { .. } => {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_started_maps_to_run_started() {
        let ev = AgentEvent::SessionStarted {
            session_id: Some("sess-1".to_string()),
        };
        let got = to_agui(&ev).unwrap();
        assert_eq!(got, json!({"type": "RUN_STARTED", "runId": "sess-1"}));
    }

    #[test]
    fn session_started_without_id_emits_null_run_id() {
        let ev = AgentEvent::SessionStarted { session_id: None };
        let got = to_agui(&ev).unwrap();
        assert_eq!(got, json!({"type": "RUN_STARTED", "runId": null}));
    }

    #[test]
    fn assistant_text_maps_to_text_message_content() {
        let ev = AgentEvent::AssistantText {
            text: "hi".to_string(),
        };
        let got = to_agui(&ev).unwrap();
        assert_eq!(got, json!({"type": "TEXT_MESSAGE_CONTENT", "delta": "hi"}));
    }

    #[test]
    fn thinking_maps_to_thinking_text_message_content() {
        let ev = AgentEvent::Thinking {
            text: "hmm".to_string(),
        };
        let got = to_agui(&ev).unwrap();
        assert_eq!(
            got,
            json!({"type": "THINKING_TEXT_MESSAGE_CONTENT", "delta": "hmm"})
        );
    }

    #[test]
    fn tool_call_maps_to_tool_call_start() {
        let ev = AgentEvent::ToolCall {
            id: Some("call-1".to_string()),
            name: "bash".to_string(),
            input: json!({"cmd": "ls"}),
        };
        let got = to_agui(&ev).unwrap();
        assert_eq!(
            got,
            json!({
                "type": "TOOL_CALL_START",
                "toolCallId": "call-1",
                "toolCallName": "bash",
                "rawArgs": {"cmd": "ls"},
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
        let got = to_agui(&ev).unwrap();
        assert_eq!(got["toolCallId"], json!("bash"));
    }

    #[test]
    fn tool_result_maps_to_tool_call_result() {
        let ev = AgentEvent::ToolResult {
            id: Some("call-1".to_string()),
            content: json!({"stdout": "ok"}),
        };
        let got = to_agui(&ev).unwrap();
        assert_eq!(
            got,
            json!({
                "type": "TOOL_CALL_RESULT",
                "toolCallId": "call-1",
                "content": {"stdout": "ok"},
            })
        );
    }

    #[test]
    fn result_success_maps_to_run_finished() {
        let ev = AgentEvent::Result {
            success: true,
            error: None,
        };
        assert_eq!(to_agui(&ev).unwrap(), json!({"type": "RUN_FINISHED"}));
    }

    #[test]
    fn result_failure_maps_to_run_error() {
        let ev = AgentEvent::Result {
            success: false,
            error: Some("boom".to_string()),
        };
        assert_eq!(
            to_agui(&ev).unwrap(),
            json!({"type": "RUN_ERROR", "message": "boom"})
        );
    }

    #[test]
    fn unmapped_variants_are_none() {
        assert_eq!(
            to_agui(&AgentEvent::ApprovalRequest {
                request_id: "r1".to_string(),
                tool_name: "bash".to_string(),
                input: json!({}),
            }),
            None
        );
        assert_eq!(
            to_agui(&AgentEvent::Usage {
                input_tokens: Some(1),
                output_tokens: Some(2),
            }),
            None
        );
        assert_eq!(
            to_agui(&AgentEvent::Log {
                level: "info".to_string(),
                message: "hi".to_string(),
            }),
            None
        );
    }
}
