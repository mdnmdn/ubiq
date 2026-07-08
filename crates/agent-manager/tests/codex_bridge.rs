//! Integration test for `agent_manager::io::CodexBridge` against a
//! committed fake `codex app-server` (`tests/fake-codex-appserver.sh`) — no
//! real `codex` binary or network access needed.
//!
//! `io::codex` is core (no feature gate — see `src/io/codex.rs`), so this
//! runs under the default build same as `tests/jsonl_bridge.rs`.
//!
//! Exercises the full round trip: the JSON-RPC handshake
//! (`initialize` → `initialized` → `thread/start`), a `send(Prompt)`
//! (`turn/start`), and draining `next_event()` to confirm (a) a
//! `SessionStarted` carrying the fake `thread.id`, (b) an `AssistantText`
//! from the v2 `item/completed` notification, (c) a terminal
//! `Result{success:true}` from `turn/completed`, and (d) that the whole
//! thing TERMINATES — the fake script exits right after emitting those
//! notifications, closing the pipe, which must close the event channel
//! rather than hang `next_event`.

use std::path::PathBuf;

use agent_manager::harness::Launch;
use agent_manager::io::{spawn_piped, AgentEvent, AgentInput, CodexBridge, IoBridge};

/// Absolute path to the fake app-server script next to this test file.
fn fake_appserver_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fake-codex-appserver.sh")
}

fn launch() -> Launch {
    Launch {
        program: fake_appserver_path().to_string_lossy().to_string(),
        args: Vec::new(),
        env: Vec::new(),
        env_remove: Vec::new(),
    }
}

#[test]
fn codex_bridge_round_trips_events_and_terminates() {
    let cwd = std::env::current_dir().unwrap();
    let child = spawn_piped(&launch(), &cwd).expect("spawn fake app-server");
    let mut bridge = CodexBridge::new(child, &cwd).expect("handshake + build bridge");

    bridge
        .send(AgentInput::Prompt {
            text: "say hi".to_string(),
        })
        .expect("send prompt (turn/start)");

    // Drain every event; the fake script exits right after emitting the
    // `turn/completed` notification, which closes the pipe and — via the
    // reader thread hitting stdout EOF — closes the event channel, ending
    // this loop. If the bridge failed to correlate responses/timeouts
    // correctly, either the handshake above or this loop would hang instead
    // of returning.
    let mut events = Vec::new();
    while let Some(ev) = bridge.next_event().expect("next_event") {
        events.push(ev);
    }

    assert!(
        events.iter().any(|e| matches!(
            e,
            AgentEvent::SessionStarted {
                session_id: Some(id)
            } if id == "t-1"
        )),
        "expected a SessionStarted event with the fake thread id, got: {events:?}"
    );
    assert!(
        events.iter().any(|e| matches!(
            e,
            AgentEvent::AssistantText { text } if text == "hello from fake codex"
        )),
        "expected an AssistantText event, got: {events:?}"
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
                input_tokens: Some(3),
                output_tokens: Some(4),
            }
        )),
        "expected a Usage event from turn/completed, got: {events:?}"
    );
}
