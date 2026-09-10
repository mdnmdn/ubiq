//! Projecting [`AgentEvent`] onto the Agent Client Protocol.
//!
//! Since the neutral model *is* ACP's `session/update` vocabulary (see
//! [`super::model`]), this is a rename and a re-casing rather than a
//! translation: the discriminant moves from `type` to `sessionUpdate`, and
//! keys go from snake_case to camelCase. `_docs/references/acp-protocol.md` is the wire
//! reference; the two rules it turns on are that **ACP keys are camelCase
//! while discriminator values stay snake_case**, and that every union is
//! internally tagged with its payload flattened beside the tag.
//!
//! **This is a projection, not an endpoint.** What comes out is the `params`
//! of a `session/update` notification, without the JSON-RPC envelope and
//! without a `sessionId` — because an event carries no session identity, by
//! design. A server built on this attaches both: it owns the connection and
//! the table of live sessions, and the mapping stays untouched. That is the
//! whole reason the identity is not on the event. The crate does speak ACP
//! for real, as a *client*: [`super::acp_client::AcpBridge`] owns a JSON-RPC
//! connection to an ACP agent and calls [`from_acp`] on every
//! `session/update` it reads.
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
    AgentEvent, CommandInfo, ConfigChoice, ConfigOption, ConfigValue, Content, Cost, Origin,
    PlanEntry, ResourceContents, StopReason, ToolCall, ToolCallUpdate, ToolContent, ToolKind,
    ToolLocation, ToolStatus,
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
            ..
        } => chunk("agent_message_chunk", content, message_id.as_deref()),
        AgentEvent::AgentThoughtChunk {
            content,
            message_id,
            ..
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
        | AgentEvent::Log { .. }
        // Not part of ACP's `session/update` vocabulary at all — Claude Code's own gauge and its
        // own compaction notice, with no upstream equivalent to project onto.
        | AgentEvent::RateLimitUpdate { .. }
        | AgentEvent::Compacted => return None,
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
    if let Some(raw) = &call.raw_output {
        object.insert("rawOutput".to_string(), raw.clone());
    }
    if matches!(call.kind, ToolKind::Delegate) {
        object.insert("_meta".to_string(), json!({ "toolKind": "delegate" }));
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
    if let Some(raw) = &update.raw_input {
        object.insert("rawInput".to_string(), raw.clone());
    }
    if let Some(raw) = &update.raw_output {
        object.insert("rawOutput".to_string(), raw.clone());
    }
    if matches!(update.kind, Some(ToolKind::Delegate)) {
        object.insert("_meta".to_string(), json!({ "toolKind": "delegate" }));
    }
    object
}

/// ACP's `stopReason`, for the `session/prompt` response a server sends when
/// it sees an [`AgentEvent::TurnEnded`].
///
/// ACP has no "the run broke" reason, and [`StopReason::Failed`] is **not**
/// `refusal` even though both mean "no answer arrived": spec section 6 says
/// `refusal` carries a specific contract — the agent *declined*, and "the
/// user prompt and everything after it will not be included in the next
/// prompt". A crash has neither property. The harness did not decline
/// anything, and a client that drops the prompt from context because of a
/// transient failure loses history it should keep for the retry. `end_turn`
/// is the honest fallback: the turn is over, and the client keeps its
/// context. A server that wants to say *why* the turn ended in failure
/// carries that in `_meta` on the `session/prompt` response — this function
/// only produces the `stopReason` enum value, since [`AgentEvent::TurnEnded`]
/// is mapped by the server, not here (see the module docs).
pub fn stop_reason(reason: &StopReason) -> &'static str {
    match reason {
        StopReason::EndTurn | StopReason::Failed => "end_turn",
        StopReason::MaxTokens => "max_tokens",
        StopReason::MaxTurnRequests => "max_turn_requests",
        StopReason::Refusal => "refusal",
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
        // ACP names no delegation kind, so it travels as the fallback rather than as a word the
        // other end would have to guess at — with `_meta.toolKind` beside it (see
        // [`tool_call_value`]) so the collapse is recoverable.
        ToolKind::Delegate => "other",
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

/// Map one ACP `session/update` params value back to an [`AgentEvent`].
///
/// The inverse of [`to_acp`], one `sessionUpdate` discriminant at a time.
/// `params` is not a full JSON-RPC notification: no envelope, and
/// `sessionId` (if present) is ignored, for the same reason [`to_acp`]
/// produces neither — identity belongs to whoever holds the session table,
/// never to the event.
///
/// Four of [`AgentEvent`]'s variants never come back out of here, mirroring
/// the three [`to_acp`] never puts in, plus one it puts in but a bridge
/// never sees on this side of the wire:
///
/// - [`AgentEvent::SessionStarted`], [`AgentEvent::PermissionRequest`] and
///   [`AgentEvent::TurnEnded`] are protocol-level in ACP — the result of
///   `session/new`, a `session/request_permission` request, and a
///   `session/prompt` response — not a `session/update` payload.
/// - [`AgentEvent::Log`], [`AgentEvent::RateLimitUpdate`] and
///   [`AgentEvent::Compacted`] have no ACP vocabulary at all.
///
/// An unrecognised or missing `sessionUpdate` answers `None`, same as any of
/// those four. Everything else degrades rather than fails: a malformed or
/// absent *subfield* falls back to that type's `Default`, so one bad key
/// never drops the whole event.
///
/// The degradation is also where the mapping is inherently lossy, not
/// buggy:
/// - [`Origin`] has no ACP vocabulary, so it is read out of `_meta` (see
///   [`meta_string`]) where the agent stamped one, and comes back
///   [`Origin::default`] where it did not. [`to_acp`] writes none — an event
///   carries no identity on the way out, same as `sessionId`. Attributing an
///   unstamped chunk to the delegate that is open needs state this pure
///   function does not have; that heuristic lives in
///   [`super::acp_client`].
/// - `UsageUpdate`'s `model` and `spend` are dropped by [`to_acp`], so they
///   come back `None` here too; `cost` comes back exactly as sent, which for
///   a real agent is the session's **cumulative** figure rather than the
///   per-report delta [`AgentEvent::UsageUpdate`] contracts for — the
///   subtraction also needs state, and also lives in
///   [`super::acp_client`].
/// - [`super::model::ConfigChoice::group`] is never written, so it comes
///   back `None`.
/// - [`ToolKind::Delegate`] travels as `kind: "other"` (ACP names no
///   delegation kind) with `_meta.toolKind` beside it, and un-collapses from
///   that marker; [`StopReason::Failed`] is not reachable from here at all,
///   since [`AgentEvent::TurnEnded`] is not a session update.
pub fn from_acp(params: &Value) -> Option<AgentEvent> {
    let update = params.get("sessionUpdate").and_then(Value::as_str)?;
    match update {
        "user_message_chunk" => Some(AgentEvent::UserMessageChunk {
            content: from_content(params.get("content")?),
            message_id: str_field(params, "messageId"),
        }),
        "agent_message_chunk" => Some(AgentEvent::AgentMessageChunk {
            content: from_content(params.get("content")?),
            message_id: str_field(params, "messageId"),
            origin: origin_from_meta(params),
        }),
        "agent_thought_chunk" => Some(AgentEvent::AgentThoughtChunk {
            content: from_content(params.get("content")?),
            message_id: str_field(params, "messageId"),
            origin: origin_from_meta(params),
        }),

        "tool_call" => Some(AgentEvent::ToolCall {
            call: from_tool_call(params),
        }),
        "tool_call_update" => Some(AgentEvent::ToolCallUpdate {
            update: from_tool_call_update(params),
        }),

        "plan" => Some(AgentEvent::Plan {
            entries: params
                .get("entries")
                .and_then(Value::as_array)
                .map(|entries| entries.iter().map(from_plan_entry).collect())
                .unwrap_or_default(),
        }),

        "available_commands_update" => Some(AgentEvent::AvailableCommandsUpdate {
            commands: params
                .get("availableCommands")
                .and_then(Value::as_array)
                .map(|commands| commands.iter().map(from_command).collect())
                .unwrap_or_default(),
        }),

        "current_mode_update" => Some(AgentEvent::CurrentModeUpdate {
            // Upstream's prose example says `modeId`; the schema — what
            // `to_acp` writes — says `currentModeId`. Accept both.
            current_mode_id: str_field(params, "currentModeId")
                .or_else(|| str_field(params, "modeId"))
                .unwrap_or_default(),
        }),

        "config_option_update" => Some(AgentEvent::ConfigOptionUpdate {
            options: params
                .get("configOptions")
                .and_then(Value::as_array)
                .map(|options| options.iter().map(from_config_option).collect())
                .unwrap_or_default(),
        }),

        "session_info_update" => Some(AgentEvent::SessionInfoUpdate {
            title: str_field(params, "title"),
            updated_at: str_field(params, "updatedAt"),
        }),

        "usage_update" => Some(AgentEvent::UsageUpdate {
            used: params
                .get("used")
                .and_then(Value::as_u64)
                .unwrap_or_default(),
            size: params
                .get("size")
                .and_then(Value::as_u64)
                .unwrap_or_default(),
            cost: params.get("cost").map(from_cost),
            model: None,
            spend: None,
            origin: origin_from_meta(params),
        }),

        _ => None,
    }
}

/// A `_meta` string under any of `keys`, flat or nested one vendor object deep.
///
/// ACP v1 names neither a subagent nor a delegation tool kind, so `_meta` —
/// present on the `session/update` notification *and* on every update payload
/// — is the only legal carrier for either. Both shapes are read: flat
/// (`_meta.parentToolCallId`) and namespaced under one vendor object
/// (`_meta.claudeCode.toolName`, which is what
/// `@zed-industries/claude-code-acp` actually emits). Flat wins.
pub(crate) fn meta_string(value: &Value, keys: &[&str]) -> Option<String> {
    let meta = value.get("_meta")?;
    let pick = |m: &Value| keys.iter().find_map(|key| str_field(m, key));
    pick(meta).or_else(|| meta.as_object()?.values().find_map(pick))
}

/// The [`Origin`] an agent stamped into an update's `_meta`, if any.
pub(crate) fn origin_from_meta(value: &Value) -> Origin {
    Origin {
        parent_tool_use_id: meta_string(value, &["parentToolCallId", "parentToolUseId"]),
        subagent_type: meta_string(value, &["subagentType", "subagent"]),
        model: meta_string(value, &["model"]),
        thinking: meta_string(value, &["thinking"]),
    }
}

/// A JSON string field, or `None` if it is absent, null, or not a string.
fn str_field(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(String::from)
}

fn from_content(value: &Value) -> Content {
    match value.get("type").and_then(Value::as_str) {
        Some("image") => Content::Image {
            data: str_field(value, "data").unwrap_or_default(),
            mime_type: str_field(value, "mimeType").unwrap_or_default(),
            uri: str_field(value, "uri"),
        },
        Some("audio") => Content::Audio {
            data: str_field(value, "data").unwrap_or_default(),
            mime_type: str_field(value, "mimeType").unwrap_or_default(),
        },
        Some("resource_link") => Content::ResourceLink {
            uri: str_field(value, "uri").unwrap_or_default(),
            name: str_field(value, "name").unwrap_or_default(),
            mime_type: str_field(value, "mimeType"),
            title: str_field(value, "title"),
            description: str_field(value, "description"),
            size: value.get("size").and_then(Value::as_u64),
        },
        Some("resource") => Content::Resource {
            resource: value
                .get("resource")
                .map(from_resource)
                .unwrap_or(ResourceContents::Text {
                    uri: String::new(),
                    text: String::new(),
                    mime_type: None,
                }),
        },
        // "text", and the fallback for anything unrecognised.
        _ => Content::Text {
            text: str_field(value, "text").unwrap_or_default(),
        },
    }
}

fn from_resource(value: &Value) -> ResourceContents {
    let uri = str_field(value, "uri").unwrap_or_default();
    let mime_type = str_field(value, "mimeType");
    // Untagged on the way out: discriminate on which body key is present.
    match str_field(value, "text") {
        Some(text) => ResourceContents::Text {
            uri,
            text,
            mime_type,
        },
        None => ResourceContents::Blob {
            uri,
            blob: str_field(value, "blob").unwrap_or_default(),
            mime_type,
        },
    }
}

fn from_contents(value: &Value) -> Vec<ToolContent> {
    value
        .as_array()
        .map(|items| items.iter().map(from_tool_content).collect())
        .unwrap_or_default()
}

fn from_tool_content(item: &Value) -> ToolContent {
    match item.get("type").and_then(Value::as_str) {
        Some("diff") => ToolContent::Diff {
            path: str_field(item, "path").unwrap_or_default(),
            // `to_acp` writes a literal `null` for "no previous text", never
            // omits the key; treat null and absent alike.
            old_text: item
                .get("oldText")
                .and_then(Value::as_str)
                .map(String::from),
            new_text: str_field(item, "newText").unwrap_or_default(),
        },
        Some("terminal") => ToolContent::Terminal {
            terminal_id: str_field(item, "terminalId").unwrap_or_default(),
        },
        // "content", and the fallback for anything unrecognised.
        _ => ToolContent::Content {
            content: item
                .get("content")
                .map(from_content)
                .unwrap_or(Content::Text {
                    text: String::new(),
                }),
        },
    }
}

fn from_locations(value: &Value) -> Vec<ToolLocation> {
    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .map(|item| ToolLocation {
                    path: str_field(item, "path").unwrap_or_default(),
                    line: item.get("line").and_then(Value::as_u64).map(|n| n as u32),
                })
                .collect()
        })
        .unwrap_or_default()
}

