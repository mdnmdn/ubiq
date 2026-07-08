//! Integration tests for the PTY passthrough runner (`agent_manager::run`),
//! driven against a committed fake harness (`tests/fake-harness.sh`) so no
//! real agent binary or network access is needed. Only compiled with the
//! `pty` feature, which the default build pulls in via `cli`.
//!
//! Env-hygiene note: `env_remove` (stripping a var inherited from the
//! parent process) is exercised at the unit level in `src/run.rs` via
//! `CommandBuilder::get_env`, not here — reproducing "a var already present
//! in the parent env" in an integration test would require
//! `std::env::set_var`, which is `unsafe` in this Rust edition, and this
//! crate forbids unsafe code. This file exercises what's achievable without
//! it: exit-code propagation, env *injection* (`launch.env`), and ephemeral
//! config-dir cleanup.
#![cfg(feature = "pty")]

use std::path::PathBuf;

use agent_manager::harness::Launch;
use agent_manager::provision::Provisioned;
use agent_manager::run::{run, spawn_in_pty};
use portable_pty::{Child, ExitStatus, MasterPty};

/// Absolute path to the fake harness script next to this test file.
fn fake_harness_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fake-harness.sh")
}

/// Build a `Provisioned` for these tests, filling in the `inproc-mcp`
/// feature's extra field (an empty server list) when that feature is on.
fn provisioned(dir: PathBuf, launch: Launch, ephemeral: bool) -> Provisioned {
    Provisioned {
        dir,
        launch,
        ephemeral,
        #[cfg(feature = "inproc-mcp")]
        inproc_servers: Vec::new(),
    }
}

/// Wait for `child`, draining the PTY master on a background thread.
///
/// A PTY consumer MUST read the master, or the child can block writing to the
/// slave once the kernel's (small, on macOS) tty buffer fills — which would
/// hang `child.wait()`. The real runner does this via `io::pump`; these
/// low-level `spawn_in_pty` tests replicate just the draining half.
fn wait_child(
    mut child: Box<dyn Child + Send + Sync>,
    master: Box<dyn MasterPty + Send>,
) -> ExitStatus {
    let mut reader = master.try_clone_reader().expect("clone reader");
    let drain = std::thread::spawn(move || {
        let mut sink = Vec::new();
        let _ = std::io::copy(&mut reader, &mut sink);
    });
    let status = child.wait().expect("wait");
    drop(master); // close the master so the reader hits EOF and the thread ends
    let _ = drain.join();
    status
}

fn base_launch() -> Launch {
    Launch {
        program: fake_harness_path().to_string_lossy().to_string(),
        args: vec!["--hello".to_string()],
        env: Vec::new(),
        env_remove: Vec::new(),
    }
}

#[test]
fn exit_code_propagates_from_the_child() {
    let mut launch = base_launch();
    launch.env.push(("FAKE_EXIT".to_string(), "7".to_string()));

    let cwd = std::env::current_dir().unwrap();
    let (child, master) = spawn_in_pty(&launch, &cwd, (24, 80)).expect("spawn");

    let status = wait_child(child, master);
    assert_eq!(status.exit_code(), 7);
}

#[test]
fn exit_code_zero_by_default() {
    let launch = base_launch();

    let cwd = std::env::current_dir().unwrap();
    let (child, master) = spawn_in_pty(&launch, &cwd, (24, 80)).expect("spawn");

    let status = wait_child(child, master);
    assert_eq!(status.exit_code(), 0);
}

#[test]
fn env_injection_reaches_the_child() {
    let out_file = tempfile::NamedTempFile::new().expect("tempfile");
    let out_path = out_file.path().to_path_buf();

    let mut launch = base_launch();
    launch
        .env
        .push(("CLAUDE_CONFIG_DIR".to_string(), "/tmp/x".to_string()));
    launch.env.push((
        "FAKE_OUT".to_string(),
        out_path.to_string_lossy().to_string(),
    ));

    let cwd = std::env::current_dir().unwrap();
    let (child, master) = spawn_in_pty(&launch, &cwd, (24, 80)).expect("spawn");
    let status = wait_child(child, master);
    assert_eq!(status.exit_code(), 0);

    let contents = std::fs::read_to_string(&out_path).expect("read FAKE_OUT");
    assert!(
        contents.contains("CLAUDE_CONFIG_DIR=/tmp/x"),
        "expected injected CLAUDE_CONFIG_DIR in child env, got:\n{contents}"
    );
}

#[test]
fn ephemeral_dir_is_removed_after_a_run() {
    let dir = tempfile::tempdir().expect("tempdir").keep();
    assert!(dir.exists());

    let provisioned = provisioned(dir.clone(), base_launch(), true);

    let cwd = std::env::current_dir().unwrap();
    let code = run(&provisioned, &cwd, false).expect("run");
    assert_eq!(code, 0);
    assert!(!dir.exists(), "ephemeral dir should have been removed");
}

#[test]
fn keep_config_preserves_the_ephemeral_dir() {
    let dir = tempfile::tempdir().expect("tempdir").keep();
    assert!(dir.exists());

    let provisioned = provisioned(dir.clone(), base_launch(), true);

    let cwd = std::env::current_dir().unwrap();
    let code = run(&provisioned, &cwd, true).expect("run");
    assert_eq!(code, 0);
    assert!(dir.exists(), "keep_config should have preserved the dir");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn pinned_dir_is_never_removed() {
    let dir = tempfile::tempdir().expect("tempdir").keep();
    assert!(dir.exists());

    let provisioned = provisioned(dir.clone(), base_launch(), false);

    let cwd = std::env::current_dir().unwrap();
    let code = run(&provisioned, &cwd, false).expect("run");
    assert_eq!(code, 0);
    assert!(dir.exists(), "a pinned (non-ephemeral) dir must survive");

    let _ = std::fs::remove_dir_all(&dir);
}
