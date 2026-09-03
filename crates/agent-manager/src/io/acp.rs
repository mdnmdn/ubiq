//! Projecting [`AgentEvent`] onto the Agent Client Protocol.
//!
//! Since the neutral model *is* ACP's `session/update` vocabulary (see
//! [`super::model`]), this is a rename and a re-casing rather than a
//! translation: the discriminant moves from `type` to `sessionUpdate`, and
//! keys go from snake_case to camelCase. `refs/acp-protocol.md` is the wire
//! reference; the two rules it turns on are that **ACP keys are camelCase
//! while discriminator values stay snake_case**, and that every union is
//! internally tagged with its payload flattened beside the tag.
//!
//! **This is still not an ACP endpoint.** What comes out is the `params` of a
//! `session/update` notification, without the JSON-RPC envelope and without a
//! `sessionId` — because an event carries no session identity, by design. A
//! server built on this attaches both: it owns the connection and the table
//! of live sessions, and the mapping stays untouched. That is the whole
//! reason the identity is not on the event.
//!
//! Three events are **not** session updates in ACP, and
//! [`to_acp`] answers `None` for them, because they belong to the protocol
//! level a bridge does not have:
//!
//! - [`AgentEvent::SessionStarted`] is the *result* of `session/new`.
//! - [`AgentEvent::PermissionRequest`] is a `session/request_permission`
//!   request **back to the client**, which needs a request id and a reply.
//! - [`AgentEvent::TurnEnded`] is the `session/prompt` *response*, carrying
//!   its `stopReason`.
//!
//! A server maps those three itself; [`stop_reason`] and [`tool_call_value`]
//! are public so it does not have to re-derive their wire form.
//!
//! This module is **core** (no feature gate): `serde_json` only.

use serde_json::{Map, Value, json};

use super::model::{
    AgentEvent, Content, StopReason, ToolCall, ToolCallUpdate, ToolContent, ToolKind, ToolLocation,
    ToolStatus,
};

/// Project one [`AgentEvent`] onto one ACP `session/update` payload.
///
/// `None` means "not a session update" — see the module docs for the three
/// events that are protocol-level in ACP.
pub fn to_acp(event: &AgentEvent) -> Option<Value> {
    let value = match event {
        AgentEvent::UserMessageChunk {
            content,
            message_id,
        } => chunk("user_message_chunk", content, message_id.as_deref()),
        AgentEvent::AgentMessageChunk {
            content,
            message_id,
        } => chunk("agent_message_chunk", content, message_id.as_deref()),
        AgentEvent::AgentThoughtChunk {
            content,
            message_id,
        } => chunk("agent_thought_chunk", content, message_id.as_deref()),

        AgentEvent::ToolCall { call } => {
            let mut object = tool_call_value(call);
            object.insert("sessionUpdate".to_string(), json!("tool_call"));
            Value::Object(object)
        }
        AgentEvent::ToolCallUpdate { update } => {
            let mut object = tool_call_update_value(update);
            object.insert("sessionUpdate".to_string(), json!("tool_call_update"));
            Value::Object(object)
        }

        AgentEvent::Plan { entries } => json!({
            "sessionUpdate": "plan",
            "entries": entries.iter().map(|entry| json!({
                "content": entry.content,
                "priority": entry.priority,
                "status": entry.status,
            })).collect::<Vec<_>>(),
        }),

        AgentEvent::AvailableCommandsUpdate { commands } => json!({
            "sessionUpdate": "available_commands_update",
            "availableCommands": commands.iter().map(|command| {
                let mut object = json!({
                    "name": command.name,
                    "description": command.description,
                });
                if let Some(hint) = &command.input_hint {
                    object["input"] = json!({ "hint": hint });
                }
                object
            }).collect::<Vec<_>>(),
        }),

        AgentEvent::CurrentModeUpdate { current_mode_id } => json!({
            "sessionUpdate": "current_mode_update",
            // The schema's name. Upstream's prose example says `modeId`, and
            // the schema is what is generated from the source.
            "currentModeId": current_mode_id,
        }),

        AgentEvent::ConfigOptionUpdate { options } => json!({
            "sessionUpdate": "config_option_update",
            "configOptions": options.iter().map(config_option_value).collect::<Vec<_>>(),
        }),

        AgentEvent::SessionInfoUpdate { title, updated_at } => {
            let mut object = Map::new();
            object.insert("sessionUpdate".to_string(), json!("session_info_update"));
            if let Some(title) = title {
                object.insert("title".to_string(), json!(title));
            }
            if let Some(updated_at) = updated_at {
                object.insert("updatedAt".to_string(), json!(updated_at));
            }
            Value::Object(object)
        }

        AgentEvent::UsageUpdate {
            used, size, cost, ..
        } => {
            let mut object = json!({
                "sessionUpdate": "usage_update",
                "used": used,
                "size": size,
            });
            if let Some(cost) = cost {
                object["cost"] = json!({ "amount": cost.amount, "currency": cost.currency });
            }
            object
        }

        // Protocol-level in ACP, not a session update.
        AgentEvent::SessionStarted { .. }
        | AgentEvent::PermissionRequest { .. }
        | AgentEvent::TurnEnded { .. }
        | AgentEvent::Log { .. } => return None,
    };
    Some(value)
}