fn from_tool_call(value: &Value) -> ToolCall {
    ToolCall {
        id: str_field(value, "toolCallId").unwrap_or_default(),
        title: str_field(value, "title").unwrap_or_default(),
        kind: if is_delegate_meta(value) {
            ToolKind::Delegate
        } else {
            value
                .get("kind")
                .and_then(Value::as_str)
                .map(from_kind)
                .unwrap_or_default()
        },
        status: value
            .get("status")
            .and_then(Value::as_str)
            .map(from_status)
            .unwrap_or_default(),
        content: value.get("content").map(from_contents).unwrap_or_default(),
        locations: value
            .get("locations")
            .map(from_locations)
            .unwrap_or_default(),
        raw_input: value.get("rawInput").cloned(),
        raw_output: value.get("rawOutput").cloned(),
        origin: origin_from_meta(value),
    }
}

fn from_tool_call_update(value: &Value) -> ToolCallUpdate {
    ToolCallUpdate {
        id: str_field(value, "toolCallId").unwrap_or_default(),
        title: str_field(value, "title"),
        kind: if is_delegate_meta(value) {
            Some(ToolKind::Delegate)
        } else {
            value.get("kind").and_then(Value::as_str).map(from_kind)
        },
        status: value.get("status").and_then(Value::as_str).map(from_status),
        content: value.get("content").map(from_contents),
        locations: value.get("locations").map(from_locations),
        raw_input: value.get("rawInput").cloned(),
        raw_output: value.get("rawOutput").cloned(),
    }
}

