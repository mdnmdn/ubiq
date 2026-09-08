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
//! `SessionStarted` carrying the fake `thread.id`, (b) an
//! `AgentMessageChunk` from the v2 `item/completed` notification, (c) a
//! terminal `TurnEnded{stop_reason: EndTurn}` from `turn/completed`, and (d)
//! that the whole thing TERMINATES — the fake script exits right after
//! emitting those notifications, closing the pipe, which must close the
//! event channel rather than hang `next_event`.
//!
//! The fake script's `turn/completed` also carries a `usage` block
//! (`input_tokens`/`output_tokens`, no context window) — per the mapping
//! rule "no window, no `UsageUpdate`", the drained events must NOT contain
//! one.

use std::path::PathBuf;

use agent_manager::harness::Launch;
use agent_manager::io::{AgentEvent, AgentInput, CodexBridge, IoBridge, StopReason, spawn_piped};

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
        env_clear: false,
    }
}

/// The same fixture, told to outlive its turns and to log every stdin line it reads — how the
/// cancellation test sees what the bridge wrote and that the session was still there afterwards.
fn staying_launch(stdin_log: &std::path::Path) -> Launch {
    Launch {
        program: fake_appserver_path().to_string_lossy().to_string(),
        args: Vec::new(),
        env: vec![
            ("AM_FAKE_CODEX_STAY".to_string(), "1".to_string()),
            (
                "AM_FAKE_STDIN".to_string(),
                stdin_log.to_string_lossy().to_string(),
            ),
        ],
        env_remove: Vec::new(),
        env_clear: false,
    }
}

/// Pump events up to and including the next `TurnEnded`, returning everything seen on the way.
fn drain_to_turn_end(bridge: &mut CodexBridge) -> Vec<AgentEvent> {
    let mut events = Vec::new();
    while let Some(ev) = bridge.next_event().expect("next_event") {
        let ended = matches!(ev, AgentEvent::TurnEnded { .. });
        events.push(ev);
        if ended {
            return events;
        }
    }
    panic!("stream ended before the turn did: {events:?}");
}

#[test]
fn codex_bridge_round_trips_events_and_terminates() {
    let cwd = std::env::current_dir().unwrap();
    let child = spawn_piped(&launch(), &cwd).expect("spawn fake app-server");
    let mut bridge = CodexBridge::new(child, &cwd).expect("handshake + build bridge");

    bridge
        .send(AgentInput::prompt("say hi"))
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
                session_id: Some(id),
                model: None,
                mode: None,
                ..
            } if id == "t-1"
        )),
        "expected a SessionStarted event with the fake thread id, got: {events:?}"
    );
    assert!(
        events.iter().any(|e| matches!(
            e,
            AgentEvent::AgentMessageChunk { content, .. }
                if content.as_text() == Some("hello from fake codex")
        )),
        "expected an AgentMessageChunk event, got: {events:?}"
    );
    assert!(
        events.iter().any(|e| matches!(
            e,
            AgentEvent::TurnEnded {
                stop_reason: StopReason::EndTurn,
                error: None,
            }
        )),
        "expected a terminal TurnEnded{{EndTurn}} event, got: {events:?}"
    );
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, AgentEvent::UsageUpdate { .. })),
        "the fake script's usage block carries no context window, so no \
         UsageUpdate should be emitted, got: {events:?}"
    );
}

/// A cancel is `turn/interrupt`, naming both the thread and the turn `turn/start` acked — not a
/// closed stdin. The thread survives it and takes the next `turn/start`
/// (`_docs/harness/codex.md` §"Process lifecycle").
#[test]
fn cancel_interrupts_the_turn_and_keeps_the_session() {
    let cwd = std::env::current_dir().unwrap();
    let seen = tempfile::TempDir::new().unwrap();
    let stdin_log = seen.path().join("stdin.ndjson");
    let child = spawn_piped(&staying_launch(&stdin_log), &cwd).expect("spawn fake app-server");
    let mut bridge = CodexBridge::new(child, &cwd).expect("handshake + build bridge");

    bridge
        .send(AgentInput::prompt("say hi"))
        .expect("send prompt (turn/start)");
    let _ = drain_to_turn_end(&mut bridge);

    bridge.send(AgentInput::Cancel).expect("cancel");
    let cancelled = drain_to_turn_end(&mut bridge);
    assert!(
        cancelled
            .iter()
            .any(|e| matches!(e, AgentEvent::TurnEnded { .. })),
        "the interrupt must end the turn, got: {cancelled:?}"
    );

    // Still there: a second prompt reaches the same app-server, which it could not if the cancel
    // had closed stdin.
    bridge
        .send(AgentInput::prompt("again"))
        .expect("the thread must still take a turn after a cancel");
    let _ = drain_to_turn_end(&mut bridge);

    let written = std::fs::read_to_string(&stdin_log).expect("the app-server read our stdin");
    let interrupt = written
        .lines()
        .find(|line| line.contains(r#""method":"turn/interrupt""#))
        .unwrap_or_else(|| panic!("cancel sent no turn/interrupt: {written}"));
    assert!(
        interrupt.contains(r#""threadId":"t-1""#) && interrupt.contains(r#""turnId":"turn-1""#),
        "turn/interrupt must name both ids: {interrupt}"
    );
    assert_eq!(
        written
            .lines()
            .filter(|line| line.contains(r#""method":"turn/start""#))
            .count(),
        2,
        "both turns must have reached the same process: {written}"
    );
}
