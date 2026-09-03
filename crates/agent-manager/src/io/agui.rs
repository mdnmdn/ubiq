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

use serde_json::{Value, json};

use crate::io::{AgentEvent, Content, StopReason, ToolCall, ToolCallUpdate};

/// Map one [`AgentEvent`] to a single AG-UI event object, or `None` if this
/// event has no AG-UI representation.
///
/// Field-name choices:
/// - [`AgentEvent::SessionStarted::session_id`] is optional upstream; it
///   becomes AG-UI's `runId`, emitted as JSON `null` when absent.
/// - A tool call's `id` is AG-UI's `toolCallId` directly — the neutral model
///   always has one (unlike the old ad-hoc event, which could omit it), so
///   there is no name fallback to make anymore.
/// - AG-UI's `TOOL_CALL_START`/`RESULT` have no `kind`/`status` fields of
///   their own; those richer facts ride along under `rawInput`/`rawOutput`
///   rather than being dropped.
pub fn to_agui(event: &AgentEvent) -> Option<Value> {
    match event {
        AgentEvent::SessionStarted { session_id, .. } => Some(json!({
            "type": "RUN_STARTED",
            "runId": session_id,
        })),
        AgentEvent::AgentMessageChunk { content, .. } => Some(json!({
            "type": "TEXT_MESSAGE_CONTENT",
            "delta": content_text(content),
        })),
        AgentEvent::AgentThoughtChunk { content, .. } => Some(json!({
            "type": "THINKING_TEXT_MESSAGE_CONTENT",
            "delta": content_text(content),
        })),

        AgentEvent::ToolCall { call } => Some(tool_call_start(call)),
        // AG-UI's `TOOL_CALL_RESULT` names only the finished call and its
        // content; a patch that only changes, say, `status` has nothing to
        // report under this event and is skipped.
        AgentEvent::ToolCallUpdate { update } => tool_call_result(update),

        AgentEvent::TurnEnded { stop_reason, error } => {
            let failed = matches!(stop_reason, StopReason::Refusal | StopReason::Failed);
            Some(if failed || error.is_some() {
                json!({
                    "type": "RUN_ERROR",
                    "message": error.clone().unwrap_or_else(|| "refused".to_string()),
                })
            } else {
                json!({"type": "RUN_FINISHED"})
            })
        }

        // No AG-UI event names these: the user-echo, the plan, the command
        // list, the mode/config pickers and the session title/mtime are all
        // ACP-only vocabulary AG-UI never grew an equivalent for.
        AgentEvent::UserMessageChunk { .. }
        | AgentEvent::Plan { .. }
        | AgentEvent::AvailableCommandsUpdate { .. }
        | AgentEvent::CurrentModeUpdate { .. }
        | AgentEvent::ConfigOptionUpdate { .. }
        | AgentEvent::SessionInfoUpdate { .. }
        // A permission ask is a request back to whoever is driving the
        // agent, not a thing AG-UI's run stream narrates.
        | AgentEvent::PermissionRequest { .. }
        // AG-UI has no context-window concept to carry `used`/`size` into.
        | AgentEvent::UsageUpdate { .. }
        | AgentEvent::Log { .. } => None,
    }
}

/// The text a chunk carries, or `""` for a non-text block — AG-UI's `delta`
/// is a string, and this mapping only ever sees [`Content::Text`] in
/// practice (no bridge emits image/audio/resource chunks yet).
fn content_text(content: &Content) -> &str {
    content.as_text().unwrap_or_default()
}

/// A `ToolCall`'s AG-UI `TOOL_CALL_START`. `kind`/`status`/`locations` have no
/// dedicated AG-UI fields, so they ride along inside `rawArgs` rather than
/// being dropped on the floor.
fn tool_call_start(call: &ToolCall) -> Value {
    json!({
        "type": "TOOL_CALL_START",
        "toolCallId": call.id,
        "toolCallName": call.title,
        "rawArgs": call.raw_input,
    })
}