fn from_plan_entry(value: &Value) -> PlanEntry {
    PlanEntry {
        content: str_field(value, "content").unwrap_or_default(),
        priority: value
            .get("priority")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default(),
        status: value
            .get("status")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default(),
    }
}

fn from_command(value: &Value) -> CommandInfo {
    CommandInfo {
        name: str_field(value, "name").unwrap_or_default(),
        description: str_field(value, "description").unwrap_or_default(),
        input_hint: value
            .get("input")
            .and_then(|input| input.get("hint"))
            .and_then(Value::as_str)
            .map(String::from),
    }
}

/// One ACP `configOptions` entry. `pub(crate)` because
/// [`super::acp_client`] synthesises the same shape from an agent's vendor
/// blocks and must not grow a second copy of this mapping.
pub(crate) fn from_config_option(value: &Value) -> ConfigOption {
    ConfigOption {
        id: str_field(value, "id").unwrap_or_default(),
        name: str_field(value, "name").unwrap_or_default(),
        description: str_field(value, "description"),
        category: value
            .get("category")
            .and_then(|v| serde_json::from_value(v.clone()).ok()),
        value: match value.get("type").and_then(Value::as_str) {
            Some("boolean") => ConfigValue::Boolean {
                current_value: value
                    .get("currentValue")
                    .and_then(Value::as_bool)
                    .unwrap_or_default(),
            },
            // "select", and the fallback for anything unrecognised.
            _ => ConfigValue::Select {
                current_value: str_field(value, "currentValue").unwrap_or_default(),
                options: value
                    .get("options")
                    .and_then(Value::as_array)
                    .map(|items| items.iter().map(from_config_choice).collect())
                    .unwrap_or_default(),
            },
        },
    }
}

