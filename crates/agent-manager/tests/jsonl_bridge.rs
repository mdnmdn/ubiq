//! Integration test for `agent_manager::io::JsonlBridge` against a
//! committed fake stream-json harness (`tests/fake-claude-streamjson.sh`) —
//! no real `claude` binary or network access needed.
//!
//! `io::jsonl` is core (no feature gate — see `src/io/jsonl.rs`), so this
//! runs under the default build same as `tests/passthrough.rs`'s `pty`
//! sibling; unlike that one, it needs no `#![cfg(feature = ...)]` guard.
//!
//! Exercises the full round trip: send a prompt, drain events, and confirm
//! (a) the fake harness's `can_use_tool` `control_request` surfaces as a
//! `PermissionRequest` and is answered by *the caller* and nobody else —
//! the fixture records the answer it received in `$AM_FAKE_ANSWER`, so an
//! auto-answer would show up as a file that exists before the test wrote
//! one — (b) `AgentInput::Cancel` denies a still-pending ask before it
//! interrupts the turn, and (c) the run terminates (the event channel closes,
//! `next_event` returns `None`) rather than hanging.
//!
//! A second fixture (`tests/fake-claude-interrupt.sh`) covers what the first cannot: it never
//! stops reading stdin, so a test can see that a cancel writes an `interrupt` `control_request`
//! and leaves the session open for the next turn, and that closing stdin is
//! `AgentInput::Shutdown`'s separate job.

use std::path::{Path, PathBuf};
use std::time::Duration;

use agent_manager::harness::Launch;
use agent_manager::io::{
    AgentEvent, AgentInput, IoBridge, JsonlBridge, PermissionKind, PermissionOutcome, StopReason,
    ToolKind, ToolStatus, spawn_piped,
};

/// Absolute path to the fake stream-json harness script next to this test file.
fn fake_harness_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fake-claude-streamjson.sh")
}

/// The fixture writes the `control_response` it was answered with to `answer_file` — which is how
/// these tests tell "answered by the caller" from "answered by the bridge".
fn launch(answer_file: &Path) -> Launch {
    Launch {
        program: fake_harness_path().to_string_lossy().to_string(),
        args: Vec::new(),
        env: vec![(
            "AM_FAKE_ANSWER".to_string(),
            answer_file.to_string_lossy().to_string(),
        )],
        env_remove: Vec::new(),
        env_clear: false,
    }
}

/// The cancellation fixture next door: it appends every stdin line it reads to `$AM_FAKE_STDIN`
/// and loops until EOF, which is how these tests see what a cancel wrote and that the session
/// outlived it.
fn interrupt_launch(stdin_log: &Path) -> Launch {
    Launch {
        program: PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fake-claude-interrupt.sh")
            .to_string_lossy()
            .to_string(),
        args: Vec::new(),
        env: vec![(
            "AM_FAKE_STDIN".to_string(),
            stdin_log.to_string_lossy().to_string(),
        )],
        env_remove: Vec::new(),
        env_clear: false,
    }
}

