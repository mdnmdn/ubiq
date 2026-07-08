//! Integration test for opencode's structured bridge.
//!
//! Tests that [`agent_manager::io::OpencodeBridge`] correctly parses
//! opencode's NDJSON event stream and translates it to normalized
//! [`AgentEvent`]s.

use agent_manager::io::{spawn_piped, AgentEvent, IoBridge, OpencodeBridge};
use agent_manager::harness::Launch;
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

    // Event 0: SessionStarted with session id from step_start.
    // We expect 6 events: step_start, text, tool_use (call+result), step_finish (usage), and terminal Result (from EOF).
    assert_eq!(events.len(), 6, "expected 6 events, got {}", events.len());

    match &events[0] {
        AgentEvent::SessionStarted {
            session_id: Some(sid),
        } => assert_eq!(sid, "fake-sess-123"),
        other => panic!("event 0: expected SessionStarted, got {:?}", other),
    }

    // Event 1: AssistantText.
    match &events[1] {
        AgentEvent::AssistantText { text } => {
            assert_eq!(text, "hello from fake opencode");
        }
        other => panic!("event 1: expected AssistantText, got {:?}", other),
    }

    // Event 2: ToolCall.
    match &events[2] {
        AgentEvent::ToolCall {
            id: Some(cid),
            name,
            input,
        } => {
            assert_eq!(cid, "call-1");
            assert_eq!(name, "bash");
            assert_eq!(input.get("command").and_then(|v| v.as_str()), Some("echo test"));
        }
        other => panic!("event 2: expected ToolCall, got {:?}", other),
    }

    // Event 3: ToolResult.
    match &events[3] {
        AgentEvent::ToolResult {
            id: Some(rid),
            content,
        } => {
            assert_eq!(rid, "call-1");
            assert_eq!(content.as_str(), Some("test"));
        }
        other => panic!("event 3: expected ToolResult, got {:?}", other),
    }

    // Event 4: Usage.
    match &events[4] {
        AgentEvent::Usage {
            input_tokens: Some(in_tok),
            output_tokens: Some(out_tok),
        } => {
            assert_eq!(*in_tok, 42);
            assert_eq!(*out_tok, 13);
        }
        other => panic!("event 4: expected Usage, got {:?}", other),
    }

    // Event 5: Terminal Result (emitted at EOF).
    match &events[5] {
        AgentEvent::Result {
            success: true,
            error: None,
        } => {
            // Expected: successful completion at stream end.
        }
        other => panic!("event 5: expected Result{{success:true}}, got {:?}", other),
    }

    // Verify the bridge terminated cleanly (no more events and no error).
    match bridge.next_event() {
        Ok(None) => {
            // Expected: stream closed.
        }
        other => panic!("expected stream end, got {:?}", other),
    }
}
