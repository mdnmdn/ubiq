//! Integration test for `agent_manager::io::JsonlBridge` against a
//! committed fake stream-json harness (`tests/fake-claude-streamjson.sh`) —
//! no real `claude` binary or network access needed.
//!
//! `io::jsonl` is core (no feature gate — see `src/io/jsonl.rs`), so this
//! runs under the default build same as `tests/passthrough.rs`'s `pty`
//! sibling; unlike that one, it needs no `#![cfg(feature = ...)]` guard.
//!
//! Exercises the full round trip: send a prompt, drain events, and confirm
//! (a) the auto-allow path answers the fake harness's `control_request`
//! without any consumer answering it — the fake script's second `read`
//! would block forever otherwise, which is exactly what would make this
//! test hang — and (b) the run terminates (the event channel closes,
//! `next_event` returns `None`) rather than hanging.

use std::path::PathBuf;

use agent_manager::harness::Launch;
use agent_manager::io::{
    AgentEvent, AgentInput, IoBridge, JsonlBridge, StopReason, ToolKind, ToolStatus, spawn_piped,
};

/// Absolute path to the fake stream-json harness script next to this test file.
fn fake_harness_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fake-claude-streamjson.sh")
}

fn launch() -> Launch {
    Launch {
        program: fake_harness_path().to_string_lossy().to_string(),
        args: Vec::new(),
        env: Vec::new(),
        env_remove: Vec::new(),
        env_clear: false,
    }
}

#[test]
fn jsonl_bridge_round_trips_events_and_terminates() {
    let cwd = std::env::current_dir().unwrap();
    let child = spawn_piped(&launch(), &cwd).expect("spawn fake harness");
    let mut bridge = JsonlBridge::new(child).expect("build bridge");

    bridge
        .send(AgentInput::prompt("say hi"))
        .expect("send prompt");

    // Drain every event; the fake script exits after the terminal `result`
    // line, which closes the channel and ends this loop. If the bridge
    // failed to auto-allow the `control_request`, the fake script would
    // block forever on its second `read` and this loop would never return —
    // that's the behavior this test is really pinning down.
    let mut events = Vec::new();
    while let Some(ev) = bridge.next_event().expect("next_event") {
        events.push(ev);
    }

    assert!(
        events.iter().any(|e| matches!(
            e,
            AgentEvent::SessionStarted {
                session_id: Some(id),
                ..
            } if id == "fake-session-1"
        )),
        "expected a SessionStarted event with the session id, got: {events:?}"
    );
    assert!(
        events.iter().any(|e| matches!(
            e,
            AgentEvent::AgentMessageChunk { content, .. }
                if content.as_text() == Some("hello from fake claude")
        )),
        "expected an AgentMessageChunk event, got: {events:?}"
    );

    // The permission ask now carries the whole tool call it's asking about,
    // not just its name — a dialog can show what it is authorising without a
    // second lookup.
    let permission_request = events.iter().find_map(|e| match e {
        AgentEvent::PermissionRequest {
            request_id,
            tool_call,
            options,
        } if request_id == "req-1" => Some((tool_call, options)),
        _ => None,
    });
    let (tool_call, options) = permission_request
        .unwrap_or_else(|| panic!("expected a PermissionRequest event, got: {events:?}"));
    assert_eq!(tool_call.id, "tool-1");
    assert_eq!(tool_call.kind, Some(ToolKind::Execute));
    assert_eq!(tool_call.status, Some(ToolStatus::Pending));
    assert!(
        !options.is_empty(),
        "expected at least one PermissionOption, got: {options:?}"
    );

    // The tool_result line updates that same call to Completed and carries
    // its output text — a patch, not a fresh call.
    let tool_update = events.iter().find_map(|e| match e {
        AgentEvent::ToolCallUpdate { update } if update.id == "tool-1" => Some(update),
        _ => None,
    });
    let tool_update =
        tool_update.unwrap_or_else(|| panic!("expected a ToolCallUpdate event, got: {events:?}"));
    assert_eq!(tool_update.status, Some(ToolStatus::Completed));

    assert!(
        events.iter().any(|e| matches!(
            e,
            AgentEvent::TurnEnded {
                stop_reason: StopReason::EndTurn,
                error: None,
            }
        )),
        "expected a terminal TurnEnded{{stop_reason: EndTurn}} event, got: {events:?}"
    );

    // `result.modelUsage` reports camelCase fields (`inputTokens`,
    // `cacheReadInputTokens`, `contextWindow`, ...); `used` sums the tokens
    // that count against the window and `size` is the window itself.
    assert!(
        events.iter().any(|e| matches!(
            e,
            AgentEvent::UsageUpdate {
                used: 5,
                size: 200_000,
                model: Some(model),
                ..
            } if model == "fake-model"
        )),
        "expected a UsageUpdate event summed from modelUsage, got: {events:?}"
    );
}