fn from_config_choice(value: &Value) -> ConfigChoice {
    ConfigChoice {
        value: str_field(value, "value").unwrap_or_default(),
        name: str_field(value, "name").unwrap_or_default(),
        description: str_field(value, "description"),
        // `to_acp` never writes a group, so there is nothing to read back.
        group: None,
    }
}

fn from_cost(value: &Value) -> Cost {
    Cost {
        amount: value
            .get("amount")
            .and_then(Value::as_f64)
            .unwrap_or_default(),
        currency: str_field(value, "currency").unwrap_or_default(),
    }
}

fn from_kind(value: &str) -> ToolKind {
    match value {
        "read" => ToolKind::Read,
        "edit" => ToolKind::Edit,
        "delete" => ToolKind::Delete,
        "move" => ToolKind::Move,
        "search" => ToolKind::Search,
        "execute" => ToolKind::Execute,
        "think" => ToolKind::Think,
        "fetch" => ToolKind::Fetch,
        "switch_mode" => ToolKind::SwitchMode,
        // "other", and the fallback for anything unrecognised. The "other"
        // that `ToolKind::Delegate` collapses to on the way out un-collapses
        // from `_meta.toolKind`, before this is reached.
        _ => ToolKind::Other,
    }
}

/// Whether `_meta` marks this tool call as a delegation — the marker
/// [`tool_call_value`] writes beside `kind: "other"`, since ACP names no
/// delegation kind of its own.
fn is_delegate_meta(value: &Value) -> bool {
    meta_string(value, &["toolKind"]).as_deref() == Some("delegate")
}