/// A `ToolCall` in ACP's shape, as a map so a caller can add the discriminant
/// it needs — `sessionUpdate` for an update, nothing for the `toolCall` field
/// of a permission request.
pub fn tool_call_value(call: &ToolCall) -> Map<String, Value> {
    let mut object = Map::new();
    object.insert("toolCallId".to_string(), json!(call.id));
    object.insert("title".to_string(), json!(call.title));
    object.insert("kind".to_string(), json!(kind(call.kind)));
    object.insert("status".to_string(), json!(status(call.status)));
    if !call.content.is_empty() {
        object.insert("content".to_string(), contents(&call.content));
    }
    if !call.locations.is_empty() {
        object.insert("locations".to_string(), locations(&call.locations));
    }
    if let Some(raw) = &call.raw_input {
        object.insert("rawInput".to_string(), raw.clone());
    }
    object
}

/// A `ToolCallUpdate` in ACP's shape. **An absent field is omitted, never
/// nulled** — a null would read as "cleared" to a client applying the patch.
pub fn tool_call_update_value(update: &ToolCallUpdate) -> Map<String, Value> {
    let mut object = Map::new();
    object.insert("toolCallId".to_string(), json!(update.id));
    if let Some(title) = &update.title {
        object.insert("title".to_string(), json!(title));
    }
    if let Some(k) = update.kind {
        object.insert("kind".to_string(), json!(kind(k)));
    }
    if let Some(s) = update.status {
        object.insert("status".to_string(), json!(status(s)));
    }
    if let Some(content) = &update.content {
        object.insert("content".to_string(), contents(content));
    }
    if let Some(locs) = &update.locations {
        object.insert("locations".to_string(), locations(locs));
    }
    if let Some(raw) = &update.raw_output {
        object.insert("rawOutput".to_string(), raw.clone());
    }
    object
}

/// ACP's `stopReason`, for the `session/prompt` response a server sends when
/// it sees an [`AgentEvent::TurnEnded`].
///
/// ACP has no "the run broke" reason, so [`StopReason::Failed`] becomes a
/// refusal — the nearest thing that says the turn produced no answer.
pub fn stop_reason(reason: &StopReason) -> &'static str {
    match reason {
        StopReason::EndTurn => "end_turn",
        StopReason::MaxTokens => "max_tokens",
        StopReason::MaxTurnRequests => "max_turn_requests",
        StopReason::Refusal | StopReason::Failed => "refusal",
        StopReason::Cancelled => "cancelled",
    }
}

fn chunk(update: &str, content: &Content, message_id: Option<&str>) -> Value {
    let mut object = json!({
        "sessionUpdate": update,
        "content": content_value(content),
    });
    if let Some(id) = message_id {
        object["messageId"] = json!(id);
    }
    object
}

