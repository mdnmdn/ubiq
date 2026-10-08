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
//! `SessionStarted` carrying the fake `thread.id`, model and mode, (b) an
//! `AgentMessageChunk` from the v2 `item/completed` notification, (c) a
//! `UsageUpdate` from `thread/tokenUsage/updated`, (d) exactly one terminal
//! `TurnEnded{stop_reason: EndTurn}` from `turn/completed`, and (e) that the
//! whole thing TERMINATES — the fake script exits right after emitting those
//! notifications, closing the pipe, which must close the event channel rather
//! than hang `next_event`.

use std::path::PathBuf;

use agent_manager::harness::Launch;
use agent_manager::io::codex::ThreadOpen;
use agent_manager::io::{
    AgentEvent, AgentInput, CodexBridge, ConfigSetting, IoBridge, PermissionOutcome, StopReason,
    spawn_piped,
};

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

/// The same fixture, told to outlive its turns, to hold each turn open until it is interrupted
/// (`hold`), and to log every stdin line it reads — how a test sees what the bridge wrote.
fn staying_launch(stdin_log: &std::path::Path, hold: bool) -> Launch {
    let mut env = vec![
        ("AM_FAKE_CODEX_STAY".to_string(), "1".to_string()),
        (
            "AM_FAKE_STDIN".to_string(),
            stdin_log.to_string_lossy().to_string(),
        ),
    ];
    if hold {
        env.push(("AM_FAKE_CODEX_HOLD".to_string(), "1".to_string()));
    }
    Launch {
        program: fake_appserver_path().to_string_lossy().to_string(),
        args: Vec::new(),
        env,
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
                model: Some(model),
                mode: Some(mode),
                ..
            } if id == "t-1" && model == "gpt-fake" && mode == "workspace-write"
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
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, AgentEvent::TurnEnded { .. }))
            .collect::<Vec<_>>(),
        vec![&AgentEvent::TurnEnded {
            stop_reason: StopReason::EndTurn,
            error: None,
        }],
        "exactly one TurnEnded{{EndTurn}} — the idle status is not a second end: {events:?}"
    );
    assert!(
        events.iter().any(|e| matches!(
            e,
            AgentEvent::UsageUpdate { used: 7, size: 1000, model: Some(model), spend: Some(_), .. }
                if model == "gpt-fake"
        )),
        "thread/tokenUsage/updated states a window, so it is a usage report: {events:?}"
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
    let child =
        spawn_piped(&staying_launch(&stdin_log, true), &cwd).expect("spawn fake app-server");
    let mut bridge = CodexBridge::new(child, &cwd).expect("handshake + build bridge");

    bridge
        .send(AgentInput::prompt("say hi"))
        .expect("send prompt (turn/start)");

    bridge.send(AgentInput::Cancel).expect("cancel");
    let cancelled = drain_to_turn_end(&mut bridge);
    assert!(
        cancelled.iter().any(|e| matches!(
            e,
            AgentEvent::TurnEnded {
                stop_reason: StopReason::Cancelled,
                ..
            }
        )),
        "the interrupt must end the turn as cancelled, got: {cancelled:?}"
    );

    // Still there: a second prompt reaches the same app-server, which it could not if the cancel
    // had closed stdin — and, the turn being over, it opens a turn rather than steering one.
    bridge
        .send(AgentInput::prompt("again"))
        .expect("the thread must still take a turn after a cancel");

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

/// A prompt into a live turn steers it — `turn/steer` naming that turn — rather than opening a
/// second one.
#[test]
fn a_prompt_during_a_turn_steers_it() {
    let cwd = std::env::current_dir().unwrap();
    let seen = tempfile::TempDir::new().unwrap();
    let stdin_log = seen.path().join("stdin.ndjson");
    let child =
        spawn_piped(&staying_launch(&stdin_log, true), &cwd).expect("spawn fake app-server");
    let mut bridge = CodexBridge::new(child, &cwd).expect("handshake + build bridge");

    bridge
        .send(AgentInput::prompt("start"))
        .expect("turn/start");
    bridge
        .send(AgentInput::prompt("use the helper"))
        .expect("turn/steer");

    let written = std::fs::read_to_string(&stdin_log).unwrap();
    let steer = written
        .lines()
        .find(|line| line.contains(r#""method":"turn/steer""#))
        .unwrap_or_else(|| panic!("no turn/steer: {written}"));
    assert!(steer.contains(r#""expectedTurnId":"turn-1""#), "{steer}");
    assert!(steer.contains("use the helper"), "{steer}");
    assert_eq!(
        written
            .lines()
            .filter(|line| line.contains(r#""method":"turn/start""#))
            .count(),
        1,
        "{written}"
    );
}

/// A steered prompt is a prompt the host counts, and Codex ends the merged turn once — so the turn's
/// end is reported once for it and once more for the merged prompt.
#[test]
fn a_steered_prompt_is_owed_its_own_turn_end() {
    let cwd = std::env::current_dir().unwrap();
    let seen = tempfile::TempDir::new().unwrap();
    let stdin_log = seen.path().join("stdin.ndjson");
    let child =
        spawn_piped(&staying_launch(&stdin_log, true), &cwd).expect("spawn fake app-server");
    let mut bridge = CodexBridge::new(child, &cwd).expect("handshake + build bridge");

    bridge.send(AgentInput::prompt("start")).unwrap();
    bridge.send(AgentInput::prompt("and this")).unwrap();
    bridge.send(AgentInput::Cancel).unwrap();

    drain_to_turn_end(&mut bridge);
    let second = bridge.next_event().expect("next_event");
    assert!(
        matches!(second, Some(AgentEvent::TurnEnded { .. })),
        "{second:?}"
    );
}

/// An approval is parked, surfaced on the tool call it authorises, and answered by the
/// caller's pick — not auto-accepted.
#[test]
fn an_approval_waits_for_the_answer() {
    let cwd = std::env::current_dir().unwrap();
    let seen = tempfile::TempDir::new().unwrap();
    let stdin_log = seen.path().join("stdin.ndjson");
    let child =
        spawn_piped(&staying_launch(&stdin_log, false), &cwd).expect("spawn fake app-server");
    let mut bridge = CodexBridge::new(child, &cwd).expect("handshake + build bridge");

    bridge
        .send(AgentInput::prompt("please ask first"))
        .expect("turn/start");
    let (request_id, call_id) = loop {
        match bridge
            .next_event()
            .unwrap()
            .expect("stream ended before the ask")
        {
            AgentEvent::PermissionRequest {
                request_id,
                tool_call,
                ..
            } => break (request_id, tool_call.id),
            _ => continue,
        }
    };
    assert_eq!(request_id, "srv-1");
    assert_eq!(call_id, "c-1", "drawn on the command it authorises");
    let before = std::fs::read_to_string(&stdin_log).unwrap();
    assert!(
        !before.contains(r#""id":"srv-1""#),
        "nothing answered it yet: {before}"
    );

    bridge
        .send(AgentInput::AnswerPermission {
            request_id,
            outcome: PermissionOutcome::Selected {
                option_id: "decline".to_string(),
            },
            updated_input: None,
        })
        .expect("answer");
    let _ = drain_to_turn_end(&mut bridge);

    let written = std::fs::read_to_string(&stdin_log).unwrap();
    let answer = written
        .lines()
        .find(|line| line.contains(r#""id":"srv-1""#))
        .unwrap_or_else(|| panic!("no answer written: {written}"));
    assert!(answer.contains(r#""decision":"decline""#), "{answer}");
}

/// A resume opens the thread with `thread/resume`, and the session it reports is that thread.
#[test]
fn a_resume_opens_the_thread_with_thread_resume() {
    let cwd = std::env::current_dir().unwrap();
    let seen = tempfile::TempDir::new().unwrap();
    let stdin_log = seen.path().join("stdin.ndjson");
    let child =
        spawn_piped(&staying_launch(&stdin_log, true), &cwd).expect("spawn fake app-server");
    let mut bridge =
        CodexBridge::open(child, &cwd, ThreadOpen::Resume("t-old".to_string())).expect("handshake");

    let first = bridge.next_event().unwrap().unwrap();
    assert!(
        matches!(&first, AgentEvent::SessionStarted { session_id: Some(id), mode: Some(mode), .. }
            if id == "t-resumed" && mode == "read-only"),
        "{first:?}"
    );
    let written = std::fs::read_to_string(&stdin_log).unwrap();
    assert!(
        written
            .lines()
            .any(|l| l.contains(r#""method":"thread/resume""#)
                && l.contains(r#""threadId":"t-old""#)),
        "{written}"
    );
    assert!(!written.contains(r#""method":"thread/start""#));
}

/// The model and effort pickers arrive after the session, and a pick rides on the next turn.
#[test]
fn a_picked_effort_rides_on_the_next_turn() {
    let cwd = std::env::current_dir().unwrap();
    let seen = tempfile::TempDir::new().unwrap();
    let stdin_log = seen.path().join("stdin.ndjson");
    let child =
        spawn_piped(&staying_launch(&stdin_log, true), &cwd).expect("spawn fake app-server");
    let mut bridge = CodexBridge::new(child, &cwd).expect("handshake");

    let _session = bridge.next_event().unwrap().unwrap();
    let options = bridge.next_event().unwrap().unwrap();
    let AgentEvent::ConfigOptionUpdate { options } = options else {
        panic!("expected the pickers, got {options:?}");
    };
    let ids: Vec<_> = options.iter().map(|o| o.id.as_str()).collect();
    assert_eq!(ids, ["model", "reasoning_effort"]);

    bridge
        .send(AgentInput::SetConfigOption {
            config_id: "reasoning_effort".to_string(),
            value: ConfigSetting::Text("high".to_string()),
        })
        .expect("pick");
    assert!(matches!(
        bridge.next_event().unwrap().unwrap(),
        AgentEvent::ConfigOptionUpdate { .. }
    ));
    bridge.send(AgentInput::prompt("go")).expect("turn/start");

    let written = std::fs::read_to_string(&stdin_log).unwrap();
    let start = written
        .lines()
        .find(|l| l.contains(r#""method":"turn/start""#))
        .unwrap();
    assert!(start.contains(r#""effort":"high""#), "{start}");
}