fn from_status(value: &str) -> ToolStatus {
    match value {
        "pending" => ToolStatus::Pending,
        "in_progress" => ToolStatus::InProgress,
        "completed" => ToolStatus::Completed,
        "failed" => ToolStatus::Failed,
        _ => ToolStatus::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::super::model::Origin;
    use super::*;

    #[test]
    fn a_message_chunk_is_an_agent_message_chunk() {
        let value = to_acp(&AgentEvent::AgentMessageChunk {
            origin: Origin::default(),
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
            spend: None,
            origin: Origin::default(),
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
        assert_eq!(stop_reason(&StopReason::MaxTokens), "max_tokens");
        assert_eq!(
            stop_reason(&StopReason::MaxTurnRequests),
            "max_turn_requests"
        );
        assert_eq!(stop_reason(&StopReason::Refusal), "refusal");
        assert_eq!(stop_reason(&StopReason::Cancelled), "cancelled");
        // ACP has no "the run broke", and `refusal` is the wrong stand-in: it promises the
        // client may drop the prompt from context, which a crash has not earned. `end_turn`
        // keeps that context intact.
        assert_eq!(stop_reason(&StopReason::Failed), "end_turn");
    }

    // ── from_acp ───────────────────────────────────────────────────────

    use super::super::model::{ConfigCategory, PlanPriority, PlanStatus};

    #[test]
    fn a_user_message_chunk_round_trips() {
        let ev = AgentEvent::UserMessageChunk {
            content: Content::text("hi"),
            message_id: Some("m1".to_string()),
        };
        let value = to_acp(&ev).unwrap();
        assert_eq!(from_acp(&value), Some(ev));
    }

    #[test]
    fn an_agent_message_chunk_round_trips() {
        let ev = AgentEvent::AgentMessageChunk {
            content: Content::text("hello"),
            message_id: Some("m1".to_string()),
            origin: Origin::default(),
        };
        let value = to_acp(&ev).unwrap();
        assert_eq!(from_acp(&value), Some(ev));
    }

    #[test]
    fn an_agent_thought_chunk_round_trips() {
        let ev = AgentEvent::AgentThoughtChunk {
            content: Content::text("thinking..."),
            message_id: None,
            origin: Origin::default(),
        };
        let value = to_acp(&ev).unwrap();
        assert_eq!(from_acp(&value), Some(ev));
    }

    #[test]
    fn a_tool_call_round_trips() {
        let mut call = ToolCall::new("t1", "Edit src/main.rs");
        call.kind = ToolKind::SwitchMode;
        call.status = ToolStatus::InProgress;
        call.content = vec![ToolContent::Diff {
            path: "/tmp/a.rs".to_string(),
            old_text: Some("old".to_string()),
            new_text: "new".to_string(),
        }];
        call.locations = vec![ToolLocation {
            path: "/tmp/a.rs".to_string(),
            line: Some(3),
        }];
        call.raw_input = Some(json!({"a": 1}));
        call.raw_output = Some(json!({"b": 2}));
        let ev = AgentEvent::ToolCall { call };
        let value = to_acp(&ev).unwrap();
        assert_eq!(from_acp(&value), Some(ev));
    }

    #[test]
    fn a_tool_call_update_round_trips() {
        let mut update = ToolCallUpdate::finished("t1", ToolStatus::Completed);
        update.title = Some("Done".to_string());
        update.kind = Some(ToolKind::Execute);
        update.content = Some(vec![ToolContent::Terminal {
            terminal_id: "term1".to_string(),
        }]);
        update.locations = Some(vec![ToolLocation {
            path: "/tmp/b.rs".to_string(),
            line: None,
        }]);
        update.raw_input = Some(json!({"a": 1}));
        update.raw_output = Some(json!({"b": 2}));
        let ev = AgentEvent::ToolCallUpdate { update };
        let value = to_acp(&ev).unwrap();
        assert_eq!(from_acp(&value), Some(ev));
    }

    /// [`ToolKind::Delegate`] travels as `kind: "other"` — ACP names no
    /// delegation kind — with `_meta.toolKind` beside it, so it survives the
    /// round trip the UI's subagent switcher depends on.
    #[test]
    fn a_delegate_tool_call_round_trips_through_meta() {
        let mut call = ToolCall::new("t1", "Explore the tree");
        call.kind = ToolKind::Delegate;
        let value = to_acp(&AgentEvent::ToolCall { call: call.clone() }).unwrap();
        assert_eq!(value["kind"], "other");
        assert_eq!(value["_meta"]["toolKind"], "delegate");
        assert_eq!(
            from_acp(&value),
            Some(AgentEvent::ToolCall { call: call.clone() })
        );

        let update = ToolCallUpdate {
            kind: Some(ToolKind::Delegate),
            ..ToolCallUpdate::finished("t1", ToolStatus::Completed)
        };
        let value = to_acp(&AgentEvent::ToolCallUpdate {
            update: update.clone(),
        })
        .unwrap();
        assert_eq!(value["_meta"]["toolKind"], "delegate");
        assert_eq!(
            from_acp(&value),
            Some(AgentEvent::ToolCallUpdate { update })
        );
    }

    /// ACP v1 has no subagent vocabulary, so `_meta` is where an agent states
    /// an [`Origin`] — flat, or namespaced under one vendor object, which is
    /// what `@zed-industries/claude-code-acp` does with everything it adds.
    #[test]
    fn a_meta_marked_chunk_carries_an_origin() {
        let flat = from_acp(&json!({
            "sessionUpdate": "agent_message_chunk",
            "content": {"type": "text", "text": "hi"},
            "_meta": {"parentToolCallId": "t1", "subagentType": "explorer"},
        }));
        let Some(AgentEvent::AgentMessageChunk { origin, .. }) = flat else {
            panic!("expected a chunk");
        };
        assert_eq!(origin.parent_tool_use_id.as_deref(), Some("t1"));
        assert_eq!(origin.subagent_type.as_deref(), Some("explorer"));

        let namespaced = from_acp(&json!({
            "sessionUpdate": "agent_thought_chunk",
            "content": {"type": "text", "text": "hmm"},
            "_meta": {"claudeCode": {"parentToolUseId": "t2", "model": "claude-haiku-4-5"}},
        }));
        let Some(AgentEvent::AgentThoughtChunk { origin, .. }) = namespaced else {
            panic!("expected a thought");
        };
        assert_eq!(origin.parent_tool_use_id.as_deref(), Some("t2"));
        assert_eq!(origin.model.as_deref(), Some("claude-haiku-4-5"));

        // Usage and tool calls read the same marker.
        let usage = from_acp(&json!({
            "sessionUpdate": "usage_update",
            "used": 1, "size": 2,
            "_meta": {"parentToolCallId": "t3"},
        }));
        let Some(AgentEvent::UsageUpdate { origin, .. }) = usage else {
            panic!("expected usage");
        };
        assert_eq!(origin.parent_tool_use_id.as_deref(), Some("t3"));

        // And an unmarked update stays the conversation's own.
        let bare = from_acp(&json!({
            "sessionUpdate": "agent_message_chunk",
            "content": {"type": "text", "text": "hi"},
        }));
        let Some(AgentEvent::AgentMessageChunk { origin, .. }) = bare else {
            panic!("expected a chunk");
        };
        assert_eq!(origin, Origin::default());
    }

    #[test]
    fn a_plan_round_trips() {
        let ev = AgentEvent::Plan {
            entries: vec![
                PlanEntry {
                    content: "step one".to_string(),
                    priority: PlanPriority::High,
                    status: PlanStatus::InProgress,
                },
                PlanEntry {
                    content: "step two".to_string(),
                    priority: PlanPriority::Low,
                    status: PlanStatus::Pending,
                },
            ],
        };
        let value = to_acp(&ev).unwrap();
        assert_eq!(from_acp(&value), Some(ev));
    }

    #[test]
    fn available_commands_round_trip() {
        let ev = AgentEvent::AvailableCommandsUpdate {
            commands: vec![
                CommandInfo {
                    name: "review".to_string(),
                    description: "Review the diff".to_string(),
                    input_hint: Some("[target]".to_string()),
                },
                CommandInfo {
                    name: "help".to_string(),
                    description: String::new(),
                    input_hint: None,
                },
            ],
        };
        let value = to_acp(&ev).unwrap();
        assert_eq!(from_acp(&value), Some(ev));
    }

    #[test]
    fn current_mode_round_trips() {
        let ev = AgentEvent::CurrentModeUpdate {
            current_mode_id: "plan".to_string(),
        };
        let value = to_acp(&ev).unwrap();
        assert_eq!(from_acp(&value), Some(ev));
    }

    #[test]
    fn config_option_update_round_trips() {
        let ev = AgentEvent::ConfigOptionUpdate {
            options: vec![
                ConfigOption {
                    id: "model".to_string(),
                    name: "Model".to_string(),
                    description: Some("Which model to use".to_string()),
                    category: Some(ConfigCategory::Model),
                    value: ConfigValue::Select {
                        current_value: "opus".to_string(),
                        options: vec![ConfigChoice {
                            value: "opus".to_string(),
                            name: "Opus".to_string(),
                            description: Some("The big one".to_string()),
                            group: None,
                        }],
                    },
                },
                ConfigOption {
                    id: "thinking".to_string(),
                    name: "Thinking".to_string(),
                    description: None,
                    category: None,
                    value: ConfigValue::Boolean {
                        current_value: true,
                    },
                },
            ],
        };
        let value = to_acp(&ev).unwrap();
        assert_eq!(from_acp(&value), Some(ev));
    }

    #[test]
    fn session_info_update_round_trips() {
        let ev = AgentEvent::SessionInfoUpdate {
            title: Some("A conversation".to_string()),
            updated_at: Some("2026-09-09T00:00:00Z".to_string()),
        };
        let value = to_acp(&ev).unwrap();
        assert_eq!(from_acp(&value), Some(ev));
    }

    #[test]
    fn usage_update_round_trips_what_to_acp_keeps() {
        let value = to_acp(&AgentEvent::UsageUpdate {
            used: 100,
            size: 200_000,
            cost: Some(Cost {
                amount: 0.5,
                currency: "USD".to_string(),
            }),
            model: Some("claude-opus-5".to_string()),
            spend: None,
            origin: Origin::default(),
        })
        .unwrap();
        // `model` and `spend` never made it onto the wire, so they cannot
        // come back — this is the lossy point the doc comment names.
        assert_eq!(
            from_acp(&value),
            Some(AgentEvent::UsageUpdate {
                used: 100,
                size: 200_000,
                cost: Some(Cost {
                    amount: 0.5,
                    currency: "USD".to_string(),
                }),
                model: None,
                spend: None,
                origin: Origin::default(),
            })
        );
    }

    #[test]
    fn from_acp_answers_none_for_an_unknown_session_update() {
        assert_eq!(from_acp(&json!({"sessionUpdate": "something_new"})), None);
    }

    #[test]
    fn from_acp_answers_none_for_a_missing_session_update() {
        assert_eq!(
            from_acp(&json!({"content": {"type": "text", "text": "hi"}})),
            None
        );
    }

    /// A patch's absent fields are "unchanged", not "reset to default" — so
    /// they must come back `None`, never `Some(Default::default())`.
    #[test]
    fn an_update_with_omitted_keys_comes_back_none_not_default() {
        let ev = from_acp(&json!({
            "sessionUpdate": "tool_call_update",
            "toolCallId": "t1",
            "status": "completed",
        }))
        .unwrap();
        let AgentEvent::ToolCallUpdate { update } = ev else {
            panic!("expected a ToolCallUpdate, got {ev:?}")
        };
        assert_eq!(update.id, "t1");
        assert_eq!(update.status, Some(ToolStatus::Completed));
        assert_eq!(update.title, None);
        assert_eq!(update.kind, None);
        assert_eq!(update.content, None);
        assert_eq!(update.locations, None);
    }

    /// `to_acp` writes a literal `oldText: null` for "the file is being
    /// created" rather than omitting the key; `from_acp` must read that the
    /// same as an absent key.
    #[test]
    fn a_null_old_text_comes_back_none() {
        let ev = from_acp(&json!({
            "sessionUpdate": "tool_call",
            "toolCallId": "t1",
            "title": "Write a.rs",
            "kind": "edit",
            "status": "pending",
            "content": [{
                "type": "diff",
                "path": "/tmp/a.rs",
                "oldText": null,
                "newText": "fn main() {}",
            }],
        }))
        .unwrap();
        let AgentEvent::ToolCall { call } = ev else {
            panic!("expected a ToolCall, got {ev:?}")
        };
        let ToolContent::Diff { old_text, .. } = &call.content[0] else {
            panic!("expected a Diff, got {:?}", call.content[0]);
        };
        assert_eq!(*old_text, None);
    }

    /// Upstream's prose example says `modeId`; the schema `to_acp` writes
    /// says `currentModeId`. `from_acp` accepts either.
    #[test]
    fn mode_id_is_accepted_as_a_fallback_for_current_mode_id() {
        let ev = from_acp(&json!({
            "sessionUpdate": "current_mode_update",
            "modeId": "plan",
        }))
        .unwrap();
        assert_eq!(
            ev,
            AgentEvent::CurrentModeUpdate {
                current_mode_id: "plan".to_string(),
            }
        );
    }

    /// An enum string neither side recognises degrades to that type's
    /// default rather than dropping the whole event.
    #[test]
    fn an_unknown_enum_string_degrades_to_default_rather_than_dropping_the_event() {
        let ev = from_acp(&json!({
            "sessionUpdate": "tool_call",
            "toolCallId": "t1",
            "title": "Do a thing",
            "kind": "teleport",
            "status": "vibing",
        }))
        .unwrap();
        let AgentEvent::ToolCall { call } = ev else {
            panic!("expected a ToolCall, got {ev:?}")
        };
        assert_eq!(call.kind, ToolKind::Other);
        assert_eq!(call.status, ToolStatus::Pending);
    }
}