/// A `ToolCallUpdate`'s AG-UI `TOOL_CALL_RESULT`, or `None` if the patch
/// carries no content to show — AG-UI's result event exists to narrate a
/// call's output, not every field a patch might touch.
fn tool_call_result(update: &ToolCallUpdate) -> Option<Value> {
    let content = update.content.as_ref()?;
    let text: Vec<&str> = content
        .iter()
        .filter_map(|item| match item {
            crate::io::ToolContent::Content { content } => content.as_text(),
            _ => None,
        })
        .collect();
    Some(json!({
        "type": "TOOL_CALL_RESULT",
        "toolCallId": update.id,
        "content": text.join(""),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::{PermissionOption, ToolCallUpdate, ToolContent, ToolKind, ToolStatus};

    #[test]
    fn session_started_maps_to_run_started() {
        let ev = AgentEvent::SessionStarted {
            session_id: Some("sess-1".to_string()),
            model: None,
            mode: None,
            tools: Vec::new(),
            agents: Vec::new(),
        };
        let got = to_agui(&ev).unwrap();
        assert_eq!(got, json!({"type": "RUN_STARTED", "runId": "sess-1"}));
    }

    #[test]
    fn session_started_without_id_emits_null_run_id() {
        let ev = AgentEvent::SessionStarted {
            session_id: None,
            model: None,
            mode: None,
            tools: Vec::new(),
            agents: Vec::new(),
        };
        let got = to_agui(&ev).unwrap();
        assert_eq!(got, json!({"type": "RUN_STARTED", "runId": null}));
    }

    #[test]
    fn agent_message_chunk_maps_to_text_message_content() {
        let ev = AgentEvent::AgentMessageChunk {
            content: Content::text("hi"),
            message_id: None,
        };
        let got = to_agui(&ev).unwrap();
        assert_eq!(got, json!({"type": "TEXT_MESSAGE_CONTENT", "delta": "hi"}));
    }

    #[test]
    fn agent_thought_chunk_maps_to_thinking_text_message_content() {
        let ev = AgentEvent::AgentThoughtChunk {
            content: Content::text("hmm"),
            message_id: None,
        };
        let got = to_agui(&ev).unwrap();
        assert_eq!(
            got,
            json!({"type": "THINKING_TEXT_MESSAGE_CONTENT", "delta": "hmm"})
        );
    }

    #[test]
    fn tool_call_maps_to_tool_call_start() {
        let mut call = ToolCall::new("call-1", "bash");
        call.kind = ToolKind::Execute;
        call.raw_input = Some(json!({"cmd": "ls"}));
        let ev = AgentEvent::ToolCall { call };
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
    fn tool_call_update_with_content_maps_to_tool_call_result() {
        let update = ToolCallUpdate {
            id: "call-1".to_string(),
            status: Some(ToolStatus::Completed),
            content: Some(vec![ToolContent::Content {
                content: Content::text("ok"),
            }]),
            ..ToolCallUpdate::default()
        };
        let ev = AgentEvent::ToolCallUpdate { update };
        let got = to_agui(&ev).unwrap();
        assert_eq!(
            got,
            json!({
                "type": "TOOL_CALL_RESULT",
                "toolCallId": "call-1",
                "content": "ok",
            })
        );
    }

    /// A patch with nothing to show (say, a bare status flip) has no AG-UI
    /// result to report.
    #[test]
    fn tool_call_update_without_content_is_none() {
        let ev = AgentEvent::ToolCallUpdate {
            update: ToolCallUpdate::finished("call-1", ToolStatus::Completed),
        };
        assert_eq!(to_agui(&ev), None);
    }

    #[test]
    fn turn_ended_end_turn_maps_to_run_finished() {
        let ev = AgentEvent::TurnEnded {
            stop_reason: StopReason::EndTurn,
            error: None,
        };
        assert_eq!(to_agui(&ev).unwrap(), json!({"type": "RUN_FINISHED"}));
    }

    #[test]
    fn turn_ended_failed_maps_to_run_error() {
        let ev = AgentEvent::TurnEnded {
            stop_reason: StopReason::Failed,
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
            to_agui(&AgentEvent::PermissionRequest {
                request_id: "r1".to_string(),
                tool_call: ToolCallUpdate::default(),
                options: Vec::<PermissionOption>::new(),
            }),
            None
        );
        assert_eq!(
            to_agui(&AgentEvent::UsageUpdate {
                used: 1,
                size: 2,
                cost: None,
                model: None,
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
