//! A structured run can be confined (G92): the fake stream-json harness from
//! `tests/jsonl_bridge.rs`, spawned through the `Launch` that
//! `isolate::confined_launch` renders, still round-trips a prompt over pipes.
//!
//! macOS only — `confined_launch` renders `sandbox-exec -p <policy> …`, which
//! `execve`s in place, so the pipes `spawn_piped` opens land on the harness
//! itself. No other platform has a rendered policy to hand a caller (see that
//! function's docs), and there this test would only assert the error.
#![cfg(target_os = "macos")]

use std::path::PathBuf;

use agent_manager::harness::Launch;
use agent_manager::io::{AgentEvent, AgentInput, IoBridge, JsonlBridge, spawn_piped};
use agent_manager::isolate::{self, IsolateOptions};
use agent_manager::spec::{Isolation, RunSpec};

fn fake_harness_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fake-claude-streamjson.sh")
}

#[test]
fn a_confined_structured_run_round_trips_over_pipes() {
    // A sandbox cannot nest, and `confined_launch` refuses rather than
    // pretending. Under a test runner that is itself confined there is
    // nothing to assert, so say so and stop.
    if isolate::ensure_can_confine().is_err() {
        eprintln!("skipped: this test process is already sandboxed, and a sandbox cannot nest");
        return;
    }

    let state = tempfile::TempDir::new().unwrap();
    let dir = tempfile::TempDir::new().unwrap();
    let cwd = tempfile::TempDir::new().unwrap();

    let launch = Launch {
        program: fake_harness_path().to_string_lossy().to_string(),
        args: Vec::new(),
        env: Vec::new(),
        env_remove: Vec::new(),
        env_clear: false,
    };

    let mut spec = RunSpec::new("claude-code".to_string(), cwd.path().to_path_buf());
    spec.isolation = Isolation::Sandboxed(String::new());

    let confined = isolate::plan(
        &launch,
        &spec,
        dir.path(),
        &IsolateOptions::new(state.path()),
    )
    .expect("planning the policy")
    .expect("a sandboxed spec plans a policy");

    let confined_launch = isolate::confined_launch(&confined).expect("rendering the policy");
    assert_eq!(confined_launch.program, "/usr/bin/sandbox-exec");
    assert_eq!(confined_launch.args[0], "-p");
    assert_eq!(
        confined_launch.args.last().map(String::as_str),
        Some(launch.program.as_str()),
        "the harness's own argv must still be what runs under the policy"
    );

    let child = spawn_piped(&confined_launch, cwd.path()).expect("spawn the confined harness");
    let mut bridge = JsonlBridge::new(child).expect("build bridge");

    bridge
        .send(AgentInput::prompt("say hi"))
        .expect("send prompt");

    let mut events = Vec::new();
    while let Some(ev) = bridge.next_event().expect("next_event") {
        events.push(ev);
    }

    assert!(
        events.iter().any(|e| matches!(
            e,
            AgentEvent::AgentMessageChunk { content, .. }
                if content.as_text() == Some("hello from fake claude")
        )),
        "a confined harness still answers over its pipes, got: {events:?}"
    );
}
