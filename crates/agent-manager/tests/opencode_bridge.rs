//! Integration test for opencode's structured bridge.
//!
//! Tests that [`agent_manager::io::OpencodeBridge`] correctly parses
//! opencode's NDJSON event stream and translates it to normalized
//! [`AgentEvent`]s.

use agent_manager::harness::Launch;
use agent_manager::io::{
    AgentEvent, Content, IoBridge, OpencodeBridge, StopReason, ToolContent, ToolKind, ToolStatus,
    spawn_piped,
};
use std::path::PathBuf;
use std::time::Duration;

/// Absolute path to the fake opencode run script next to this test file.
fn fake_harness_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fake-opencode-run.sh")
}

#[test]
fn opencode_bridge_drains_fake_stream_to_completion() {
    // This test runs a fake opencode process (shell script that emits NDJSON)
    // and verifies the bridge reads all events in order and terminates.
    let script_path = fake_harness_path();

    assert!(
        script_path.exists(),
        "fake opencode script not found at {}",
        script_path.display()
    );

    let launch = Launch {
        program: script_path.to_string_lossy().to_string(),
        args: vec![],
        env: vec![],
        env_remove: vec![],
        env_clear: false,
    };

    let cwd = std::env::current_dir().expect("could not get current dir");
    let child = spawn_piped(&launch, &cwd).expect("spawn_piped failed");
    let mut bridge = OpencodeBridge::new(child).expect("OpencodeBridge::new failed");

    // Collect all events.
    let mut events = Vec::new();
    let start = std::time::Instant::now();
    loop {
        match bridge.next_event() {
            Ok(Some(ev)) => events.push(ev),
            Ok(None) => break,
            Err(e) => panic!("next_event error: {e}"),
        }
        // Guard against hanging: if we haven't finished after 30s, something is wrong.
        if start.elapsed() > Duration::from_secs(30) {
            panic!("opencode bridge test timed out");
        }
    }

    // Verify we got the expected events in order.
    assert!(
        !events.is_empty(),
        "expected events but got none (bridge hung?)"
    );

    // We expect 5 events: step_start, text, tool_use (call+update), and the
    // terminal TurnEnded from EOF. step_finish's tokens carry no context
    // window, so it contributes no UsageUpdate — see `io/opencode.rs`.
    assert_eq!(events.len(), 5, "expected 5 events, got {events:#?}");

    match &events[0] {
        AgentEvent::SessionStarted {
            session_id: Some(sid),
            model: None,
            mode: None,
            tools,
            agents,
        } => {
            assert_eq!(sid, "fake-sess-123");
            assert!(tools.is_empty());
            assert!(agents.is_empty());
        }
        other => panic!("event 0: expected SessionStarted, got {other:?}"),
    }

    // Event 1: AgentMessageChunk.
    match &events[1] {
        AgentEvent::AgentMessageChunk { content, .. } => {
            assert_eq!(content, &Content::text("hello from fake opencode"));
        }
        other => panic!("event 1: expected AgentMessageChunk, got {other:?}"),
    }

    // Event 2: ToolCall, with the kind and status the new vocabulary carries.
    match &events[2] {
        AgentEvent::ToolCall { call } => {
            assert_eq!(call.id, "call-1");
            assert_eq!(call.title, "bash echo test");
            assert_eq!(call.kind, ToolKind::Execute);
            assert_eq!(call.status, ToolStatus::InProgress);
        }
        other => panic!("event 2: expected ToolCall, got {other:?}"),
    }

    // Event 3: ToolCallUpdate completing that same call.
    match &events[3] {
        AgentEvent::ToolCallUpdate { update } => {
            assert_eq!(update.id, "call-1");
            assert_eq!(update.status, Some(ToolStatus::Completed));
            assert_eq!(
                update.content,
                Some(vec![ToolContent::Content {
                    content: Content::text("test"),
                }])
            );
        }
        other => panic!("event 3: expected ToolCallUpdate, got {other:?}"),
    }

    // Event 4: the terminal TurnEnded, emitted at EOF since the stream never
    // sent one of its own (no `error` line).
    match &events[4] {
        AgentEvent::TurnEnded {
            stop_reason: StopReason::EndTurn,
            error: None,
        } => {
            // Expected: successful completion at stream end.
        }
        other => panic!("event 4: expected TurnEnded{{EndTurn}}, got {other:?}"),
    }

    // Verify the bridge terminated cleanly (no more events and no error).
    match bridge.next_event() {
        Ok(None) => {
            // Expected: stream closed.
        }
        other => panic!("expected stream end, got {other:?}"),
    }
}