fn content_value(content: &Content) -> Value {
    match content {
        Content::Text { text } => json!({ "type": "text", "text": text }),
        Content::Image {
            data,
            mime_type,
            uri,
        } => {
            let mut object = json!({ "type": "image", "data": data, "mimeType": mime_type });
            if let Some(uri) = uri {
                object["uri"] = json!(uri);
            }
            object
        }
        Content::Audio { data, mime_type } => {
            json!({ "type": "audio", "data": data, "mimeType": mime_type })
        }
        Content::ResourceLink {
            uri,
            name,
            mime_type,
            title,
            description,
            size,
        } => {
            let mut object = json!({ "type": "resource_link", "uri": uri, "name": name });
            if let Some(mime_type) = mime_type {
                object["mimeType"] = json!(mime_type);
            }
            if let Some(title) = title {
                object["title"] = json!(title);
            }
            if let Some(description) = description {
                object["description"] = json!(description);
            }
            if let Some(size) = size {
                object["size"] = json!(size);
            }
            object
        }
        Content::Resource { resource } => {
            json!({ "type": "resource", "resource": resource_value(resource) })
        }
    }
}

fn resource_value(resource: &super::model::ResourceContents) -> Value {
    use super::model::ResourceContents;
    match resource {
        ResourceContents::Text {
            uri,
            text,
            mime_type,
        } => {
            let mut object = json!({ "uri": uri, "text": text });
            if let Some(mime_type) = mime_type {
                object["mimeType"] = json!(mime_type);
            }
            object
        }
        ResourceContents::Blob {
            uri,
            blob,
            mime_type,
        } => {
            let mut object = json!({ "uri": uri, "blob": blob });
            if let Some(mime_type) = mime_type {
                object["mimeType"] = json!(mime_type);
            }
            object
        }
    }
}

fn contents(items: &[ToolContent]) -> Value {
    Value::Array(
        items
            .iter()
            .map(|item| match item {
                ToolContent::Content { content } => {
                    json!({ "type": "content", "content": content_value(content) })
                }
                ToolContent::Diff {
                    path,
                    old_text,
                    new_text,
                } => json!({
                    "type": "diff",
                    "path": path,
                    "oldText": old_text,
                    "newText": new_text,
                }),
                ToolContent::Terminal { terminal_id } => {
                    json!({ "type": "terminal", "terminalId": terminal_id })
                }
            })
            .collect(),
    )
}

fn locations(items: &[ToolLocation]) -> Value {
    Value::Array(
        items
            .iter()
            .map(|location| {
                let mut object = json!({ "path": location.path });
                if let Some(line) = location.line {
                    object["line"] = json!(line);
                }
                object
            })
            .collect(),
    )
}

fn config_option_value(option: &super::model::ConfigOption) -> Value {
    use super::model::ConfigValue;
    let mut object = json!({ "id": option.id, "name": option.name });
    if let Some(description) = &option.description {
        object["description"] = json!(description);
    }
    if let Some(category) = &option.category {
        object["category"] = json!(category);
    }
    match &option.value {
        ConfigValue::Select {
            current_value,
            options,
        } => {
            object["type"] = json!("select");
            object["currentValue"] = json!(current_value);
            object["options"] = Value::Array(
                options
                    .iter()
                    .map(|choice| {
                        let mut item = json!({ "value": choice.value, "name": choice.name });
                        if let Some(description) = &choice.description {
                            item["description"] = json!(description);
                        }
                        item
                    })
                    .collect(),
            );
        }
        ConfigValue::Boolean { current_value } => {
            object["type"] = json!("boolean");
            object["currentValue"] = json!(current_value);
        }
    }
    object
}

fn kind(kind: ToolKind) -> &'static str {
    match kind {
        ToolKind::Read => "read",
        ToolKind::Edit => "edit",
        ToolKind::Delete => "delete",
        ToolKind::Move => "move",
        ToolKind::Search => "search",
        ToolKind::Execute => "execute",
        ToolKind::Think => "think",
        ToolKind::Fetch => "fetch",
        ToolKind::SwitchMode => "switch_mode",
        ToolKind::Other => "other",
    }
}

