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
use agent_manager::io::{spawn_piped, AgentEvent, AgentInput, IoBridge, JsonlBridge};

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
    }
}

#[test]
fn jsonl_bridge_round_trips_events_and_terminates() {
    let cwd = std::env::current_dir().unwrap();
    let child = spawn_piped(&launch(), &cwd).expect("spawn fake harness");
    let mut bridge = JsonlBridge::new(child).expect("build bridge");

    bridge
        .send(AgentInput::Prompt {
            text: "say hi".to_string(),
        })
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
                session_id: Some(id)
            } if id == "fake-session-1"
        )),
        "expected a SessionStarted event with the session id, got: {events:?}"
    );
    assert!(
        events.iter().any(|e| matches!(
            e,
            AgentEvent::AssistantText { text } if text == "hello from fake claude"
        )),
        "expected an AssistantText event, got: {events:?}"
    );
    assert!(
        events.iter().any(|e| matches!(
            e,
            AgentEvent::ApprovalRequest { request_id, tool_name, .. }
                if request_id == "req-1" && tool_name == "Bash"
        )),
        "expected an ApprovalRequest event, got: {events:?}"
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, AgentEvent::ToolResult { id: Some(id), .. } if id == "tool-1")),
        "expected a ToolResult event, got: {events:?}"
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, AgentEvent::Result { success: true, .. })),
        "expected a terminal Result{{success:true}} event, got: {events:?}"
    );
    assert!(
        events.iter().any(|e| matches!(
            e,
            AgentEvent::Usage {
                input_tokens: Some(5),
                output_tokens: Some(7),
            }
        )),
        "expected a Usage event summed from modelUsage, got: {events:?}"
    );
}