/// Pump events up to and including the next `TurnEnded`, returning everything seen on the way.
/// Each of the cancellation fixture's turns ends with a `result`, so this is one turn's worth.
fn drain_to_turn_end(bridge: &mut JsonlBridge) -> Vec<AgentEvent> {
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

/// Pump events until the harness asks for permission, returning what it asked *and* everything
/// seen on the way there.
fn drain_to_permission_ask(bridge: &mut JsonlBridge) -> (Vec<AgentEvent>, String) {
    let mut events = Vec::new();
    while let Some(ev) = bridge.next_event().expect("next_event") {
        let request_id = match &ev {
            AgentEvent::PermissionRequest { request_id, .. } => Some(request_id.clone()),
            _ => None,
        };
        events.push(ev);
        if let Some(request_id) = request_id {
            return (events, request_id);
        }
    }
    panic!("stream ended before a PermissionRequest: {events:?}");
}

#[test]
fn jsonl_bridge_round_trips_events_and_terminates() {
    let cwd = std::env::current_dir().unwrap();
    let answers = tempfile::TempDir::new().unwrap();
    let answer_file = answers.path().join("answer.json");
    let child = spawn_piped(&launch(&answer_file), &cwd).expect("spawn fake harness");
    let mut bridge = JsonlBridge::new(child).expect("build bridge");

    bridge
        .send(AgentInput::prompt("say hi"))
        .expect("send prompt");

    // The harness stops at its ask and waits, because nothing but this test can answer it.
    let (mut events, request_id) = drain_to_permission_ask(&mut bridge);
    assert_eq!(request_id, "req-1");
    std::thread::sleep(Duration::from_millis(300));
    assert!(
        !answer_file.exists(),
        "the bridge answered the ask by itself: {:?}",
        std::fs::read_to_string(&answer_file)
    );

    bridge
        .send(AgentInput::AnswerPermission {
            request_id,
            outcome: PermissionOutcome::Selected {
                option_id: "allow".to_string(),
            },
            updated_input: None,
        })
        .expect("answer the permission request");

    // Drain the rest; the fake script exits after the terminal `result` line, which closes the
    // channel and ends this loop.
    while let Some(ev) = bridge.next_event().expect("next_event") {
        events.push(ev);
    }

    let answer = std::fs::read_to_string(&answer_file).expect("the harness was answered");
    assert!(
        answer.contains(r#""behavior":"allow""#) && answer.contains(r#""request_id":"req-1""#),
        "unexpected control_response: {answer}"
    );

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
    // Allow, the "always" the request's own `permission_suggestions` offers, then deny.
    let ids: Vec<&str> = options.iter().map(|o| o.option_id.as_str()).collect();
    assert_eq!(ids, ["allow", "allow_always:0", "deny"]);
    assert_eq!(options[1].kind, PermissionKind::AllowAlways);

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

    // `result.modelUsage` reports camelCase fields (`inputTokens`, `cacheReadInputTokens`,
    // `contextWindow`, ...): `size` is the window it names, and what it bills is `spend`.
    //
    // The expected `used` changed from 5 to 0 with the occupancy/spend split: `modelUsage` is
    // session billing, not what sits in the window, so a `result` reports spend and leaves the
    // ring where the last assistant message put it — and this fake stream's assistant line carries
    // no `usage` at all, so there is no occupancy to carry. `size` is 0 with it: a level nothing
    // has stated draws no ring, rather than an empty one against a window it never filled.
    assert!(
        events.iter().any(|e| matches!(
            e,
            AgentEvent::UsageUpdate {
                used: 0,
                size: 0,
                model: Some(model),
                spend: Some(spend),
                ..
            } if model == "fake-model" && spend.input == 5 && spend.output == 7
        )),
        "expected a UsageUpdate event carrying modelUsage's spend, got: {events:?}"
    );
}

/// Cancelling a turn with an ask outstanding denies it before it interrupts: the harness would
/// otherwise fail the tool with "Tool permission stream closed before response received"
/// (`_docs/io-modes.md` §"Permissions and cancellation").
#[test]
fn cancel_denies_a_pending_permission_request_before_interrupting() {
    let cwd = std::env::current_dir().unwrap();
    let answers = tempfile::TempDir::new().unwrap();
    let answer_file = answers.path().join("answer.json");
    let child = spawn_piped(&launch(&answer_file), &cwd).expect("spawn fake harness");
    let mut bridge = JsonlBridge::new(child).expect("build bridge");

    bridge
        .send(AgentInput::prompt("say hi"))
        .expect("send prompt");
    let (_events, request_id) = drain_to_permission_ask(&mut bridge);
    assert_eq!(request_id, "req-1");

    // The result is deliberately not asserted: the deny goes out first and is ignored either
    // way, and the interrupt line behind it lands on a fixture that answers its one ask and
    // exits — so a broken pipe here is the harness already being gone, not a cancel that failed.
    // What the cancel *wrote* is asserted below; `cancel_interrupts_the_turn_and_keeps_the_session`
    // is where the interrupt line itself is.
    let _ = bridge.send(AgentInput::Cancel);

    // Drain to end-of-stream, which is also how we know the harness got past its blocking read.
    while bridge.next_event().expect("next_event").is_some() {}

    let answer = std::fs::read_to_string(&answer_file)
        .expect("cancel must answer a pending ask, not just close stdin");
    assert!(
        answer.contains(r#""behavior":"deny""#),
        "a cancelled ask must be denied, got: {answer}"
    );
}

/// A cancel interrupts the turn and stops there: it writes Claude Code's own `interrupt`
/// `control_request` and leaves stdin open, so the conversation takes the next prompt on the same
/// process. Closing stdin is `AgentInput::Shutdown`'s job — a cancel that closed it would end the
/// harness, which is not what a stop button means
/// (`_docs/harness/claude-code.md` §"Process lifecycle").
#[test]
fn cancel_interrupts_the_turn_and_keeps_the_session() {
    let cwd = std::env::current_dir().unwrap();
    let seen = tempfile::TempDir::new().unwrap();
    let stdin_log = seen.path().join("stdin.ndjson");
    let child = spawn_piped(&interrupt_launch(&stdin_log), &cwd).expect("spawn fake harness");
    let mut bridge = JsonlBridge::new(child).expect("build bridge");

    bridge
        .send(AgentInput::prompt("first"))
        .expect("send the first prompt");
    let first = drain_to_turn_end(&mut bridge);
    assert!(
        first.iter().any(|e| matches!(
            e,
            AgentEvent::AgentMessageChunk { content, .. } if content.as_text() == Some("turn 1")
        )),
        "expected the first turn's reply, got: {first:?}"
    );

    bridge.send(AgentInput::Cancel).expect("cancel");
    let cancelled = drain_to_turn_end(&mut bridge);
    assert!(
        cancelled
            .iter()
            .any(|e| matches!(e, AgentEvent::TurnEnded { .. })),
        "the interrupt must end the turn, got: {cancelled:?}"
    );

    // The session survived the cancel: a second prompt reaches the *same* process, which it could
    // not if the cancel had closed stdin.
    bridge
        .send(AgentInput::prompt("second"))
        .expect("the session must still take a prompt after a cancel");
    let second = drain_to_turn_end(&mut bridge);
    assert!(
        second.iter().any(|e| matches!(
            e,
            AgentEvent::AgentMessageChunk { content, .. } if content.as_text() == Some("turn 2")
        )),
        "expected a second turn on the same process, got: {second:?}"
    );

    let written = std::fs::read_to_string(&stdin_log).expect("the harness read our stdin");
    let interrupt = written
        .lines()
        .find(|line| line.contains(r#""subtype":"interrupt""#))
        .unwrap_or_else(|| panic!("cancel wrote no interrupt control_request: {written}"));
    assert!(
        interrupt.contains(r#""type":"control_request""#),
        "the interrupt must be a control_request: {interrupt}"
    );
    // `reason` is the value Claude forwards to the turn's `AbortSignal.reason`; `interrupt` is
    // the one that reads as "the human pressed stop" and suppresses a tool's error output.
    assert!(
        interrupt.contains(r#""reason":"interrupt""#),
        "the interrupt must name its reason: {interrupt}"
    );
}

/// A shutdown closes stdin, which is what makes the harness exit — the teardown a cancel no
/// longer does.
#[test]
fn shutdown_closes_stdin_and_ends_the_harness() {
    let cwd = std::env::current_dir().unwrap();
    let seen = tempfile::TempDir::new().unwrap();
    let stdin_log = seen.path().join("stdin.ndjson");
    let child = spawn_piped(&interrupt_launch(&stdin_log), &cwd).expect("spawn fake harness");
    let mut bridge = JsonlBridge::new(child).expect("build bridge");

    bridge
        .send(AgentInput::prompt("first"))
        .expect("send the first prompt");
    let _ = drain_to_turn_end(&mut bridge);

    bridge.send(AgentInput::Shutdown).expect("shutdown");

    // The fixture loops on stdin forever and only leaves on EOF, so reaching end-of-stream here
    // *is* the proof stdin was closed.
    while bridge.next_event().expect("next_event").is_some() {}
}