fn status(status: ToolStatus) -> &'static str {
    match status {
        ToolStatus::Pending => "pending",
        ToolStatus::InProgress => "in_progress",
        ToolStatus::Completed => "completed",
        ToolStatus::Failed => "failed",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_message_chunk_is_an_agent_message_chunk() {
        let value = to_acp(&AgentEvent::AgentMessageChunk {
            content: Content::text("hello"),
            message_id: Some("m1".to_string()),
        })
        .unwrap();
        assert_eq!(value["sessionUpdate"], "agent_message_chunk");
        assert_eq!(value["content"]["type"], "text");
        assert_eq!(value["content"]["text"], "hello");
        assert_eq!(value["messageId"], "m1");
    }

    /// Keys camelCase, discriminator values snake_case — the one casing rule
    /// that is easy to get half right.
    #[test]
    fn a_tool_call_uses_acps_casing_on_both_sides() {
        let mut call = ToolCall::new("t1", "Edit src/main.rs");
        call.kind = ToolKind::SwitchMode;
        call.status = ToolStatus::InProgress;
        call.raw_input = Some(json!({"a": 1}));
        let value = to_acp(&AgentEvent::ToolCall { call }).unwrap();

        assert_eq!(value["sessionUpdate"], "tool_call");
        assert_eq!(value["toolCallId"], "t1");
        assert_eq!(value["kind"], "switch_mode");
        assert_eq!(value["status"], "in_progress");
        assert_eq!(value["rawInput"], json!({"a": 1}));
    }

    #[test]
    fn a_diff_carries_acps_field_names() {
        let mut call = ToolCall::new("t1", "Write a.rs");
        call.content = vec![ToolContent::Diff {
            path: "/tmp/a.rs".to_string(),
            old_text: None,
            new_text: "fn main() {}".to_string(),
        }];
        let value = to_acp(&AgentEvent::ToolCall { call }).unwrap();
        let diff = &value["content"][0];
        assert_eq!(diff["type"], "diff");
        assert_eq!(diff["path"], "/tmp/a.rs");
        assert_eq!(diff["oldText"], Value::Null);
        assert_eq!(diff["newText"], "fn main() {}");
    }

    /// A patch must omit what it does not change: a null would read as
    /// "cleared" to a client applying it.
    #[test]
    fn an_update_omits_rather_than_nulls() {
        let value = to_acp(&AgentEvent::ToolCallUpdate {
            update: ToolCallUpdate::finished("t1", ToolStatus::Completed),
        })
        .unwrap();
        assert_eq!(value["sessionUpdate"], "tool_call_update");
        assert_eq!(value["status"], "completed");
        assert!(value.get("title").is_none(), "value was: {value}");
        assert!(value.get("kind").is_none(), "value was: {value}");
    }

    #[test]
    fn usage_is_a_used_over_size_ratio() {
        let value = to_acp(&AgentEvent::UsageUpdate {
            used: 100,
            size: 200_000,
            cost: Some(super::super::Cost {
                amount: 0.5,
                currency: "USD".to_string(),
            }),
            model: Some("claude-opus-5".to_string()),
        })
        .unwrap();
        assert_eq!(value["sessionUpdate"], "usage_update");
        assert_eq!(value["used"], 100);
        assert_eq!(value["size"], 200_000);
        assert_eq!(value["cost"]["currency"], "USD");
    }

    /// The three that belong to the protocol level a bridge does not have.
    #[test]
    fn protocol_level_events_are_not_session_updates() {
        assert!(
            to_acp(&AgentEvent::SessionStarted {
                session_id: None,
                model: None,
                mode: None,
                tools: Vec::new(),
                agents: Vec::new(),
            })
            .is_none()
        );
        assert!(
            to_acp(&AgentEvent::TurnEnded {
                stop_reason: StopReason::EndTurn,
                error: None,
            })
            .is_none()
        );
        assert!(
            to_acp(&AgentEvent::PermissionRequest {
                request_id: "r1".to_string(),
                tool_call: ToolCallUpdate::default(),
                options: Vec::new(),
            })
            .is_none()
        );
    }

    #[test]
    fn stop_reasons_map_onto_acps_five() {
        assert_eq!(stop_reason(&StopReason::EndTurn), "end_turn");
        assert_eq!(stop_reason(&StopReason::Cancelled), "cancelled");
        // ACP has no "the run broke", so the nearest true thing is used.
        assert_eq!(stop_reason(&StopReason::Failed), "refusal");
    }
}
